//! Observable engine state (Android `PlaybackEngine.kt` types).

use ap_model::PlaybackStateType;
use tokio::time::Instant;

/// Who caused a state change; sync never pushes `Sync`/`Restore` changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ChangeOrigin {
    #[default]
    User,
    Sync,
    Restore,
}

/// A DJ break that is playing; the queue cursor stays on the song before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveBreak {
    pub break_id: String,
    pub duration_ms: i64,
}

/// Position published as an anchor: `position_ms` at `at`, advancing at [`PlayerState::rate`].
/// Nothing republishes state just because the position moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionAnchor {
    pub position_ms: i64,
    pub at: Instant,
}

/// Transport state.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    pub status: PlaybackStateType,
    /// True while the engine intends to play, even across run boundaries.
    pub play_requested: bool,
    pub current_uid: Option<u64>,
    pub duration_ms: i64,
    pub anchor: PositionAnchor,
    /// UI volume 0..=100.
    pub volume: i32,
    pub normalization: bool,
    pub error: Option<String>,
    /// The "Playback disabled: …" guard text; `None` when playback is allowed.
    pub disabled_reason: Option<String>,
    pub active_break: Option<ActiveBreak>,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            status: PlaybackStateType::Idle,
            play_requested: false,
            current_uid: None,
            duration_ms: 0,
            anchor: PositionAnchor {
                position_ms: 0,
                at: Instant::now(),
            },
            volume: 50,
            normalization: false,
            error: None,
            disabled_reason: None,
            active_break: None,
        }
    }
}

impl PlayerState {
    /// Playback rate of the anchor: 1.0 while playing, otherwise 0.0.
    #[must_use]
    pub fn rate(&self) -> f32 {
        if self.status == PlaybackStateType::Playing {
            1.0
        } else {
            0.0
        }
    }

    /// The position extrapolated from the anchor at `now`, clamped to the duration when known.
    #[must_use]
    pub fn position_at(&self, now: Instant) -> i64 {
        let elapsed = if self.status == PlaybackStateType::Playing {
            i64::try_from(now.saturating_duration_since(self.anchor.at).as_millis())
                .unwrap_or(i64::MAX)
        } else {
            0
        };
        let position = self.anchor.position_ms.saturating_add(elapsed);
        if self.duration_ms > 0 {
            position.min(self.duration_ms)
        } else {
            position
        }
    }
}

/// Engine notifications for the DJ coordinator, media controls and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    /// A queue entry started playing (natural advance, skip, jump or load).
    ItemStarted {
        uid: u64,
        order_pos: usize,
    },
    BreakStarted {
        break_id: String,
    },
    BreakEnded {
        break_id: String,
    },
    /// A pending break was discarded (target passed/skipped, queue replaced); the owner deletes
    /// its file. Never emitted for a break that started.
    BreakDropped {
        break_id: String,
    },
    QueueReplaced,
    /// A backend reported an error for `uid` (the engine skips or stops).
    Error {
        uid: Option<u64>,
        message: String,
    },
}

/// Emitted for every queue/player mutation with its origin (for sync push decisions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineChange {
    pub origin: ChangeOrigin,
    pub content_changed: bool,
}
