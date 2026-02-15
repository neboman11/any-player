/// Playlist management commands
use crate::commands::{AppState, PlaylistInfo, TrackInfo};
use crate::Source;
use tauri::State;

fn parse_source(source: &str) -> Result<Source, String> {
    match source.to_lowercase().as_str() {
        "spotify" => Ok(Source::Spotify),
        "jellyfin" => Ok(Source::Jellyfin),
        "plex" => Ok(Source::Plex),
        "custom" => Ok(Source::Custom),
        _ => Err(format!(
            "Unknown source: '{}'. Supported sources are: spotify, jellyfin, plex, custom",
            source
        )),
    }
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

    let parsed_source = parse_source(&source)?;
    if matches!(parsed_source, Source::Custom) {
        return Err("Playing custom tracks directly is not yet supported. Please play from a custom playlist instead.".to_string());
    }

    let track = providers
        .get_track(parsed_source, &track_id)
        .await
        .map_err(|e| format!("Failed to get {} track: {}", parsed_source, e))?;

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

    let parsed_source = parse_source(&source)?;
    if matches!(parsed_source, Source::Custom) {
        return Err("Queuing custom tracks directly is not yet supported. Please queue from a custom playlist instead.".to_string());
    }

    let track = providers
        .get_track(parsed_source, &track_id)
        .await
        .map_err(|e| format!("Failed to get {} track: {}", parsed_source, e))?;

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

    let parsed_source = parse_source(&source)?;

    // Get the playlist with all tracks from the appropriate provider
    let playlist = match parsed_source {
        Source::Spotify | Source::Jellyfin | Source::Plex => providers
            .get_playlist(parsed_source, &playlist_id)
            .await
            .map_err(|e| format!("Failed to get {} playlist: {}", parsed_source, e))?,
        Source::Custom => {
            // Drop providers lock before calling internal function
            drop(providers);
            return super::custom_playlists::play_custom_playlist_internal(&state, playlist_id)
                .await;
        }
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
    preserve_first_in_shuffle: Option<bool>,
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
            "spotify" => Source::Spotify,
            "jellyfin" => Source::Jellyfin,
            "plex" => Source::Plex,
            _ => Source::Custom,
        };

        // Get auth headers for sources that need them (e.g., Jellyfin)
        let auth_headers = providers.get_auth_headers(source).await;

        internal_tracks.push(crate::models::Track {
            id: track_info.id,
            title: track_info.title,
            artist: track_info.artist,
            album: track_info.album,
            duration_ms: track_info.duration,
            image_url: track_info.image_url,
            source,
            url: track_info.url,
            bitrate_kbps: track_info.bitrate_kbps,
            sample_rate_hz: track_info.sample_rate_hz,
            auth_headers,
            enriched: false,
        });
    }

    // Ensure we have at least one track after conversion
    if internal_tracks.is_empty() {
        return Err("No valid tracks to play".to_string());
    }

    drop(providers); // Release providers lock

    // Set up queue and determine which track should start
    let playback = state.playback.lock().await;

    // Clear queue and add all tracks
    playback.clear_queue().await;
    playback.queue_tracks(internal_tracks.clone()).await;

    // Check if shuffle is enabled and determine first track index
    let info = playback.get_info().await;
    let preserve_first = preserve_first_in_shuffle.unwrap_or(false);
    let first_track_index = if info.shuffle {
        let queue_arc = playback.get_queue_arc();
        let mut queue = queue_arc.lock().await;
        if preserve_first {
            // For play-from-track flows, keep the supplied first track first.
            queue.generate_shuffle_order_keep_first();
        } else {
            // For play-all flows, start from a random track.
            queue.generate_shuffle_order();
        }
        queue.current_index = 0;

        if !queue.shuffle_order.is_empty() && queue.shuffle_order[0] < queue.tracks.len() {
            queue.shuffle_order[0]
        } else {
            0
        }
    } else {
        0
    };

    drop(playback);

    // Enrich the chosen first track before playback if needed (e.g., Jellyfin auth headers)
    let first_track_for_enrichment = internal_tracks[first_track_index].clone();
    let needs_enrichment = matches!(
        first_track_for_enrichment.source,
        Source::Spotify | Source::Jellyfin | Source::Plex
    );

    let enriched_first_track = if needs_enrichment {
        let providers = state.providers.lock().await;
        match first_track_for_enrichment.source {
            Source::Spotify => providers
                .get_track(Source::Spotify, &first_track_for_enrichment.id)
                .await
                .ok(),
            Source::Jellyfin => providers
                .get_track(Source::Jellyfin, &first_track_for_enrichment.id)
                .await
                .ok(),
            Source::Plex => providers
                .get_track(Source::Plex, &first_track_for_enrichment.id)
                .await
                .ok(),
            _ => None,
        }
    } else {
        None
    };

    // Play the track (either enriched or original)
    let track_to_play = enriched_first_track
        .clone()
        .unwrap_or_else(|| internal_tracks[first_track_index].clone());

    // Ensure queue contains the enriched first track so subsequent playback uses it
    let playback = state.playback.lock().await;
    if let Some(enriched) = &enriched_first_track {
        let queue_arc = playback.get_queue_arc();
        let mut queue = queue_arc.lock().await;
        if first_track_index < queue.tracks.len() {
            queue.tracks[first_track_index] = enriched.clone();
        }
    }

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
