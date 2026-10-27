mod support;
use ap_library::transfer::*;
use ap_model::{
    ColumnPreferences, CustomPlaylist, PlaybackStateType, PlaybackStatus, PlaylistTrack,
    PlaylistType, RepeatMode, SourceType, UnionPlaylistSource, wire::*,
};
use std::sync::Arc;
use support::*;

fn p(id: &str, name: &str, kind: PlaylistType) -> CustomPlaylist {
    CustomPlaylist {
        id: id.into(),
        name: name.into(),
        description: None,
        image_url: None,
        created_at: "2023-11-14T22:13:20Z".into(),
        updated_at: "2023-11-14T22:13:20Z".into(),
        track_count: 1,
        playlist_type: kind,
        is_distinct: false,
    }
}
fn t(id: &str, parent: &str) -> PlaylistTrack {
    PlaylistTrack {
        id: id.into(),
        playlist_id: parent.into(),
        track_source: SourceType::Plex,
        track_id: "song".into(),
        position: 0,
        added_at: "2023-11-14T22:13:20Z".into(),
        title: "Song".into(),
        artist: "Artist".into(),
        album: None,
        duration_ms: None,
        image_url: None,
        url: None,
    }
}
fn sample() -> AnyPlayerStateData {
    AnyPlayerStateData {
        custom_playlists: vec![p("p", "local", PlaylistType::Standard)],
        playlist_tracks: vec![t("row", "p")],
        ..Default::default()
    }
}
fn envelope(data: AnyPlayerStateData) -> String {
    AnyPlayerStateEnvelope::new(
        "2026-01-01T00:00:00Z".into(),
        SourceAppInfo {
            platform: "android".into(),
            app_version: "test".into(),
        },
        data,
    )
    .to_json()
}

