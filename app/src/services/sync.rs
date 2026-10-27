//! Sync UI observes only committed settings and account state.
use super::error;
use crate::vm::{Result, SyncChoice, SyncCommand, SyncPort, SyncSnapshot};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::watch;
pub struct SyncAdapter {
    engine: ap_sync::SyncEngine,
    settings: Arc<ap_store::settings::SettingsStore>,
    secrets: Arc<ap_store::secrets::Secrets>,
    state: watch::Sender<SyncSnapshot>,
}
impl SyncAdapter {
    pub fn new(
        engine: ap_sync::SyncEngine,
        settings: Arc<ap_store::settings::SettingsStore>,
        secrets: Arc<ap_store::secrets::Secrets>,
    ) -> Arc<Self> {
        let this = Arc::new(Self {
            engine,
            settings,
            secrets,
            state: watch::Sender::new(SyncSnapshot::default()),
        });
        this.publish();
        let task = this.clone();
        tokio::spawn(async move {
            let mut state = task.engine.state();
            let mut settings = task.settings.subscribe();
            let mut secrets = task.secrets.subscribe();
            loop {
                tokio::select! {
                    r = state.changed() => if r.is_err() { break },
                    r = settings.changed() => if r.is_err() { break },
                    r = secrets.changed() => if r.is_err() { break },
                }
                task.publish();
            }
        });
        this
    }
    fn publish(&self) {
        let s = self.settings.get().sync;
        let state = self.engine.state().borrow().clone();
        self.state.send_replace(SyncSnapshot {
            url: s.server_target,
            token: self
                .secrets
                .get(ap_store::secrets::SecretAccount::SyncToken)
                .unwrap_or_default(),
            domains: [
                s.domains.app_state,
                s.domains.playlists,
                s.domains.provider_configuration,
                s.domains.settings,
            ],
            connected: state.confirmed,
            conflict: state.conflict_pending,
            overwrite: state.playlist_confirmation_required,
            status: state.status,
            import_status: state.import_status.unwrap_or_default(),
        });
    }
}
#[async_trait]
impl SyncPort for SyncAdapter {
    fn subscribe(&self) -> watch::Receiver<SyncSnapshot> {
        self.state.subscribe()
    }
    async fn sync(&self, c: SyncCommand) -> Result {
        match c {
            SyncCommand::Commit { url, token } => {
                let target = ap_sync::SyncTarget::new(&url, &token);
                self.secrets
                    .set(
                        ap_store::secrets::SecretAccount::SyncToken,
                        Some(target.auth_token),
                    )
                    .await
                    .map_err(error)?;
                self.settings
                    .update(|s| s.sync.server_target = target.server_target)
                    .await
                    .map_err(error)?;
            }
            SyncCommand::Domain { index, enabled } => {
                self.settings
                    .update(|s| match index {
                        0 => s.sync.domains.app_state = enabled,
                        1 => s.sync.domains.playlists = enabled,
                        2 => s.sync.domains.provider_configuration = enabled,
                        3 => s.sync.domains.settings = enabled,
                        _ => {}
                    })
                    .await
                    .map_err(error)?;
            }
            SyncCommand::Connect => self.engine.connect(),
            SyncCommand::Pull => self.engine.pull(self.state.borrow().overwrite),
            SyncCommand::Resolve(choice) => match choice {
                SyncChoice::Server => self.engine.resolve_conflict(false),
                SyncChoice::Device => self.engine.resolve_conflict(true),
                SyncChoice::Cancel => self.engine.dismiss_conflict(),
            },
        }
        self.publish();
        Ok(())
    }
}
