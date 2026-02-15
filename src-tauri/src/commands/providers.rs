/// Provider-specific commands for Spotify and Jellyfin
use crate::commands::{AppState, PlaylistInfo, PlaylistResponse, TrackInfo};
use crate::Source;
use tauri::State;

// ============================================================================
// Spotify Commands
// ============================================================================

/// Get Spotify playlists
#[tauri::command]
pub async fn get_spotify_playlists(
    state: State<'_, AppState>,
) -> Result<Vec<PlaylistInfo>, String> {
    let provider = {
        let providers = state.providers.lock().await;
        providers
            .get(Source::Spotify)
            .ok_or_else(|| "Spotify provider not initialized".to_string())?
    };

    let provider_locked = provider.lock().await;
    let playlists = provider_locked
        .get_playlists()
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
    let provider = {
        let providers = state.providers.lock().await;
        providers
            .get(Source::Spotify)
            .ok_or_else(|| "Spotify provider not initialized".to_string())?
    };

    let provider_locked = provider.lock().await;
    let playlist = provider_locked
        .get_playlist(&id)
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
            bitrate_kbps: t.bitrate_kbps,
            sample_rate_hz: t.sample_rate_hz,
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
    let provider = {
        let providers = state.providers.lock().await;
        providers
            .get(Source::Spotify)
            .ok_or_else(|| "Spotify provider not initialized".to_string())?
    };

    let provider_locked = provider.lock().await;
    let tracks = provider_locked
        .search_tracks(&query)
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
            bitrate_kbps: t.bitrate_kbps,
            sample_rate_hz: t.sample_rate_hz,
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
    let provider = {
        let providers = state.providers.lock().await;
        providers
            .get(Source::Jellyfin)
            .ok_or_else(|| "Jellyfin provider not initialized".to_string())?
    };

    let provider_locked = provider.lock().await;
    let playlists = provider_locked
        .get_playlists()
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
    let provider = {
        let providers = state.providers.lock().await;
        providers
            .get(Source::Jellyfin)
            .ok_or_else(|| "Jellyfin provider not initialized".to_string())?
    };

    let provider_locked = provider.lock().await;
    let playlist = provider_locked
        .get_playlist(&id)
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
            bitrate_kbps: t.bitrate_kbps,
            sample_rate_hz: t.sample_rate_hz,
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
    let provider = {
        let providers = state.providers.lock().await;
        providers
            .get(Source::Jellyfin)
            .ok_or_else(|| "Jellyfin provider not initialized".to_string())?
    };

    let provider_locked = provider.lock().await;
    let tracks = provider_locked
        .search_tracks(&query)
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
            bitrate_kbps: t.bitrate_kbps,
            sample_rate_hz: t.sample_rate_hz,
        })
        .collect())
}

/// Search playlists on Jellyfin
#[tauri::command]
pub async fn search_jellyfin_playlists(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<PlaylistInfo>, String> {
    let provider = {
        let providers = state.providers.lock().await;
        providers
            .get(Source::Jellyfin)
            .ok_or_else(|| "Jellyfin provider not initialized".to_string())?
    };

    let provider_locked = provider.lock().await;
    let playlists = provider_locked
        .search_playlists(&query)
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
    let provider = {
        let providers = state.providers.lock().await;
        providers
            .get(Source::Jellyfin)
            .ok_or_else(|| "Jellyfin provider not initialized".to_string())?
    };

    let provider_locked = provider.lock().await;
    let tracks = provider_locked
        .get_recently_played(limit)
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
            bitrate_kbps: t.bitrate_kbps,
            sample_rate_hz: t.sample_rate_hz,
        })
        .collect())
}
