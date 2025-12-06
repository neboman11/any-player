pub mod config;
/// Any Player - Multi-Source Music Client
pub mod models;
pub mod playback;
pub mod providers;

pub use config::Config;
pub use models::{PlaybackInfo, PlaybackState, Playlist, RepeatMode, Source, Track};
pub use playback::PlaybackManager;
pub use providers::{MusicProvider, ProviderError, ProviderRegistry};
