/// Spotify Connect device resolution, playback control, and local-app
/// auto-launch.
///
/// Mirrors the Android app's `SpotifyConnectBridge`: playback is driven
/// entirely through the Web API's Connect endpoints (`/v1/me/player/*`),
/// targeting whichever device is active on the account - the same way the
/// official Spotify app's own "Devices" picker would - rather than an
/// in-process decoder or in-webview SDK. When no device is active, the local
/// Spotify desktop app is auto-launched so it registers itself as one.
use crate::models::Source;
use crate::providers::spotify::{
    spotify_connect_active_device_id, spotify_connect_pause, spotify_connect_playback_state,
    spotify_connect_resume, spotify_connect_seek, spotify_connect_set_volume,
    spotify_connect_start_playback, SpotifyProvider,
};
use crate::providers::{ProviderHandle, ProviderRegistry};
use rspotify::AuthCodePkceSpotify;
use std::fmt;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

/// Give the Spotify app a moment to register itself as a Connect device
/// after being auto-launched, before giving up on the current command.
const DEVICE_WAIT_ATTEMPTS: u32 = 5;
const DEVICE_WAIT_INTERVAL_MS: u64 = 1_500;

#[derive(Debug, Clone)]
pub enum SpotifyConnectError {
    NotAuthenticated,
    NoDevice(String),
    Api(String),
}

impl fmt::Display for SpotifyConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpotifyConnectError::NotAuthenticated => write!(f, "Spotify is not authenticated"),
            SpotifyConnectError::NoDevice(message) => write!(f, "{}", message),
            SpotifyConnectError::Api(message) => write!(f, "{}", message),
        }
    }
}

/// Attempts to launch the local Spotify desktop app so it registers itself as
/// a Connect device, mirroring the Android app's auto-launch of the mobile
/// app. Returns `false` if no launch mechanism succeeded (most likely because
/// Spotify isn't installed).
pub fn launch_spotify_app() -> bool {
    #[cfg(target_os = "macos")]
    {
        Command::new("open").args(["-a", "Spotify"]).spawn().is_ok()
    }
    #[cfg(target_os = "windows")]
    {
        // An empty title argument is required before the URI - `start` treats
        // the first quoted argument as the new console window's title.
        Command::new("cmd")
            .args(["/C", "start", "", "spotify:"])
            .spawn()
            .is_ok()
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("spotify").spawn().is_ok()
            || Command::new("xdg-open").arg("spotify:").spawn().is_ok()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        false
    }
}

pub struct SpotifyConnectBridge {
    providers: Arc<Mutex<ProviderRegistry>>,
}

impl SpotifyConnectBridge {
    pub fn new(providers: Arc<Mutex<ProviderRegistry>>) -> Self {
        Self { providers }
    }

    async fn provider_handle(&self) -> Result<ProviderHandle, SpotifyConnectError> {
        let providers = self.providers.lock().await;
        providers
            .get(Source::Spotify)
            .ok_or(SpotifyConnectError::NotAuthenticated)
    }

    /// Clone the rspotify client out from behind the provider lock and drop
    /// the lock immediately, so the actual (potentially multi-retry) HTTP
    /// request below doesn't hold the provider mutex and block unrelated
    /// commands (e.g. a poll tick retrying a transient error shouldn't be
    /// able to stall a user's pause/resume/seek click).
    async fn client(&self) -> Result<AuthCodePkceSpotify, SpotifyConnectError> {
        let handle = self.provider_handle().await?;
        let provider = handle.lock().await;
        let spotify = provider
            .as_any()
            .downcast_ref::<SpotifyProvider>()
            .ok_or(SpotifyConnectError::NotAuthenticated)?;
        spotify
            .connect_client()
            .map_err(|e| SpotifyConnectError::Api(e.to_string()))
    }

    async fn active_device_id(&self) -> Result<Option<String>, SpotifyConnectError> {
        let client = self.client().await?;
        spotify_connect_active_device_id(&client)
            .await
            .map_err(|e| SpotifyConnectError::Api(e.to_string()))
    }

    /// Resolve an active Connect device id, auto-launching the local Spotify
    /// app and retrying if none is active yet.
    async fn resolve_device_id(&self) -> Result<String, SpotifyConnectError> {
        if let Some(device_id) = self.active_device_id().await? {
            return Ok(device_id);
        }

        if !launch_spotify_app() {
            return Err(SpotifyConnectError::NoDevice(
                "Spotify isn't installed. Install it, open it, and try again.".to_string(),
            ));
        }

        for _ in 0..DEVICE_WAIT_ATTEMPTS {
            sleep(Duration::from_millis(DEVICE_WAIT_INTERVAL_MS)).await;
            if let Some(device_id) = self.active_device_id().await? {
                return Ok(device_id);
            }
        }

        Err(SpotifyConnectError::NoDevice(
            "No active Spotify Connect device found. Open Spotify on this computer (or another device on the same account) and try again.".to_string(),
        ))
    }

    /// Start playback of a single `spotify:track:...` id, auto-launching the
    /// local Spotify app to register a Connect device if none is active.
    ///
    /// `position_ms` is folded into the start-playback request itself (rather
    /// than a separate seek call afterward), since the device may not be
    /// ready to accept commands until playback has actually started.
    /// `volume_percent`, if given, is applied via a follow-up call once
    /// playback has started (failures there are logged and otherwise
    /// ignored, since playback already started successfully).
    pub async fn play_uri(
        &self,
        track_id: &str,
        position_ms: Option<i64>,
        volume_percent: Option<u8>,
    ) -> Result<(), SpotifyConnectError> {
        let device_id = self.resolve_device_id().await?;
        let client = self.client().await?;
        spotify_connect_start_playback(
            &client,
            &[track_id.to_string()],
            Some(device_id.as_str()),
            position_ms,
        )
        .await
        .map_err(|e| SpotifyConnectError::Api(e.to_string()))?;

        if let Some(volume_percent) = volume_percent {
            if let Err(error) =
                spotify_connect_set_volume(&client, volume_percent, Some(device_id.as_str())).await
            {
                tracing::warn!(
                    "Failed to set Spotify Connect volume on playback start: {}",
                    error
                );
            }
        }

        Ok(())
    }

    pub async fn resume(&self) -> Result<(), SpotifyConnectError> {
        let client = self.client().await?;
        spotify_connect_resume(&client, None)
            .await
            .map_err(|e| SpotifyConnectError::Api(e.to_string()))
    }

    pub async fn pause(&self) -> Result<(), SpotifyConnectError> {
        let client = self.client().await?;
        spotify_connect_pause(&client, None)
            .await
            .map_err(|e| SpotifyConnectError::Api(e.to_string()))
    }

    pub async fn seek(&self, position_ms: i64) -> Result<(), SpotifyConnectError> {
        let client = self.client().await?;
        spotify_connect_seek(&client, position_ms, None)
            .await
            .map_err(|e| SpotifyConnectError::Api(e.to_string()))
    }

    pub async fn set_volume(&self, volume_percent: u8) -> Result<(), SpotifyConnectError> {
        let client = self.client().await?;
        spotify_connect_set_volume(&client, volume_percent, None)
            .await
            .map_err(|e| SpotifyConnectError::Api(e.to_string()))
    }

    /// Poll the account's current Connect playback state. Returns `None` both
    /// on failure and when nothing is playing - callers can't distinguish the
    /// two from this alone, but both mean there's no state to report.
    pub async fn playback_state(&self) -> Option<rspotify::model::CurrentPlaybackContext> {
        let client = self.client().await.ok()?;
        spotify_connect_playback_state(&client).await.ok().flatten()
    }
}
