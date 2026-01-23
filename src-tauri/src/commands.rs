// Tauri command handlers for Any Player desktop app
use crate::{Database, PlaybackManager, PlaybackState, ProviderRegistry, RepeatMode};
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
    pub database: Arc<Mutex<Database>>,
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

    // Trigger eager loading for upcoming tracks in the background
    let playback_arc = state.playback.clone();
    let providers_arc = state.providers.clone();
    tokio::spawn(async move {
        let pb = playback_arc.lock().await;
        let info = pb.get_info().await;
        let current_idx = info.current_index;
        drop(pb);

        enrich_queued_tracks_eager(playback_arc, providers_arc, current_idx).await;
    });

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

    // Normalize source to lowercase
    let normalized_source = source.to_lowercase();

    // Get the track from the appropriate provider
    let track = match normalized_source.as_str() {
        "spotify" => providers
            .get_spotify_track(&track_id)
            .await
            .map_err(|e| format!("Failed to get Spotify track: {}", e))?,
        "jellyfin" => providers
            .get_jellyfin_track(&track_id)
            .await
            .map_err(|e| format!("Failed to get Jellyfin track: {}", e))?,
        "custom" => {
            return Err("Playing custom tracks directly is not yet supported. Please play from a custom playlist instead.".to_string());
        }
        _ => {
            return Err(format!(
                "Unknown source: '{}'. Supported sources are: spotify, jellyfin",
                source
            ))
        }
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

    // Normalize source to lowercase
    let normalized_source = source.to_lowercase();

    // Get the track from the appropriate provider
    let track = match normalized_source.as_str() {
        "spotify" => providers
            .get_spotify_track(&track_id)
            .await
            .map_err(|e| format!("Failed to get Spotify track: {}", e))?,
        "jellyfin" => providers
            .get_jellyfin_track(&track_id)
            .await
            .map_err(|e| format!("Failed to get Jellyfin track: {}", e))?,
        "custom" => {
            return Err("Queuing custom tracks directly is not yet supported. Please queue from a custom playlist instead.".to_string());
        }
        _ => {
            return Err(format!(
                "Unknown source: '{}'. Supported sources are: spotify, jellyfin",
                source
            ))
        }
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

/// Play a playlist by loading all its tracks
#[tauri::command]
pub async fn play_playlist(
    state: State<'_, AppState>,
    playlist_id: String,
    source: String,
) -> Result<(), String> {
    let providers = state.providers.lock().await;

    // Get the playlist with all tracks from the appropriate provider
    let playlist = match source.as_str() {
        "spotify" => providers
            .get_spotify_playlist(&playlist_id)
            .await
            .map_err(|e| format!("Failed to get Spotify playlist: {}", e))?,
        "jellyfin" => providers
            .get_jellyfin_playlist(&playlist_id)
            .await
            .map_err(|e| format!("Failed to get Jellyfin playlist: {}", e))?,
        "custom" => {
            // Handle custom playlists from database
            let db = state.database.lock().await;

            // Check if this is a union playlist
            let playlist_info = db
                .get_playlist(&playlist_id)
                .map_err(|e| format!("Failed to get playlist info: {}", e))?
                .ok_or_else(|| format!("Playlist not found: {}", playlist_id))?;

            let tracks_with_urls = if playlist_info.playlist_type == "union" {
                // For union playlists, get tracks from all source playlists
                let sources = db
                    .get_union_playlist_sources(&playlist_id)
                    .map_err(|e| format!("Failed to get union playlist sources: {}", e))?;

                drop(db); // Release database lock before provider calls

                let mut all_tracks = Vec::new();

                for source in sources {
                    match source.source_type.as_str() {
                        "spotify" => {
                            match providers
                                .get_spotify_playlist(&source.source_playlist_id)
                                .await
                            {
                                Ok(playlist) => {
                                    all_tracks.extend(playlist.tracks);
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to get Spotify playlist {}: {}",
                                        source.source_playlist_id,
                                        e
                                    );
                                }
                            }
                        }
                        "jellyfin" => {
                            match providers
                                .get_jellyfin_playlist(&source.source_playlist_id)
                                .await
                            {
                                Ok(playlist) => {
                                    all_tracks.extend(playlist.tracks);
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to get Jellyfin playlist {}: {}",
                                        source.source_playlist_id,
                                        e
                                    );
                                }
                            }
                        }
                        "custom" => {
                            let db = state.database.lock().await;
                            match db.get_playlist_tracks(&source.source_playlist_id) {
                                Ok(tracks) => {
                                    all_tracks.extend(tracks.into_iter().map(|t| t.to_track()));
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to get custom playlist {}: {}",
                                        source.source_playlist_id,
                                        e
                                    );
                                }
                            }
                            drop(db);
                        }
                        _ => {
                            tracing::warn!("Unknown source type: {}", source.source_type);
                        }
                    }
                }

                all_tracks
            } else {
                // For standard custom playlists, get tracks normally
                let playlist_tracks = db
                    .get_playlist_tracks(&playlist_id)
                    .map_err(|e| format!("Failed to get custom playlist tracks: {}", e))?;

                drop(db);

                // Fetch full track details with URLs from the original providers
                let mut tracks = Vec::new();
                for pt in playlist_tracks {
                    let track_result = match pt.track_source.as_str() {
                        "Spotify" | "spotify" => providers.get_spotify_track(&pt.track_id).await,
                        "Jellyfin" | "jellyfin" => providers.get_jellyfin_track(&pt.track_id).await,
                        _ => {
                            // For custom or unknown sources, use the cached metadata without URL
                            Ok(pt.to_track())
                        }
                    };

                    match track_result {
                        Ok(track) => tracks.push(track),
                        Err(e) => {
                            tracing::warn!(
                                "Failed to fetch track {} from {}: {}. Using cached metadata.",
                                pt.track_id,
                                pt.track_source,
                                e
                            );
                            // Fall back to cached metadata
                            tracks.push(pt.to_track());
                        }
                    }
                }

                tracks
            };

            if tracks_with_urls.is_empty() {
                return Err("Playlist is empty".to_string());
            }

            drop(providers);

            // Clear queue and add all tracks
            let playback = state.playback.lock().await;
            playback.clear_queue().await;
            playback.queue_tracks(tracks_with_urls.clone()).await;

            // Check if shuffle is enabled and generate shuffle order if needed
            let info = playback.get_info().await;
            if info.shuffle {
                // Access the queue to generate shuffle order
                let queue_arc = playback.get_queue_arc();
                let mut queue = queue_arc.lock().await;
                queue.generate_shuffle_order();
                queue.current_index = 0;

                // Get the first track according to shuffle order
                let first_track_index = if !queue.shuffle_order.is_empty()
                    && queue.shuffle_order[0] < tracks_with_urls.len()
                {
                    queue.shuffle_order[0]
                } else {
                    0
                };
                drop(queue);

                playback
                    .play_track(tracks_with_urls[first_track_index].clone())
                    .await;

                // Trigger eager loading for the next tracks
                let first_idx = first_track_index;
                drop(playback);

                let playback_arc = state.playback.clone();
                let providers_arc = state.providers.clone();
                tokio::spawn(async move {
                    enrich_queued_tracks_eager(playback_arc, providers_arc, first_idx).await;
                });
            } else {
                // Play the first track normally
                playback.play_track(tracks_with_urls[0].clone()).await;
                drop(playback);

                // Trigger eager loading for the next tracks
                let playback_arc = state.playback.clone();
                let providers_arc = state.providers.clone();
                tokio::spawn(async move {
                    enrich_queued_tracks_eager(playback_arc, providers_arc, 0).await;
                });
            }

            return Ok(());
        }
        _ => return Err("Unknown source".to_string()),
    };

    if playlist.tracks.is_empty() {
        return Err("Playlist is empty".to_string());
    }

    drop(providers);

    // Clear queue and add all tracks from the playlist
    let playback = state.playback.lock().await;
    playback.clear_queue().await;
    playback.queue_tracks(playlist.tracks.clone()).await;

    // Check if shuffle is enabled and generate shuffle order if needed
    let info = playback.get_info().await;
    let first_track_index = if info.shuffle {
        // Access the queue to generate shuffle order
        let queue_arc = playback.get_queue_arc();
        let mut queue = queue_arc.lock().await;
        queue.generate_shuffle_order();
        queue.current_index = 0;

        // Get the first track according to shuffle order
        if !queue.shuffle_order.is_empty() && queue.shuffle_order[0] < playlist.tracks.len() {
            queue.shuffle_order[0]
        } else {
            0
        }
    } else {
        0
    };

    // Play the first track
    playback
        .play_track(playlist.tracks[first_track_index].clone())
        .await;

    drop(playback);

    // Trigger eager loading for the next tracks in the background
    let playback_arc = state.playback.clone();
    let providers_arc = state.providers.clone();
    tokio::spawn(async move {
        enrich_queued_tracks_eager(playback_arc, providers_arc, first_track_index).await;
    });

    Ok(())
}

