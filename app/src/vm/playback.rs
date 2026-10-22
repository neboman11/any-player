//! Playback page view model. Snapshot updates are consumed by the main-thread presenter.
use super::{PlaybackCommand, PlaybackPort, PlaybackSnapshot, Result};
use std::sync::Arc;
use tokio::sync::watch;

pub struct PlaybackVm {
    pub port: Arc<dyn PlaybackPort>,
    pub state: watch::Receiver<PlaybackSnapshot>,
}
impl PlaybackVm {
    pub fn new(port: Arc<dyn PlaybackPort>) -> Self {
        let state = port.subscribe();
        Self { port, state }
    }
    pub async fn dispatch(&self, command: PlaybackCommand) -> Result {
        self.port.playback(command).await
    }
}
