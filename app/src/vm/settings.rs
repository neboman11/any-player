//! Settings page view model. Snapshot updates are consumed by the main-thread presenter.
use super::{Result, SettingsCommand, SettingsPort, SettingsSnapshot};
use std::sync::Arc;
use tokio::sync::watch;

pub struct SettingsVm {
    pub port: Arc<dyn SettingsPort>,
    pub state: watch::Receiver<SettingsSnapshot>,
}
impl SettingsVm {
    pub fn new(port: Arc<dyn SettingsPort>) -> Self {
        let state = port.subscribe();
        Self { port, state }
    }
    pub async fn dispatch(&self, command: SettingsCommand) -> Result {
        self.port.settings(command).await
    }
}
