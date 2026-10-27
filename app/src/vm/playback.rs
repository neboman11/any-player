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
/// Tracks the displayed error independently from the engine's persistent error field.
#[derive(Default)]
pub struct PlaybackNotice {
    message: String,
    last_error: Option<String>,
    last_event: Option<String>,
    dismissed: Option<String>,
}

impl PlaybackNotice {
    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn dismiss(&mut self) {
        self.dismissed = self.last_error.clone();
        self.message.clear();
    }

    pub fn observe(&mut self, error: Option<&str>, playing: bool) {
        if playing {
            self.message.clear();
            self.last_error = None;
            self.last_event = None;
            self.dismissed = None;
        } else if let Some(error) = error
            && self.dismissed.as_deref() != Some(error)
        {
            self.message = friendly_error(error);
            self.last_error = Some(error.to_owned());
        }
    }

    pub fn event(&mut self, error: &str) -> Option<String> {
        if self.last_event.as_deref() == Some(error) {
            return None;
        }
        self.last_event = Some(error.to_owned());
        self.last_error = Some(error.to_owned());
        self.dismissed = None;
        self.message = friendly_error(error);
        Some(self.message.clone())
    }
}

fn friendly_error(error: &str) -> String {
    if error.contains("Spotify playback stalled while reporting playing") {
        "Spotify accepted the command but the device didn't start playing. Open the Spotify app or click play once in the web player, then try again.".into()
    } else {
        error.to_owned()
    }
}

/// Once resolved during a session the migration reminder must not return.
pub struct SyncMigrationBanner(bool);

impl SyncMigrationBanner {
    pub fn new(visible: bool) -> Self {
        Self(visible)
    }

    pub fn visible(&self) -> bool {
        self.0
    }

    pub fn observe_target(&mut self, target: &str) {
        if !target.is_empty() {
            self.0 = false;
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_errors_are_dismissible_deduplicated_and_clear_on_success() {
        let mut notice = PlaybackNotice::default();
        let raw = "Spotify HTTP 0: Spotify playback stalled while reporting playing";
        assert!(notice.event(raw).is_some());
        assert!(notice.message().contains("device didn't start playing"));
        assert!(notice.event(raw).is_none());
        notice.dismiss();
        notice.observe(Some(raw), false);
        assert_eq!(notice.message(), "");
        notice.observe(None, true);
        assert!(notice.event(raw).is_some());
        notice.observe(None, true);
        assert_eq!(notice.message(), "");
    }

    #[test]
    fn migration_banner_stays_hidden_after_setting_target() {
        let mut banner = SyncMigrationBanner::new(true);
        assert!(banner.visible());
        banner.observe_target("");
        assert!(banner.visible());
        banner.observe_target("https://sync.example");
        assert!(!banner.visible());
        banner.observe_target("");
        assert!(!banner.visible());
    }
}