/// Play tracks directly from a list (optimized for union playlists)
/// This command accepts tracks from the frontend and starts playback immediately
/// without fetching full details for all tracks upfront
#[tauri::command]
pub async fn play_tracks_immediate(
    state: State<'_, AppState>,
    tracks: Vec<TrackInfo>,
) -> Result<(), String> {
    if tracks.is_empty() {
        return Err("No tracks provided".to_string());
    }

    // Convert TrackInfo to Track for internal use
    // Get provider registry to fetch auth headers for tracks that need them
    let providers = state.providers.lock().await;

    let mut internal_tracks = Vec::new();
    for track_info in tracks {
        let source = match track_info.source.to_lowercase().as_str() {
            "spotify" => crate::models::Source::Spotify,
            "jellyfin" => crate::models::Source::Jellyfin,
            _ => crate::models::Source::Custom,
        };

        // Get auth headers for sources that need them (e.g., Jellyfin)
        let auth_headers = providers.get_auth_headers(source.clone()).await;

        internal_tracks.push(crate::models::Track {
            id: track_info.id,
            title: track_info.title,
            artist: track_info.artist,
            album: track_info.album,
            duration_ms: track_info.duration,
            image_url: None,
            source,
            url: track_info.url,
            auth_headers,
        });
    }

    drop(providers);

    let playback = state.playback.lock().await;

    // Clear queue and add all tracks
    playback.clear_queue().await;
    playback.queue_tracks(internal_tracks.clone()).await;

    // Check if shuffle is enabled and generate shuffle order if needed
    let info = playback.get_info().await;
    let first_track_index = if info.shuffle {
        // Generate shuffle order
        let queue_arc = playback.get_queue_arc();
        let mut queue = queue_arc.lock().await;
        queue.generate_shuffle_order();
        queue.current_index = 0;

        // Get the first track according to shuffle order
        if !queue.shuffle_order.is_empty() && queue.shuffle_order[0] < internal_tracks.len() {
            queue.shuffle_order[0]
        } else {
            0
        }
    } else {
        0
    };

    drop(playback); // Release playback lock

    // Enrich the first track immediately before playing (critical for Jellyfin auth)
    let providers = state.providers.lock().await;
    let first_track = &internal_tracks[first_track_index];

    let enriched_first_track = match first_track.source {
        crate::models::Source::Spotify => providers.get_spotify_track(&first_track.id).await.ok(),
        crate::models::Source::Jellyfin => {
            // Must enrich Jellyfin tracks immediately to get auth headers
            providers.get_jellyfin_track(&first_track.id).await.ok()
        }
        _ => None,
    };

    drop(providers); // Release providers lock

    // Update the first track in the queue if we successfully enriched it
    if let Some(enriched) = enriched_first_track {
        let playback = state.playback.lock().await;
        let queue_arc = playback.get_queue_arc();
        let mut queue = queue_arc.lock().await;
        if first_track_index < queue.tracks.len() {
            queue.tracks[first_track_index] = enriched.clone();
        }
        drop(queue);

        // Play the enriched first track
        playback.play_track(enriched).await;
    } else {
        // Fall back to playing the track as-is (may fail for Jellyfin without auth)
        let playback = state.playback.lock().await;
        playback
            .play_track(internal_tracks[first_track_index].clone())
            .await;
    }

    // Spawn background task to eagerly enrich the next tracks
    // This ensures Jellyfin tracks have auth headers ready before playback reaches them
    let playback_arc = state.playback.clone();
    let providers_arc = state.providers.clone();
    let first_track_index_clone = first_track_index;
    tokio::spawn(async move {
        enrich_queued_tracks_eager(playback_arc, providers_arc, first_track_index_clone).await;
    });

    Ok(())
}

