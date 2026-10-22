mod common;

use ap_model::{RepeatMode, SourceType};
use ap_store::{
    legacy::*,
    queue,
    settings::{self, keys},
};
use common::{TestEnv, track};
use serde_json::json;

fn legacy_json() -> String {
    json!({"queue": [
        {"id":"a","title":"A","artist":"artist","album":"album","duration_ms":1000,"source":"Spotify"},
        {"id":"b","title":"B","artist":"artist","album":"album","duration_ms":2000,"source":"Plex", "url":"http://plex/b?X-Plex-Token=secret","auth_headers":[["Authorization","secret"]]},
        {"id":"c","title":"C","artist":"artist","source":"Jellyfin"}
    ], "shuffle":true, "shuffle_order":[2,0,1], "current_index":1, "position_ms":12345, "volume":123, "repeat_mode":"All", "audio_normalization_enabled":true}).to_string()
}

#[tokio::test]
async fn real_legacy_file_maps_shuffle_settings_sanitizes_and_retires_idempotently() {
    let env = TestEnv::new();
    let file = env.paths.legacy_playback_state();
    std::fs::write(&file, legacy_json()).unwrap();
    let paths = env.paths.clone();
    let db = env.db().await;
    db.call(move |c| {
        assert_eq!(
            migrate_playback_state(c, &paths)?,
            PlaybackStateOutcome::Migrated { queue_entries: 3 }
        );
        let q = queue::load(c)?;
        assert_eq!(&*q.order, &[2, 0, 1]);
        assert_eq!(q.cursor, 1);
        assert_eq!(q.current().unwrap().track.id, "a");
        assert_eq!(q.entries[1].track.source, SourceType::Plex);
        assert_eq!(q.entries[1].track.url.as_deref(), Some("http://plex/b"));
        let state = settings::load(c)?;
        assert_eq!(state.player.volume, 100);
        assert_eq!(state.player.position_ms, 12345);
        assert_eq!(state.player.repeat, RepeatMode::All);
        assert!(state.player.shuffle && state.player.normalization_enabled);
        assert!(settings::migration_done(c, PLAYBACK_STATE_MIGRATION)?);
        assert_eq!(
            migrate_playback_state(c, &paths)?,
            PlaybackStateOutcome::AlreadyDone
        );
        Ok(())
    })
    .await
    .unwrap();
    assert!(!file.exists());
    let retired = file.with_file_name("playback_state.json.migrated-v2");
    assert_eq!(std::fs::read_to_string(retired).unwrap(), legacy_json());
}

#[test]
fn legacy_queue_clamps_cursor_and_repairs_invalid_shuffle_permutation() {
    let mut legacy: LegacyPlaybackState = serde_json::from_str(&legacy_json()).unwrap();
    legacy.current_index = usize::MAX;
    assert_eq!(legacy.to_queue().unwrap().cursor, 2);
    legacy.shuffle = false;
    assert_eq!(&*legacy.to_queue().unwrap().order, &[0, 1, 2]);
    legacy.shuffle = true;
    legacy.current_index = 2;
    legacy.shuffle_order = vec![2, 2, 0, 999];
    let q = legacy.to_queue().unwrap();
    assert_eq!(&*q.order, &[2, 0, 1]);
    assert_eq!(q.current().unwrap().track.id, "a");
    legacy.queue.clear();
    assert!(legacy.to_queue().is_none());
}

