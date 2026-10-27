//! DJ coordinator is constructed only after first show or playback.
use super::{error, persistence::Persistence};
use crate::vm::{DjCommand, DjPort, DjSnapshot, Result, ServiceError};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::{Mutex, watch};
pub struct DjAdapter {
    controller: Mutex<Option<ap_dj::DjController>>,
    state: watch::Sender<DjSnapshot>,
    engine: ap_playback::Engine,
    persistence: Arc<Persistence>,
    root: std::path::PathBuf,
    secrets: Arc<ap_store::secrets::Secrets>,
}
impl DjAdapter {
    pub fn new(
        engine: ap_playback::Engine,
        persistence: Arc<Persistence>,
        root: std::path::PathBuf,
        secrets: Arc<ap_store::secrets::Secrets>,
    ) -> Arc<Self> {
        let s = persistence.settings.get().dj;
        Arc::new(Self {
            controller: Mutex::new(None),
            state: watch::Sender::new(DjSnapshot {
                enabled: s.enabled,
                show_queue: s.show_entries_in_queue,
                level: super::voice_level(s.voice_level),
                voices: vec![],
                voice: s.voice_id.unwrap_or_default(),
                status: "AI DJ needs a sync server configured in Settings → General.".into(),
                songs_away: 0,
                debug: s.debug_logging,
                logs: vec![],
            }),
            engine,
            persistence,
            root,
            secrets,
        })
    }
    pub async fn start(self: &Arc<Self>) -> Result {
        let mut guard = self.controller.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        let account = |settings: &ap_store::settings::Settings,
                       secrets: &ap_store::secrets::SecretsState| {
            ap_dj::DjAccount::new(
                &settings.sync.server_target,
                secrets
                    .get(ap_store::secrets::SecretAccount::SyncToken)
                    .unwrap_or_default(),
            )
        };
        let (account_tx, account_rx) = watch::channel(account(
            &self.persistence.settings.get(),
            &self.secrets.subscribe().borrow(),
        ));
        let controller = ap_dj::DjCoordinator::spawn(ap_dj::DjCoordinatorDeps {
            engine: self.engine.clone(),
            root: self.root.clone(),
            settings: self.persistence.clone(),
            passage_state: self.persistence.clone(),
            account: account_rx,
            diagnostics: ap_dj::DjDiagnostics::new(),
        })
        .await
        .map_err(error)?;
        controller.set_debug_logging(self.persistence.settings.get().dj.debug_logging);
        let mut state = controller.subscribe();
        let diagnostics = controller.diagnostics();
        let mut logs = diagnostics.subscribe();
        let this = self.clone();
        tokio::spawn(async move {
            let mut settings = this.persistence.settings.subscribe();
            let mut secrets = this.secrets.subscribe();
            loop {
                let account_changed = tokio::select! {
                    r = state.changed() => if r.is_err() { break } else { false },
                    r = logs.changed() => if r.is_err() { break } else { false },
                    r = settings.changed() => if r.is_err() { break } else { true },
                    r = secrets.changed() => if r.is_err() { break } else { true },
                };
                if account_changed {
                    account_tx.send_replace(account(&settings.borrow(), &secrets.borrow()));
                }
                let s = state.borrow().clone();
                this.state.send_replace(DjSnapshot {
                    enabled: s.settings.enabled,
                    show_queue: s.settings.show_entries_in_queue,
                    level: s.settings.voice_level,
                    voices: s
                        .voice_catalog
                        .voices
                        .iter()
                        .map(|v| v.id.clone())
                        .collect(),
                    voice: s.settings.selected_voice_id.unwrap_or_default(),
                    status: s.message.unwrap_or_else(|| format!("{:?}", s.preparation)),
                    songs_away: usize::try_from(s.songs_away.unwrap_or(0)).unwrap_or(0),
                    debug: diagnostics.debug_logging(),
                    logs: logs
                        .borrow()
                        .iter()
                        .map(|e| format!("{} {}: {}", e.timestamp, e.tag, e.message))
                        .collect(),
                });
            }
        });
        *guard = Some(controller);
        Ok(())
    }
}
#[async_trait]
impl DjPort for DjAdapter {
    fn subscribe(&self) -> watch::Receiver<DjSnapshot> {
        self.state.subscribe()
    }
    async fn dj(&self, c: DjCommand) -> Result {
        let guard = self.controller.lock().await;
        let Some(controller) = guard.as_ref() else {
            return Err(ServiceError(
                "AI DJ starts after the window is shown".into(),
            ));
        };
        match c {
            DjCommand::Enable(v) => controller.set_enabled(v),
            DjCommand::ShowQueue(v) => controller.set_show_entries_in_queue(v),
            DjCommand::Level(v) => controller.set_voice_level(v),
            DjCommand::Voice(v) => controller.select_voice(v),
            DjCommand::Debug(v) => {
                controller.set_debug_logging(v);
                self.persistence
                    .settings
                    .update(|s| s.dj.debug_logging = v)
                    .await
                    .map_err(error)?;
            }
            DjCommand::ClearLogs => controller.clear_logs(),
        }
        Ok(())
    }
}