#[test]
fn keep_local_remaps_children_and_custom_references_deterministically() {
    let existing = sample();
    let mut incoming = sample();
    incoming.custom_playlists[0].name = "incoming".into();
    incoming.union_playlist_sources.push(UnionPlaylistSource {
        id: "link".into(),
        union_playlist_id: "p".into(),
        source_type: SourceType::Custom,
        source_playlist_id: "p".into(),
        position: 0,
        added_at: "now".into(),
    });
    let first = plan_playlist_import(
        incoming.clone(),
        &existing,
        MergePolicy::MergeKeepLocal,
        "import",
        true,
    )
    .unwrap();
    let second = plan_playlist_import(
        incoming,
        &existing,
        MergePolicy::MergeKeepLocal,
        "import",
        true,
    )
    .unwrap();
    assert_eq!(first.data, second.data);
    assert!(first.data.custom_playlists[0].id.starts_with("p_import_"));
    assert_ne!(first.data.playlist_tracks[0].id, "row");
    assert_eq!(
        first.data.custom_playlists[0].id,
        first.data.playlist_tracks[0].playlist_id
    );
    assert_eq!(
        first.data.custom_playlists[0].id,
        first.data.union_playlist_sources[0].source_playlist_id
    );
    assert_eq!(first.summary.tracks_added, 1);
}
#[test]
fn prefer_import_counts_updates_and_stable_positions() {
    let existing = sample();
    let mut incoming = sample();
    incoming.custom_playlists[0].name = "incoming".into();
    incoming.playlist_tracks = [5, 2, 5]
        .into_iter()
        .enumerate()
        .map(|(i, position)| {
            let mut row = t(["a", "row", "c"][i], "p");
            row.position = position;
            row
        })
        .collect();
    let plan = plan_playlist_import(
        incoming,
        &existing,
        MergePolicy::MergePreferImport,
        "import",
        true,
    )
    .unwrap();
    assert_eq!(plan.summary.playlists_updated, 1);
    assert_eq!(plan.summary.tracks_updated, 1);
    assert_eq!(plan.summary.tracks_added, 2);
    assert_eq!(
        plan.data
            .playlist_tracks
            .iter()
            .map(|t| (t.id.as_str(), t.position))
            .collect::<Vec<_>>(),
        [("row", 0), ("a", 1), ("c", 2)]
    );
}
#[tokio::test]
async fn state_roundtrip_preserves_wire_ids_iso_precision_url_columns_and_empty_unions() {
    let (_dir, db) = database().await;
    let service = TransferService::new(db, Arc::new(Connections::default()));
    let mut data = sample();
    data.custom_playlists[0].created_at = "2023-11-14T22:13:20.123Z".into();
    data.playlist_tracks[0].added_at = "2023-11-14T22:13:20.456Z".into();
    data.playlist_tracks[0].url = Some("file:///song.flac".into());
    data.custom_playlists
        .push(p("empty", "empty", PlaylistType::Union));
    data.column_preferences.insert(
        "artist".into(),
        ColumnPreferences {
            key: "artist".into(),
            visible: false,
            position: 7,
        },
    );
    let raw = envelope(data.clone());
    let summary = service
        .import_state(&raw, MergePolicy::ReplaceAll, None, false)
        .await
        .unwrap();
    assert_eq!(summary.playlists_added, 2);
    let snapshot = service.snapshot().await.unwrap();
    // Database listing order is updated_at DESC/id; compare each domain by identity.
    assert_eq!(snapshot.playlist_tracks, data.playlist_tracks);
    assert_eq!(snapshot.column_preferences, data.column_preferences);
    assert_eq!(
        snapshot
            .custom_playlists
            .iter()
            .find(|p| p.id == "p")
            .unwrap(),
        &data.custom_playlists[0]
    );
    let again = service
        .import_state(&raw, MergePolicy::MergePreferImport, None, false)
        .await
        .unwrap();
    assert_eq!(again.tracks_updated, 1);
    assert_eq!(again.tracks_added, 0);
}
#[tokio::test]
async fn config_exports_match_every_golden_fixture_byte_for_byte_and_reimport_is_idempotent() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/config_ids.json")).unwrap();
    for case in fixtures["export_json"].as_array().unwrap() {
        let (_dir, db) = database().await;
        let service = TransferService::new(db, Arc::new(Connections::default()));
        let expected = case["json"].as_str().unwrap();
        let input = format!(
            "{{\"export_version\":1,\"provider_configs\":{{}},\"custom_playlists\":{expected}}}"
        );
        service
            .import_config(&input, MergePolicy::ReplaceAll, false)
            .await
            .unwrap();
        assert_eq!(
            service.export_playlists_json().await.unwrap().to_string(),
            expected,
            "{}",
            case["name"]
        );
        let first = service.snapshot().await.unwrap();
        service
            .import_config(&input, MergePolicy::ReplaceAll, false)
            .await
            .unwrap();
        assert_eq!(first, service.snapshot().await.unwrap());
    }
}
#[tokio::test]
async fn repeated_tracks_and_union_sources_get_occurrence_aware_ids() {
    let (_dir, db) = database().await;
    let service = TransferService::new(db, Arc::new(Connections::default()));
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/config_ids.json")).unwrap();
    let mut entries: serde_json::Value =
        serde_json::from_str(fixture["export_json"][1]["json"].as_str().unwrap()).unwrap();
    let track = entries[0]["tracks"][0].clone();
    entries[0]["tracks"].as_array_mut().unwrap().push(track);
    let input =
        serde_json::json!({"export_version":1,"provider_configs":{},"custom_playlists":entries});
    service
        .import_config(&input.to_string(), MergePolicy::ReplaceAll, false)
        .await
        .unwrap();
    let data = service.snapshot().await.unwrap();
    assert_eq!(data.playlist_tracks.len(), 3);
    assert_eq!(
        data.playlist_tracks[0].id,
        config_track_row_id("p1", SourceType::Spotify, "abc", 0)
    );
    assert_eq!(
        data.playlist_tracks[1].id,
        config_track_row_id("p1", SourceType::Spotify, "abc", 1)
    );
    let mut entries: serde_json::Value =
        serde_json::from_str(fixture["export_json"][2]["json"].as_str().unwrap()).unwrap();
    let source = entries[0]["union_sources"][0].clone();
    entries[0]["union_sources"]
        .as_array_mut()
        .unwrap()
        .push(source);
    let input =
        serde_json::json!({"export_version":1,"provider_configs":{},"custom_playlists":entries});
    service
        .import_config(&input.to_string(), MergePolicy::ReplaceAll, false)
        .await
        .unwrap();
    let data = service.snapshot().await.unwrap();
    assert_eq!(
        data.union_playlist_sources[0].id,
        config_union_source_row_id("u1", SourceType::Jellyfin, "jf", 0)
    );
    assert_eq!(
        data.union_playlist_sources[1].id,
        config_union_source_row_id("u1", SourceType::Jellyfin, "jf", 1)
    );
}
#[tokio::test]
async fn dry_run_changes_neither_playlists_connections_nor_playback() {
    let (_dir, db) = database().await;
    let connections = Arc::new(Connections::default());
    let service = TransferService::new(db.clone(), connections.clone());
    let mut data = sample();
    data.connections.insert(
        SourceType::Plex,
        ExportedConnection {
            source: SourceType::Plex,
            server_url: Some("http://plex".into()),
            username: None,
            token: Some("secret".into()),
            api_key: None,
            spotify_connected: None,
            spotify_premium: None,
        },
    );
    let summary = service
        .import_state(
            &envelope(data),
            MergePolicy::ReplaceAll,
            Some("unused"),
            true,
        )
        .await
        .unwrap();
    assert_eq!(summary.playlists_added, 1);
    assert_eq!(summary.connections_imported, 1);
    assert!(
        service
            .snapshot()
            .await
            .unwrap()
            .custom_playlists
            .is_empty()
    );
    assert!(connections.0.lock().unwrap().is_empty());
    assert!(summary.playback_settings.is_none());
    let tables = db
        .read(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'library_%'",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(tables, 0);
}
#[tokio::test]
async fn merge_replaces_children_only_for_imported_playlists_and_bad_references_are_atomic() {
    let (_dir, db) = database().await;
    let service = TransferService::new(db, Arc::new(Connections::default()));
    let mut initial = sample();
    initial
        .custom_playlists
        .push(p("keep", "keep", PlaylistType::Standard));
    initial.playlist_tracks.push(t("keep-row", "keep"));
    service
        .import_data(initial, MergePolicy::ReplaceAll, false, "import", true)
        .await
        .unwrap();
    let mut incoming = sample();
    incoming.playlist_tracks.clear();
    service
        .import_data(
            incoming,
            MergePolicy::MergePreferImport,
            false,
            "import",
            true,
        )
        .await
        .unwrap();
    let before = service.snapshot().await.unwrap();
    assert_eq!(before.playlist_tracks.len(), 1);
    assert_eq!(before.playlist_tracks[0].playlist_id, "keep");
    let mut malformed = sample();
    malformed.playlist_tracks[0].playlist_id = "missing".into();
    assert!(
        service
            .import_data(malformed, MergePolicy::ReplaceAll, false, "import", true)
            .await
            .is_err()
    );
    assert_eq!(before, service.snapshot().await.unwrap());
}
fn playback() -> PlaybackStatus {
    PlaybackStatus {
        state: PlaybackStateType::Paused,
        shuffle: true,
        repeat_mode: RepeatMode::All,
        volume: 57,
        current_track: None,
        position: 0,
        duration: 0,
        queue: vec![],
        ordered_queue: vec![],
        error_message: None,
    }
}
#[tokio::test]
async fn portable_private_and_connection_merging_follow_android_policy() {
    let (_dir, db) = database().await;
    let connections = Arc::new(Connections::default());
    for source in [SourceType::Jellyfin, SourceType::Spotify] {
        let mut c = StoredConnection::new(source);
        c.token = Some("secret".into());
        c.server_url = Some("http://local".into());
        connections.0.lock().unwrap().insert(source, c);
    }
    let service = TransferService::new(db, connections.clone());
    let portable = service
        .export_state(ExportMode::Portable, None, Some(playback()), "test")
        .await
        .unwrap();
    assert!(!portable.contains("secret"));
    assert!(
        service
            .export_state(ExportMode::Private, None, None, "test")
            .await
            .is_err()
    );
    let private = service
        .export_state(
            ExportMode::Private,
            Some("password"),
            Some(playback()),
            "test",
        )
        .await
        .unwrap();
    let decoded = decode_and_verify(&private, Some("password")).unwrap();
    assert_eq!(
        decoded.data.connections[&SourceType::Jellyfin]
            .token
            .as_deref(),
        Some("secret")
    );
    assert_eq!(decoded.data.connections[&SourceType::Spotify].token, None);
    assert!(
        service
            .import_state(&private, MergePolicy::ReplaceAll, Some("wrong"), false)
            .await
            .is_err()
    );
    let summary = service
        .import_state(&private, MergePolicy::ReplaceAll, Some("password"), false)
        .await
        .unwrap();
    assert_eq!(summary.playback_settings.unwrap().volume, 57);
    assert!(
        service
            .import_state(
                &private,
                MergePolicy::MergeKeepLocal,
                Some("password"),
                false
            )
            .await
            .unwrap()
            .playback_settings
            .is_none()
    );
    let mut data = AnyPlayerStateData::default();
    data.connections.insert(
        SourceType::Spotify,
        ExportedConnection {
            source: SourceType::Spotify,
            server_url: None,
            username: Some("new".into()),
            token: Some("ignore".into()),
            api_key: None,
            spotify_connected: Some(true),
            spotify_premium: Some(true),
        },
    );
    let summary = service
        .import_state(
            &envelope(data),
            MergePolicy::MergeKeepLocal,
            Some("pass"),
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        summary.warnings,
        ["Spotify raw access token ignored by policy"]
    );
    let stored = connections.0.lock().unwrap();
    assert_eq!(
        stored[&SourceType::Spotify].token.as_deref(),
        Some("secret")
    );
    assert_eq!(
        stored[&SourceType::Spotify].server_url.as_deref(),
        Some("http://local")
    );
}
#[tokio::test]
async fn legacy_desktop_seconds_config_import_unknown_types_and_cfg_remap() {
    let (_dir, db) = database().await;
    let connections = Arc::new(Connections::default());
    let service = TransferService::new(db, connections.clone());
    service
        .import_data(sample(), MergePolicy::ReplaceAll, false, "import", true)
        .await
        .unwrap();
    let raw = r#"{"export_version":1,"provider_configs":{"spotify":{"client_id":"build"},"plex":{"base_url":"http://plex"}},"custom_playlists":[{"playlist":{"id":"p","name":"desktop","created_at":1700000000,"updated_at":1700000001,"track_count":0,"playlist_type":"future"},"tracks":[{"id":1,"playlist_id":"p","track_source":"unknown","track_id":"bad","position":99,"added_at":1700000000,"title":"bad","artist":"bad"}]}]}"#;
    let summary = service
        .import_config(raw, MergePolicy::MergeKeepLocal, false)
        .await
        .unwrap();
    assert_eq!(summary.warnings.len(), 3);
    assert_eq!(summary.connections_imported, 1);
    let snapshot = service.snapshot().await.unwrap();
    assert_eq!(snapshot.custom_playlists.len(), 2);
    assert!(
        snapshot
            .custom_playlists
            .iter()
            .any(|p| p.id.starts_with("p_cfg_") && p.updated_at == "2023-11-14T22:13:21Z")
    );
    assert_eq!(
        service
            .export_provider_config_json()
            .await
            .unwrap()
            .to_string(),
        r#"{"plex":{"base_url":"http://plex"}}"#
    );
}
#[test]
fn summary_format_matches_android() {
    assert_eq!(
        ImportSummary::default().to_string(),
        "Import complete. playlists +0/~0, tracks +0/~0, union +0/~0, connections +0/skip 0, no warnings"
    );
}

#[tokio::test]
async fn all_android_state_vectors_import_without_losing_wire_data() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/state_transfer.json")).unwrap();
    for case in fixtures["cases"].as_array().unwrap() {
        let (_dir, db) = database().await;
        let service = TransferService::new(db, Arc::new(Connections::default()));
        let raw = case["envelope_json"].as_str().unwrap();
        let expected = decode_and_verify(raw, None).unwrap();
        service
            .import_state(raw, MergePolicy::ReplaceAll, None, false)
            .await
            .unwrap();
        let mut actual = service.snapshot().await.unwrap();
        let mut playlists = expected.data.custom_playlists;
        actual.custom_playlists.sort_by(|a, b| a.id.cmp(&b.id));
        playlists.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(actual.custom_playlists, playlists, "{}", case["name"]);
        assert_eq!(
            actual.playlist_tracks, expected.data.playlist_tracks,
            "{}",
            case["name"]
        );
        assert_eq!(
            actual.union_playlist_sources, expected.data.union_playlist_sources,
            "{}",
            case["name"]
        );
        assert_eq!(
            actual.column_preferences, expected.data.column_preferences,
            "{}",
            case["name"]
        );
    }
}

