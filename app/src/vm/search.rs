//! Search page view model. Snapshot updates are consumed by the main-thread presenter.
use super::{Result, SearchPort, SearchRequest, SearchSnapshot};
use std::sync::Arc;
use tokio::sync::watch;

pub struct SearchVm {
    pub port: Arc<dyn SearchPort>,
    pub state: watch::Receiver<SearchSnapshot>,
}
impl SearchVm {
    pub fn new(port: Arc<dyn SearchPort>) -> Self {
        let state = port.subscribe();
        Self { port, state }
    }
    pub async fn dispatch(&self, command: SearchRequest) -> Result {
        self.port.search(command).await
    }
}
