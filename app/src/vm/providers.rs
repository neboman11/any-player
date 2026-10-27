//! Providers page view model. Snapshot updates are consumed by the main-thread presenter.
use super::{ProvidersCommand, ProvidersPort, ProvidersSnapshot, Result};
use std::sync::Arc;
use tokio::sync::watch;

pub struct ProvidersVm {
    pub port: Arc<dyn ProvidersPort>,
    pub state: watch::Receiver<ProvidersSnapshot>,
}
impl ProvidersVm {
    pub fn new(port: Arc<dyn ProvidersPort>) -> Self {
        let state = port.subscribe();
        Self { port, state }
    }
    pub async fn dispatch(&self, command: ProvidersCommand) -> Result {
        self.port.providers(command).await
    }
}
