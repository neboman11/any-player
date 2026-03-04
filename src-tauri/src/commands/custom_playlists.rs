/// Custom playlist management commands
use crate::commands::AppState;
use crate::database::{ColumnPreferences, CustomPlaylist, PlaylistTrack, UnionPlaylistSource};
use crate::models::Track;
use crate::Source;
use any_player_core::config_export::{
    ExportConfigPayload, ExportCustomPlaylist, ExportPlaylist, ExportPlaylistTrack,
    ExportProviderConfigs, ExportServerConfig, ExportSpotifyConfig, ExportUnionPlaylistSource,
    CONFIG_EXPORT_VERSION,
};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::State;

fn map_export_playlist(value: CustomPlaylist) -> ExportPlaylist {
    ExportPlaylist {
        id: value.id,
        name: value.name,
        description: value.description,
        image_url: value.image_url,
        created_at: value.created_at,
        updated_at: value.updated_at,
        track_count: value.track_count,
        playlist_type: value.playlist_type,
    }
}

fn map_export_track(value: PlaylistTrack) -> ExportPlaylistTrack {
    ExportPlaylistTrack {
        id: value.id,
        playlist_id: value.playlist_id,
        track_source: value.track_source,
        track_id: value.track_id,
        position: value.position,
        added_at: value.added_at,
        title: value.title,
        artist: value.artist,
        album: value.album,
        duration_ms: value.duration_ms,
        image_url: value.image_url,
    }
}

fn map_export_union_source(value: UnionPlaylistSource) -> ExportUnionPlaylistSource {
    ExportUnionPlaylistSource {
        id: value.id,
        union_playlist_id: value.union_playlist_id,
        source_type: value.source_type,
        source_playlist_id: value.source_playlist_id,
        position: value.position,
        added_at: value.added_at,
    }
}

fn provider_source_from_str(source_type: &str) -> Option<Source> {
    match source_type.to_lowercase().as_str() {
        "spotify" => Some(Source::Spotify),
        "jellyfin" => Some(Source::Jellyfin),
        "plex" => Some(Source::Plex),
        _ => None,
    }
}

