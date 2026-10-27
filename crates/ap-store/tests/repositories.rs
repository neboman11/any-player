mod common;

use ap_model::{Playlist, PlaylistType, QueueState, SourceType};
use ap_store::{
    StoreError, catalog, dj_state, in_tx, playlists::*, queue, settings, sync_state, track_gain,
};
use common::{TestEnv, track};
use serde_json::json;

fn playlist(id: &str, source: SourceType) -> Playlist {
    Playlist {
        id: id.into(),
        name: id.into(),
        owner: "owner".into(),
        description: None,
        image_url: None,
        track_count: 7,
        source,
        tracks: None,
    }
}

fn sample_queue() -> QueueState {
    let legacy: ap_store::legacy::LegacyPlaybackState = serde_json::from_value(json!({
        "queue": [track("a", SourceType::Spotify), track("b", SourceType::Plex)],
        "shuffle": true, "shuffle_order": [1, 0], "current_index": 0
    }))
    .unwrap();
    legacy.to_queue().unwrap()
}

#[tokio::test]
async fn standard_crud_duplicates_positions_timestamps_and_cascades() {
    let env = TestEnv::new();
    let db = env.db().await;
    db.call(|c| {
        let p = create_playlist(c, &NewPlaylist::new("standard", PlaylistType::Standard))?;
        assert!((ap_store::now_secs() - p.created_at).abs() <= 1);
        let rows = add_tracks(
            c,
            &p.id,
            &[
                track("x", SourceType::Spotify),
                track("x", SourceType::Spotify),
                track("z", SourceType::Plex),
            ],
        )?;
        assert_ne!(rows[0].id, rows[1].id);
        assert_eq!(get_playlist(c, &p.id)?.unwrap().track_count, 3);
        assert_eq!(move_track(c, &p.id, rows[0].id, i64::MAX)?, Some(2));
        assert_eq!(move_track(c, &p.id, rows[2].id, i64::MIN)?, Some(0));
        assert!(!remove_track(c, "other", rows[0].id)?);
        assert!(remove_track(c, &p.id, rows[1].id)?);
        let left = list_tracks(c, &p.id)?;
        assert_eq!(
            left.iter()
                .map(|t| (&*t.track_id, t.position))
                .collect::<Vec<_>>(),
            [("z", 0), ("x", 1)]
        );
        c.execute(
            "UPDATE playlist_tracks SET position = 9 WHERE id = ?1",
            [rows[0].id],
        )?;
        assert_eq!(
            add_tracks(c, &p.id, &[track("new", SourceType::Jellyfin)])?[0].position,
            10
        );
        assert_eq!(get_playlist(c, &p.id)?.unwrap().track_count, 3);
        let changed = update_playlist(
            c,
            &p.id,
            &PlaylistPatch {
                name: Some("renamed".into()),
                description: Some(None),
                image_url: Some(Some("http://h/a?X-Plex-Token=secret".into())),
            },
        )?
        .unwrap();
        assert_eq!(changed.name, "renamed");
        assert_eq!(changed.image_url.as_deref(), Some("http://h/a"));
        assert!(set_playlist_distinct(c, &p.id, true)?);
        let mut imported = get_playlist(c, &p.id)?.unwrap();
        imported.name = "import".into();
        upsert_playlist(c, &imported)?;
        assert_eq!(list_tracks(c, &p.id)?.len(), 3);
        catalog::replace_tracks(
            c,
            &catalog::union_owner_key(&p.id),
            &[track("x", SourceType::Spotify)],
            1,
        )?;
        assert!(delete_playlist(c, &p.id)?);
        assert_eq!(
            list_tracks(c, &p.id)?,
            [] as [ap_store::playlists::PlaylistTrackRecord; 0]
        );
        assert!(catalog::tracks(c, &catalog::union_owner_key(&p.id))?.is_none());
        assert!(!delete_playlist(c, &p.id)?);
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn playlist_mutations_rollback_all_rows_counters_and_timestamps() {
    let env = TestEnv::new();
    env.db().await.call(|c| {
        let p = create_playlist(c, &NewPlaylist::new("p", PlaylistType::Standard))?;
        add_tracks(c, &p.id, &[track("old", SourceType::Spotify)])?;
        c.execute_batch("CREATE TRIGGER fail_track BEFORE INSERT ON playlist_tracks WHEN NEW.track_id = 'bad' BEGIN SELECT RAISE(ABORT, 'injected'); END;")?;
        let before = get_playlist(c, &p.id)?;
        assert!(add_tracks(c, &p.id, &[track("good", SourceType::Plex), track("bad", SourceType::Plex)]).is_err());
        assert!(replace_tracks(c, &p.id, &[NewPlaylistTrack { track: track("bad", SourceType::Plex), added_at: 42 }]).is_err());
        assert_eq!(list_tracks(c, &p.id)?[0].track_id, "old");
        assert_eq!(get_playlist(c, &p.id)?, before);
        let imported = NewPlaylistTrack { track: track("imported", SourceType::Plex), added_at: 123 };
        replace_tracks(c, &p.id, &[imported])?;
        assert_eq!(list_tracks(c, &p.id)?[0].added_at, 123);
        assert_eq!(get_playlist(c, &p.id)?.unwrap().updated_at, before.unwrap().updated_at);
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn union_sources_clamp_reorder_remove_and_replace_atomically() {
    let env = TestEnv::new();
    env.db()
        .await
        .call(|c| {
            let p = create_playlist(c, &NewPlaylist::new("union", PlaylistType::Union))?;
            assert!(add_tracks(c, &p.id, &[track("x", SourceType::Spotify)]).is_err());
            let a = add_union_source(c, &p.id, SourceType::Spotify, "a")?;
            let b = add_union_source(c, &p.id, SourceType::Plex, "b")?;
            let d = add_union_source(c, &p.id, SourceType::Jellyfin, "d")?;
            assert_eq!(move_union_source(c, &p.id, a.id, 100)?, Some(2));
            assert_eq!(move_union_source(c, &p.id, d.id, -1)?, Some(0));
            assert!(!remove_union_source(c, "wrong", b.id)?);
            assert!(remove_union_source(c, &p.id, b.id)?);
            let before = list_union_sources(c, &p.id)?;
            assert_eq!(
                before.iter().map(|s| s.position).collect::<Vec<_>>(),
                [0, 1]
            );
            assert!(
                replace_union_sources(
                    c,
                    &p.id,
                    &[NewUnionSource {
                        source_type: SourceType::Custom,
                        source_playlist_id: p.id.clone(),
                        added_at: 1
                    }]
                )
                .is_err()
            );
            assert_eq!(list_union_sources(c, &p.id)?, before);
            assert!(delete_playlist(c, &p.id)?);
            assert_eq!(
                list_union_sources(c, &p.id)?,
                [] as [ap_store::playlists::UnionSourceRecord; 0]
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn provider_preferences_upsert_preserves_row_and_other_sources() {
    let env = TestEnv::new();
    env.db()
        .await
        .call(|c| {
            assert!(!provider_playlist_distinct(c, SourceType::Plex, "id")?);
            set_provider_playlist_distinct(c, SourceType::Plex, "id", true)?;
            let row: i64 =
                c.query_row("SELECT rowid FROM provider_playlist_preferences", [], |r| {
                    r.get(0)
                })?;
            set_provider_playlist_distinct(c, SourceType::Spotify, "id", true)?;
            set_provider_playlist_distinct(c, SourceType::Plex, "id", false)?;
            assert_eq!(
                c.query_row(
                    "SELECT rowid FROM provider_playlist_preferences WHERE source='plex'",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                row
            );
            assert!(!provider_playlist_distinct(c, SourceType::Plex, "id")?);
            assert!(provider_playlist_distinct(c, SourceType::Spotify, "id")?);
            assert_eq!(list_provider_playlist_preferences(c)?.len(), 2);
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn column_layout_normalizes_legacy_duration_width_with_canonical_precedence() {
    let env = TestEnv::new();
    env.db().await.call(|c| {
        assert_eq!(column_layout(c)?, ColumnLayout::default());
        c.execute("INSERT INTO column_preferences VALUES (1, '[\"duration_ms\",\"title\"]', '[1,0]', ?1)", [r#"{"duration_ms":123.6,"title":222.2}"#])?;
        let layout = column_layout(c)?;
        assert_eq!(layout.columns, ["duration", "title"]);
        assert_eq!(layout.column_widths.get("duration"), Some(&124));
        assert!(!layout.column_widths.contains_key("duration_ms"));
        let mut layout = layout;
        layout.column_widths.insert("duration".into(), 88);
        layout.column_widths.insert("duration_ms".into(), 77);
        set_column_layout(c, &layout)?;
        assert_eq!(column_layout(c)?.column_widths.get("duration"), Some(&88));
        c.execute("UPDATE column_preferences SET column_widths = 'garbage'", [])?;
        assert!(column_layout(c)?.column_widths.is_empty());
        c.execute("UPDATE column_preferences SET columns = 'garbage'", [])?;
        assert_eq!(column_layout(c)?, ColumnLayout::default());
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn queue_roundtrip_version_qualified_cursor_and_clear() {
    let env = TestEnv::new();
    env.db()
        .await
        .call(|c| {
            let mut q = sample_queue();
            assert!(!queue::has_queue(c)?);
            settings::put(c, settings::keys::PLAYER_SHUFFLE, &true)?;
            queue::write_content(c, &q)?;
            let restored = queue::load(c)?;
            assert_eq!(restored.entries, q.entries);
            assert_eq!(restored.order, q.order);
            assert_eq!(restored.current().unwrap().track.id, "b");
            q.cursor = 1;
            assert!(queue::write_cursor(
                c,
                q.current().map(|e| e.uid),
                q.content_version
            )?);
            assert_eq!(queue::load(c)?.current().unwrap().track.id, "a");
            q.content_version += 1;
            q.cursor = 0;
            assert!(!queue::write_cursor(
                c,
                q.current().map(|e| e.uid),
                q.content_version
            )?);
            assert_eq!(queue::load(c)?.current().unwrap().track.id, "a");
            assert!(!queue::write_cursor(c, Some(999), q.content_version - 1)?);
            assert!(queue::write_cursor(c, Some(u64::MAX), q.content_version - 1).is_err());
            assert!(queue::write_cursor(c, None, q.content_version - 1)?);
            assert!(queue::load(c)?.current().is_none());
            queue::clear(c)?;
            assert!(!queue::has_queue(c)?);
            assert!(queue::load(c)?.entries.is_empty());
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn queue_rejects_invalid_orders_and_rolls_back_failed_content() {
    let env = TestEnv::new();
    env.db()
        .await
        .call(|c| {
            let q = sample_queue();
            queue::write_content(c, &q)?;
            let mut bad = q.clone();
            std::sync::Arc::make_mut(&mut bad.entries)[1].uid = u64::MAX;
            assert!(queue::write_content(c, &bad).is_err());
            assert_eq!(queue::load(c)?.entries, q.entries);
            for invalid in ["2,1,1", "2,1,999", "garbage,2,1", "1"] {
                c.execute("UPDATE queue_meta SET order_uids=?1", [invalid])?;
                if invalid == "1" {
                    assert!(queue::load(c).is_err(), "accepted incomplete order");
                } else {
                    // Android maps known uids and deduplicates before checking coverage.
                    assert_eq!(&*queue::load(c)?.order, &[1, 0]);
                }
            }
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn catalog_scopes_owners_removes_vanished_and_preserves_freshness() {
    let env = TestEnv::new();
    env.db()
        .await
        .call(|c| {
            assert_eq!(
                catalog::provider_owner_key(SourceType::Plex, "a:b"),
                "p:PLEX:a:b"
            );
            assert_eq!(catalog::union_owner_key("a:b"), "u:a:b");
            let plex = catalog::provider_owner_key(SourceType::Plex, "gone");
            let spotify = catalog::provider_owner_key(SourceType::Spotify, "gone");
            for key in [&plex, &spotify, "u:gone"] {
                catalog::replace_tracks(c, key, &[track("x", SourceType::Plex)], 12)?;
            }
            catalog::replace_playlists(
                c,
                SourceType::Plex,
                &[playlist("gone", SourceType::Plex)],
                123,
            )?;
            assert_eq!(
                catalog::playlists(c, Some(SourceType::Plex))?[0].fetched_at_ms,
                123
            );
            assert_eq!(
                catalog::replace_playlists(
                    c,
                    SourceType::Plex,
                    &[playlist("kept", SourceType::Plex)],
                    456
                )?,
                std::slice::from_ref(&plex)
            );
            assert!(catalog::tracks(c, &plex)?.is_none());
            assert!(catalog::tracks(c, &spotify)?.is_some());
            assert!(catalog::tracks(c, "u:gone")?.is_some());
            catalog::replace_provider_tracks(
                c,
                SourceType::Plex,
                "kept",
                &[track("y", SourceType::Plex)],
                789,
            )?;
            assert_eq!(catalog::playlists(c, None)?[0].playlist.track_count, 1);
            let key = catalog::provider_owner_key(SourceType::Plex, "kept");
            assert!(catalog::touch_owner(c, &key, 900)?);
            assert_eq!(catalog::tracks(c, &key)?.unwrap().updated_at_ms, 900);
            catalog::clear_provider(c, SourceType::Plex)?;
            assert_eq!(
                catalog::playlists(c, None)?,
                [] as [ap_store::catalog::CachedPlaylist; 0]
            );
            assert!(catalog::tracks(c, &key)?.is_none());
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn catalog_listing_and_owner_replacements_rollback_on_insert_failure() {
    let env = TestEnv::new();
    env.db().await.call(|c| {
        let key = catalog::provider_owner_key(SourceType::Plex, "old");
        catalog::replace_playlists(c, SourceType::Plex, &[playlist("old", SourceType::Plex)], 1)?;
        catalog::replace_tracks(c, &key, &[track("old", SourceType::Plex)], 1)?;
        c.execute_batch("CREATE TRIGGER fail_listing BEFORE INSERT ON cached_playlists WHEN NEW.id='bad' BEGIN SELECT RAISE(ABORT, 'injected'); END; CREATE TRIGGER fail_cache BEFORE INSERT ON cached_tracks WHEN NEW.position=1 BEGIN SELECT RAISE(ABORT, 'injected'); END;")?;
        assert!(catalog::replace_playlists(c, SourceType::Plex, &[playlist("bad", SourceType::Plex)], 2).is_err());
        assert_eq!(catalog::playlists(c, None)?[0].playlist.id, "old");
        assert!(catalog::replace_tracks(c, &key, &[track("a", SourceType::Plex), track("b", SourceType::Plex)], 2).is_err());
        let cache = catalog::tracks(c, &key)?.unwrap();
        assert_eq!(cache.updated_at_ms, 1);
        assert_eq!(cache.track_count, 1);
        assert_eq!(cache.tracks[0].id, "old");
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn sync_bases_versions_are_target_scoped_and_clear_is_atomic() {
    use sync_state::{SyncDomain, *};
    let env = TestEnv::new();
    env.db().await.call(|c| {
        for d in SyncDomain::ALL { set_base(c, d, "one", "remote", "local")?; }
        record_version(c, "one", 42)?;
        record_version(c, "two", 3)?;
        set_base(c, SyncDomain::Settings, "two", "r2", "l2")?;
        assert!(base(c, SyncDomain::Settings, "one")?.is_none());
        c.execute_batch("CREATE TRIGGER fail_clear BEFORE DELETE ON sync_base BEGIN SELECT RAISE(ABORT, 'injected'); END;")?;
        assert!(clear_target(c, "one").is_err());
        assert_eq!(version(c, "one")?, Some(42));
        c.execute_batch("DROP TRIGGER fail_clear")?;
        clear_target(c, "one")?;
        assert_eq!(version(c, "one")?, None);
        assert_eq!(version(c, "two")?, Some(3));
        assert_eq!(base(c, SyncDomain::Settings, "two")?.unwrap().local, "l2");
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn dj_and_gain_state_roundtrip_upsert_isolation_corruption_and_delete() {
    let env = TestEnv::new();
    env.db()
        .await
        .call(|c| {
            dj_state::save(c, "one", &json!({"played": [1,2]}))?;
            dj_state::save(c, "two", &json!({"played": [3]}))?;
            dj_state::save(c, "one", &json!({"played": [4]}))?;
            assert_eq!(
                dj_state::load::<serde_json::Value>(c, "one")?.unwrap(),
                json!({"played": [4]})
            );
            c.execute(
                "UPDATE dj_passage_state SET state_json='bad' WHERE account_key='one'",
                [],
            )?;
            assert!(dj_state::load::<serde_json::Value>(c, "one")?.is_none());
            assert!(dj_state::delete(c, "one")?);
            assert!(!dj_state::delete(c, "one")?);
            assert!(dj_state::load::<serde_json::Value>(c, "two")?.is_some());
            let key = track_gain::track_gain_key(SourceType::Plex, "same");
            assert_eq!(key, "PLEX:same");
            let gain = track_gain::TrackGain {
                gain: 1.25,
                analyzed_at_ms: 1234,
            };
            track_gain::put(c, &key, gain)?;
            track_gain::put(
                c,
                &key,
                track_gain::TrackGain {
                    gain: 0.75,
                    analyzed_at_ms: 2345,
                },
            )?;
            assert_eq!(track_gain::get(c, &key)?.unwrap().analyzed_at_ms, 2345);
            assert!((track_gain::get(c, &key)?.unwrap().gain - 0.75).abs() < f32::EPSILON);
            assert!(track_gain::get(c, "SPOTIFY:same")?.is_none());
            assert!(track_gain::delete(c, &key)?);
            assert!(!track_gain::delete(c, &key)?);
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn nested_transactions_rollback_only_failed_scope_and_panics() {
    let env = TestEnv::new();
    env.db()
        .await
        .call(|c| {
            in_tx(c, |c| {
                settings::put(c, "outer", &1)?;
                let failure: ap_store::Result<()> = in_tx(c, |c| {
                    settings::put(c, "inner", &2)?;
                    Err(StoreError::Invalid("injected".into()))
                });
                assert!(failure.is_err());
                assert_eq!(settings::get::<i32>(c, "inner")?, None);
                Ok(())
            })?;
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _: ap_store::Result<()> = in_tx(c, |c| {
                    settings::put(c, "panic", &3)?;
                    panic!("injected")
                });
            }));
            assert!(panic.is_err());
            assert!(c.is_autocommit());
            assert_eq!(settings::get::<i32>(c, "outer")?, Some(1));
            assert_eq!(settings::get::<i32>(c, "panic")?, None);
            Ok(())
        })
        .await
        .unwrap();
}
