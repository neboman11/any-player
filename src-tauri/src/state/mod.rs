/// Persistent state management for playback session
use crate::models::{PlaybackState, RepeatMode, Track};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Persistent playback state that gets saved to disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentPlaybackState {
    /// Current playing track
    pub current_track: Option<Track>,
    /// Queue of tracks
    pub queue: Vec<Track>,
    /// Current index in queue
    pub current_index: usize,
    /// Playback position in milliseconds
    pub position_ms: u64,
    /// Shuffle enabled
    pub shuffle: bool,
    /// Repeat mode
    pub repeat_mode: RepeatMode,
    /// Volume (0-100)
    pub volume: u32,
    /// Shuffle order
    pub shuffle_order: Vec<usize>,
    /// Playback state (playing/paused/stopped)
    pub state: PlaybackState,
}

impl Default for PersistentPlaybackState {
    fn default() -> Self {
        Self {
            current_track: None,
            queue: Vec::new(),
            current_index: 0,
            position_ms: 0,
            shuffle: false,
            repeat_mode: RepeatMode::Off,
            volume: 50,
            shuffle_order: Vec::new(),
            state: PlaybackState::Stopped,
        }
    }
}

impl PersistentPlaybackState {
    /// Get the path to the state file
    fn get_state_file_path() -> Result<PathBuf, String> {
        let data_dir =
            dirs::data_dir().ok_or_else(|| "Failed to get data directory".to_string())?;

        let state_dir = data_dir.join("any-player");

        // Ensure directory exists
        fs::create_dir_all(&state_dir)
            .map_err(|e| format!("Failed to create state directory: {}", e))?;

        Ok(state_dir.join("playback_state.json"))
    }

    /// Save state to disk
    pub fn save(&self) -> Result<(), String> {
        let path = Self::get_state_file_path()?;

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize state: {}", e))?;

        fs::write(&path, json).map_err(|e| format!("Failed to write state file: {}", e))?;

        tracing::info!("Saved playback state to {:?}", path);
        Ok(())
    }

    /// Load state from disk
    pub fn load() -> Result<Self, String> {
        let path = Self::get_state_file_path()?;

        if !path.exists() {
            tracing::info!("No saved state found, using default");
            return Ok(Self::default());
        }

        let json =
            fs::read_to_string(&path).map_err(|e| format!("Failed to read state file: {}", e))?;

        let state = serde_json::from_str(&json)
            .map_err(|e| format!("Failed to deserialize state: {}", e))?;

        tracing::info!("Loaded playback state from {:?}", path);
        Ok(state)
    }

    /// Delete the saved state file
    pub fn delete() -> Result<(), String> {
        let path = Self::get_state_file_path()?;

        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("Failed to delete state file: {}", e))?;
            tracing::info!("Deleted playback state file");
        }

        Ok(())
    }
}
