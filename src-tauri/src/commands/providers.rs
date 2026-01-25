/// Provider-specific commands for Spotify and Jellyfin
use crate::commands::{AppState, PlaylistInfo, PlaylistResponse, TrackInfo};
use tauri::State;

// ============================================================================
// Spotify Commands
// ============================================================================

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
            image_url: t.image_url.clone(),
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
            image_url: t.image_url,
        })
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
            image_url: t.image_url.clone(),
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
            image_url: t.image_url,
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
            image_url: t.image_url,
        })
        .collect())
}
