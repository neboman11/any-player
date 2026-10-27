//! Dj page view model. Snapshot updates are consumed by the main-thread presenter.
use super::{DjCommand, DjPort, DjSnapshot, Result};
use std::sync::Arc;
use tokio::sync::watch;

pub struct DjVm {
    pub port: Arc<dyn DjPort>,
    pub state: watch::Receiver<DjSnapshot>,
}
impl DjVm {
    pub fn new(port: Arc<dyn DjPort>) -> Self {
        let state = port.subscribe();
        Self { port, state }
    }
    pub async fn dispatch(&self, command: DjCommand) -> Result {
        self.port.dj(command).await
    }
}
