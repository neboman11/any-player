/// Provider-specific commands for Spotify and Jellyfin
use crate::commands::{AppState, PlaylistInfo, PlaylistResponse, TrackInfo};
use crate::Source;
use tauri::State;

fn source_key(source: Source) -> &'static str {
    match source {
        Source::Spotify => "spotify",
        Source::Jellyfin => "jellyfin",
        Source::Plex => "plex",
        Source::Custom => "custom",
    }
}

async fn get_provider_handle(
    state: &State<'_, AppState>,
    source: Source,
) -> Result<crate::providers::ProviderHandle, String> {
    let providers = state.providers.lock().await;
    providers
        .get(source)
        .ok_or_else(|| format!("{} provider not initialized", source))
}

fn to_track_info(track: crate::Track, source: &str) -> TrackInfo {
    TrackInfo {
        id: track.id,
        title: track.title,
        artist: track.artist,
        album: track.album,
        duration: track.duration_ms,
        source: source.to_string(),
        url: track.url,
        image_url: track.image_url,
        bitrate_kbps: track.bitrate_kbps,
        sample_rate_hz: track.sample_rate_hz,
    }
}

fn to_playlist_info(playlist: crate::Playlist, source: &str, track_count: usize) -> PlaylistInfo {
    PlaylistInfo {
        id: playlist.id,
        name: playlist.name,
        description: playlist.description,
        track_count,
        owner: playlist.owner,
        source: source.to_string(),
    }
}

fn to_playlist_response(playlist: crate::Playlist, source: &str) -> PlaylistResponse {
    let track_count = playlist.tracks.len();
    let tracks = playlist
        .tracks
        .into_iter()
        .map(|track| to_track_info(track, source))
        .collect();

    PlaylistResponse {
        id: playlist.id,
        name: playlist.name,
        description: playlist.description,
        track_count,
        owner: playlist.owner,
        source: source.to_string(),
        tracks,
    }
}

// ============================================================================
// Spotify Commands
// ============================================================================

/// Get Spotify playlists
#[tauri::command]
pub async fn get_spotify_playlists(
    state: State<'_, AppState>,
) -> Result<Vec<PlaylistInfo>, String> {
    let provider = get_provider_handle(&state, Source::Spotify).await?;

    let provider_locked = provider.lock().await;
    let playlists = provider_locked
        .get_playlists()
        .await
        .map_err(|e| format!("Failed to get playlists: {}", e))?;

    Ok(playlists
        .into_iter()
        .map(|playlist| {
            to_playlist_info(
                playlist.clone(),
                source_key(Source::Spotify),
                playlist.track_count,
            )
        })
        .collect())
}

/// Get a specific Spotify playlist with tracks
#[tauri::command]
pub async fn get_spotify_playlist(
    state: State<'_, AppState>,
    id: String,
) -> Result<PlaylistResponse, String> {
    let provider = get_provider_handle(&state, Source::Spotify).await?;

    let provider_locked = provider.lock().await;
    let playlist = provider_locked
        .get_playlist(&id)
        .await
        .map_err(|e| format!("Failed to get Spotify playlist: {}", e))?;

    Ok(to_playlist_response(playlist, source_key(Source::Spotify)))
}

