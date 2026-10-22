mod common;

use ap_model::RepeatMode;
use ap_store::{
    MemoryBackend, SecretAccount, Secrets, SettingsStore, StoreError,
    settings::{self, PreferredDevice, TrackSort, WindowGeometry, keys},
};
use common::TestEnv;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn typed_settings_defaults_roundtrip_every_key_and_boundaries() {
    let env = TestEnv::new();
    let db = env.db().await;
    let store = SettingsStore::open(db.clone()).await.unwrap();
    let initial = store.get();
    assert!(
        !(initial.player.shuffle
            || initial.player.normalization_enabled
            || initial.player.normalization_strict)
    );
    assert_eq!(initial.player.volume, 50);
    assert_eq!(initial.player.repeat, RepeatMode::Off);
    assert!(!(initial.dj.enabled || initial.dj.show_entries_in_queue || initial.dj.debug_logging));
    assert_eq!(initial.player.position_ms, 0);
    assert_eq!(initial.dj.voice_id, None);
    assert!((initial.dj.voice_level - 1.0).abs() < f64::EPSILON);
    assert!(initial.sync.server_target.is_empty());
    assert!(initial.sync.client_id.starts_with("desktop-"));
    assert!(
        initial.sync.domains.app_state
            && initial.sync.domains.playlists
            && initial.sync.domains.settings
            && initial.sync.domains.provider_configuration
    );
    assert!(initial.ui.close_to_tray);
    assert!(initial.ui.window.is_none() && initial.ui.track_sort.is_empty());
    assert_eq!(initial.spotify.preferred_device, None);
    assert_eq!(initial.providers.jellyfin_page_size, 300);
    assert_eq!(initial.providers.plex_page_size, 300);
    assert_eq!(initial.providers.jellyfin_user_id, None);
    let mut watch = store.subscribe();
    let updated = store
        .update(|s| {
            s.player.volume = 999;
            s.player.shuffle = true;
            s.player.repeat = RepeatMode::All;
            s.player.position_ms = 4321;
            s.player.normalization_enabled = true;
            s.player.normalization_strict = true;
            s.dj.enabled = true;
            s.dj.show_entries_in_queue = true;
            s.dj.voice_id = Some("voice".into());
            s.dj.voice_level = 3.0;
            s.dj.debug_logging = true;
            s.sync.server_target = "https://sync".into();
            s.sync.domains.app_state = false;
            s.spotify.preferred_device = Some(PreferredDevice {
                id: "device".into(),
                name: "Desktop".into(),
            });
            s.ui.close_to_tray = false;
            s.ui.window = Some(WindowGeometry {
                x: -3,
                y: 5,
                width: 900,
                height: 700,
                maximized: true,
            });
            s.ui.track_sort.insert(
                "table".into(),
                TrackSort {
                    column: "artist".into(),
                    ascending: false,
                },
            );
            s.providers.jellyfin_page_size = 0;
            s.providers.plex_page_size = 1001;
            s.providers.jellyfin_user_id = Some("user".into());
        })
        .await
        .unwrap();
    assert_eq!(updated.player.volume, 100);
    assert!((updated.dj.voice_level - 2.0).abs() < f64::EPSILON);
    assert_eq!(updated.providers.jellyfin_page_size, 1);
    assert_eq!(updated.providers.plex_page_size, 1000);
    watch.changed().await.unwrap();
    assert_eq!(*watch.borrow_and_update(), updated);
    assert_eq!(
        SettingsStore::open(db.clone()).await.unwrap().get(),
        updated
    );
    store.update(|_| {}).await.unwrap();
    assert!(!watch.has_changed().unwrap());
    let lower = store
        .update(|s| {
            s.dj.voice_level = -1.0;
            s.player.position_ms = -3;
            s.ui.track_sort.clear();
        })
        .await
        .unwrap();
    assert!((lower.dj.voice_level - 0.5).abs() < f64::EPSILON);
    assert_eq!(lower.player.position_ms, 0);
    assert!(
        db.call(|c| settings::get_json(c, "ui.track_sort.table"))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(store.reload().await.unwrap(), lower);
}

#[tokio::test]
async fn settings_invalid_rows_default_and_failed_write_publishes_nothing() {
    let env = TestEnv::new();
    let db = env.db().await;
    db.call(|c| {
        c.execute(
            "INSERT INTO settings VALUES ('player.volume', 'broken')",
            [],
        )?;
        settings::set_json(c, keys::PLAYER_SHUFFLE, &json!("not bool"))?;
        settings::set_json(c, keys::SYNC_DOMAINS, &json!({"playlists": false}))?;
        Ok(())
    })
    .await
    .unwrap();
    let store = SettingsStore::open(db.clone()).await.unwrap();
    assert_eq!(store.get().player.volume, 50);
    assert!(!store.get().player.shuffle);
    assert!(!store.get().sync.domains.playlists);
    assert!(store.get().sync.domains.settings);
    let old = store.get();
    let watch = store.subscribe();
    db.call(|c| {
        c.execute_batch("CREATE TRIGGER fail_setting BEFORE INSERT ON settings WHEN NEW.key='player.volume' BEGIN SELECT RAISE(ABORT, 'injected'); END;")?;
        Ok(())
    }).await.unwrap();
    assert!(
        store
            .update(|s| {
                s.dj.enabled = true;
                s.player.volume = 20;
            })
            .await
            .is_err()
    );
    assert_eq!(store.get(), old);
    assert!(!watch.has_changed().unwrap());
    assert_eq!(
        db.call(|c| settings::get::<bool>(c, keys::DJ_ENABLED))
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn writer_errors_and_panics_rollback_and_leave_connection_usable() {
    let env = TestEnv::new();
    let db = env.db().await;
    let err: ap_store::Result<()> = db
        .call(|c| {
            c.execute_batch("BEGIN; INSERT INTO settings VALUES ('failed', '1')")?;
            Err(StoreError::Invalid("injected".into()))
        })
        .await;
    assert!(err.is_err());
    db.call(|c| {
        assert!(c.is_autocommit());
        assert_eq!(settings::get::<i32>(c, "failed")?, None);
        Ok(())
    })
    .await
    .unwrap();
    let panic: ap_store::Result<()> = db
        .call(|c| {
            c.execute_batch("BEGIN; INSERT INTO settings VALUES ('panicked', '1')")?;
            panic!("injected")
        })
        .await;
    assert!(matches!(panic, Err(StoreError::Panicked)));
    db.call(|c| {
        assert!(c.is_autocommit());
        assert_eq!(settings::get::<i32>(c, "panicked")?, None);
        settings::put(c, "after", &7)
    })
    .await
    .unwrap();
    assert_eq!(
        db.read(|c| settings::get::<i32>(c, "after")).await.unwrap(),
        Some(7)
    );
}

#[tokio::test]
async fn read_pool_recovers_errors_panics_and_rejects_writes() {
    let env = TestEnv::new();
    let db = env.db().await;
    assert!(
        db.read(|c| settings::put(c, "forbidden", &1))
            .await
            .is_err()
    );
    let panic: ap_store::Result<()> = db.read(|_| panic!("reader injected")).await;
    assert!(matches!(panic, Err(StoreError::Panicked)));
    for _ in 0..10 {
        assert!(
            db.read(|_| Err::<(), _>(StoreError::Invalid("injected".into())))
                .await
                .is_err()
        );
        assert_eq!(
            db.read(|c| settings::get::<i32>(c, "forbidden"))
                .await
                .unwrap(),
            None
        );
    }
    let path = db.path().to_owned();
    tokio::task::spawn_blocking(move || db.call_blocking(|c| settings::put(c, "shutdown", &9)))
        .await
        .unwrap()
        .unwrap();
    let reopened = ap_store::Db::open(path).await.unwrap();
    assert_eq!(
        reopened
            .read(|c| settings::get::<i32>(c, "shutdown"))
            .await
            .unwrap(),
        Some(9)
    );
}

#[tokio::test]
async fn secrets_load_wait_delete_failure_retry_and_verbatim_token_json() {
    assert_eq!(ap_store::secrets::SERVICE, "any-player");
    assert_eq!(
        SecretAccount::ALL.map(SecretAccount::account),
        [
            "spotify-token",
            "jellyfin-url",
            "jellyfin-api-key",
            "plex-url",
            "plex-token",
            "sync-token"
        ]
    );
    // rspotify 0.16 serializes chrono timestamps as RFC3339 and renames scopes to scope.
    let token = r#"{"access_token":"access","expires_in":3600,"expires_at":"2024-05-01T12:00:00.123456789Z","refresh_token":"refresh","scope":"playlist-read-private user-read-email"}"#;
    let parsed: rspotify_model::Token = serde_json::from_str(token).unwrap();
    let serialized = serde_json::to_value(parsed).unwrap();
    assert_eq!(serialized["expires_at"], "2024-05-01T12:00:00.123456789Z");
    assert_eq!(serialized["expires_in"], 3600);
    let mut scopes: Vec<_> = serialized["scope"]
        .as_str()
        .unwrap()
        .split_whitespace()
        .collect();
    scopes.sort_unstable();
    assert_eq!(scopes, ["playlist-read-private", "user-read-email"]);
    assert!(serialized.get("scopes").is_none());
    let backend = Arc::new(
        MemoryBackend::new()
            .with(SecretAccount::SpotifyToken, token)
            .with(SecretAccount::PlexToken, "secret"),
    );
    let secrets = Secrets::new(backend.clone());
    assert!(!secrets.snapshot().loaded);
    let (loaded, waited) = tokio::join!(secrets.load(), secrets.await_loaded());
    assert_eq!(loaded, waited);

    assert_eq!(loaded.get(SecretAccount::SpotifyToken), Some(token));
    assert!(!format!("{secrets:?}").contains("access_token"));
    secrets.set(SecretAccount::PlexToken, None).await.unwrap();
    assert_eq!(backend.raw(SecretAccount::PlexToken), None);
    assert_eq!(secrets.get(SecretAccount::PlexToken), None);
    secrets.set(SecretAccount::PlexToken, None).await.unwrap();
    backend.set_failing(true);
    assert!(
        secrets
            .set(SecretAccount::SyncToken, Some("never persisted".into()))
            .await
            .is_err()
    );
    assert_eq!(backend.raw(SecretAccount::SyncToken), None);
    let failed = secrets.load().await;
    assert!(failed.loaded && failed.has_errors());
    assert_eq!(failed.errors.len(), 6);
    assert!(failed.values.is_empty());
    backend.set_failing(false);
    assert!(!secrets.load().await.has_errors());
    assert_eq!(
        secrets.get(SecretAccount::SpotifyToken).as_deref(),
        Some(token)
    );
}

#[tokio::test]
async fn secrets_backend_runs_off_runtime_thread() {
    use ap_store::{SecretBackend, SecretsError};
    struct ThreadBackend(std::thread::ThreadId);
    impl SecretBackend for ThreadBackend {
        fn get(&self, _: SecretAccount) -> Result<Option<String>, SecretsError> {
            assert_ne!(std::thread::current().id(), self.0);
            Ok(None)
        }
        fn set(&self, _: SecretAccount, _: &str) -> Result<(), SecretsError> {
            assert_ne!(std::thread::current().id(), self.0);
            Ok(())
        }
        fn delete(&self, _: SecretAccount) -> Result<(), SecretsError> {
            assert_ne!(std::thread::current().id(), self.0);
            Ok(())
        }
    }
    let secrets = Secrets::new(Arc::new(ThreadBackend(std::thread::current().id())));
    assert!(!secrets.load().await.has_errors());
    secrets
        .set(SecretAccount::SyncToken, Some("token".into()))
        .await
        .unwrap();
    secrets.set(SecretAccount::SyncToken, None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wal_reader_keeps_snapshot_while_writer_commits() {
    let env = TestEnv::new();
    let db = env.db().await;
    db.call(|c| settings::put(c, "snapshot", &1)).await.unwrap();
    let reader = db.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let read = tokio::spawn(async move {
        reader
            .read(move |c| {
                assert_eq!(settings::get::<i32>(c, "snapshot")?, Some(1));
                started_tx.send(()).unwrap();
                resume_rx.recv().unwrap();
                assert_eq!(settings::get::<i32>(c, "snapshot")?, Some(1));
                Ok(())
            })
            .await
            .unwrap();
    });
    started_rx.await.unwrap();
    db.call(|c| settings::put(c, "snapshot", &2)).await.unwrap();
    resume_tx.send(()).unwrap();
    read.await.unwrap();
    assert_eq!(
        db.read(|c| settings::get::<i32>(c, "snapshot"))
            .await
            .unwrap(),
        Some(2)
    );
}

#[tokio::test]
async fn secrets_partial_write_keeps_successes_without_exposing_failed_values() {
    use ap_store::{SecretBackend, SecretsError};
    struct Partial(MemoryBackend);
    impl SecretBackend for Partial {
        fn get(&self, account: SecretAccount) -> Result<Option<String>, SecretsError> {
            self.0.get(account)
        }
        fn set(&self, account: SecretAccount, value: &str) -> Result<(), SecretsError> {
            if account == SecretAccount::SyncToken {
                Err(SecretsError::Keyring {
                    account: account.account(),
                    message: "injected".into(),
                })
            } else {
                self.0.set(account, value)
            }
        }
        fn delete(&self, account: SecretAccount) -> Result<(), SecretsError> {
            self.0.delete(account)
        }
    }
    let backend = Arc::new(Partial(MemoryBackend::new()));
    let secrets = Secrets::new(backend.clone());
    secrets.load().await;
    assert!(
        secrets
            .set_many(vec![
                (SecretAccount::JellyfinUrl, Some("https://server".into())),
                (SecretAccount::SyncToken, Some("failed token".into())),
                (SecretAccount::PlexToken, Some("not attempted".into())),
            ])
            .await
            .is_err()
    );
    assert_eq!(
        secrets.get(SecretAccount::JellyfinUrl).as_deref(),
        Some("https://server")
    );
    assert_eq!(
        backend.0.raw(SecretAccount::JellyfinUrl).as_deref(),
        Some("https://server")
    );
    assert_eq!(secrets.get(SecretAccount::SyncToken), None);
    assert_eq!(backend.0.raw(SecretAccount::SyncToken), None);
    assert_eq!(backend.0.raw(SecretAccount::PlexToken), None);
}

#[tokio::test]
async fn settings_loaded_ranges_clamp_and_nonfinite_voice_uses_default() {
    let env = TestEnv::new();
    let db = env.db().await;
    db.call(|c| {
        settings::put(c, keys::PROVIDERS_JELLYFIN_PAGE_SIZE, &0)?;
        settings::put(c, keys::PROVIDERS_PLEX_PAGE_SIZE, &u32::MAX)?;
        settings::put(c, keys::DJ_VOICE_LEVEL, &0.1)?;
        Ok(())
    })
    .await
    .unwrap();
    let store = SettingsStore::open(db).await.unwrap();
    assert_eq!(store.get().providers.jellyfin_page_size, 1);
    assert_eq!(store.get().providers.plex_page_size, 1000);
    assert!((store.get().dj.voice_level - 0.5).abs() < f64::EPSILON);
    let normalized = store.update(|s| s.dj.voice_level = f64::NAN).await.unwrap();
    assert!((normalized.dj.voice_level - 1.0).abs() < f64::EPSILON);
}

#[test]
fn applying_null_track_sort_clears_existing_sort() {
    let mut state = settings::Settings::default();
    state.apply_entry(
        "ui.track_sort.table",
        json!({"column": "title", "ascending": true}),
    );
    assert!(state.ui.track_sort.contains_key("table"));
    state.apply_entry("ui.track_sort.table", serde_json::Value::Null);
    assert!(!state.ui.track_sort.contains_key("table"));
}
