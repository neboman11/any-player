//! Sync page view model. Snapshot updates are consumed by the main-thread presenter.
use super::{Result, SyncCommand, SyncPort, SyncSnapshot};
use std::sync::Arc;
use tokio::sync::watch;

pub struct SyncVm {
    pub port: Arc<dyn SyncPort>,
    pub state: watch::Receiver<SyncSnapshot>,
}
impl SyncVm {
    pub fn new(port: Arc<dyn SyncPort>) -> Self {
        let state = port.subscribe();
        Self { port, state }
    }
    pub async fn dispatch(&self, command: SyncCommand) -> Result {
        self.port.sync(command).await
    }
}