/// Eagerly enrich queued tracks with full details (URLs, auth headers, etc.)
/// Prioritizes tracks near the current playback position and loads them immediately
async fn enrich_queued_tracks_eager(
    playback: Arc<Mutex<PlaybackManager>>,
    providers: Arc<Mutex<ProviderRegistry>>,
    current_index: usize,
) {
    const LOOKAHEAD_COUNT: usize = 10; // Number of tracks to load ahead

    let pb = playback.lock().await;
    let queue_arc = pb.get_queue_arc();
    drop(pb); // Release playback lock

    let queue = queue_arc.lock().await;
    let total_tracks = queue.tracks.len();
    let shuffle_enabled = !queue.shuffle_order.is_empty();
    drop(queue); // Release queue lock temporarily

    // Calculate which tracks to load first (starting from index 1 since 0 is already playing)
    let mut indices_to_load = Vec::new();
    for i in 1..=LOOKAHEAD_COUNT.min(total_tracks.saturating_sub(1)) {
        let actual_index = if shuffle_enabled {
            let queue = queue_arc.lock().await;
            let shuffle_pos = (current_index + i) % total_tracks;
            if shuffle_pos < queue.shuffle_order.len() {
                queue.shuffle_order[shuffle_pos]
            } else {
                i
            }
        } else {
            (current_index + i) % total_tracks
        };
        indices_to_load.push(actual_index);
    }

    // Load track details for the prioritized tracks
    let providers_lock = providers.lock().await;

    for &track_idx in &indices_to_load {
        let queue = queue_arc.lock().await;
        if track_idx >= queue.tracks.len() {
            continue;
        }

        let track = &queue.tracks[track_idx];

        // Skip if track already has a URL (already enriched)
        if track.url.is_some() {
            drop(queue);
            continue;
        }

        let track_id = track.id.clone();
        let source = track.source;
        drop(queue); // Release lock before async call

        // Fetch full track details
        let enriched_track_result = match source {
            crate::models::Source::Spotify => providers_lock.get_spotify_track(&track_id).await,
            crate::models::Source::Jellyfin => providers_lock.get_jellyfin_track(&track_id).await,
            _ => continue, // Skip custom tracks
        };

        // Update the track in the queue with enriched data
        if let Ok(enriched_track) = enriched_track_result {
            let mut queue = queue_arc.lock().await;
            if track_idx < queue.tracks.len() {
                queue.tracks[track_idx] = enriched_track;
                tracing::debug!("Eagerly enriched track {} at index {}", track_id, track_idx);
            }
        } else {
            tracing::warn!("Failed to enrich track {} at index {}", track_id, track_idx);
        }

        // Small delay to avoid overwhelming the API
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    drop(providers_lock);

    tracing::info!(
        "Completed eager loading of {} priority tracks (total queue: {})",
        indices_to_load.len(),
        total_tracks
    );
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
            track_count: p.track_count,
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
    use crate::config::Config;

    let mut providers = state.providers.lock().await;

    providers
        .authenticate_jellyfin(&url, &api_key)
        .await
        .map_err(|e| format!("Failed to authenticate Jellyfin: {}", e))?;

    // Save credentials to secure storage after successful authentication
    let mut tokens = Config::load_tokens().map_err(|e| format!("Failed to load tokens: {}", e))?;
    tokens.jellyfin_api_key = Some(api_key);
    tokens.jellyfin_url = Some(url);
    Config::save_tokens(&tokens)
        .map_err(|e| format!("Failed to save Jellyfin credentials: {}", e))?;

    tracing::info!("Jellyfin credentials saved to secure storage");

    Ok(())
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
            track_count: p.track_count,
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

/// Search tracks on Spotify
#[tauri::command]
pub async fn search_spotify_tracks(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<TrackInfo>, String> {
    let providers = state.providers.lock().await;

    let tracks = providers
        .search_spotify_tracks(&query)
        .await
        .map_err(|e| format!("Failed to search Spotify tracks: {}", e))?;

    Ok(tracks
        .into_iter()
        .map(|t| TrackInfo {
            id: t.id,
            title: t.title,
            artist: t.artist,
            album: t.album,
            duration: t.duration_ms,
            source: "spotify".to_string(),
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
    use crate::config::Config;

    let mut providers = state.providers.lock().await;

    providers
        .disconnect_jellyfin()
        .await
        .map_err(|e| format!("Failed to disconnect Jellyfin: {}", e))?;

    // Clear stored Jellyfin credentials from secure storage
    let mut tokens = Config::load_tokens().map_err(|e| format!("Failed to load tokens: {}", e))?;
    tokens.jellyfin_api_key = None;
    tokens.jellyfin_url = None;
    Config::save_tokens(&tokens)
        .map_err(|e| format!("Failed to clear Jellyfin credentials: {}", e))?;

    tracing::info!("Jellyfin credentials cleared from secure storage");

    Ok(())
}

/// Get stored Jellyfin credentials
#[tauri::command]
pub async fn get_jellyfin_credentials(
    _state: State<'_, AppState>,
) -> Result<Option<(String, String)>, String> {
    use crate::config::Config;

    let tokens = Config::load_tokens().map_err(|e| format!("Failed to load tokens: {}", e))?;

    match (tokens.jellyfin_url, tokens.jellyfin_api_key) {
        (Some(url), Some(api_key)) => Ok(Some((url, api_key))),
        _ => Ok(None),
    }
}

/// Restore Jellyfin session from saved credentials
#[tauri::command]
pub async fn restore_jellyfin_session(state: State<'_, AppState>) -> Result<bool, String> {
    let mut providers = state.providers.lock().await;

    // Try to restore session using the provider registry
    providers
        .restore_jellyfin_session()
        .await
        .map_err(|e| format!("Failed to restore Jellyfin session: {}", e))
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

// ============================================================================
// Custom Playlist Commands
// ============================================================================

use crate::database::{ColumnPreferences, CustomPlaylist, PlaylistTrack, UnionPlaylistSource};
use crate::models::Track;

#[tauri::command]
pub async fn create_custom_playlist(
    state: State<'_, AppState>,
    name: String,
    description: Option<String>,
    image_url: Option<String>,
) -> Result<CustomPlaylist, String> {
    let db = state.database.lock().await;
    db.create_playlist(name, description, image_url)
        .map_err(|e| format!("Failed to create playlist: {}", e))
}

#[tauri::command]
pub async fn create_union_playlist(
    state: State<'_, AppState>,
    name: String,
    description: Option<String>,
    image_url: Option<String>,
) -> Result<CustomPlaylist, String> {
    let db = state.database.lock().await;
    db.create_playlist_with_type(name, description, image_url, "union".to_string())
        .map_err(|e| format!("Failed to create union playlist: {}", e))
}

#[tauri::command]
pub async fn get_custom_playlists(
    state: State<'_, AppState>,
) -> Result<Vec<CustomPlaylist>, String> {
    let db = state.database.lock().await;
    let mut playlists = db
        .get_all_playlists()
        .map_err(|e| format!("Failed to get playlists: {}", e))?;

    // Calculate track count for union playlists
    let providers = state.providers.lock().await;
    for playlist in &mut playlists {
        if playlist.playlist_type == "union" {
            let sources = db
                .get_union_playlist_sources(&playlist.id)
                .map_err(|e| format!("Failed to get union playlist sources: {}", e))?;

            let mut total_tracks: i64 = 0;
            for source in sources {
                match source.source_type.as_str() {
                    "spotify" => {
                        if let Ok(p) = providers
                            .get_spotify_playlist(&source.source_playlist_id)
                            .await
                        {
                            total_tracks += p.track_count as i64;
                        }
                    }
                    "jellyfin" => {
                        if let Ok(p) = providers
                            .get_jellyfin_playlist(&source.source_playlist_id)
                            .await
                        {
                            total_tracks += p.track_count as i64;
                        }
                    }
                    "custom" => {
                        if let Ok(tracks) = db.get_playlist_tracks(&source.source_playlist_id) {
                            total_tracks += tracks.len() as i64;
                        }
                    }
                    _ => {}
                }
            }
            playlist.track_count = total_tracks;
        }
    }

    drop(providers);
    drop(db);
    Ok(playlists)
}

#[tauri::command]
pub async fn get_custom_playlist(
    state: State<'_, AppState>,
    playlist_id: String,
) -> Result<Option<CustomPlaylist>, String> {
    let db = state.database.lock().await;
    db.get_playlist(&playlist_id)
        .map_err(|e| format!("Failed to get playlist: {}", e))
}

#[tauri::command]
pub async fn update_custom_playlist(
    state: State<'_, AppState>,
    playlist_id: String,
    name: Option<String>,
    description: Option<String>,
    image_url: Option<String>,
) -> Result<(), String> {
    let db = state.database.lock().await;
    db.update_playlist(&playlist_id, name, description, image_url)
        .map_err(|e| format!("Failed to update playlist: {}", e))
}

#[tauri::command]
pub async fn delete_custom_playlist(
    state: State<'_, AppState>,
    playlist_id: String,
) -> Result<(), String> {
    let db = state.database.lock().await;
    db.delete_playlist(&playlist_id)
        .map_err(|e| format!("Failed to delete playlist: {}", e))
}

#[tauri::command]
pub async fn add_track_to_custom_playlist(
    state: State<'_, AppState>,
    playlist_id: String,
    track: Track,
) -> Result<PlaylistTrack, String> {
    let db = state.database.lock().await;
    db.add_track_to_playlist(&playlist_id, &track)
        .map_err(|e| format!("Failed to add track: {}", e))
}

#[tauri::command]
pub async fn get_custom_playlist_tracks(
    state: State<'_, AppState>,
    playlist_id: String,
) -> Result<Vec<PlaylistTrack>, String> {
    let db = state.database.lock().await;
    db.get_playlist_tracks(&playlist_id)
        .map_err(|e| format!("Failed to get playlist tracks: {}", e))
}

#[tauri::command]
pub async fn remove_track_from_custom_playlist(
    state: State<'_, AppState>,
    track_id: i64,
) -> Result<(), String> {
    let db = state.database.lock().await;
    db.remove_track_from_playlist(track_id)
        .map_err(|e| format!("Failed to remove track: {}", e))
}

#[tauri::command]
pub async fn reorder_custom_playlist_tracks(
    state: State<'_, AppState>,
    playlist_id: String,
    track_id: i64,
    new_position: i64,
) -> Result<(), String> {
    let db = state.database.lock().await;
    db.reorder_tracks(&playlist_id, track_id, new_position)
        .map_err(|e| format!("Failed to reorder tracks: {}", e))
}

#[tauri::command]
pub async fn get_column_preferences(
    state: State<'_, AppState>,
) -> Result<ColumnPreferences, String> {
    let db = state.database.lock().await;
    db.get_column_preferences()
        .map_err(|e| format!("Failed to get column preferences: {}", e))
}

#[tauri::command]
pub async fn save_column_preferences(
    state: State<'_, AppState>,
    preferences: ColumnPreferences,
) -> Result<(), String> {
    let db = state.database.lock().await;
    db.save_column_preferences(&preferences)
        .map_err(|e| format!("Failed to save column preferences: {}", e))
}

#[tauri::command]
pub async fn add_source_to_union_playlist(
    state: State<'_, AppState>,
    union_playlist_id: String,
    source_type: String,
    source_playlist_id: String,
) -> Result<UnionPlaylistSource, String> {
    let db = state.database.lock().await;
    db.add_source_to_union_playlist(&union_playlist_id, &source_type, &source_playlist_id)
        .map_err(|e| format!("Failed to add source to union playlist: {}", e))
}

#[tauri::command]
pub async fn get_union_playlist_sources(
    state: State<'_, AppState>,
    union_playlist_id: String,
) -> Result<Vec<UnionPlaylistSource>, String> {
    let db = state.database.lock().await;
    db.get_union_playlist_sources(&union_playlist_id)
        .map_err(|e| format!("Failed to get union playlist sources: {}", e))
}

#[tauri::command]
pub async fn remove_source_from_union_playlist(
    state: State<'_, AppState>,
    source_id: i64,
) -> Result<(), String> {
    let db = state.database.lock().await;
    db.remove_source_from_union_playlist(source_id)
        .map_err(|e| format!("Failed to remove source from union playlist: {}", e))
}

#[tauri::command]
pub async fn reorder_union_playlist_sources(
    state: State<'_, AppState>,
    union_playlist_id: String,
    source_id: i64,
    new_position: i64,
) -> Result<(), String> {
    let db = state.database.lock().await;
    db.reorder_union_sources(&union_playlist_id, source_id, new_position)
        .map_err(|e| format!("Failed to reorder union playlist sources: {}", e))
}

#[tauri::command]
pub async fn get_union_playlist_tracks(
    state: State<'_, AppState>,
    union_playlist_id: String,
) -> Result<Vec<Track>, String> {
    let db = state.database.lock().await;
    let providers = state.providers.lock().await;

    // Get all source playlists
    let sources = db
        .get_union_playlist_sources(&union_playlist_id)
        .map_err(|e| format!("Failed to get union playlist sources: {}", e))?;

    tracing::info!(
        "Getting tracks for union playlist {} with {} sources",
        union_playlist_id,
        sources.len()
    );

    let mut all_tracks = Vec::new();

    for source in sources {
        tracing::debug!(
            "Processing source: type={}, playlist_id={}",
            source.source_type,
            source.source_playlist_id
        );

        match source.source_type.as_str() {
            "spotify" => {
                match providers
                    .get_spotify_playlist(&source.source_playlist_id)
                    .await
                {
                    Ok(playlist) => {
                        tracing::info!(
                            "Got {} tracks from Spotify playlist {}",
                            playlist.tracks.len(),
                            source.source_playlist_id
                        );
                        all_tracks.extend(playlist.tracks);
                    }
                    Err(e) => {
                        tracing::error!("Failed to get Spotify playlist tracks: {}", e);
                        eprintln!("Failed to get Spotify playlist tracks: {}", e);
                    }
                }
            }
            "jellyfin" => {
                match providers
                    .get_jellyfin_playlist(&source.source_playlist_id)
                    .await
                {
                    Ok(playlist) => {
                        tracing::info!(
                            "Got {} tracks from Jellyfin playlist {}",
                            playlist.tracks.len(),
                            source.source_playlist_id
                        );
                        all_tracks.extend(playlist.tracks);
                    }
                    Err(e) => {
                        tracing::error!("Failed to get Jellyfin playlist tracks: {}", e);
                        eprintln!("Failed to get Jellyfin playlist tracks: {}", e);
                    }
                }
            }
            "custom" => {
                // Get tracks from custom playlist
                let tracks = db
                    .get_playlist_tracks(&source.source_playlist_id)
                    .map_err(|e| format!("Failed to get custom playlist tracks: {}", e))?;
                tracing::info!(
                    "Got {} tracks from custom playlist {}",
                    tracks.len(),
                    source.source_playlist_id
                );
                all_tracks.extend(tracks.into_iter().map(|t| t.to_track()));
            }
            _ => {
                tracing::warn!("Unknown source type: {}", source.source_type);
                eprintln!("Unknown source type: {}", source.source_type);
            }
        }
    }

    tracing::info!(
        "Total tracks collected for union playlist {}: {}",
        union_playlist_id,
        all_tracks.len()
    );

    Ok(all_tracks)
}

// ============================================================================
// Cache Commands
// ============================================================================

/// Write playlists cache to disk
#[tauri::command]
pub async fn write_playlists_cache(data: String) -> Result<(), String> {
    crate::cache::write_playlists_cache(&data)
        .map_err(|e| format!("Failed to write playlists cache: {}", e))
}

/// Read playlists cache from disk
#[tauri::command]
pub async fn read_playlists_cache() -> Result<Option<String>, String> {
    crate::cache::read_playlists_cache()
        .map_err(|e| format!("Failed to read playlists cache: {}", e))
}

/// Clear playlists cache
#[tauri::command]
pub async fn clear_playlists_cache() -> Result<(), String> {
    crate::cache::clear_playlists_cache()
        .map_err(|e| format!("Failed to clear playlists cache: {}", e))
}

/// Write custom playlists cache to disk
#[tauri::command]
pub async fn write_custom_playlists_cache(data: String) -> Result<(), String> {
    crate::cache::write_custom_playlists_cache(&data)
        .map_err(|e| format!("Failed to write custom playlists cache: {}", e))
}

/// Read custom playlists cache from disk
#[tauri::command]
pub async fn read_custom_playlists_cache() -> Result<Option<String>, String> {
    crate::cache::read_custom_playlists_cache()
        .map_err(|e| format!("Failed to read custom playlists cache: {}", e))
}

/// Clear custom playlists cache
#[tauri::command]
pub async fn clear_custom_playlists_cache() -> Result<(), String> {
    crate::cache::clear_custom_playlists_cache()
        .map_err(|e| format!("Failed to clear custom playlists cache: {}", e))
}

/// Write custom playlist tracks cache to disk
#[tauri::command]
pub async fn write_custom_playlist_tracks_cache(
    playlist_id: String,
    data: String,
) -> Result<(), String> {
    crate::cache::write_custom_playlist_tracks_cache(&playlist_id, &data)
        .map_err(|e| format!("Failed to write custom playlist tracks cache: {}", e))
}

/// Read custom playlist tracks cache from disk
#[tauri::command]
pub async fn read_custom_playlist_tracks_cache(
    playlist_id: String,
) -> Result<Option<String>, String> {
    crate::cache::read_custom_playlist_tracks_cache(&playlist_id)
        .map_err(|e| format!("Failed to read custom playlist tracks cache: {}", e))
}

/// Clear custom playlist tracks cache
#[tauri::command]
pub async fn clear_custom_playlist_tracks_cache(playlist_id: String) -> Result<(), String> {
    crate::cache::clear_custom_playlist_tracks_cache(&playlist_id)
        .map_err(|e| format!("Failed to clear custom playlist tracks cache: {}", e))
}

/// Write union playlist tracks cache to disk
#[tauri::command]
pub async fn write_union_playlist_tracks_cache(
    playlist_id: String,
    data: String,
) -> Result<(), String> {
    crate::cache::write_union_playlist_tracks_cache(&playlist_id, &data)
        .map_err(|e| format!("Failed to write union playlist tracks cache: {}", e))
}

/// Read union playlist tracks cache from disk
#[tauri::command]
pub async fn read_union_playlist_tracks_cache(
    playlist_id: String,
) -> Result<Option<String>, String> {
    crate::cache::read_union_playlist_tracks_cache(&playlist_id)
        .map_err(|e| format!("Failed to read union playlist tracks cache: {}", e))
}

/// Clear union playlist tracks cache
#[tauri::command]
pub async fn clear_union_playlist_tracks_cache(playlist_id: String) -> Result<(), String> {
    crate::cache::clear_union_playlist_tracks_cache(&playlist_id)
        .map_err(|e| format!("Failed to clear union playlist tracks cache: {}", e))
}

// ============================================================================
// Playback State Commands
// ============================================================================

/// Save current playback state to disk
#[tauri::command]
pub async fn save_playback_state(state: State<'_, AppState>) -> Result<(), String> {
    let playback = state.playback.lock().await;
    playback.save_state().await
}

/// Restore playback state from disk
#[tauri::command]
pub async fn restore_playback_state(state: State<'_, AppState>) -> Result<(), String> {
    let playback = state.playback.lock().await;
    playback.restore_state().await
}