#[tokio::test]
async fn migration_failure_rolls_back_settings_queue_flag_and_keeps_file_for_retry() {
    let env = TestEnv::new();
    std::fs::write(env.paths.legacy_playback_state(), legacy_json()).unwrap();
    let paths = env.paths.clone();
    env.db().await.call(move |c| {
        c.execute_batch("CREATE TRIGGER fail_queue BEFORE INSERT ON queue_entries BEGIN SELECT RAISE(ABORT, 'injected'); END;")?;
        assert!(migrate_playback_state(c, &paths).is_err());
        assert!(!settings::migration_done(c, PLAYBACK_STATE_MIGRATION)?);
        assert!(!queue::has_queue(c)?);
        assert_eq!(settings::get::<u32>(c, keys::PLAYER_VOLUME)?, None);
        assert!(paths.legacy_playback_state().exists());
        c.execute_batch("DROP TRIGGER fail_queue")?;
        assert_eq!(migrate_playback_state(c, &paths)?, PlaybackStateOutcome::Migrated { queue_entries: 3 });
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn existing_v2_queue_and_player_settings_take_precedence_over_legacy() {
    let env = TestEnv::new();
    std::fs::write(env.paths.legacy_playback_state(), legacy_json()).unwrap();
    let paths = env.paths.clone();
    env.db()
        .await
        .call(move |c| {
            let legacy = LegacyPlaybackState {
                queue: vec![track("v2", SourceType::Spotify)],
                ..LegacyPlaybackState::default()
            };
            queue::write_content(c, &legacy.to_queue().unwrap())?;
            settings::put(c, keys::PLAYER_VOLUME, &12)?;
            settings::put(c, keys::PLAYER_SHUFFLE, &false)?;
            assert_eq!(
                migrate_playback_state(c, &paths)?,
                PlaybackStateOutcome::Migrated { queue_entries: 0 }
            );
            assert_eq!(queue::load(c)?.current().unwrap().track.id, "v2");
            assert_eq!(settings::get::<u32>(c, keys::PLAYER_VOLUME)?, Some(12));
            assert_eq!(settings::get::<bool>(c, keys::PLAYER_SHUFFLE)?, Some(false));
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn empty_queue_ignored_and_missing_malformed_and_postcommit_files_idempotent() {
    for raw in [None, Some("{broken"), Some("{\"queue\":[],\"volume\":23}")] {
        let env = TestEnv::new();
        if let Some(raw) = raw {
            std::fs::write(env.paths.legacy_playback_state(), raw).unwrap();
        }
        let paths = env.paths.clone();
        env.db()
            .await
            .call(move |c| {
                let outcome = migrate_playback_state(c, &paths)?;
                match raw {
                    None => assert_eq!(outcome, PlaybackStateOutcome::NoFile),
                    Some("{broken") => {
                        assert!(matches!(outcome, PlaybackStateOutcome::Undecodable(_)));
                    }
                    Some(_) => {
                        assert_eq!(outcome, PlaybackStateOutcome::Migrated { queue_entries: 0 });
                    }
                }
                assert!(!queue::has_queue(c)?);
                assert!(settings::migration_done(c, PLAYBACK_STATE_MIGRATION)?);
                std::fs::write(paths.legacy_playback_state(), legacy_json())?;
                assert_eq!(
                    migrate_playback_state(c, &paths)?,
                    PlaybackStateOutcome::AlreadyDone
                );
                assert!(!paths.legacy_playback_state().exists());
                assert!(!queue::has_queue(c)?);
                Ok(())
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn cache_sweep_matches_only_legacy_files_in_cache_and_temp_and_is_idempotent() {
    let env = TestEnv::new();
    std::fs::create_dir_all(env.paths.temp_dir()).unwrap();
    let removed = [
        "playlists_cache.json",
        "custom_playlists_cache.json",
        "custom_playlist_tracks_one",
        "union_playlist_tracks_one.json",
        "provider_playlist_plex_one",
        "track_metadata_spotify_one.json",
    ];
    for file in removed {
        std::fs::write(env.paths.cache_dir().join(file), "legacy").unwrap();
    }
    for file in ["any-player-audio-one.mp3", "any-player-audio-two.MP3"] {
        std::fs::write(env.paths.temp_dir().join(file), "audio").unwrap();
    }
    for file in ["keep.json", "any-player-audio-one.wav"] {
        std::fs::write(env.paths.temp_dir().join(file), "keep").unwrap();
    }
    std::fs::write(
        env.paths
            .images_cache_dir()
            .join("track_metadata_spotify_image"),
        "keep",
    )
    .unwrap();
    std::fs::create_dir(env.paths.cache_dir().join("provider_playlist_directory")).unwrap();
    let paths = env.paths.clone();
    env.db()
        .await
        .call(move |c| {
            let report = sweep_legacy_cache(c, &paths)?;
            assert_eq!(report.removed, 8);
            assert_eq!(report.failed, 0);
            assert!(settings::migration_done(c, CACHE_SWEEP_MIGRATION)?);
            for file in removed {
                assert!(!paths.cache_dir().join(file).exists());
            }
            assert!(paths.temp_dir().join("keep.json").exists());
            assert!(paths.temp_dir().join("any-player-audio-one.wav").exists());
            assert!(
                paths
                    .images_cache_dir()
                    .join("track_metadata_spotify_image")
                    .exists()
            );
            assert!(
                paths
                    .cache_dir()
                    .join("provider_playlist_directory")
                    .is_dir()
            );
            assert!(sweep_legacy_cache(c, &paths)?.already_done);
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn cache_sweep_directory_failure_keeps_flag_unset_for_retry() {
    let env = TestEnv::new();
    let invalid = env.dir.path().join("not-a-directory");
    std::fs::write(&invalid, "file").unwrap();
    let paths = ap_store::Paths::new(env.paths.data_dir(), invalid, env.paths.temp_dir());
    env.db()
        .await
        .call(move |c| {
            assert_eq!(sweep_legacy_cache(c, &paths)?.failed, 1);
            assert!(!settings::migration_done(c, CACHE_SWEEP_MIGRATION)?);
            Ok(())
        })
        .await
        .unwrap();
}
