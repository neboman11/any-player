//! Production adapters: no Slint state, SQL or audio types escape through UI ports.
pub mod dj;
pub mod library;
pub mod persistence;
pub mod playback;
pub mod providers;
pub mod settings;
pub mod spotify;
pub mod sync;
#[cfg(test)]
mod tests;
use crate::vm::{self, Result, ServiceError};
use ap_playback::{
    Engine,
    engine::EngineDeps,
    local::{CpalSink, LocalBackend, http::AudioCache},
};
use ap_store::{Db, paths::Paths, secrets::Secrets, settings::SettingsStore};
use std::sync::Arc;
pub(crate) fn error(e: impl std::fmt::Display) -> ServiceError {
    ServiceError(e.to_string())
}
/// Stored DJ voice levels live in `VOICE_LEVEL_RANGE` (0.5–2.0), so narrowing to `f32` is lossless enough.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn voice_level(stored: f64) -> f32 {
    let range = ap_store::settings::VOICE_LEVEL_RANGE;
    stored.clamp(*range.start(), *range.end()) as f32
}
pub struct ProductionServices {
    pub engine: Engine,
    pub db: Db,
    pub settings_store: Arc<SettingsStore>,
    pub settings: Arc<settings::SettingsAdapter>,
    pub providers: Arc<providers::ProvidersAdapter>,
    playback: Arc<playback::PlaybackAdapter>,
    library: Arc<library::LibraryAdapter>,
    search: Arc<library::SearchAdapter>,
    dj: Arc<dj::DjAdapter>,
    sync: Arc<sync::SyncAdapter>,
    sync_engine: ap_sync::SyncEngine,
    catalog: ap_library::CatalogRepository,
    deferred: std::sync::atomic::AtomicBool,
}
impl ProductionServices {
    pub async fn build(paths: &Paths, secrets: Arc<Secrets>) -> Result<Arc<Self>> {
        paths.ensure_dirs().map_err(error)?;
        let legacy_db = crate::startup::legacy_schema_present(&paths.db_path());
        let db = Db::open(paths.db_path()).await.map_err(error)?;
        let report = ap_store::legacy::run_first_run_migrations(&db, paths)
            .await
            .map_err(error)?;
        if legacy_db
            || matches!(
                report.playback_state,
                ap_store::legacy::PlaybackStateOutcome::Migrated { .. }
                    | ap_store::legacy::PlaybackStateOutcome::Undecodable(_)
            )
        {
            crate::startup::record_legacy_upgrade(&db).await?;
        }
        let settings_store = Arc::new(SettingsStore::open(db.clone()).await.map_err(error)?);
        let catalog = ap_library::CatalogRepository::new(db.clone(), vec![])
            .await
            .map_err(error)?;
        let playlists = ap_library::PlaylistRepository::new(db.clone(), catalog.clone())
            .await
            .map_err(error)?;
        let providers = providers::ProvidersAdapter::new(
            secrets.clone(),
            settings_store.clone(),
            db.clone(),
            catalog.clone(),
        );
        let persistence = Arc::new(persistence::Persistence {
            db: db.clone(),
            settings: settings_store.clone(),
        });
        let local = Arc::new(
            LocalBackend::new(
                providers.clone(),
                persistence.clone(),
                AudioCache::new(paths.audio_cache_dir()).map_err(error)?,
                Arc::new(CpalSink),
            )
            .map_err(error)?,
        );
        let saved_output = db
            .read(|c| ap_store::settings::get::<String>(c, "player.output_device"))
            .await
            .map_err(error)?;
        local
            .select_output_device(saved_output.filter(|s| !s.is_empty()))
            .await;
        let api = Arc::new(spotify::SpotifyApi::new(providers.tokens.clone()));
        let spotify = Arc::new(ap_playback::spotify::SpotifyBackend::new(
            api,
            providers.devices.clone(),
        ));
        let engine = Engine::spawn(EngineDeps {
            backends: vec![local.clone(), spotify],
            persistence: persistence.clone(),
            gate: providers.clone(),
        });
        let transfer = ap_library::transfer::TransferService::new(
            db.clone(),
            Arc::new(providers::Connections {
                secrets: secrets.clone(),
            }),
        );
        let settings = settings::SettingsAdapter::new(
            db.clone(),
            settings_store.clone(),
            engine.clone(),
            local,
            transfer.clone(),
            playlists.clone(),
            providers.clone(),
        )
        .await?;
        let library = Arc::new(
            library::LibraryAdapter::new(catalog.clone(), playlists.clone(), engine.clone())
                .await?,
        );
        let search = Arc::new(library::SearchAdapter::new(catalog.clone()));
        let dj = dj::DjAdapter::new(
            engine.clone(),
            persistence,
            paths.dj_filler_dir(),
            secrets.clone(),
        );
        let sync_engine = ap_sync::SyncEngine::spawn(ap_sync::SyncDeps {
            engine: engine.clone(),
            db: db.clone(),
            settings: settings_store.clone(),
            secrets: secrets.clone(),
            transfer,
            playlists,
            catalog: catalog.clone(),
            providers: providers.clone(),
            network: ap_sync::network_availability(),
        });
        let sync = sync::SyncAdapter::new(sync_engine.clone(), settings_store.clone(), secrets);
        Ok(Arc::new(Self {
            playback: playback::PlaybackAdapter::new(engine.clone()),
            engine,
            db,
            settings_store,
            settings,
            providers,
            library,
            search,
            dj,
            sync,
            sync_engine,
            catalog,
            deferred: std::sync::atomic::AtomicBool::new(false),
        }))
    }
    pub fn ports(&self) -> vm::Services {
        vm::Services {
            playback: self.playback.clone(),
            library: self.library.clone(),
            search: self.search.clone(),
            settings: self.settings.clone(),
            providers: self.providers.clone(),
            dj: self.dj.clone(),
            sync: self.sync.clone(),
        }
    }
    pub fn start_deferred(self: &Arc<Self>) {
        if self
            .deferred
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this.dj.start().await {
                tracing::warn!(error=%e,"DJ startup failed");
            }
            this.sync_engine.start();
            match this.catalog.refresh_all().await {
                Ok(failures) => {
                    for failure in failures {
                        tracing::warn!(source=?failure.source,error=%failure.message,"Catalog refresh failed");
                    }
                }
                Err(e) => tracing::warn!(error=%e,"Catalog refresh failed"),
            }
        });
    }
}