/// Search tracks on Spotify
#[tauri::command]
pub async fn search_spotify_tracks(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<TrackInfo>, String> {
    let provider = get_provider_handle(&state, Source::Spotify).await?;

    let provider_locked = provider.lock().await;
    let tracks = provider_locked
        .search_tracks(&query)
        .await
        .map_err(|e| format!("Failed to search Spotify tracks: {}", e))?;

    Ok(tracks
        .into_iter()
        .map(|track| to_track_info(track, source_key(Source::Spotify)))
        .collect())
}

// ============================================================================
// Jellyfin Commands
// ============================================================================

/// Get Jellyfin playlists
#[tauri::command]
pub async fn get_jellyfin_playlists(
    state: State<'_, AppState>,
) -> Result<Vec<PlaylistInfo>, String> {
    let provider = get_provider_handle(&state, Source::Jellyfin).await?;

    let provider_locked = provider.lock().await;
    let playlists = provider_locked
        .get_playlists()
        .await
        .map_err(|e| format!("Failed to get Jellyfin playlists: {}", e))?;

    Ok(playlists
        .into_iter()
        .map(|playlist| {
            to_playlist_info(
                playlist.clone(),
                source_key(Source::Jellyfin),
                playlist.track_count,
            )
        })
        .collect())
}

/// Get a specific Jellyfin playlist with tracks
#[tauri::command]
pub async fn get_jellyfin_playlist(
    state: State<'_, AppState>,
    id: String,
) -> Result<PlaylistResponse, String> {
    let provider = get_provider_handle(&state, Source::Jellyfin).await?;

    let provider_locked = provider.lock().await;
    let playlist = provider_locked
        .get_playlist(&id)
        .await
        .map_err(|e| format!("Failed to get Jellyfin playlist: {}", e))?;

    Ok(to_playlist_response(playlist, source_key(Source::Jellyfin)))
}

/// Search tracks on Jellyfin
#[tauri::command]
pub async fn search_jellyfin_tracks(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<TrackInfo>, String> {
    let provider = get_provider_handle(&state, Source::Jellyfin).await?;

    let provider_locked = provider.lock().await;
    let tracks = provider_locked
        .search_tracks(&query)
        .await
        .map_err(|e| format!("Failed to search Jellyfin tracks: {}", e))?;

    Ok(tracks
        .into_iter()
        .map(|track| to_track_info(track, source_key(Source::Jellyfin)))
        .collect())
}

/// Search playlists on Jellyfin
#[tauri::command]
pub async fn search_jellyfin_playlists(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<PlaylistInfo>, String> {
    let provider = get_provider_handle(&state, Source::Jellyfin).await?;

    let provider_locked = provider.lock().await;
    let playlists = provider_locked
        .search_playlists(&query)
        .await
        .map_err(|e| format!("Failed to search Jellyfin playlists: {}", e))?;

    Ok(playlists
        .into_iter()
        .map(|playlist| {
            let track_count = playlist.tracks.len();
            to_playlist_info(playlist, source_key(Source::Jellyfin), track_count)
        })
        .collect())
}

/// Get recently played tracks from Jellyfin
#[tauri::command]
pub async fn get_jellyfin_recently_played(
    state: State<'_, AppState>,
    limit: usize,
) -> Result<Vec<TrackInfo>, String> {
    let provider = get_provider_handle(&state, Source::Jellyfin).await?;

    let provider_locked = provider.lock().await;
    let tracks = provider_locked
        .get_recently_played(limit)
        .await
        .map_err(|e| format!("Failed to get recently played: {}", e))?;

    Ok(tracks
        .into_iter()
        .map(|track| to_track_info(track, source_key(Source::Jellyfin)))
        .collect())
}

// ============================================================================
// Plex Commands
// ============================================================================

/// Get Plex playlists
#[tauri::command]
pub async fn get_plex_playlists(state: State<'_, AppState>) -> Result<Vec<PlaylistInfo>, String> {
    let provider = get_provider_handle(&state, Source::Plex).await?;

    let provider_locked = provider.lock().await;
    let playlists = provider_locked
        .get_playlists()
        .await
        .map_err(|e| format!("Failed to get Plex playlists: {}", e))?;

    Ok(playlists
        .into_iter()
        .map(|playlist| {
            to_playlist_info(
                playlist.clone(),
                source_key(Source::Plex),
                playlist.track_count,
            )
        })
        .collect())
}

/// Get a specific Plex playlist with tracks
#[tauri::command]
pub async fn get_plex_playlist(
    state: State<'_, AppState>,
    id: String,
) -> Result<PlaylistResponse, String> {
    let provider = get_provider_handle(&state, Source::Plex).await?;

    let provider_locked = provider.lock().await;
    let playlist = provider_locked
        .get_playlist(&id)
        .await
        .map_err(|e| format!("Failed to get Plex playlist: {}", e))?;

    Ok(to_playlist_response(playlist, source_key(Source::Plex)))
}

/// Search tracks on Plex
#[tauri::command]
pub async fn search_plex_tracks(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<TrackInfo>, String> {
    let provider = get_provider_handle(&state, Source::Plex).await?;

    let provider_locked = provider.lock().await;
    let tracks = provider_locked
        .search_tracks(&query)
        .await
        .map_err(|e| format!("Failed to search Plex tracks: {}", e))?;

    Ok(tracks
        .into_iter()
        .map(|track| to_track_info(track, source_key(Source::Plex)))
        .collect())
}

/// Search playlists on Plex
#[tauri::command]
pub async fn search_plex_playlists(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<PlaylistInfo>, String> {
    let provider = get_provider_handle(&state, Source::Plex).await?;

    let provider_locked = provider.lock().await;
    let playlists = provider_locked
        .search_playlists(&query)
        .await
        .map_err(|e| format!("Failed to search Plex playlists: {}", e))?;

    Ok(playlists
        .into_iter()
        .map(|playlist| {
            let track_count = playlist.tracks.len();
            to_playlist_info(playlist, source_key(Source::Plex), track_count)
        })
        .collect())
}

/// Get recently played tracks from Plex
#[tauri::command]
pub async fn get_plex_recently_played(
    state: State<'_, AppState>,
    limit: usize,
) -> Result<Vec<TrackInfo>, String> {
    let provider = get_provider_handle(&state, Source::Plex).await?;

    let provider_locked = provider.lock().await;
    let tracks = provider_locked
        .get_recently_played(limit)
        .await
        .map_err(|e| format!("Failed to get recently played: {}", e))?;

    Ok(tracks
        .into_iter()
        .map(|track| to_track_info(track, source_key(Source::Plex)))
        .collect())
}
