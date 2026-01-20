// Tauri command handlers for Any Player desktop app
use crate::{PlaybackManager, PlaybackState, ProviderRegistry, RepeatMode};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

/// Maximum age of temporary audio files before cleanup (1 hour)
const TEMP_FILE_MAX_AGE_SECONDS: u64 = 3600;

/// Minimum interval between cleanup runs (5 minutes)
const CLEANUP_INTERVAL_SECONDS: u64 = 300;

/// Last cleanup timestamp (in seconds since UNIX epoch)
static LAST_CLEANUP: AtomicU64 = AtomicU64::new(0);

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
#[allow(dead_code)]
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

/// Helper function to initialize Spotify session for premium users
/// Consolidates the duplicated logic from authenticate_spotify and check_oauth_code
async fn initialize_premium_session_if_needed(state: &AppState) -> Result<(), String> {
    let providers = state.providers.lock().await;

    match providers.is_spotify_premium().await {
        Some(true) => {
            tracing::info!("User is Spotify Premium - initializing session");

            if let Some(access_token) = providers.get_spotify_access_token().await {
                tracing::info!(
                    "Retrieved Spotify access token (len={})",
                    access_token.len()
                );
                drop(providers);

                let playback = state.playback.lock().await;

                tracing::info!("Initializing Spotify session with token");
                playback.initialize_spotify_session(&access_token).await?;

                if playback.is_spotify_session_ready().await {
                    tracing::info!("✓ Spotify session successfully initialized for premium user");
                } else {
                    tracing::warn!("Session initialization completed but not verified as ready");
                }

                Ok(())
            } else {
                tracing::error!(
                    "Could not retrieve Spotify access token for session initialization"
                );
                Err("Failed to retrieve access token".to_string())
            }
        }
        Some(false) => {
            tracing::info!("User has Spotify Free Tier - full track playback not available");
            Ok(())
        }
        None => {
            tracing::error!("Could not determine Spotify subscription status");
            Err("Failed to determine subscription status".to_string())
        }
    }
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
    tracing::info!("Starting Spotify authentication with authorization code");

    let providers = state.providers.lock().await;
    providers
        .authenticate_spotify(&code)
        .await
        .map_err(|e| format!("Failed to authenticate: {}", e))?;
    drop(providers);

    tracing::info!("Spotify authentication successful");

    // Initialize session for premium users
    initialize_premium_session_if_needed(&state).await?;

    Ok(())
}

/// Check if Spotify is connected and authenticated
#[tauri::command]
pub async fn is_spotify_authenticated(state: State<'_, AppState>) -> Result<bool, String> {
    let providers = state.providers.lock().await;
    let authenticated = providers.is_spotify_authenticated().await;
    tracing::debug!("is_spotify_authenticated query result: {}", authenticated);
    Ok(authenticated)
}

/// Check if user has Spotify Premium
///
/// Returns true if authenticated user has Spotify Premium, false otherwise
#[tauri::command]
pub async fn check_spotify_premium(state: State<'_, AppState>) -> Result<bool, String> {
    let providers = state.providers.lock().await;
    providers
        .is_spotify_premium()
        .await
        .ok_or_else(|| "Spotify not authenticated".to_string())
}

/// Initialize Spotify session for premium track streaming
///
/// This should be called after successful Spotify authentication to enable
/// full track streaming for premium users via librespot.
#[tauri::command]
pub async fn initialize_spotify_session(
    state: State<'_, AppState>,
    access_token: String,
) -> Result<(), String> {
    let playback = state.playback.lock().await;
    playback.initialize_spotify_session(&access_token).await
}

/// Initialize Spotify session using the stored provider access token
/// This convenience command lets the frontend ask the backend to initialize
/// the librespot session using the provider-managed OAuth token, avoiding
/// the need for the frontend to pass the token value across IPC.
#[tauri::command]
pub async fn initialize_spotify_session_from_provider(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let providers = state.providers.lock().await;
    if let Some(access_token) = providers.get_spotify_access_token().await {
        drop(providers);
        let playback = state.playback.lock().await;
        playback
            .initialize_spotify_session(&access_token)
            .await
            .map_err(|e| format!("Failed to initialize session: {}", e))
    } else {
        Err("No Spotify access token available in provider registry".to_string())
    }
}

/// Check if Spotify session is initialized and ready for playback
#[tauri::command]
pub async fn is_spotify_session_ready(state: State<'_, AppState>) -> Result<bool, String> {
    let playback = state.playback.lock().await;
    Ok(playback.is_spotify_session_ready().await)
}

