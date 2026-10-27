//! External dependencies of the engine and backends, wired by the app (ap-store, ap-providers).

use ap_model::{QueueState, RepeatMode, SourceType, Track};
use async_trait::async_trait;
use reqwest::Url;
use reqwest::header::HeaderMap;
use tokio::sync::watch;

/// Error type of the ports.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Player flags and position persisted in the `settings` table (design §6.3, `player.*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerSettings {
    /// `player.volume`, 0..=100.
    pub volume: i32,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    /// `player.position_ms`.
    pub position_ms: i64,
    pub normalization_enabled: bool,
    /// Not shown in the UI; stored so the sync settings namespace round-trips Android's key.
    pub normalization_strict: bool,
}

impl Default for PlayerSettings {
    fn default() -> Self {
        Self {
            volume: 50,
            shuffle: false,
            repeat: RepeatMode::Off,
            position_ms: 0,
            normalization_enabled: false,
            normalization_strict: false,
        }
    }
}

/// Queue, cursor, player settings and the per-track gain cache (ap-store).
#[async_trait]
pub trait PlaybackPersistence: Send + Sync + 'static {
    /// The persisted queue (`queue_entries` + `queue_meta`); an empty state when none.
    async fn load_queue(&self) -> Result<QueueState, BoxError>;
    /// Replaces the persisted queue content and cursor with `queue` (one transaction).
    async fn save_queue_content(&self, queue: &QueueState) -> Result<(), BoxError>;
    /// Updates the cursor only when the stored `content_version` equals `content_version`.
    async fn save_cursor(
        &self,
        cursor_uid: Option<u64>,
        content_version: u64,
    ) -> Result<(), BoxError>;
    /// The persisted player settings (defaults for missing keys).
    async fn load_settings(&self) -> Result<PlayerSettings, BoxError>;
    async fn save_settings(&self, settings: &PlayerSettings) -> Result<(), BoxError>;
    /// Cached loudness gain for `track_key` (`${SOURCE}:${trackId}`).
    async fn track_gain(&self, track_key: &str) -> Option<f32>;
    async fn put_track_gain(&self, track_key: &str, gain: f32);
}

/// Provider connection state for the playback gate.
pub trait ConnectionGate: Send + Sync + 'static {
    /// `None` when `source` may play; otherwise the provider name shown in the disabled reason
    /// ("Playback disabled: <Name> is not configured/authenticated…"). Must fail open.
    fn disabled_name(&self, source: SourceType) -> Option<String>;
    /// Ticks whenever the connection state changes.
    fn subscribe(&self) -> watch::Receiver<()>;
}

/// A stream request for a local item: URL plus request-time headers (Jellyfin/Plex auth).
#[derive(Debug, Clone)]
pub struct StreamRequest {
    pub url: Url,
    pub headers: HeaderMap,
    /// Accept invalid/self-signed TLS certificates for this request. Only set when the user
    /// enabled "Allow self-signed certificate" for the connection (design §7, Plex).
    pub allow_invalid_certs: bool,
}

/// Resolves a track to its stream request (ap-providers `stream_request`).
#[async_trait]
pub trait StreamResolver: Send + Sync + 'static {
    async fn resolve(&self, track: &Track) -> Result<StreamRequest, BoxError>;
}

/// `${SOURCE}:${trackId}` (Kotlin enum name, e.g. `JELLYFIN:abc`): the audio-cache and track-gain key.
#[must_use]
pub fn track_key(track: &Track) -> String {
    format!("{}:{}", track.source.name(), track.id)
}
