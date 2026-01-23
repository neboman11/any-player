/// Persistent state management for playback session
use crate::models::{PlaybackState, RepeatMode, Track};
use serde::{
    de::Error as DeError, ser::Error as SerError, Deserialize, Deserializer, Serialize, Serializer,
};
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;

#[cfg(test)]
use std::sync::OnceLock;
#[cfg(test)]
static TEST_UUID: OnceLock<String> = OnceLock::new();

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
fn deserialize_option_track_sanitized<'de, D>(deserializer: D) -> Result<Option<Track>, D::Error>
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
fn serialize_track_vec_sanitized<S>(tracks: &Vec<Track>, serializer: S) -> Result<S::Ok, S::Error>
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
fn deserialize_track_vec_sanitized<'de, D>(deserializer: D) -> Result<Vec<Track>, D::Error>
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    #[cfg(not(test))]
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

    /// Get the path to the state file (test version with unique paths per thread)
    #[cfg(test)]
    async fn get_state_file_path() -> Result<PathBuf, String> {
        let temp_dir = std::env::temp_dir();
        let thread_id = std::thread::current().id();

        // Use a combination of process ID, test run UUID, and thread ID for uniqueness
        let test_id =
            TEST_UUID.get_or_init(|| format!("{}-{}", std::process::id(), uuid::Uuid::new_v4()));
        let state_dir = temp_dir.join(format!("any-player-test-{}-{:?}", test_id, thread_id));

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Source;

    /// Helper to create a test track
    fn create_test_track(id: &str, with_auth: bool) -> Track {
        Track {
            id: id.to_string(),
            title: format!("Test Track {}", id),
            artist: "Test Artist".to_string(),
            album: "Test Album".to_string(),
            duration_ms: 180000,
            image_url: Some("https://example.com/image.jpg".to_string()),
            source: Source::Jellyfin,
            url: Some("https://example.com/track.mp3".to_string()),
            auth_headers: if with_auth {
                Some(vec![
                    ("X-Auth-Token".to_string(), "secret-token-123".to_string()),
                    ("X-API-Key".to_string(), "api-key-456".to_string()),
                ])
            } else {
                None
            },
        }
    }

    #[test]
    fn test_default_persistent_state() {
        let state = PersistentPlaybackState::default();
        assert_eq!(state.current_track, None);
        assert_eq!(state.queue.len(), 0);
        assert_eq!(state.current_index, 0);
        assert_eq!(state.position_ms, 0);
        assert!(!state.shuffle);
        assert_eq!(state.repeat_mode, RepeatMode::Off);
        assert_eq!(state.volume, 50);
        assert_eq!(state.shuffle_order.len(), 0);
        assert_eq!(state.state, PlaybackState::Stopped);
    }

    #[test]
    fn test_serialize_sanitizes_auth_headers() {
        let track_with_auth = create_test_track("1", true);
        let state = PersistentPlaybackState {
            current_track: Some(track_with_auth.clone()),
            queue: vec![track_with_auth.clone()],
            current_index: 0,
            position_ms: 5000,
            shuffle: false,
            repeat_mode: RepeatMode::Off,
            volume: 75,
            shuffle_order: vec![],
            state: PlaybackState::Playing,
        };

        // Serialize to JSON
        let json = serde_json::to_string(&state).expect("Failed to serialize");

        // Verify auth_headers are not present in the JSON
        assert!(
            !json.contains("secret-token-123"),
            "Auth token should not be in serialized JSON"
        );
        assert!(
            !json.contains("api-key-456"),
            "API key should not be in serialized JSON"
        );
        assert!(
            !json.contains("X-Auth-Token"),
            "Auth header name should not be in serialized JSON"
        );
        assert!(
            !json.contains("X-API-Key"),
            "API key header name should not be in serialized JSON"
        );

        // Verify other fields are present
        assert!(json.contains("Test Track 1"));
        assert!(json.contains("Test Artist"));
    }

    #[test]
    fn test_deserialize_strips_auth_headers() {
        // Create JSON with auth_headers manually included
        let json = r#"{
            "current_track": {
                "id": "1",
                "title": "Test Track",
                "artist": "Test Artist",
                "album": "Test Album",
                "duration_ms": 180000,
                "image_url": "https://example.com/image.jpg",
                "source": "Jellyfin",
                "url": "https://example.com/track.mp3",
                "auth_headers": [
                    ["X-Auth-Token", "secret-token-123"],
                    ["X-API-Key", "api-key-456"]
                ]
            },
            "queue": [],
            "current_index": 0,
            "position_ms": 0,
            "shuffle": false,
            "repeat_mode": "Off",
            "volume": 50,
            "shuffle_order": [],
            "state": "Stopped"
        }"#;

        // Deserialize
        let state: PersistentPlaybackState =
            serde_json::from_str(json).expect("Failed to deserialize");

        // Verify auth_headers are stripped
        assert!(state.current_track.is_some());
        let track = state.current_track.unwrap();
        assert_eq!(
            track.auth_headers, None,
            "Auth headers should be stripped during deserialization"
        );
    }

    #[test]
    fn test_round_trip_serialization() {
        let track1 = create_test_track("1", true);
        let track2 = create_test_track("2", false);

        let original_state = PersistentPlaybackState {
            current_track: Some(track1.clone()),
            queue: vec![track1.clone(), track2.clone()],
            current_index: 1,
            position_ms: 45000,
            shuffle: true,
            repeat_mode: RepeatMode::All,
            volume: 80,
            shuffle_order: vec![1, 0],
            state: PlaybackState::Paused,
        };

        // Serialize
        let json = serde_json::to_string(&original_state).expect("Failed to serialize");

        // Deserialize
        let restored_state: PersistentPlaybackState =
            serde_json::from_str(&json).expect("Failed to deserialize");

        // Verify all fields except auth_headers
        assert!(restored_state.current_track.is_some());
        let restored_track = restored_state.current_track.as_ref().unwrap();
        assert_eq!(restored_track.id, "1");
        assert_eq!(restored_track.title, "Test Track 1");
        assert_eq!(
            restored_track.auth_headers, None,
            "Auth headers should be None after round-trip"
        );

        assert_eq!(restored_state.queue.len(), 2);
        assert_eq!(
            restored_state.queue[0].auth_headers, None,
            "Queue track 0 auth headers should be None"
        );
        assert_eq!(
            restored_state.queue[1].auth_headers, None,
            "Queue track 1 auth headers should be None"
        );

        assert_eq!(restored_state.current_index, 1);
        assert_eq!(restored_state.position_ms, 45000);
        assert!(restored_state.shuffle);
        assert_eq!(restored_state.repeat_mode, RepeatMode::All);
        assert_eq!(restored_state.volume, 80);
        assert_eq!(restored_state.shuffle_order, vec![1, 0]);
        assert_eq!(restored_state.state, PlaybackState::Paused);
    }

    #[tokio::test]
    async fn test_save_and_load() {
        // Clean up any existing state
        let _ = PersistentPlaybackState::delete().await;

        let track = create_test_track("1", true);
        let state = PersistentPlaybackState {
            current_track: Some(track.clone()),
            queue: vec![track.clone()],
            current_index: 0,
            position_ms: 10000,
            shuffle: false,
            repeat_mode: RepeatMode::One,
            volume: 60,
            shuffle_order: vec![],
            state: PlaybackState::Playing,
        };

        // Save
        state.save().await.expect("Failed to save state");

        // Load
        let loaded_state = PersistentPlaybackState::load()
            .await
            .expect("Failed to load state")
            .expect("State should exist");

        // Verify
        assert_eq!(loaded_state.position_ms, 10000);
        assert_eq!(loaded_state.volume, 60);
        assert_eq!(loaded_state.state, PlaybackState::Playing);
        assert!(loaded_state.current_track.is_some());
        assert_eq!(
            loaded_state.current_track.as_ref().unwrap().auth_headers,
            None,
            "Loaded track should not have auth headers"
        );

        // Clean up
        PersistentPlaybackState::delete()
            .await
            .expect("Failed to delete state");
    }

    #[tokio::test]
    async fn test_load_nonexistent_returns_none() {
        // Clean up any existing state
        let _ = PersistentPlaybackState::delete().await;

        // Try to load when no file exists
        let result = PersistentPlaybackState::load()
            .await
            .expect("Load should not error");
        assert_eq!(result, None, "Should return None when no file exists");
    }

    #[tokio::test]
    async fn test_delete_removes_file() {
        // Clean up any existing state
        let _ = PersistentPlaybackState::delete().await;

        // Create and save a state
        let state = PersistentPlaybackState::default();
        state.save().await.expect("Failed to save state");

        // Verify it exists
        let loaded = PersistentPlaybackState::load()
            .await
            .expect("Failed to load");
        assert!(loaded.is_some(), "State should exist after save");

        // Delete
        PersistentPlaybackState::delete()
            .await
            .expect("Failed to delete state");

        // Verify it's gone
        let loaded_after_delete = PersistentPlaybackState::load()
            .await
            .expect("Failed to load");
        assert_eq!(
            loaded_after_delete, None,
            "State should not exist after delete"
        );
    }
}
