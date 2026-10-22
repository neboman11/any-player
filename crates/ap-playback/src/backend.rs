//! The backend contract (Android `PlaybackBackend`): a backend plays one [`BackendRun`] at a time
//! while the engine owns the order.

use ap_model::{BackendKind, Track};
use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::state::ChangeOrigin;

/// Identifies a run item independent of its index: a queue entry uid or a DJ break id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ItemKey {
    Song(u64),
    Break(String),
}

/// One item of a run as handed to a backend: the planned item with its track resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunItem {
    Song {
        uid: u64,
        track: Track,
    },
    /// A DJ break; `path` is a filesystem path or a `file://` URL of a WAV file.
    Break {
        break_id: String,
        path: String,
    },
}

impl RunItem {
    #[must_use]
    pub fn key(&self) -> ItemKey {
        match self {
            Self::Song { uid, .. } => ItemKey::Song(*uid),
            Self::Break { break_id, .. } => ItemKey::Break(break_id.clone()),
        }
    }
}

/// A run with tracks resolved (`ap_model::Run` carries only uids and order positions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRun {
    pub kind: BackendKind,
    pub items: Vec<RunItem>,
}

/// Every event carries the `load_id` of the load that produced it; the engine drops stale ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendEvent {
    ItemStarted {
        load_id: u64,
        key: ItemKey,
    },
    /// `is_playing == false` with `by_player == true`: the pause originated outside the engine
    /// (device lost, external Spotify pause) and is adopted into intent.
    PlayingChanged {
        load_id: u64,
        is_playing: bool,
        buffering: bool,
        by_player: bool,
    },
    /// Position jumped or drifted; the engine re-anchors from `position_ms`.
    PositionDiscontinuity {
        load_id: u64,
        position_ms: i64,
        duration_ms: i64,
    },
    /// The last item of the run (or the stop point set by `update_upcoming`) finished.
    RunEnded {
        load_id: u64,
    },
    Error {
        load_id: u64,
        key: Option<ItemKey>,
        message: String,
    },
}

impl BackendEvent {
    #[must_use]
    pub fn load_id(&self) -> u64 {
        match self {
            Self::ItemStarted { load_id, .. }
            | Self::PlayingChanged { load_id, .. }
            | Self::PositionDiscontinuity { load_id, .. }
            | Self::RunEnded { load_id }
            | Self::Error { load_id, .. } => *load_id,
        }
    }
}

/// Capacity of every backend event channel.
pub const BACKEND_EVENT_CAPACITY: usize = 256;

/// Plays one run at a time. Commands are serialized per backend and never cancelled half-way
/// (a dropped caller future does not abort the command); results of superseded loads are
/// discarded by the engine through `load_id`.
#[async_trait]
pub trait Backend: Send + Sync + 'static {
    fn kind(&self) -> BackendKind;

    /// A new receiver of this backend's events.
    fn subscribe(&self) -> broadcast::Receiver<BackendEvent>;

    /// Loads `run` and starts at `run.items[start_index]` / `position_ms`. `origin` is the origin
    /// of the play intent; only `User` may launch a local client (Spotify device resolution).
    async fn load(
        &self,
        load_id: u64,
        run: BackendRun,
        start_index: usize,
        position_ms: i64,
        play_when_ready: bool,
        origin: ChangeOrigin,
    );

    /// The upcoming part of the run changed while `current` keeps playing.
    async fn update_upcoming(&self, load_id: u64, run: BackendRun, current: ItemKey);

    /// Moves to another item of the already loaded run without rebuilding it.
    async fn seek_to_item(&self, key: ItemKey, position_ms: i64);

    async fn play(&self, origin: ChangeOrigin);
    async fn pause(&self);
    async fn seek_to(&self, position_ms: i64);
    /// Normalized output volume 0..=100 (`ap_model::volume_normalization::effective`).
    async fn set_volume(&self, effective_percent: i32);
    /// Whether the desktop-only per-track loudness gain applies (local items only).
    async fn set_normalization(&self, _enabled: bool) {}
    async fn set_repeat_one(&self, on: bool);
    async fn stop(&self);

    /// Optional warm-up before a run is handed over (Spotify device pre-resolution ~20 s before a
    /// LOCAL→SPOTIFY boundary). Never loads a run, starts audio or emits events. True when ready.
    async fn prepare(&self, _origin: ChangeOrigin) -> bool {
        true
    }

    fn position_ms(&self) -> i64;
    fn duration_ms(&self) -> i64;
}
