/// Custom playlist management commands
use crate::commands::AppState;
use crate::database::{ColumnPreferences, CustomPlaylist, PlaylistTrack, UnionPlaylistSource};
use crate::models::Track;
use tauri::State;

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
    let (mut playlists, union_sources_map) = {
        let db = state.database.lock().await;
        let playlists = db
            .get_all_playlists()
            .map_err(|e| format!("Failed to get playlists: {}", e))?;

        let mut union_sources_map = std::collections::HashMap::new();
        for playlist in &playlists {
            if playlist.playlist_type == "union" {
                let sources = db
                    .get_union_playlist_sources(&playlist.id)
                    .map_err(|e| format!("Failed to get union playlist sources: {}", e))?;
                union_sources_map.insert(playlist.id.clone(), sources);
            }
        }

        (playlists, union_sources_map)
    };

    let mut custom_playlist_ids = Vec::new();
    for sources in union_sources_map.values() {
        for source in sources {
            if source.source_type == "custom" {
                custom_playlist_ids.push(source.source_playlist_id.clone());
            }
        }
    }

    let custom_track_counts: std::collections::HashMap<String, usize> = {
        let db = state.database.lock().await;
        custom_playlist_ids
            .into_iter()
            .filter_map(|id| {
                db.get_playlist_tracks(&id)
                    .ok()
                    .map(|tracks| (id, tracks.len()))
            })
            .collect()
    };

    let providers = state.providers.lock().await;
    for playlist in &mut playlists {
        if playlist.playlist_type == "union" {
            if let Some(sources) = union_sources_map.get(&playlist.id) {
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
                            if let Some(&count) =
                                custom_track_counts.get(&source.source_playlist_id)
                            {
                                total_tracks += count as i64;
                            }
                        }
                        _ => {}
                    }
                }
                playlist.track_count = total_tracks;
            }
        }
    }

    drop(providers);
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
                    }
                }
            }
            "custom" => {
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

/// Internal helper for playing custom playlists
pub(super) async fn play_custom_playlist_internal(
    state: &AppState,
    playlist_id: String,
) -> Result<(), String> {
    let db = state.database.lock().await;
    let providers = state.providers.lock().await;

    let playlist_info = db
        .get_playlist(&playlist_id)
        .map_err(|e| format!("Failed to get playlist info: {}", e))?
        .ok_or_else(|| format!("Playlist not found: {}", playlist_id))?;

    let tracks_with_urls = if playlist_info.playlist_type == "union" {
        let sources = db
            .get_union_playlist_sources(&playlist_id)
            .map_err(|e| format!("Failed to get union playlist sources: {}", e))?;

        drop(db);

        let mut all_tracks = Vec::new();

        for source in sources {
            match source.source_type.as_str() {
                "spotify" => {
                    if let Ok(playlist) = providers
                        .get_spotify_playlist(&source.source_playlist_id)
                        .await
                    {
                        all_tracks.extend(playlist.tracks);
                    }
                }
                "jellyfin" => {
                    if let Ok(playlist) = providers
                        .get_jellyfin_playlist(&source.source_playlist_id)
                        .await
                    {
                        all_tracks.extend(playlist.tracks);
                    }
                }
                "custom" => {
                    let db = state.database.lock().await;
                    if let Ok(tracks) = db.get_playlist_tracks(&source.source_playlist_id) {
                        all_tracks.extend(tracks.into_iter().map(|t| t.to_track()));
                    }
                    drop(db);
                }
                _ => {}
            }
        }

        all_tracks
    } else {
        let playlist_tracks = db
            .get_playlist_tracks(&playlist_id)
            .map_err(|e| format!("Failed to get custom playlist tracks: {}", e))?;

        drop(db);

        let mut tracks = Vec::new();
        for pt in playlist_tracks {
            let track_result = match pt.track_source.as_str() {
                "Spotify" | "spotify" => providers.get_spotify_track(&pt.track_id).await,
                "Jellyfin" | "jellyfin" => providers.get_jellyfin_track(&pt.track_id).await,
                _ => Ok(pt.to_track()),
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

    let playback = state.playback.lock().await;
    playback.clear_queue().await;
    playback.queue_tracks(tracks_with_urls.clone()).await;

    let info = playback.get_info().await;
    if info.shuffle {
        let queue_arc = playback.get_queue_arc();
        let mut queue = queue_arc.lock().await;
        queue.generate_shuffle_order();
        queue.current_index = 0;

        let first_track_index =
            if !queue.shuffle_order.is_empty() && queue.shuffle_order[0] < tracks_with_urls.len() {
                queue.shuffle_order[0]
            } else {
                0
            };
        drop(queue);

        playback
            .play_track(tracks_with_urls[first_track_index].clone())
            .await;

        let first_idx = first_track_index;
        drop(playback);

        let playback_arc = state.playback.clone();
        let providers_arc = state.providers.clone();
        tokio::spawn(async move {
            super::helpers::enrich_queued_tracks_eager(playback_arc, providers_arc, first_idx)
                .await;
        });
    } else {
        playback.play_track(tracks_with_urls[0].clone()).await;
        drop(playback);

        let playback_arc = state.playback.clone();
        let providers_arc = state.providers.clone();
        tokio::spawn(async move {
            super::helpers::enrich_queued_tracks_eager(playback_arc, providers_arc, 0).await;
        });
    }

    Ok(())
}
