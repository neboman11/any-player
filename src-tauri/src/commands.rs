// Tauri command handlers for Any Player desktop app
use crate::{PlaybackManager, PlaybackState, ProviderRegistry, RepeatMode};
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
    pub current_track: Option<TrackInfo>,
    pub position: u64,
    pub volume: u32,
    pub shuffle: bool,
    pub repeat_mode: String,
    pub duration: u64,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlaylistResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub track_count: usize,
    pub owner: String,
    pub source: String,
    pub tracks: Vec<TrackInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JellyfinAuthRequest {
    pub url: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
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

    let duration = info
        .current_track
        .as_ref()
        .map(|t| t.duration_ms)
        .unwrap_or(0);
    let current_track = info.current_track.map(|t| TrackInfo {
        id: t.id,
        title: t.title,
        artist: t.artist,
        album: t.album,
        duration: t.duration_ms,
        source: t.source.to_string(),
        url: t.url,
    });

    Ok(PlaybackStatus {
        state: state_str,
        current_track,
        position: info.position_ms,
        volume: info.volume,
        shuffle: info.shuffle,
        repeat_mode: repeat_str,
        duration,
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

/// Play a track from a source
#[tauri::command]
pub async fn play_track(
    state: State<'_, AppState>,
    track_id: String,
    source: String,
) -> Result<(), String> {
    let providers = state.providers.lock().await;

    // Get the track from the appropriate provider
    let track = match source.as_str() {
        "spotify" => providers
            .get_spotify_track(&track_id)
            .await
            .map_err(|e| format!("Failed to get Spotify track: {}", e))?,
        "jellyfin" => providers
            .get_jellyfin_track(&track_id)
            .await
            .map_err(|e| format!("Failed to get Jellyfin track: {}", e))?,
        _ => return Err("Unknown source".to_string()),
    };

    // Clear queue, add track, and start playing
    let playback = state.playback.lock().await;
    playback.clear_queue().await;
    playback.play_track(track).await;

    Ok(())
}

/// Queue a track or playlist
#[tauri::command]
pub async fn queue_track(
    state: State<'_, AppState>,
    track_id: String,
    source: String,
) -> Result<(), String> {
    let providers = state.providers.lock().await;

    // Get the track from the appropriate provider
    let track = match source.as_str() {
        "spotify" => providers
            .get_spotify_track(&track_id)
            .await
            .map_err(|e| format!("Failed to get Spotify track: {}", e))?,
        "jellyfin" => providers
            .get_jellyfin_track(&track_id)
            .await
            .map_err(|e| format!("Failed to get Jellyfin track: {}", e))?,
        _ => return Err("Unknown source".to_string()),
    };

    // Queue the track
    let playback = state.playback.lock().await;
    playback.queue_track(track).await;

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

/// Get a specific Spotify playlist with tracks
#[tauri::command]
pub async fn get_spotify_playlist(
    state: State<'_, AppState>,
    id: String,
) -> Result<PlaylistResponse, String> {
    let providers = state.providers.lock().await;

    let playlist = providers
        .get_spotify_playlist(&id)
        .await
        .map_err(|e| format!("Failed to get Spotify playlist: {}", e))?;

    let tracks = playlist
        .tracks
        .iter()
        .map(|t| TrackInfo {
            id: t.id.clone(),
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            duration: t.duration_ms,
            source: "spotify".to_string(),
            url: t.url.clone(),
        })
        .collect();

    Ok(PlaylistResponse {
        id: playlist.id,
        name: playlist.name,
        description: playlist.description,
        track_count: playlist.tracks.len(),
        owner: playlist.owner,
        source: "spotify".to_string(),
        tracks,
    })
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

/// Disconnect and revoke Spotify authentication
#[tauri::command]
pub async fn disconnect_spotify(state: State<'_, AppState>) -> Result<(), String> {
    let mut providers = state.providers.lock().await;

    providers
        .disconnect_spotify()
        .await
        .map_err(|e| format!("Failed to disconnect Spotify: {}", e))
}

/// Jellyfin authentication and connection
#[tauri::command]
pub async fn authenticate_jellyfin(
    state: State<'_, AppState>,
    url: String,
    api_key: String,
) -> Result<(), String> {
    let mut providers = state.providers.lock().await;

    providers
        .authenticate_jellyfin(&url, &api_key)
        .await
        .map_err(|e| format!("Failed to authenticate Jellyfin: {}", e))
}

/// Check if Jellyfin is connected and authenticated
#[tauri::command]
pub async fn is_jellyfin_authenticated(state: State<'_, AppState>) -> Result<bool, String> {
    let providers = state.providers.lock().await;
    Ok(providers.is_jellyfin_authenticated().await)
}

/// Get Jellyfin playlists
#[tauri::command]
pub async fn get_jellyfin_playlists(
    state: State<'_, AppState>,
) -> Result<Vec<PlaylistInfo>, String> {
    let providers = state.providers.lock().await;

    let playlists = providers
        .get_jellyfin_playlists()
        .await
        .map_err(|e| format!("Failed to get Jellyfin playlists: {}", e))?;

    Ok(playlists
        .into_iter()
        .map(|p| PlaylistInfo {
            id: p.id,
            name: p.name,
            description: p.description,
            track_count: p.tracks.len(),
            owner: p.owner,
            source: "jellyfin".to_string(),
        })
        .collect())
}

/// Get a specific Jellyfin playlist with tracks
#[tauri::command]
pub async fn get_jellyfin_playlist(
    state: State<'_, AppState>,
    id: String,
) -> Result<PlaylistResponse, String> {
    let providers = state.providers.lock().await;

    let playlist = providers
        .get_jellyfin_playlist(&id)
        .await
        .map_err(|e| format!("Failed to get Jellyfin playlist: {}", e))?;

    let tracks = playlist
        .tracks
        .iter()
        .map(|t| TrackInfo {
            id: t.id.clone(),
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            duration: t.duration_ms,
            source: "jellyfin".to_string(),
            url: t.url.clone(),
        })
        .collect();

    Ok(PlaylistResponse {
        id: playlist.id,
        name: playlist.name,
        description: playlist.description,
        track_count: playlist.tracks.len(),
        owner: playlist.owner,
        source: "jellyfin".to_string(),
        tracks,
    })
}

/// Search tracks on Jellyfin
#[tauri::command]
pub async fn search_jellyfin_tracks(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<TrackInfo>, String> {
    let providers = state.providers.lock().await;

    let tracks = providers
        .search_jellyfin_tracks(&query)
        .await
        .map_err(|e| format!("Failed to search Jellyfin tracks: {}", e))?;

    Ok(tracks
        .into_iter()
        .map(|t| TrackInfo {
            id: t.id,
            title: t.title,
            artist: t.artist,
            album: t.album,
            duration: t.duration_ms,
            source: "jellyfin".to_string(),
            url: t.url,
        })
        .collect())
}

/// Search playlists on Jellyfin
#[tauri::command]
pub async fn search_jellyfin_playlists(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<PlaylistInfo>, String> {
    let providers = state.providers.lock().await;

    let playlists = providers
        .search_jellyfin_playlists(&query)
        .await
        .map_err(|e| format!("Failed to search Jellyfin playlists: {}", e))?;

    Ok(playlists
        .into_iter()
        .map(|p| PlaylistInfo {
            id: p.id,
            name: p.name,
            description: p.description,
            track_count: p.tracks.len(),
            owner: p.owner,
            source: "jellyfin".to_string(),
        })
        .collect())
}

/// Get recently played tracks from Jellyfin
#[tauri::command]
pub async fn get_jellyfin_recently_played(
    state: State<'_, AppState>,
    limit: usize,
) -> Result<Vec<TrackInfo>, String> {
    let providers = state.providers.lock().await;

    let tracks = providers
        .get_jellyfin_recently_played(limit)
        .await
        .map_err(|e| format!("Failed to get recently played: {}", e))?;

    Ok(tracks
        .into_iter()
        .map(|t| TrackInfo {
            id: t.id,
            title: t.title,
            artist: t.artist,
            album: t.album,
            duration: t.duration_ms,
            source: "jellyfin".to_string(),
            url: t.url,
        })
        .collect())
}

/// Disconnect and revoke Jellyfin authentication
#[tauri::command]
pub async fn disconnect_jellyfin(state: State<'_, AppState>) -> Result<(), String> {
    let mut providers = state.providers.lock().await;

    providers
        .disconnect_jellyfin()
        .await
        .map_err(|e| format!("Failed to disconnect Jellyfin: {}", e))
}

/// Download audio to a temporary file and return the path as a file:// URL
#[tauri::command]
pub async fn get_audio_file(url: String) -> Result<String, String> {
    use std::io::Write;

    tracing::info!("Downloading audio from: {}", url);

    // Fetch the audio file
    let response = reqwest::Client::new()
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch audio: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Failed to fetch audio: HTTP {}", response.status()));
    }

    // Read audio bytes
    let audio_bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read audio bytes: {}", e))?;

    // Create temp file in system temp directory
    let temp_dir = std::env::temp_dir();
    let filename = format!("any-player-audio-{}.mp3", uuid::Uuid::new_v4());
    let file_path = temp_dir.join(&filename);

    // Write audio to file
    let mut file = std::fs::File::create(&file_path)
        .map_err(|e| format!("Failed to create temp file: {}", e))?;

    file.write_all(&audio_bytes)
        .map_err(|e| format!("Failed to write audio to file: {}", e))?;

    // Return as file:// URL
    let file_url = format!("file://{}", file_path.display());
    tracing::info!("Audio saved to: {}", file_url);
    Ok(file_url)
}
