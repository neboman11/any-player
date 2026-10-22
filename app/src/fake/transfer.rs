//! Demo transfer uses the same model envelopes as production, never a UI-private file format.
use crate::vm::{LibrarySnapshot, PlaylistCard, Result, ServiceError, TrackOccurrence};
use ap_model::{
    CustomPlaylist, PlaylistTrack, PlaylistType, SourceType, Track, UnionPlaylistSource, wire,
};
const DATE: &str = "2026-01-01T00:00:00Z";
fn position(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

pub fn state(library: &LibrarySnapshot) -> wire::AnyPlayerStateEnvelope {
    let mut data = wire::AnyPlayerStateData::default();
    for playlist in library
        .catalog
        .iter()
        .filter(|p| p.source == SourceType::Custom)
    {
        data.custom_playlists.push(CustomPlaylist {
            id: playlist.id.clone(),
            name: playlist.name.clone(),
            description: Some(playlist.description.clone()),
            image_url: None,
            created_at: DATE.into(),
            updated_at: DATE.into(),
            track_count: position(playlist.count),
            playlist_type: if playlist.union {
                PlaylistType::Union
            } else {
                PlaylistType::Standard
            },
            is_distinct: playlist.distinct,
        });
        for (index, row) in library
            .tracks
            .get(&playlist.id)
            .into_iter()
            .flatten()
            .enumerate()
        {
            data.playlist_tracks.push(PlaylistTrack {
                id: row.key.clone(),
                playlist_id: playlist.id.clone(),
                track_source: row.track.source,
                track_id: row.track.id.clone(),
                position: position(index),
                added_at: DATE.into(),
                title: row.track.title.clone(),
                artist: row.track.artist.clone(),
                album: row.track.album.clone(),
                duration_ms: row.track.duration_ms,
                image_url: row.track.image_url.clone(),
                url: row.track.url.clone(),
            });
        }
        for (index, id) in library
            .sources
            .get(&playlist.id)
            .into_iter()
            .flatten()
            .enumerate()
        {
            if let Some(source) = library.catalog.iter().find(|p| &p.id == id) {
                data.union_playlist_sources.push(UnionPlaylistSource {
                    id: format!("{}:{index}", playlist.id),
                    union_playlist_id: playlist.id.clone(),
                    source_type: source.source,
                    source_playlist_id: id.clone(),
                    position: position(index),
                    added_at: DATE.into(),
                });
            }
        }
    }
    wire::AnyPlayerStateEnvelope::new(
        DATE.into(),
        wire::SourceAppInfo {
            platform: "desktop".into(),
            app_version: env!("CARGO_PKG_VERSION").into(),
        },
        data,
    )
}
pub fn library(mut data: wire::AnyPlayerStateData) -> LibrarySnapshot {
    data.playlist_tracks.sort_by_key(|r| r.position);
    data.union_playlist_sources.sort_by_key(|r| r.position);
    let mut state = LibrarySnapshot::default();
    for p in data.custom_playlists {
        state.catalog.push(PlaylistCard {
            id: p.id.clone(),
            name: p.name,
            description: p.description.unwrap_or_default(),
            owner: "You".into(),
            source: SourceType::Custom,
            union: p.playlist_type == PlaylistType::Union,
            distinct: p.is_distinct,
            count: usize::try_from(p.track_count).unwrap_or_default(),
        });
        if p.playlist_type == PlaylistType::Union {
            state.sources.insert(p.id, Vec::new());
        } else {
            state.tracks.insert(p.id, Vec::new());
        }
    }
    for row in data.playlist_tracks {
        let track = Track {
            album: row.album,
            duration_ms: row.duration_ms,
            image_url: row.image_url,
            url: row.url,
            ..Track::new(row.track_id, row.title, row.artist, row.track_source)
        };
        state
            .tracks
            .entry(row.playlist_id)
            .or_default()
            .push(TrackOccurrence { key: row.id, track });
    }
    for row in data.union_playlist_sources {
        state
            .sources
            .entry(row.union_playlist_id)
            .or_default()
            .push(row.source_playlist_id);
    }
    state
}
pub fn config(library: &LibrarySnapshot) -> wire::ConfigFile {
    let state = state(library).data;
    let custom_playlists = state
        .custom_playlists
        .into_iter()
        .map(|p| {
            let tracks = state
                .playlist_tracks
                .iter()
                .filter(|r| r.playlist_id == p.id)
                .enumerate()
                .map(|(i, r)| wire::ConfigTrack {
                    id: position(i),
                    playlist_id: r.playlist_id.clone(),
                    track_source: r.track_source.serial_name().into(),
                    track_id: r.track_id.clone(),
                    position: r.position,
                    added_at: 1_767_225_600,
                    title: r.title.clone(),
                    artist: r.artist.clone(),
                    album: r.album.clone(),
                    duration_ms: r.duration_ms,
                    image_url: r.image_url.clone(),
                })
                .collect();
            let union_sources = state
                .union_playlist_sources
                .iter()
                .filter(|r| r.union_playlist_id == p.id)
                .enumerate()
                .map(|(i, r)| wire::ConfigUnionSource {
                    id: position(i),
                    union_playlist_id: r.union_playlist_id.clone(),
                    source_type: r.source_type.serial_name().into(),
                    source_playlist_id: r.source_playlist_id.clone(),
                    position: r.position,
                    added_at: 1_767_225_600,
                })
                .collect();
            wire::ConfigCustomPlaylist {
                playlist: wire::ConfigPlaylistEntry {
                    id: p.id,
                    name: p.name,
                    description: p.description,
                    image_url: p.image_url,
                    created_at: 1_767_225_600,
                    updated_at: 1_767_225_600,
                    track_count: p.track_count,
                    playlist_type: if p.playlist_type == PlaylistType::Union {
                        "union"
                    } else {
                        "standard"
                    }
                    .into(),
                    is_distinct: p.is_distinct,
                },
                tracks,
                union_sources,
            }
        })
        .collect();
    wire::ConfigFile {
        export_version: wire::CONFIG_EXPORT_VERSION,
        provider_configs: wire::ConfigProviderConfigs::default(),
        custom_playlists,
    }
}
pub fn from_config(file: wire::ConfigFile) -> Result<LibrarySnapshot> {
    if file.export_version != wire::CONFIG_EXPORT_VERSION {
        return Err(ServiceError("Unsupported config version".into()));
    }
    let mut data = wire::AnyPlayerStateData::default();
    for item in file.custom_playlists {
        let p = item.playlist;
        let playlist_type = wire::parse_config_playlist_type(&p.playlist_type)
            .ok_or_else(|| ServiceError("Invalid playlist type".into()))?;
        data.custom_playlists.push(CustomPlaylist {
            id: p.id.clone(),
            name: p.name,
            description: p.description,
            image_url: p.image_url,
            created_at: DATE.into(),
            updated_at: DATE.into(),
            track_count: p.track_count,
            playlist_type,
            is_distinct: p.is_distinct,
        });
        for row in item.tracks {
            data.playlist_tracks.push(PlaylistTrack {
                id: format!("{}:{}", p.id, row.id),
                playlist_id: p.id.clone(),
                track_source: wire::parse_config_source_type(&row.track_source)
                    .ok_or_else(|| ServiceError("Invalid track source".into()))?,
                track_id: row.track_id,
                position: row.position,
                added_at: DATE.into(),
                title: row.title,
                artist: row.artist,
                album: row.album,
                duration_ms: row.duration_ms,
                image_url: row.image_url,
                url: None,
            });
        }
        for row in item.union_sources {
            data.union_playlist_sources.push(UnionPlaylistSource {
                id: format!("{}:{}", p.id, row.id),
                union_playlist_id: p.id.clone(),
                source_type: wire::parse_config_source_type(&row.source_type)
                    .ok_or_else(|| ServiceError("Invalid union source".into()))?,
                source_playlist_id: row.source_playlist_id,
                position: row.position,
                added_at: DATE.into(),
            });
        }
    }
    Ok(library(data))
}
