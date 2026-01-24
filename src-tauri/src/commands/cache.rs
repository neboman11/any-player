/// Cache management commands

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
