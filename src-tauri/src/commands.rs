// Tauri command handlers for Any Player desktop app
use any_player::{PlaybackManager, PlaybackState, ProviderRegistry, RepeatMode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

/// Shared application state
pub struct AppState {
    pub playback: Arc<Mutex<PlaybackManager>>,
    pub providers: Arc<Mutex<ProviderRegistry>>,
    pub oauth_code: Arc<Mutex<Option<String>>>,
}

/// Command response types
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlaybackStatus {
    pub state: String,
    pub current_track: Option<String>,
    pub position: u64,
    pub volume: u32,
    pub shuffle: bool,
    pub repeat_mode: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlaylistInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub track_count: usize,
    pub owner: String,
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TrackInfo {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: u64,
    pub source: String,
}

/// Get current playback status
#[tauri::command]
pub async fn get_playback_status(state: State<'_, AppState>) -> Result<PlaybackStatus, String> {
    let info = {
        let playback = state.playback.lock().await;
        playback.get_info().await
    };

    let state_str = match info.state {
        PlaybackState::Playing => "playing".to_string(),
        PlaybackState::Paused => "paused".to_string(),
        PlaybackState::Stopped => "stopped".to_string(),
    };

    let repeat_str = match info.repeat_mode {
        RepeatMode::Off => "off".to_string(),
        RepeatMode::One => "one".to_string(),
        RepeatMode::All => "all".to_string(),
    };

    Ok(PlaybackStatus {
        state: state_str,
        current_track: None, // TODO: Get from queue
        position: info.position_ms,
        volume: info.volume,
        shuffle: info.shuffle,
        repeat_mode: repeat_str,
    })
}

/// Play current track in queue
#[tauri::command]
pub async fn play(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.play().await;
    Ok(())
}

/// Pause playback
#[tauri::command]
pub async fn pause(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.pause().await;
    Ok(())
}

/// Toggle play/pause
#[tauri::command]
pub async fn toggle_play_pause(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.toggle_play_pause().await;
    Ok(())
}

/// Play next track
#[tauri::command]
pub async fn next_track(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    let _ = playback.next_track().await;
    Ok(())
}

/// Play previous track
#[tauri::command]
pub async fn previous_track(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    let _ = playback.previous_track().await;
    Ok(())
}

/// Seek to position in milliseconds
#[tauri::command]
pub async fn seek(state: State<'_, AppState>, position: u64) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.seek(position).await;
    Ok(())
}

/// Set volume (0-100)
#[tauri::command]
pub async fn set_volume(state: State<'_, AppState>, volume: u32) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.set_volume(volume).await;
    Ok(())
}

/// Toggle shuffle mode
#[tauri::command]
pub async fn toggle_shuffle(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.toggle_shuffle().await;
    Ok(())
}

/// Set repeat mode
#[tauri::command]
pub async fn set_repeat_mode(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    let repeat_mode = match mode.as_str() {
        "off" => RepeatMode::Off,
        "one" => RepeatMode::One,
        "all" => RepeatMode::All,
        _ => return Err("Invalid repeat mode".to_string()),
    };

    let playback = { state.playback.lock().await };
    playback.set_repeat_mode(repeat_mode).await;
    Ok(())
}

/// Get list of playlists from a provider
#[tauri::command]
pub async fn get_playlists(
    state: State<'_, AppState>,
    _source: String,
) -> Result<Vec<PlaylistInfo>, String> {
    let _providers = state.providers.lock().await;

    // TODO: Implement based on source
    // This would require implementing provider lookup and async operations
    // For now, return empty list
    Ok(Vec::new())
}

/// Queue a track or playlist
#[tauri::command]
pub async fn queue_track(
    _state: State<'_, AppState>,
    _track_id: String,
    _source: String,
) -> Result<(), String> {
    // TODO: Implement track queueing
    Ok(())
}

/// Clear the queue
#[tauri::command]
pub async fn clear_queue(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.clear_queue().await;
    Ok(())
}

/// Initialize Spotify OAuth flow and get authorization URL (no credentials needed)
#[tauri::command]
pub async fn get_spotify_auth_url(state: State<'_, AppState>) -> Result<String, String> {
    let mut providers = state.providers.lock().await;

    let auth_url = providers
        .get_spotify_auth_url_default()
        .map_err(|e| format!("Failed to get auth URL: {}", e))?;

    Ok(auth_url)
}

/// Complete Spotify OAuth authentication with authorization code
#[tauri::command]
pub async fn authenticate_spotify(state: State<'_, AppState>, code: String) -> Result<(), String> {
    let providers = state.providers.lock().await;

    providers
        .authenticate_spotify(&code)
        .await
        .map_err(|e| format!("Failed to authenticate: {}", e))
}

/// Check if Spotify is connected and authenticated
#[tauri::command]
pub async fn is_spotify_authenticated(state: State<'_, AppState>) -> Result<bool, String> {
    let providers = state.providers.lock().await;
    Ok(providers.is_spotify_authenticated().await)
}

/// Get Spotify playlists
#[tauri::command]
pub async fn get_spotify_playlists(
    state: State<'_, AppState>,
) -> Result<Vec<PlaylistInfo>, String> {
    let providers = state.providers.lock().await;

    let playlists = providers
        .get_spotify_playlists()
        .await
        .map_err(|e| format!("Failed to get playlists: {}", e))?;

    Ok(playlists
        .into_iter()
        .map(|p| PlaylistInfo {
            id: p.id,
            name: p.name,
            description: p.description,
            track_count: p.tracks.len(),
            owner: p.owner,
            source: "spotify".to_string(),
        })
        .collect())
}

/// Check for and process pending OAuth code
#[tauri::command]
pub async fn check_oauth_code(state: State<'_, AppState>) -> Result<bool, String> {
    let mut oauth_code = state.oauth_code.lock().await;

    if let Some(code) = oauth_code.take() {
        // We have a pending code - authenticate with it
        let providers = state.providers.lock().await;
        providers
            .authenticate_spotify(&code)
            .await
            .map_err(|e| format!("Failed to authenticate: {}", e))?;

        Ok(true)
    } else {
        Ok(false)
    }
}
