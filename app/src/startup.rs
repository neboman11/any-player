//! Startup restores local state without playing and bounds provider restoration.
use crate::{
    services::ProductionServices,
    vm::{Result, ServiceError},
};
use ap_store::{paths::Paths, secrets::Secrets};
use std::sync::Arc;
#[derive(Clone, Debug, Default)]
pub struct StartupStatus {
    pub warnings: Vec<String>,
    pub sync_migration_banner: bool,
}
pub async fn boot(
    paths: &Paths,
    secrets: Arc<Secrets>,
) -> Result<(Arc<ProductionServices>, StartupStatus)> {
    let services = ProductionServices::build(paths, secrets).await?;
    let engine = services.engine.clone();
    let providers = services.providers.clone();
    let (restored, warnings) = tokio::join!(
        tokio::time::timeout(std::time::Duration::from_secs(10), engine.await_restored()),
        providers.restore()
    );
    let mut status = StartupStatus {
        warnings,
        sync_migration_banner: false,
    };
    if restored.is_err() {
        status.warnings.push("Playback restore timed out".into());
    }
    let no_sync = services.settings_store.get().sync.server_target.is_empty();
    status.sync_migration_banner = no_sync
        && services
            .db
            .read(|c| {
                Ok(ap_store::settings::migration_done(c, LEGACY_UPGRADE)?
                    && !ap_store::settings::migration_done(c, SYNC_BANNER_DISMISSED)?)
            })
            .await
            .map_err(|e| ServiceError(e.to_string()))?;
    Ok((services, status))
}
/// Set once when this data directory was written by the legacy (Tauri 0.1.x) desktop.
const LEGACY_UPGRADE: &str = "legacy_upgrade";
/// Set when the user dismisses the "sync settings didn't carry over" banner.
const SYNC_BANNER_DISMISSED: &str = "sync_banner_dismissed";
/// Whether `db` is an unmigrated legacy database (`user_version` 0 holding the legacy tables).
/// Probed read-only before [`ap_store::Db::open`] migrates it; any open error means "no" and
/// is reported by the real open.
pub(crate) fn legacy_schema_present(db: &std::path::Path) -> bool {
    use ap_store::rusqlite::{Connection, OpenFlags};
    if !db.exists() {
        return false;
    }
    let probe = || -> ap_store::rusqlite::Result<bool> {
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version != 0 {
            return Ok(false);
        }
        conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name IN \
             ('custom_playlists', 'playlist_tracks', 'union_playlist_sources', \
              'provider_playlist_preferences', 'column_preferences'))",
            [],
            |r| r.get(0),
        )
    };
    probe().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "could not probe for a legacy database");
        false
    })
}
pub(crate) async fn record_legacy_upgrade(db: &ap_store::Db) -> Result<()> {
    db.call(|c| ap_store::settings::mark_migration_done(c, LEGACY_UPGRADE))
        .await
        .map_err(|e| ServiceError(e.to_string()))
}
/// Persists the dismissal so the sync banner never returns.
pub async fn dismiss_sync_banner(db: &ap_store::Db) -> Result<()> {
    db.call(|c| ap_store::settings::mark_migration_done(c, SYNC_BANNER_DISMISSED))
        .await
        .map_err(|e| ServiceError(e.to_string()))
}
pub fn tracing(paths: &Paths) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::util::SubscriberInitExt;
    paths
        .ensure_dirs()
        .map_err(|e| ServiceError(e.to_string()))?;
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("any-player")
        .max_log_files(7)
        .build(paths.logs_dir())
        .map_err(|e| ServiceError(e.to_string()))?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(writer)
        .finish()
        .try_init();
    Ok(guard)
}
#[cfg(test)]
mod tests {
    use super::*;
    use ap_playback::ports::ConnectionGate;
    #[tokio::test]
    async fn real_graph_boots_migrates_restores_paused_and_gates_unconfigured_providers() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(temp.path());
        let secrets = Arc::new(Secrets::new(Arc::new(
            ap_store::secrets::MemoryBackend::new(),
        )));
        let (services, status) = boot(&paths, secrets).await.unwrap();
        assert!(status.warnings.is_empty());
        assert!(
            !status.sync_migration_banner,
            "fresh install shows no upgrade banner"
        );
        assert!(!services.engine.player().borrow().play_requested);
        assert!(services.engine.queue().borrow().entries.is_empty());
        assert!(
            services
                .providers
                .disabled_name(ap_model::SourceType::Jellyfin)
                .is_some()
        );
        assert!(
            services
                .providers
                .disabled_name(ap_model::SourceType::Plex)
                .is_some()
        );
        assert!(
            services
                .providers
                .disabled_name(ap_model::SourceType::Spotify)
                .is_some()
        );
        let version = services
            .db
            .read(|c| Ok(c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?))
            .await
            .unwrap();
        assert_eq!(version, 2);
        services
            .engine
            .set_volume(37, ap_playback::state::ChangeOrigin::User);
        services.engine.flush().await;
        assert_eq!(services.settings_store.get().player.volume, 37);
        drop(services);
        let (_, status) = boot(&paths, memory_secrets()).await.unwrap();
        assert!(
            !status.sync_migration_banner,
            "a v2 restart is not an upgrade"
        );
    }
    fn memory_secrets() -> Arc<Secrets> {
        Arc::new(Secrets::new(Arc::new(
            ap_store::secrets::MemoryBackend::new(),
        )))
    }
    #[tokio::test]
    async fn legacy_database_upgrade_shows_sync_banner_until_dismissed() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(temp.path());
        paths.ensure_dirs().unwrap();
        {
            let conn = ap_store::rusqlite::Connection::open(paths.db_path()).unwrap();
            conn.execute_batch(
                "CREATE TABLE custom_playlists (id TEXT PRIMARY KEY, name TEXT NOT NULL, \
                 description TEXT, image_url TEXT, created_at INTEGER NOT NULL, \
                 updated_at INTEGER NOT NULL, track_count INTEGER DEFAULT 0, \
                 playlist_type TEXT DEFAULT 'standard', is_distinct INTEGER DEFAULT 0);
                 INSERT INTO custom_playlists (id, name, created_at, updated_at) \
                 VALUES ('p1', 'Old mix', 1, 1);",
            )
            .unwrap();
        }
        let (services, status) = boot(&paths, memory_secrets()).await.unwrap();
        assert!(status.sync_migration_banner);
        drop(services);
        // Still pending on the next start: only dismissal (or a sync target) clears it.
        let (services, status) = boot(&paths, memory_secrets()).await.unwrap();
        assert!(status.sync_migration_banner);
        dismiss_sync_banner(&services.db).await.unwrap();
        drop(services);
        let (_, status) = boot(&paths, memory_secrets()).await.unwrap();
        assert!(!status.sync_migration_banner);
    }
    #[tokio::test]
    async fn legacy_playback_state_upgrade_shows_banner_unless_sync_is_configured() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(temp.path());
        paths.ensure_dirs().unwrap();
        std::fs::write(paths.legacy_playback_state(), r#"{"volume": 40}"#).unwrap();
        let (services, status) = boot(&paths, memory_secrets()).await.unwrap();
        assert!(status.sync_migration_banner);
        services
            .db
            .call(|c| {
                ap_store::settings::put(
                    c,
                    ap_store::settings::keys::SYNC_SERVER_TARGET,
                    &"https://sync.example",
                )
            })
            .await
            .unwrap();
        drop(services);
        let (_, status) = boot(&paths, memory_secrets()).await.unwrap();
        assert!(!status.sync_migration_banner);
    }
}
