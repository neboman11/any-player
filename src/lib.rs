/// Any Player - Multi-Source Music Client
pub mod models;
pub mod providers;
pub mod config;
pub mod playback;
pub mod ui;

pub use models::{Track, Playlist, Source, PlaybackInfo, PlaybackState, RepeatMode};
pub use providers::{MusicProvider, ProviderRegistry, ProviderError};
pub use config::Config;
pub use playback::PlaybackManager;
