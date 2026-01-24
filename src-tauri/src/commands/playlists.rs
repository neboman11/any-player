/// Playlist management commands
use crate::commands::{AppState, PlaylistInfo, TrackInfo};
use tauri::State;

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
            // Drop providers lock before calling internal function
            drop(providers);
            return super::custom_playlists::play_custom_playlist_internal(&state, playlist_id)
                .await;
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
        super::helpers::enrich_queued_tracks_eager(playback_arc, providers_arc, first_track_index)
            .await;
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
        let auth_headers = providers.get_auth_headers(source).await;

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

    // Ensure we have at least one track after conversion
    if internal_tracks.is_empty() {
        return Err("No valid tracks to play".to_string());
    }

    // Store first track for later enrichment
    let first_track_for_enrichment = internal_tracks[0].clone();
    let needs_enrichment = matches!(
        first_track_for_enrichment.source,
        crate::models::Source::Spotify | crate::models::Source::Jellyfin
    );

    // Enrich the first track before setting up the queue (if needed)
    let enriched_first_track = if needs_enrichment {
        match first_track_for_enrichment.source {
            crate::models::Source::Spotify => providers
                .get_spotify_track(&first_track_for_enrichment.id)
                .await
                .ok(),
            crate::models::Source::Jellyfin => {
                // Must enrich Jellyfin tracks immediately to get auth headers
                providers
                    .get_jellyfin_track(&first_track_for_enrichment.id)
                    .await
                    .ok()
            }
            _ => None,
        }
    } else {
        None
    };

    drop(providers); // Release providers lock

    // Now set up the queue and start playback with the enriched track
    let playback = state.playback.lock().await;

    // Clear queue and add all tracks
    playback.clear_queue().await;
    playback.queue_tracks(internal_tracks.clone()).await;

    // Check if shuffle is enabled and determine first track index
    let info = playback.get_info().await;
    let first_track_index = if info.shuffle {
        // Generate shuffle order
        let queue_arc = playback.get_queue_arc();
        let mut queue = queue_arc.lock().await;
        queue.generate_shuffle_order();
        queue.current_index = 0;

        // Get the first track according to shuffle order
        let idx =
            if !queue.shuffle_order.is_empty() && queue.shuffle_order[0] < internal_tracks.len() {
                queue.shuffle_order[0]
            } else {
                0
            };

        // If we enriched the first track and it's the one we're playing, update it in the queue
        if idx == 0 && enriched_first_track.is_some() {
            queue.tracks[idx] = enriched_first_track.clone().unwrap();
        }
        drop(queue);
        idx
    } else {
        // If we enriched the first track, update it in the queue
        if let Some(enriched) = &enriched_first_track {
            let queue_arc = playback.get_queue_arc();
            let mut queue = queue_arc.lock().await;
            if !queue.tracks.is_empty() {
                queue.tracks[0] = enriched.clone();
            }
        }
        0
    };

    // Play the track (either enriched or original)
    let track_to_play = if first_track_index == 0 {
        enriched_first_track.unwrap_or_else(|| internal_tracks[first_track_index].clone())
    } else {
        internal_tracks[first_track_index].clone()
    };

    playback.play_track(track_to_play).await;
    drop(playback);

    // Spawn background task to eagerly enrich the next tracks
    // This ensures Jellyfin tracks have auth headers ready before playback reaches them
    let playback_arc = state.playback.clone();
    let providers_arc = state.providers.clone();
    let first_track_index_clone = first_track_index;
    tokio::spawn(async move {
        super::helpers::enrich_queued_tracks_eager(
            playback_arc,
            providers_arc,
            first_track_index_clone,
        )
        .await;
    });

    Ok(())
}