#[tokio::test]
async fn malformed_files_do_not_mutate_database_and_legacy_seed_imports() {
    let (_dir, db) = database().await;
    let service = TransferService::new(db, Arc::new(Connections::default()));
    let legacy = include_str!("../../../fixtures/legacy-state.json");
    assert_eq!(
        service
            .import_state(legacy, MergePolicy::MergePreferImport, None, true)
            .await
            .unwrap(),
        ImportSummary::default()
    );
    service
        .import_data(sample(), MergePolicy::ReplaceAll, false, "import", true)
        .await
        .unwrap();
    let before = service.snapshot().await.unwrap();
    let raw = envelope(sample()).replace("\"local\"", "\"tampered\"");
    assert!(
        service
            .import_state(&raw, MergePolicy::ReplaceAll, None, false)
            .await
            .is_err()
    );
    assert!(
        service
            .import_config(
                r#"{"export_version":2,"provider_configs":{}}"#,
                MergePolicy::ReplaceAll,
                false
            )
            .await
            .is_err()
    );
    assert_eq!(before, service.snapshot().await.unwrap());
}

#[tokio::test]
async fn keep_local_repeated_import_is_stable_and_keeps_local_children() {
    let (_dir, db) = database().await;
    let service = TransferService::new(db, Arc::new(Connections::default()));
    service
        .import_data(sample(), MergePolicy::ReplaceAll, false, "import", true)
        .await
        .unwrap();
    let mut incoming = sample();
    incoming.custom_playlists[0].name = "incoming".into();
    incoming.playlist_tracks[0].title = "Imported".into();
    service
        .import_data(
            incoming.clone(),
            MergePolicy::MergeKeepLocal,
            false,
            "import",
            true,
        )
        .await
        .unwrap();
    let first = service.snapshot().await.unwrap();
    service
        .import_data(incoming, MergePolicy::MergeKeepLocal, false, "import", true)
        .await
        .unwrap();
    assert_eq!(first, service.snapshot().await.unwrap());
    assert_eq!(
        first
            .playlist_tracks
            .iter()
            .find(|t| t.playlist_id == "p")
            .unwrap()
            .title,
        "Song"
    );
}

