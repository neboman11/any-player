//! Desktop platform integration: OAuth, single instance, media controls, tray, and Spotify.
pub mod media_controls;
pub mod oauth_loopback;
pub mod single_instance;
pub mod spotify_launcher;
pub mod tray;

pub use oauth_loopback::{
    CALLBACK_PATH, DEFAULT_PORT, DEFAULT_TIMEOUT, Listening, LoopbackError, SignInFlow,
};
