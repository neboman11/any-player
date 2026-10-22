//! Playback orchestration and audio for Any Player.
//!
//! `Engine::spawn` restores paused and accepts thread-safe commands. Wire persistence,
//! provider authentication and stream URLs through `ports`; no provider/store dependency
//! is required. `SpotifyBackend` controls Connect only. `LocalBackend` decodes on a
//! dedicated thread and renders through an injectable `AudioSink` (CPAL or capture).
pub mod backend;
pub mod engine;
pub mod local;
pub mod ports;
pub mod spotify;
pub mod state;
pub use backend::{Backend, BackendEvent, BackendRun, ItemKey, RunItem};
pub use engine::{Engine, EngineDeps};
pub use local::{AudioSink, CaptureSink, CpalSink, LocalBackend, NullSink};
pub use ports::{
    ConnectionGate, PlaybackPersistence, PlayerSettings, StreamRequest, StreamResolver,
};
pub use spotify::{SpotifyBackend, SpotifyDeviceResolver, SpotifyLauncher, SpotifyPlayerApi};
pub use state::{
    ActiveBreak, ChangeOrigin, EngineChange, EngineEvent, PlayerState, PositionAnchor,
};
