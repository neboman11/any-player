//! Persisted UI/player settings and interoperable transfer operations.
use super::error;
use crate::vm::{
    Result, SettingsCommand, SettingsPort, SettingsSnapshot, SortPreference, TransferKind,
    TransferRequest,
};
use ap_playback::{Engine, local::LocalBackend, state::ChangeOrigin};
use ap_store::{Db, settings::SettingsStore};
use async_trait::async_trait;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::watch;
pub struct SettingsAdapter {
    db: Db,
    store: Arc<SettingsStore>,
    engine: Engine,
    local: Arc<LocalBackend>,
    transfer: ap_library::transfer::TransferService,
    playlists: ap_library::PlaylistRepository,
    providers: Arc<super::providers::ProvidersAdapter>,
    state: watch::Sender<SettingsSnapshot>,
    tray: Arc<AtomicBool>,
}
impl SettingsAdapter {
    pub async fn new(
        db: Db,
        store: Arc<SettingsStore>,
        engine: Engine,
        local: Arc<LocalBackend>,
        transfer: ap_library::transfer::TransferService,
        playlists: ap_library::PlaylistRepository,
        providers: Arc<super::providers::ProvidersAdapter>,
    ) -> Result<Arc<Self>> {
        let adapter = Arc::new(Self {
            db,
            store,
            engine,
            local,
            transfer,
            playlists,
            providers,
            state: watch::Sender::new(SettingsSnapshot {
                normalize: false,
                output_device: String::new(),
                output_devices: vec![],
                close_to_tray: true,
                tray_available: false,
                columns: vec![],
                column_widths: BTreeMap::new(),
                sorts: BTreeMap::new(),
            }),
            tray: Arc::new(AtomicBool::new(false)),
        });
        adapter.refresh().await?;
        let this = adapter.clone();
        tokio::spawn(async move {
            let mut updates = this.store.subscribe();
            while updates.changed().await.is_ok() {
                if let Err(e) = this.refresh().await {
                    tracing::warn!(error=%e,"Settings refresh failed");
                }
            }
        });
        Ok(adapter)
    }
    pub fn set_tray_available(&self, available: bool) {
        self.tray.store(available, Ordering::Relaxed);
        self.state.send_modify(|s| s.tray_available = available);
    }
    async fn refresh(&self) -> Result {
        let s = self.store.get();
        let layout = self
            .db
            .read(ap_store::playlists::column_layout)
            .await
            .map_err(error)?;
        let output = self
            .db
            .read(|c| ap_store::settings::get::<String>(c, "player.output_device"))
            .await
            .map_err(error)?
            .unwrap_or_default();
        let known = [
            "index", "title", "artist", "album", "duration", "source", "actions",
        ];
        let columns = known
            .iter()
            .enumerate()
            .map(|(i, key)| ap_model::ColumnPreferences {
                key: (*key).into(),
                visible: layout.columns.iter().any(|v| v == key),
                position: i32::try_from(layout.columns.iter().position(|v| v == key).unwrap_or(i))
                    .unwrap_or(0),
            })
            .collect();
        self.state.send_replace(SettingsSnapshot {
            normalize: s.player.normalization_enabled,
            output_device: output,
            output_devices: self.local.output_devices().unwrap_or_default(),
            close_to_tray: s.ui.close_to_tray,
            tray_available: self.tray.load(Ordering::Relaxed),
            columns,
            column_widths: layout
                .column_widths
                .into_iter()
                .map(|(k, v)| (k, f32::from(i16::try_from(v).unwrap_or(i16::MAX))))
                .collect(),
            sorts: s
                .ui
                .track_sort
                .into_iter()
                .map(|(k, v)| {
                    (
                        k,
                        SortPreference {
                            column: v.column,
                            descending: !v.ascending,
                        },
                    )
                })
                .collect(),
        });
        Ok(())
    }
    fn playback_status(&self) -> ap_model::PlaybackStatus {
        let q = self.engine.queue().borrow().clone();
        let p = self.engine.player().borrow().clone();
        ap_model::PlaybackStatus {
            state: p.status,
            shuffle: q.shuffle,
            repeat_mode: q.repeat,
            volume: p.volume,
            current_track: q.current().map(|e| e.track.clone()),
            position: self.engine.position_ms(),
            duration: p.duration_ms,
            queue: q.entries.iter().map(|e| e.track.clone()).collect(),
            ordered_queue: q
                .order
                .iter()
                .map(|i| q.entries[*i].track.clone())
                .collect(),
            error_message: p.error,
        }
    }
}
#[async_trait]
impl SettingsPort for SettingsAdapter {
    fn subscribe(&self) -> watch::Receiver<SettingsSnapshot> {
        self.state.subscribe()
    }
    async fn settings(&self, command: SettingsCommand) -> Result {
        match command {
            SettingsCommand::Normalize(enabled) => {
                let s = self
                    .store
                    .update(|s| s.player.normalization_enabled = enabled)
                    .await
                    .map_err(error)?;
                self.engine
                    .set_normalization(enabled, s.player.normalization_strict);
            }
            SettingsCommand::Output(name) => {
                let stored = name.clone();
                self.db
                    .call(move |c| ap_store::settings::put(c, "player.output_device", &stored))
                    .await
                    .map_err(error)?;
                self.local
                    .select_output_device((!name.is_empty()).then_some(name))
                    .await;
            }
            SettingsCommand::CloseToTray(enabled) => {
                self.store
                    .update(|s| s.ui.close_to_tray = enabled)
                    .await
                    .map_err(error)?;
            }
            SettingsCommand::Sort { key, preference } => {
                self.store
                    .update(|s| {
                        if preference.column.is_empty() {
                            s.ui.track_sort.remove(&key);
                        } else {
                            s.ui.track_sort.insert(
                                key,
                                ap_store::settings::TrackSort {
                                    column: preference.column,
                                    ascending: !preference.descending,
                                },
                            );
                        }
                    })
                    .await
                    .map_err(error)?;
            }
            SettingsCommand::Columns(mut columns) => {
                columns.sort_by_key(|c| c.position);
                self.db
                    .call(move |c| {
                        let mut layout = ap_store::playlists::column_layout(c)?;
                        layout.columns = columns
                            .iter()
                            .filter(|c| c.visible)
                            .map(|c| c.key.clone())
                            .collect();
                        layout.column_order =
                            columns.iter().map(|c| i64::from(c.position)).collect();
                        ap_store::playlists::set_column_layout(c, &layout)
                    })
                    .await
                    .map_err(error)?;
            }
        }
        self.refresh().await
    }
    async fn transfer(&self, r: TransferRequest) -> Result<String> {
        use ap_library::transfer::{ExportMode, MergePolicy};
        let pass = (!r.passphrase.is_empty()).then_some(r.passphrase.as_str());
        let policy = if r.replace {
            MergePolicy::ReplaceAll
        } else {
            MergePolicy::MergePreferImport
        };
        match r.kind {
            TransferKind::ExportState | TransferKind::ExportConfig => {
                let raw = if matches!(r.kind, TransferKind::ExportConfig) {
                    self.transfer.export_config().await
                } else {
                    self.transfer
                        .export_state(
                            if r.private {
                                ExportMode::Private
                            } else {
                                ExportMode::Portable
                            },
                            pass,
                            Some(self.playback_status()),
                            env!("CARGO_PKG_VERSION"),
                        )
                        .await
                }
                .map_err(error)?;
                tokio::fs::write(r.path, raw).await.map_err(error)?;
                Ok("Export complete".into())
            }
            TransferKind::ImportState | TransferKind::ImportConfig => {
                let raw = tokio::fs::read_to_string(r.path).await.map_err(error)?;
                let summary = if matches!(r.kind, TransferKind::ImportConfig) {
                    self.transfer.import_config(&raw, policy, r.dry_run).await
                } else {
                    self.transfer
                        .import_state(&raw, policy, pass, r.dry_run)
                        .await
                }
                .map_err(error)?;
                if !r.dry_run {
                    if let Some(s) = &summary.playback_settings {
                        self.engine.set_volume(s.volume, ChangeOrigin::User);
                        self.engine.set_shuffle(s.shuffle, ChangeOrigin::User);
                        self.engine.set_repeat(s.repeat, ChangeOrigin::User);
                    }
                    self.store.reload().await.map_err(error)?;
                    self.playlists.reload().await.map_err(error)?;
                    self.providers.restore().await;
                    self.refresh().await?;
                }
                Ok(summary.format(if r.dry_run { "Dry run" } else { "Import" }))
            }
        }
    }
}