async fn playlist_track_counts_from_provider(
    provider: Option<crate::providers::ProviderHandle>,
    source: Source,
) -> HashMap<String, i64> {
    if let Some(provider) = provider {
        let provider_locked = provider.lock().await;
        match provider_locked.get_playlists().await {
            Ok(playlists) => playlists
                .into_iter()
                .map(|playlist| (playlist.id, playlist.track_count as i64))
                .collect(),
            Err(e) => {
                tracing::warn!(
                    "Failed to load {} playlist summaries for union counts: {}",
                    source,
                    e
                );
                HashMap::new()
            }
        }
    } else {
        HashMap::new()
    }
}

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
    db.create_playlist_with_type(name, description, image_url, "union".to_string(), false)
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

    let custom_track_counts: HashMap<String, usize> = {
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

    // Fetch provider playlist summaries once (lightweight) and reuse them for
    // union track-count calculation to avoid expensive full playlist track fetches.
    let (spotify_provider, jellyfin_provider, plex_provider) = {
        let providers = state.providers.lock().await;
        (
            providers.get(Source::Spotify),
            providers.get(Source::Jellyfin),
            providers.get(Source::Plex),
        )
    };

    let spotify_track_counts =
        playlist_track_counts_from_provider(spotify_provider, Source::Spotify).await;
    let jellyfin_track_counts =
        playlist_track_counts_from_provider(jellyfin_provider, Source::Jellyfin).await;
    let plex_track_counts = playlist_track_counts_from_provider(plex_provider, Source::Plex).await;

    let provider_track_counts: HashMap<Source, HashMap<String, i64>> = HashMap::from([
        (Source::Spotify, spotify_track_counts),
        (Source::Jellyfin, jellyfin_track_counts),
        (Source::Plex, plex_track_counts),
    ]);

    for playlist in &mut playlists {
        if playlist.playlist_type == "union" {
            if let Some(sources) = union_sources_map.get(&playlist.id) {
                let mut total_tracks: i64 = 0;
                for source in sources {
                    if source.source_type.eq_ignore_ascii_case("custom") {
                        if let Some(&count) = custom_track_counts.get(&source.source_playlist_id) {
                            total_tracks += count as i64;
                        }
                        continue;
                    }

                    if let Some(provider_source) = provider_source_from_str(&source.source_type) {
                        if let Some(source_counts) = provider_track_counts.get(&provider_source) {
                            if let Some(count) =
                                source_counts.get(source.source_playlist_id.as_str())
                            {
                                total_tracks += *count;
                            }
                        }
                    }
                }
                playlist.track_count = total_tracks;
            }
        }
    }

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
pub async fn export_app_config(state: State<'_, AppState>) -> Result<ExportConfigPayload, String> {
    let config =
        crate::config::Config::load().map_err(|e| format!("Failed to load config: {}", e))?;
    let tokens = crate::config::Config::load_tokens()
        .map_err(|e| format!("Failed to load provider tokens: {}", e))?;

    let provider_configs = ExportProviderConfigs {
        spotify: ExportSpotifyConfig {
            client_id: config
                .spotify
                .as_ref()
                .and_then(|spotify| spotify.client_id.clone()),
            redirect_uri: config
                .spotify
                .as_ref()
                .and_then(|spotify| spotify.redirect_uri.clone()),
        },
        jellyfin: ExportServerConfig {
            base_url: tokens.jellyfin_url,
        },
        plex: ExportServerConfig {
            base_url: tokens.plex_url,
        },
    };

    let custom_playlists = {
        let db = state.database.lock().await;
        let playlists = db
            .get_all_playlists()
            .map_err(|e| format!("Failed to get playlists for export: {}", e))?;

        let mut exported = Vec::with_capacity(playlists.len());
        for playlist in playlists {
            let tracks = db
                .get_playlist_tracks(&playlist.id)
                .map_err(|e| format!("Failed to get playlist tracks for export: {}", e))?;
            let union_sources = if playlist.playlist_type == "union" {
                db.get_union_playlist_sources(&playlist.id).map_err(|e| {
                    format!("Failed to get union playlist sources for export: {}", e)
                })?
            } else {
                Vec::new()
            };

            exported.push(ExportCustomPlaylist {
                playlist: map_export_playlist(playlist),
                tracks: tracks.into_iter().map(map_export_track).collect(),
                union_sources: union_sources
                    .into_iter()
                    .map(map_export_union_source)
                    .collect(),
            });
        }

        exported
    };

    Ok(ExportConfigPayload {
        export_version: CONFIG_EXPORT_VERSION,
        provider_configs,
        custom_playlists,
    })
}

#[tauri::command]
pub async fn export_app_config_to_file(state: State<'_, AppState>) -> Result<String, String> {
    let payload: ExportConfigPayload = export_app_config(state).await?;
    let content = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("Failed to serialize export payload: {}", e))?;

    let export_dir = dirs::download_dir()
        .or_else(dirs::document_dir)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "Unable to resolve export directory".to_string())?;

    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let base_name = format!("any-player-config-{}.json", date);
    let mut export_path = export_dir.join(&base_name);

    let mut suffix = 1;
    while export_path.exists() {
        export_path = export_dir.join(format!("any-player-config-{}-{}.json", date, suffix));
        suffix += 1;
    }

    std::fs::write(&export_path, content)
        .map_err(|e| format!("Failed to write export file: {}", e))?;

    Ok(export_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn export_app_config_to_path(
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let trimmed_path = path.trim();
    if trimmed_path.is_empty() {
        return Err("Export path cannot be empty".to_string());
    }

    let payload: ExportConfigPayload = export_app_config(state).await?;
    let content = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("Failed to serialize export payload: {}", e))?;

    let export_path = PathBuf::from(trimmed_path);
    if let Some(parent) = export_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to prepare export directory: {}", e))?;
    }

    std::fs::write(&export_path, content)
        .map_err(|e| format!("Failed to write export file: {}", e))?;

    Ok(export_path.to_string_lossy().to_string())
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

        if source.source_type.eq_ignore_ascii_case("custom") {
            let tracks = db
                .get_playlist_tracks(&source.source_playlist_id)
                .map_err(|e| format!("Failed to get custom playlist tracks: {}", e))?;
            tracing::info!(
                "Got {} tracks from custom playlist {}",
                tracks.len(),
                source.source_playlist_id
            );
            all_tracks.extend(tracks.into_iter().map(|t| t.to_track()));
            continue;
        }

        if let Some(provider_source) = provider_source_from_str(&source.source_type) {
            match providers
                .get_playlist(provider_source, &source.source_playlist_id)
                .await
            {
                Ok(playlist) => {
                    tracing::info!(
                        "Got {} tracks from {} playlist {}",
                        playlist.tracks.len(),
                        provider_source,
                        source.source_playlist_id
                    );
                    all_tracks.extend(playlist.tracks);
                }
                Err(e) => {
                    tracing::error!("Failed to get {} playlist tracks: {}", provider_source, e);
                }
            }
        } else {
            tracing::warn!("Unknown source type: {}", source.source_type);
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
            if source.source_type.eq_ignore_ascii_case("custom") {
                let db = state.database.lock().await;
                if let Ok(tracks) = db.get_playlist_tracks(&source.source_playlist_id) {
                    all_tracks.extend(tracks.into_iter().map(|t| t.to_track()));
                }
                drop(db);
                continue;
            }

            if let Some(provider_source) = provider_source_from_str(&source.source_type) {
                if let Ok(playlist) = providers
                    .get_playlist(provider_source, &source.source_playlist_id)
                    .await
                {
                    all_tracks.extend(playlist.tracks);
                }
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
            let track_result =
                if let Some(provider_source) = provider_source_from_str(&pt.track_source) {
                    providers.get_track(provider_source, &pt.track_id).await
                } else {
                    Ok(pt.to_track())
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
