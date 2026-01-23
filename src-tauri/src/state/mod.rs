/// Persistent state management for playback session
use crate::models::{PlaybackState, RepeatMode, Track};
use serde::{de::Error as DeError, ser::Error as SerError, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;

/// Serialize an Option<Track> while stripping any auth_headers field from the JSON representation.
fn serialize_option_track_sanitized<S>(
    track: &Option<Track>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match track {
        None => serializer.serialize_none(),
        Some(t) => {
            // Convert Track to a generic JSON value so we can drop sensitive fields.
            let mut value = serde_json::to_value(t).map_err(SerError::custom)?;
            if let Value::Object(ref mut map) = value {
                // Remove sensitive auth_headers from the serialized form.
                map.remove("auth_headers");
            }
            serializer.serialize_some(&value)
        }
    }
}

/// Deserialize an Option<Track> from JSON that may or may not contain auth_headers.
fn deserialize_option_track_sanitized<'de, D>(
    deserializer: D,
) -> Result<Option<Track>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt_value = Option::<Value>::deserialize(deserializer)?;
    match opt_value {
        None => Ok(None),
        Some(mut value) => {
            if let Value::Object(ref mut map) = value {
                // Ensure any stored auth_headers are discarded on load as well.
                map.remove("auth_headers");
            }
            let track = serde_json::from_value(value).map_err(DeError::custom)?;
            Ok(Some(track))
        }
    }
}

/// Serialize a Vec<Track> while stripping auth_headers from each element.
fn serialize_track_vec_sanitized<S>(
    tracks: &Vec<Track>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut sanitized: Vec<Value> = Vec::with_capacity(tracks.len());
    for t in tracks {
        let mut value = serde_json::to_value(t).map_err(SerError::custom)?;
        if let Value::Object(ref mut map) = value {
            map.remove("auth_headers");
        }
        sanitized.push(value);
    }
    sanitized.serialize(serializer)
}

/// Deserialize a Vec<Track> from JSON, discarding any auth_headers field.
fn deserialize_track_vec_sanitized<'de, D>(
    deserializer: D,
) -> Result<Vec<Track>, D::Error>
where
    D: Deserializer<'de>,
{
    let mut values = Vec::<Value>::deserialize(deserializer)?;
    for value in &mut values {
        if let Value::Object(ref mut map) = value {
            map.remove("auth_headers");
        }
    }
    serde_json::from_value(Value::Array(values)).map_err(DeError::custom)
}

/// Persistent playback state that gets saved to disk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentPlaybackState {
    /// Current playing track
    #[serde(
        serialize_with = "serialize_option_track_sanitized",
        deserialize_with = "deserialize_option_track_sanitized"
    )]
    pub current_track: Option<Track>,
    /// Queue of tracks
    #[serde(
        serialize_with = "serialize_track_vec_sanitized",
        deserialize_with = "deserialize_track_vec_sanitized"
    )]
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
    async fn get_state_file_path() -> Result<PathBuf, String> {
        let data_dir =
            dirs::data_dir().ok_or_else(|| "Failed to get data directory".to_string())?;

        let state_dir = data_dir.join("any-player");

        // Ensure directory exists
        fs::create_dir_all(&state_dir)
            .await
            .map_err(|e| format!("Failed to create state directory: {}", e))?;

        Ok(state_dir.join("playback_state.json"))
    }

    /// Save state to disk (async, non-blocking)
    pub async fn save(&self) -> Result<(), String> {
        let path = Self::get_state_file_path().await?;
        let state_clone = self.clone();
        
        // Serialize in blocking task since it can be CPU-intensive
        let json = tokio::task::spawn_blocking(move || {
            serde_json::to_string_pretty(&state_clone)
                .map_err(|e| format!("Failed to serialize state: {}", e))
        })
        .await
        .map_err(|e| format!("Failed to spawn blocking task: {}", e))??;

        // Write to disk using async I/O
        fs::write(&path, json)
            .await
            .map_err(|e| format!("Failed to write state file: {}", e))?;

        tracing::info!("Saved playback state to {:?}", path);
        Ok(())
    }

    /// Load state from disk (async, non-blocking)
    /// Returns None if no saved state exists
    pub async fn load() -> Result<Option<Self>, String> {
        let path = Self::get_state_file_path().await?;

        if !fs::try_exists(&path)
            .await
            .map_err(|e| format!("Failed to check state file: {}", e))?
        {
            tracing::info!("No saved state found");
            return Ok(None);
        }

        let json = fs::read_to_string(&path)
            .await
            .map_err(|e| format!("Failed to read state file: {}", e))?;

        let state = tokio::task::spawn_blocking(move || {
            serde_json::from_str(&json).map_err(|e| format!("Failed to deserialize state: {}", e))
        })
        .await
        .map_err(|e| format!("Failed to spawn blocking task: {}", e))??;

        tracing::info!("Loaded playback state from {:?}", path);
        Ok(Some(state))
    }

    /// Delete the saved state file (async, non-blocking)
    pub async fn delete() -> Result<(), String> {
        let path = Self::get_state_file_path().await?;

        if fs::try_exists(&path)
            .await
            .map_err(|e| format!("Failed to check state file: {}", e))?
        {
            fs::remove_file(&path)
                .await
                .map_err(|e| format!("Failed to delete state file: {}", e))?;
            tracing::info!("Deleted playback state file");
        }

        Ok(())
    }
}
