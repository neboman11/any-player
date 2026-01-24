/// Command modules organization
pub mod auth;
pub mod cache;
pub mod custom_playlists;
pub mod helpers;
pub mod playback;
pub mod playlists;
pub mod providers;
pub mod state;
pub mod types;

// Re-export AppState and types for convenience
pub use state::AppState;
pub use types::*;

// Re-export all command functions
pub use auth::*;
pub use cache::*;
pub use custom_playlists::*;
pub use helpers::*;
pub use playback::*;
pub use playlists::*;
pub use providers::*;
