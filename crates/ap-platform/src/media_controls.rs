//! OS media metadata and command adapter.
use souvlaki::{
    MediaControlEvent, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection,
};
use std::{ffi::c_void, time::Duration};
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub enum MediaCommand {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Seek {
        forward: bool,
        amount: Option<Duration>,
    },
    SetPosition(Duration),
    Volume(f64),
}
#[allow(clippy::needless_pass_by_value)]
pub fn map_command(event: MediaControlEvent) -> Option<MediaCommand> {
    match event {
        MediaControlEvent::Play => Some(MediaCommand::Play),
        MediaControlEvent::Pause => Some(MediaCommand::Pause),
        MediaControlEvent::Toggle => Some(MediaCommand::Toggle),
        MediaControlEvent::Next => Some(MediaCommand::Next),
        MediaControlEvent::Previous => Some(MediaCommand::Previous),
        MediaControlEvent::Seek(direction) => Some(MediaCommand::Seek {
            forward: direction == SeekDirection::Forward,
            amount: None,
        }),
        MediaControlEvent::SeekBy(direction, amount) => Some(MediaCommand::Seek {
            forward: direction == SeekDirection::Forward,
            amount: Some(amount),
        }),
        MediaControlEvent::SetPosition(MediaPosition(position)) => {
            Some(MediaCommand::SetPosition(position))
        }
        MediaControlEvent::SetVolume(volume) => Some(MediaCommand::Volume(volume)),
        MediaControlEvent::Stop
        | MediaControlEvent::OpenUri(_)
        | MediaControlEvent::Raise
        | MediaControlEvent::Quit => None,
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Paused(Duration),
    Playing(Duration),
}
pub fn map_playback(state: PlaybackState) -> MediaPlayback {
    match state {
        PlaybackState::Stopped => MediaPlayback::Stopped,
        PlaybackState::Paused(position) => MediaPlayback::Paused {
            progress: Some(MediaPosition(position)),
        },
        PlaybackState::Playing(position) => MediaPlayback::Playing {
            progress: Some(MediaPosition(position)),
        },
    }
}
pub struct MediaControls(souvlaki::MediaControls);
impl MediaControls {
    pub fn new(
        hwnd: Option<*mut c_void>,
        commands: mpsc::UnboundedSender<MediaCommand>,
    ) -> Result<Self, souvlaki::Error> {
        let mut controls = souvlaki::MediaControls::new(PlatformConfig {
            dbus_name: "any_player",
            display_name: "Any Player",
            hwnd,
        })?;
        controls.attach(move |event| {
            if let Some(command) = map_command(event) {
                let _ = commands.send(command);
            }
        })?;
        Ok(Self(controls))
    }
    pub fn set_metadata(&mut self, metadata: MediaMetadata<'_>) -> Result<(), souvlaki::Error> {
        self.0.set_metadata(metadata)
    }
    pub fn set_playback(&mut self, playback: PlaybackState) -> Result<(), souvlaki::Error> {
        self.0.set_playback(map_playback(playback))
    }
    /// Publishes the player volume. Only MPRIS (Linux) has a volume property; Windows SMTC and
    /// macOS Now Playing expose none, so there the call succeeds without effect.
    #[cfg(target_os = "linux")]
    pub fn set_volume(&mut self, volume: f64) -> Result<(), souvlaki::Error> {
        self.0.set_volume(volume)
    }
    /// Publishes the player volume. Only MPRIS (Linux) has a volume property; Windows SMTC and
    /// macOS Now Playing expose none, so there the call succeeds without effect.
    #[cfg(not(target_os = "linux"))]
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn set_volume(&mut self, _volume: f64) -> Result<(), souvlaki::Error> {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_playback_and_commands() {
        let position = Duration::from_secs(5);
        assert_eq!(
            map_playback(PlaybackState::Playing(position)),
            MediaPlayback::Playing {
                progress: Some(MediaPosition(position))
            }
        );
        assert_eq!(
            map_playback(PlaybackState::Paused(position)),
            MediaPlayback::Paused {
                progress: Some(MediaPosition(position))
            }
        );
        assert_eq!(map_playback(PlaybackState::Stopped), MediaPlayback::Stopped);
        assert_eq!(
            map_command(MediaControlEvent::SeekBy(SeekDirection::Backward, position)),
            Some(MediaCommand::Seek {
                forward: false,
                amount: Some(position)
            })
        );
        assert_eq!(
            map_command(MediaControlEvent::SetPosition(MediaPosition(position))),
            Some(MediaCommand::SetPosition(position))
        );
        assert_eq!(
            map_command(MediaControlEvent::SetVolume(0.5)),
            Some(MediaCommand::Volume(0.5))
        );
    }
}
