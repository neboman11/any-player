//! Catalog cache tables (`cached_playlists`, `cached_track_owners`, `cached_tracks`), a port of
//! Android `CatalogCacheDao`.
//!
//! Track lists are keyed by an owner: [`provider_owner_key`] (`p:<SOURCE>:<id>`) or
//! [`union_owner_key`] (`u:<id>`). `cached_playlists.source` holds the lowercase serial name.
//! Plex tokens are stripped from every stored URL.

use ap_model::{Playlist, SourceType, Track};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::db::in_tx;
use crate::error::Result;
use crate::sanitize::{persisted_track_json, strip_opt};
use crate::util::to_i64;

/// Owner key of a provider playlist's cached tracks (Android `providerOwnerKey`).
pub fn provider_owner_key(source: SourceType, playlist_id: &str) -> String {
    format!("p:{}:{playlist_id}", source.name())
}

/// Owner key of a union playlist's materialized tracks.
pub fn union_owner_key(union_id: &str) -> String {
    format!("u:{union_id}")
}

/// A cached provider playlist summary (`tracks` is always `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedPlaylist {
    pub playlist: Playlist,
    /// Position in the provider's listing.
    pub sort_index: i64,
    pub fetched_at_ms: i64,
}

/// A cached track list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedTracks {
    pub tracks: Vec<Track>,
    pub updated_at_ms: i64,
    /// Stored count (equals `tracks.len()` unless rows failed to decode).
    pub track_count: i64,
}

const PLAYLIST_COLS: &str =
    "source, id, name, owner, description, image_url, track_count, sort_index, fetched_at_ms";

fn playlist_from_row(r: &Row<'_>) -> rusqlite::Result<Option<CachedPlaylist>> {
    let raw: String = r.get(0)?;
    let Some(source) = SourceType::parse_lenient(&raw) else {
        return Ok(None);
    };
    Ok(Some(CachedPlaylist {
        playlist: Playlist {
            id: r.get(1)?,
            name: r.get(2)?,
            owner: r.get(3)?,
            description: r.get(4)?,
            image_url: r.get(5)?,
            track_count: i32::try_from(r.get::<_, i64>(6)?).unwrap_or(i32::MAX),
            source,
            tracks: None,
        },
        sort_index: r.get(7)?,
        fetched_at_ms: r.get(8)?,
    }))
}

/// Cached playlists of one source, or of every source (Jellyfin, then Plex, then the rest),
/// in listing order.
pub fn playlists(conn: &Connection, source: Option<SourceType>) -> Result<Vec<CachedPlaylist>> {
    let order = "ORDER BY CASE source WHEN 'jellyfin' THEN 0 WHEN 'plex' THEN 1 ELSE 2 END, source, sort_index";
    let rows = if let Some(source) = source {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {PLAYLIST_COLS} FROM cached_playlists WHERE source = ?1 {order}"
        ))?;
        stmt.query_map([source.serial_name()], playlist_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {PLAYLIST_COLS} FROM cached_playlists {order}"
        ))?;
        stmt.query_map([], playlist_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok(rows.into_iter().flatten().collect())
}