#[tokio::test]
async fn exported_local_wire_ids_survive_remove_reorder_and_append() {
    let (_dir, db) = database().await;
    let catalog = ap_library::CatalogRepository::new(db.clone(), vec![])
        .await
        .unwrap();
    let playlists = ap_library::PlaylistRepository::new(db.clone(), catalog)
        .await
        .unwrap();
    let service = TransferService::new(db, Arc::new(Connections::default()));
    let p = playlists
        .create("local", PlaylistType::Standard)
        .await
        .unwrap();
    let track = track("same", SourceType::Plex);
    playlists
        .add_tracks(&p.id, vec![track.clone(), track.clone()])
        .await
        .unwrap();
    let before = service.snapshot().await.unwrap();
    playlists.remove_track_at(&p.id, 0).await.unwrap();
    playlists.add_tracks(&p.id, vec![track]).await.unwrap();
    let after = service.snapshot().await.unwrap();
    assert_eq!(after.playlist_tracks[0].id, before.playlist_tracks[1].id);
    assert_ne!(after.playlist_tracks[0].id, after.playlist_tracks[1].id);
}

#[tokio::test]
async fn sync_export_import_and_base_record_share_one_writer_transaction() {
    let (_dir, db) = database().await;
    let service = TransferService::new(db.clone(), Arc::new(Connections::default()));
    service
        .import_data(sample(), MergePolicy::ReplaceAll, false, "import", true)
        .await
        .unwrap();
    db.call(|c| {
        ap_store::in_tx(c, |c| {
            let rows = export_playlists_json_conn(c)
                .map_err(|e| ap_store::StoreError::Invalid(e.to_string()))?;
            let mut stored = indexmap::IndexMap::new();
            let mut connection = StoredConnection::new(SourceType::Plex);
            connection.server_url = Some("http://plex".into());
            stored.insert(SourceType::Plex, connection);
            let providers = export_provider_config_json_conn(c, &stored)
                .map_err(|e| ap_store::StoreError::Invalid(e.to_string()))?;
            let file = ap_model::json::JsonElement::parse(&format!(
                r#"{{"export_version":1,"provider_configs":{providers},"custom_playlists":{rows}}}"#
            ))
            .unwrap();
            let summary = import_json_conn(c, &file, MergePolicy::ReplaceAll, false)
                .map_err(|e| ap_store::StoreError::Invalid(e.to_string()))?;
            assert_eq!(summary.playlists_updated, 1);
            assert_eq!(
                summary.connection_updates,
                [ConnectionUpdate {
                    source: SourceType::Plex,
                    server_url: "http://plex".into()
                }]
            );
            assert_eq!(export_playlists_json_conn(c).unwrap(), rows);
            c.execute(
                "INSERT INTO settings (key,value) VALUES ('test.sync.base','1')",
                [],
            )?;
            Ok(())
        })
    })
    .await
    .unwrap();
    let before = service.snapshot().await.unwrap();
    assert!(
        db.call(|c| ap_store::in_tx(c, |c| {
            let file =
                ap_model::json::JsonElement::parse(r#"{"export_version":1,"provider_configs":{}}"#)
                    .unwrap();
            import_json_conn(c, &file, MergePolicy::ReplaceAll, false)
                .map_err(|e| ap_store::StoreError::Invalid(e.to_string()))?;
            c.execute(
                "UPDATE settings SET value='2' WHERE key='test.sync.base'",
                [],
            )?;
            Err::<(), _>(ap_store::StoreError::Invalid(
                "rollback sync transaction".into(),
            ))
        }))
        .await
        .is_err()
    );
    assert_eq!(service.snapshot().await.unwrap(), before);
    let base = db
        .read(|c| {
            Ok(c.query_row(
                "SELECT value FROM settings WHERE key='test.sync.base'",
                [],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(base, "1");
}