/// Refresh Spotify OAuth token and reinitialize session if needed
///
/// Called periodically or when token expires to maintain active authentication
/// and session state for premium playback features.
#[tauri::command]
pub async fn refresh_spotify_token(state: State<'_, AppState>) -> Result<(), String> {
    let mut providers = state.providers.lock().await;
    providers
        .refresh_spotify_token()
        .await
        .map_err(|e| format!("Failed to refresh Spotify token: {}", e))?;

    // If token was refreshed and user is premium, reinitialize session
    if let Some(true) = providers.is_spotify_premium().await {
        if let Some(access_token) = providers.get_spotify_access_token().await {
            drop(providers); // Release providers lock
            let playback = state.playback.lock().await;
            match playback.initialize_spotify_session(&access_token).await {
                Ok(()) => {
                    tracing::info!("Spotify session reinitialized after token refresh");
                }
                Err(e) => {
                    tracing::warn!("Failed to reinitialize session after token refresh: {}", e);
                }
            }
        }
    }

    Ok(())
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
        tracing::info!("OAuth code found in storage");
        drop(oauth_code);

        let providers = state.providers.lock().await;
        providers
            .authenticate_spotify(&code)
            .await
            .map_err(|e| format!("Failed to authenticate: {}", e))?;
        drop(providers);

        tracing::info!("Provider authentication succeeded");

        // Initialize session for premium users
        initialize_premium_session_if_needed(&state).await?;

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

/// Restore Spotify session from saved tokens
#[tauri::command]
pub async fn restore_spotify_session(state: State<'_, AppState>) -> Result<bool, String> {
    let mut providers = state.providers.lock().await;

    // Try to restore session using the provider registry
    providers
        .restore_spotify_session()
        .await
        .map_err(|e| format!("Failed to restore Spotify session: {}", e))
}

/// Clear saved Spotify session tokens and in-memory Spotify session state
#[tauri::command]
pub async fn clear_spotify_session(state: State<'_, AppState>) -> Result<(), String> {
    use crate::config::Config;

    // First disconnect Spotify to clear any in-memory session state
    let mut providers = state.providers.lock().await;
    providers
        .disconnect_spotify()
        .await
        .map_err(|e| format!("Failed to disconnect Spotify during session clear: {}", e))?;
    drop(providers);

    // Then clear any persisted tokens on disk
    Config::clear_tokens().map_err(|e| format!("Failed to clear tokens: {}", e))
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
/// Automatically cleans up old temporary audio files to prevent disk space issues
#[tauri::command]
pub async fn get_audio_file(url: String) -> Result<String, String> {
    use std::io::Write;

    tracing::info!("Downloading audio from: {}", url);

    // Clean up old temporary audio files first
    cleanup_old_temp_audio_files();

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

/// Clean up old temporary audio files (older than 1 hour)
/// Rate-limited to run at most once every 5 minutes to avoid expensive directory scans
fn cleanup_old_temp_audio_files() {
    use std::time::SystemTime;

    // Check if enough time has passed since last cleanup
    let Ok(now_duration) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) else {
        tracing::warn!("System time is before UNIX epoch, skipping cleanup");
        return;
    };
    let now = now_duration.as_secs();

    let last = LAST_CLEANUP.load(Ordering::Relaxed);
    if now.saturating_sub(last) < CLEANUP_INTERVAL_SECONDS {
        return; // Skip cleanup if run too recently
    }

    // Update last cleanup time
    LAST_CLEANUP.store(now, Ordering::Relaxed);

    let temp_dir = std::env::temp_dir();

    if let Ok(entries) = std::fs::read_dir(&temp_dir) {
        for entry in entries.flatten() {
            if let Ok(file_name) = entry.file_name().into_string() {
                // Only process our temporary audio files
                if file_name.starts_with("any-player-audio-") && file_name.ends_with(".mp3") {
                    // Check if file is older than configured max age
                    if let Ok(metadata) = entry.metadata() {
                        if let Ok(modified) = metadata.modified() {
                            if let Ok(elapsed) = SystemTime::now().duration_since(modified) {
                                if elapsed.as_secs() > TEMP_FILE_MAX_AGE_SECONDS {
                                    if let Err(e) = std::fs::remove_file(entry.path()) {
                                        tracing::warn!(
                                            "Failed to remove old temp file {}: {}",
                                            file_name,
                                            e
                                        );
                                    } else {
                                        tracing::info!("Cleaned up old temp file: {}", file_name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Clean up all temporary audio files created by the application
/// This is called on application shutdown to ensure cleanup happens even if
/// the application doesn't run long enough for the rate-limited cleanup to trigger
pub fn cleanup_all_temp_audio_files() {
    let temp_dir = std::env::temp_dir();

    if let Ok(entries) = std::fs::read_dir(&temp_dir) {
        for entry in entries.flatten() {
            if let Ok(file_name) = entry.file_name().into_string() {
                // Only process our temporary audio files
                if file_name.starts_with("any-player-audio-") && file_name.ends_with(".mp3") {
                    if let Err(e) = std::fs::remove_file(entry.path()) {
                        tracing::warn!(
                            "Failed to remove temp file {} on shutdown: {}",
                            file_name,
                            e
                        );
                    } else {
                        tracing::debug!("Cleaned up temp file on shutdown: {}", file_name);
                    }
                }
            }
        }
    }
}