/// Replaces the cached listing of `source` with `listing` (in order) in one transaction.
/// Playlists that vanished lose their row **and** their cached tracks; every `p:<SOURCE>:*`
/// owner not in the new listing is removed. Returns the vanished owner keys.
pub fn replace_playlists(
    conn: &Connection,
    source: SourceType,
    listing: &[Playlist],
    fetched_at_ms: i64,
) -> Result<Vec<String>> {
    in_tx(conn, |conn| {
        let prefix = provider_owner_key(source, "");
        let retained: std::collections::HashSet<String> = listing
            .iter()
            .map(|p| provider_owner_key(source, &p.id))
            .collect();
        let existing: Vec<String> = conn
            .prepare_cached("SELECT owner_key FROM cached_track_owners WHERE substr(owner_key, 1, length(?1)) = ?1")?
            .query_map([&prefix], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let vanished: Vec<String> = existing
            .into_iter()
            .filter(|k| !retained.contains(k))
            .collect();
        for key in &vanished {
            // Cascades to cached_tracks.
            conn.execute(
                "DELETE FROM cached_track_owners WHERE owner_key = ?1",
                [key],
            )?;
        }
        conn.execute(
            "DELETE FROM cached_playlists WHERE source = ?1",
            [source.serial_name()],
        )?;
        let mut insert = conn.prepare_cached(&format!(
            "INSERT INTO cached_playlists ({PLAYLIST_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(source, id) DO UPDATE SET name = excluded.name, owner = excluded.owner,
               description = excluded.description, image_url = excluded.image_url,
               track_count = excluded.track_count, sort_index = excluded.sort_index,
               fetched_at_ms = excluded.fetched_at_ms"
        ))?;
        for (i, p) in listing.iter().enumerate() {
            insert.execute(params![
                source.serial_name(),
                p.id,
                p.name,
                p.owner,
                p.description,
                strip_opt(p.image_url.as_deref()),
                p.track_count,
                to_i64(i),
                fetched_at_ms,
            ])?;
        }
        Ok(vanished)
    })
}

/// Drops every cached playlist and track list of `source` (provider disconnected).
pub fn clear_provider(conn: &Connection, source: SourceType) -> Result<()> {
    replace_playlists(conn, source, &[], 0).map(drop)
}

/// The cached tracks of `owner_key`, `None` when never cached. Undecodable rows are skipped.
pub fn tracks(conn: &Connection, owner_key: &str) -> Result<Option<CachedTracks>> {
    let owner = conn
        .prepare_cached(
            "SELECT updated_at_ms, track_count FROM cached_track_owners WHERE owner_key = ?1",
        )?
        .query_row([owner_key], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })
        .optional()?;
    let Some((updated_at_ms, track_count)) = owner else {
        return Ok(None);
    };
    let mut stmt = conn.prepare_cached(
        "SELECT position, track_json FROM cached_tracks WHERE owner_key = ?1 ORDER BY position",
    )?;
    let rows = stmt
        .query_map([owner_key], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let tracks = rows
        .into_iter()
        .filter_map(|(position, json)| match serde_json::from_str::<Track>(&json) {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!(owner_key, position, error = %e, "skipping undecodable cached track");
                None
            }
        })
        .collect();
    Ok(Some(CachedTracks {
        tracks,
        updated_at_ms,
        track_count,
    }))
}

/// Replaces the track list of `owner_key` (owner upserted, old rows deleted) in one transaction.
pub fn replace_tracks(
    conn: &Connection,
    owner_key: &str,
    tracks: &[Track],
    updated_at_ms: i64,
) -> Result<()> {
    in_tx(conn, |conn| {
        conn.prepare_cached(
            "INSERT INTO cached_track_owners (owner_key, updated_at_ms, track_count) VALUES (?1, ?2, ?3)
             ON CONFLICT(owner_key) DO UPDATE SET updated_at_ms = excluded.updated_at_ms,
               track_count = excluded.track_count",
        )?
        .execute(params![owner_key, updated_at_ms, to_i64(tracks.len())])?;
        conn.execute(
            "DELETE FROM cached_tracks WHERE owner_key = ?1",
            [owner_key],
        )?;
        let mut insert = conn.prepare_cached(
            "INSERT INTO cached_tracks (owner_key, position, track_json) VALUES (?1, ?2, ?3)",
        )?;
        for (i, t) in tracks.iter().enumerate() {
            insert.execute(params![owner_key, to_i64(i), persisted_track_json(t)?])?;
        }
        Ok(())
    })
}

/// [`replace_tracks`] for a provider playlist; a non-empty fetch also updates the cached
/// playlist's `track_count` (Android `replaceProviderTracks`).
pub fn replace_provider_tracks(
    conn: &Connection,
    source: SourceType,
    playlist_id: &str,
    tracks: &[Track],
    updated_at_ms: i64,
) -> Result<()> {
    in_tx(conn, |conn| {
        replace_tracks(
            conn,
            &provider_owner_key(source, playlist_id),
            tracks,
            updated_at_ms,
        )?;
        if !tracks.is_empty() {
            conn.execute(
                "UPDATE cached_playlists SET track_count = ?3 WHERE source = ?1 AND id = ?2",
                params![source.serial_name(), playlist_id, to_i64(tracks.len())],
            )?;
        }
        Ok(())
    })
}

/// Marks a cached list fresh without rewriting it.
pub fn touch_owner(conn: &Connection, owner_key: &str, updated_at_ms: i64) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE cached_track_owners SET updated_at_ms = ?2 WHERE owner_key = ?1",
        params![owner_key, updated_at_ms],
    )? > 0)
}

/// Deletes a cached list (its tracks cascade).
pub fn remove_owner(conn: &Connection, owner_key: &str) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM cached_track_owners WHERE owner_key = ?1",
        [owner_key],
    )? > 0)
}
