/// Helper functions for track management and enrichment
use crate::commands::AppState;
use crate::{PlaybackManager, ProviderRegistry};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Delay between API calls when eagerly enriching queued tracks (in milliseconds)
/// This prevents overwhelming external APIs with rapid consecutive requests
const TRACK_ENRICHMENT_DELAY_MS: u64 = 50;

/// Eagerly enrich queued tracks with full details (URLs, auth headers, etc.)
/// Prioritizes tracks near the current playback position and loads them immediately
pub async fn enrich_queued_tracks_eager(
    playback: Arc<Mutex<PlaybackManager>>,
    providers: Arc<Mutex<ProviderRegistry>>,
    current_index: usize,
) {
    const LOOKAHEAD_COUNT: usize = 10; // Number of tracks to load ahead

    let pb = playback.lock().await;
    let queue_arc = pb.get_queue_arc();
    drop(pb); // Release playback lock

    // Gather all information we need in a single lock acquisition
    let (total_tracks, _shuffle_enabled, _shuffle_order, tracks_to_enrich) = {
        let queue = queue_arc.lock().await;
        let total_tracks = queue.tracks.len();
        let shuffle_enabled = !queue.shuffle_order.is_empty();
        let shuffle_order = queue.shuffle_order.clone();

        // Calculate which tracks to load first (starting from index 1 since 0 is already playing)
        let mut indices_to_load = Vec::new();
        for i in 1..=LOOKAHEAD_COUNT.min(total_tracks.saturating_sub(1)) {
            let actual_index = if shuffle_enabled {
                let shuffle_pos = (current_index + i) % total_tracks;
                if shuffle_pos < shuffle_order.len() {
                    shuffle_order[shuffle_pos]
                } else {
                    i
                }
            } else {
                (current_index + i) % total_tracks
            };
            indices_to_load.push(actual_index);
        }

        // Gather track info that needs enrichment
        let mut tracks_info = Vec::new();
        for &track_idx in &indices_to_load {
            if track_idx < queue.tracks.len() {
                let track = &queue.tracks[track_idx];
                // Skip if track already has a URL (already enriched)
                if track.url.is_none() {
                    tracks_info.push((track_idx, track.id.clone(), track.source));
                }
            }
        }

        (total_tracks, shuffle_enabled, shuffle_order, tracks_info)
    };

    // Now fetch track details without holding any locks
    let providers_lock = providers.lock().await;
    let mut enriched_tracks = Vec::new();

    for (track_idx, track_id, source) in tracks_to_enrich {
        // Fetch full track details
        let enriched_track_result = match source {
            crate::models::Source::Spotify => providers_lock.get_spotify_track(&track_id).await,
            crate::models::Source::Jellyfin => providers_lock.get_jellyfin_track(&track_id).await,
            _ => continue, // Skip custom tracks
        };

        if let Ok(enriched_track) = enriched_track_result {
            enriched_tracks.push((track_idx, enriched_track));
            tracing::debug!("Eagerly enriched track {} at index {}", track_id, track_idx);
        } else {
            tracing::warn!("Failed to enrich track {} at index {}", track_id, track_idx);
        }

        // Small delay to avoid overwhelming the API
        tokio::time::sleep(tokio::time::Duration::from_millis(
            TRACK_ENRICHMENT_DELAY_MS,
        ))
        .await;
    }

    drop(providers_lock);

    // Update all enriched tracks in a single lock acquisition
    if !enriched_tracks.is_empty() {
        let mut queue = queue_arc.lock().await;
        for (track_idx, enriched_track) in enriched_tracks {
            if track_idx < queue.tracks.len() {
                queue.tracks[track_idx] = enriched_track;
            }
        }
    }

    tracing::info!(
        "Completed eager loading for queue (total tracks: {})",
        total_tracks
    );
}

/// Helper function to initialize Spotify session for premium users
/// Consolidates the duplicated logic from authenticate_spotify and check_oauth_code
pub async fn initialize_premium_session_if_needed(state: &AppState) -> Result<(), String> {
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

/// Maximum age of temporary audio files before cleanup (1 hour)
const TEMP_FILE_MAX_AGE_SECONDS: u64 = 3600;

/// Minimum interval between cleanup runs (5 minutes)
const CLEANUP_INTERVAL_SECONDS: u64 = 300;

/// Last cleanup timestamp (in seconds since UNIX epoch)
use std::sync::atomic::{AtomicU64, Ordering};
static LAST_CLEANUP: AtomicU64 = AtomicU64::new(0);

/// Clean up old temporary audio files (older than 1 hour)
/// Rate-limited to run at most once every 5 minutes to avoid expensive directory scans
pub fn cleanup_old_temp_audio_files() {
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
