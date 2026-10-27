//! Pure domain model for Any Player desktop: Rust ports of Android `:core:model` plus the pure
//! sync and transfer functions. No IO and no async runtime; randomness is injected.
//!
//! Byte compatibility with Android is pinned by the golden vectors in `fixtures/`.

pub mod break_schedule;
pub mod dedupe;
pub mod enums;
pub mod ids;
pub mod json;
pub mod models;
pub mod planning;
pub mod queue;
pub mod signatures;
pub mod text;
pub mod transfer_crypto;
pub mod volume_normalization;
pub mod wire;

pub use break_schedule::{BreakSchedule, ScheduleEvent};
pub use enums::{PlaybackStateType, PlaylistType, RepeatMode, SourceType};
pub use ids::{
    is_valid_spotify_track_id, name_uuid_from_bytes, normalize_playlist_id,
    normalize_spotify_track_id, track_ids_match,
};
pub use json::JsonElement;
pub use models::{
    ColumnPreferences, CustomPlaylist, PlaybackStatus, Playlist, PlaylistTrack,
    ProviderConnectionProfile, Track, UnionPlaylistSource,
};
pub use planning::{BackendKind, PendingBreak, PlanItem, Run, backend_for, break_run, plan_run};
pub use queue::{Previous, QueueEntry, QueueOps, QueueState};
