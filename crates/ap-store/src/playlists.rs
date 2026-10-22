//! Custom playlists (STANDARD and UNION), provider playlist preferences and the column layout,
//! on the legacy tables (design §6.2).
//!
//! Timestamps are epoch **seconds**; source columns hold lowercase serial names. Every
//! mutation is one transaction ([`in_tx`]). STANDARD appends use
//! `COALESCE(MAX(position) + 1, 0)` and keep `track_count`/`updated_at` in the same
//! transaction; removes and moves shift positions densely and clamp targets to `0..len-1`.

use std::collections::BTreeMap;

use ap_model::{PlaylistType, SourceType, Track};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::db::in_tx;
use crate::error::{Result, StoreError};
use crate::sanitize::strip_opt;
use crate::util::{now_secs, to_i64};

/// A `custom_playlists` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistRecord {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Stored counter. Exact for STANDARD; UNION counts are computed by the library layer.
    pub track_count: i64,
    pub playlist_type: PlaylistType,
    pub is_distinct: bool,
}

/// Input of [`create_playlist`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPlaylist {
    /// `None` generates a UUIDv4 (legacy behaviour).
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub playlist_type: PlaylistType,
    pub is_distinct: bool,
}

impl NewPlaylist {
    pub fn new(name: impl Into<String>, playlist_type: PlaylistType) -> Self {
        Self {
            id: None,
            name: name.into(),
            description: None,
            image_url: None,
            playlist_type,
            is_distinct: false,
        }
    }
}

/// Partial update for [`update_playlist`]. `Some(None)` clears a nullable field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaylistPatch {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub image_url: Option<Option<String>>,
}

impl PlaylistPatch {
    fn is_empty(&self) -> bool {
        self.name.is_none() && self.description.is_none() && self.image_url.is_none()
    }
}

/// A `playlist_tracks` row (cached metadata of a STANDARD playlist entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistTrackRecord {
    /// `INTEGER AUTOINCREMENT` row id; duplicates of one track have distinct row ids.
    pub id: i64,
    pub playlist_id: String,
    pub track_source: SourceType,
    pub track_id: String,
    pub position: i64,
    pub added_at: i64,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: Option<i64>,
    pub image_url: Option<String>,
}

impl PlaylistTrackRecord {
    /// The cached metadata as a [`Track`] (no stream URL; providers re-resolve it).
    pub fn to_track(&self) -> Track {
        Track {
            album: self.album.clone(),
            duration_ms: self.duration_ms,
            image_url: self.image_url.clone(),
            ..Track::new(
                self.track_id.clone(),
                self.title.clone(),
                self.artist.clone(),
                self.track_source,
            )
        }
    }
}

/// Input of [`replace_tracks`] (imports keep their own `added_at`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPlaylistTrack {
    pub track: Track,
    pub added_at: i64,
}

/// A `union_playlist_sources` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnionSourceRecord {
    pub id: i64,
    pub union_playlist_id: String,
    pub source_type: SourceType,
    pub source_playlist_id: String,
    pub position: i64,
    pub added_at: i64,
}

/// Input of [`replace_union_sources`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUnionSource {
    pub source_type: SourceType,
    pub source_playlist_id: String,
    pub added_at: i64,
}

/// A `provider_playlist_preferences` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPlaylistPreference {
    pub source: SourceType,
    pub playlist_id: String,
    pub is_distinct: bool,
}

/// The legacy `column_preferences` singleton (track table layout).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnLayout {
    pub columns: Vec<String>,
    pub column_order: Vec<i64>,
    pub column_widths: BTreeMap<String, i64>,
}

impl Default for ColumnLayout {
    /// Legacy default: `[title, artist, album, duration, source]`, identity order and widths
    /// `{title:300, artist:200, album:200, duration:100, source:100}`.
    fn default() -> Self {
        let columns = ["title", "artist", "album", "duration", "source"];
        let widths = [300, 200, 200, 100, 100];
        Self {
            columns: columns.iter().map(|c| (*c).to_owned()).collect(),
            column_order: (0..5).collect(),
            column_widths: columns
                .iter()
                .zip(widths)
                .map(|(c, w)| ((*c).to_owned(), w))
                .collect(),
        }
    }
}

// ---- custom_playlists ----

const PLAYLIST_COLS: &str = "id, name, description, image_url, created_at, updated_at, track_count, playlist_type, is_distinct";

fn playlist_from_row(r: &Row<'_>) -> rusqlite::Result<PlaylistRecord> {
    let kind: Option<String> = r.get(7)?;
    Ok(PlaylistRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        image_url: r.get(3)?,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
        track_count: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
        playlist_type: kind
            .as_deref()
            .and_then(PlaylistType::parse_lenient)
            .unwrap_or(PlaylistType::Standard),
        is_distinct: r.get::<_, Option<i64>>(8)?.unwrap_or(0) != 0,
    })
}

/// All custom playlists, most recently updated first (legacy order).
pub fn list_playlists(conn: &Connection) -> Result<Vec<PlaylistRecord>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {PLAYLIST_COLS} FROM custom_playlists ORDER BY updated_at DESC, id"
    ))?;
    let rows = stmt
        .query_map([], playlist_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_playlist(conn: &Connection, id: &str) -> Result<Option<PlaylistRecord>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {PLAYLIST_COLS} FROM custom_playlists WHERE id = ?1"
    ))?;
    Ok(stmt.query_row([id], playlist_from_row).optional()?)
}

fn require_playlist(conn: &Connection, id: &str, kind: PlaylistType) -> Result<PlaylistRecord> {
    let p = get_playlist(conn, id)?
        .ok_or_else(|| StoreError::NotFound(format!("custom playlist {id}")))?;
    if p.playlist_type == kind {
        Ok(p)
    } else {
        Err(StoreError::Invalid(format!(
            "playlist {id} is not a {} playlist",
            kind.serial_name()
        )))
    }
}

fn touch(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE custom_playlists SET updated_at = ?2 WHERE id = ?1",
        params![id, now_secs()],
    )?;
    Ok(())
}

/// Creates a playlist with `created_at = updated_at = now` and no tracks.
pub fn create_playlist(conn: &Connection, new: &NewPlaylist) -> Result<PlaylistRecord> {
    let now = now_secs();
    let record = PlaylistRecord {
        id: new
            .id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        name: new.name.clone(),
        description: new.description.clone(),
        image_url: strip_opt(new.image_url.as_deref()),
        created_at: now,
        updated_at: now,
        track_count: 0,
        playlist_type: new.playlist_type,
        is_distinct: new.is_distinct,
    };
    conn.execute(
        &format!("INSERT INTO custom_playlists ({PLAYLIST_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"),
        params![
            record.id,
            record.name,
            record.description,
            record.image_url,
            record.created_at,
            record.updated_at,
            record.track_count,
            record.playlist_type.serial_name(),
            record.is_distinct,
        ],
    )?;
    Ok(record)
}

/// Inserts or overwrites a playlist row exactly as given (imports and sync). Tracks and union
/// sources are untouched; the row id survives (`ON CONFLICT DO UPDATE`).
pub fn upsert_playlist(conn: &Connection, p: &PlaylistRecord) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT INTO custom_playlists ({PLAYLIST_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, description = excluded.description,
               image_url = excluded.image_url, created_at = excluded.created_at, updated_at = excluded.updated_at,
               track_count = excluded.track_count, playlist_type = excluded.playlist_type,
               is_distinct = excluded.is_distinct"
        ),
        params![
            p.id,
            p.name,
            p.description,
            strip_opt(p.image_url.as_deref()),
            p.created_at,
            p.updated_at,
            p.track_count,
            p.playlist_type.serial_name(),
            p.is_distinct,
        ],
    )?;
    Ok(())
}

/// Applies `patch` and bumps `updated_at`. An empty patch changes nothing. `None` when the
/// playlist does not exist.
pub fn update_playlist(
    conn: &Connection,
    id: &str,
    patch: &PlaylistPatch,
) -> Result<Option<PlaylistRecord>> {
    in_tx(conn, |conn| {
        let Some(mut p) = get_playlist(conn, id)? else {
            return Ok(None);
        };
        if patch.is_empty() {
            return Ok(Some(p));
        }
        if let Some(name) = &patch.name {
            p.name.clone_from(name);
        }
        if let Some(description) = &patch.description {
            p.description.clone_from(description);
        }
        if let Some(image_url) = &patch.image_url {
            p.image_url = strip_opt(image_url.as_deref());
        }
        p.updated_at = now_secs();
        conn.execute(
            "UPDATE custom_playlists SET name = ?2, description = ?3, image_url = ?4, updated_at = ?5 WHERE id = ?1",
            params![p.id, p.name, p.description, p.image_url, p.updated_at],
        )?;
        Ok(Some(p))
    })
}

/// Deletes a playlist. Its tracks and union sources go with it (foreign-key cascade), as does
/// its union catalog cache (`u:<id>`). Returns whether a row was deleted.
pub fn delete_playlist(conn: &Connection, id: &str) -> Result<bool> {
    in_tx(conn, |conn| {
        let deleted = conn.execute("DELETE FROM custom_playlists WHERE id = ?1", [id])? > 0;
        conn.execute(
            "DELETE FROM cached_track_owners WHERE owner_key = ?1",
            [crate::catalog::union_owner_key(id)],
        )?;
        Ok(deleted)
    })
}

/// Sets the distinct (dedupe on play) flag and bumps `updated_at`.
pub fn set_playlist_distinct(conn: &Connection, id: &str, is_distinct: bool) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE custom_playlists SET is_distinct = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, is_distinct, now_secs()],
    )? > 0)
}

// ---- ordered child tables ----

/// `playlist_tracks` / `union_playlist_sources`: rows ordered densely by `position` under a parent.
struct Ordered {
    table: &'static str,
    parent: &'static str,
}

const TRACKS: Ordered = Ordered {
    table: "playlist_tracks",
    parent: "playlist_id",
};
const SOURCES: Ordered = Ordered {
    table: "union_playlist_sources",
    parent: "union_playlist_id",
};

impl Ordered {
    fn len(&self, conn: &Connection, parent: &str) -> Result<i64> {
        let sql = format!(
            "SELECT COUNT(*) FROM {} WHERE {} = ?1",
            self.table, self.parent
        );
        Ok(conn
            .prepare_cached(&sql)?
            .query_row([parent], |r| r.get(0))?)
    }

    fn next_position(&self, conn: &Connection, parent: &str) -> Result<i64> {
        let sql = format!(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM {} WHERE {} = ?1",
            self.table, self.parent
        );
        Ok(conn
            .prepare_cached(&sql)?
            .query_row([parent], |r| r.get(0))?)
    }

    fn position_of(&self, conn: &Connection, parent: &str, row_id: i64) -> Result<Option<i64>> {
        let sql = format!(
            "SELECT position FROM {} WHERE id = ?1 AND {} = ?2",
            self.table, self.parent
        );
        Ok(conn
            .prepare_cached(&sql)?
            .query_row(params![row_id, parent], |r| r.get(0))
            .optional()?)
    }

    /// Deletes `row_id` and closes the gap. `false` when the row is not under `parent`.
    fn remove(&self, conn: &Connection, parent: &str, row_id: i64) -> Result<bool> {
        let Some(pos) = self.position_of(conn, parent, row_id)? else {
            return Ok(false);
        };
        conn.execute(
            &format!("DELETE FROM {} WHERE id = ?1", self.table),
            [row_id],
        )?;
        conn.execute(
            &format!(
                "UPDATE {} SET position = position - 1 WHERE {} = ?1 AND position > ?2",
                self.table, self.parent
            ),
            params![parent, pos],
        )?;
        Ok(true)
    }

    /// Moves `row_id` to `target` clamped to `0..len-1`, shifting the rows in between.
    /// Returns the final position, or `None` when the row is not under `parent`.
    fn move_to(
        &self,
        conn: &Connection,
        parent: &str,
        row_id: i64,
        target: i64,
    ) -> Result<Option<i64>> {
        let Some(old) = self.position_of(conn, parent, row_id)? else {
            return Ok(None);
        };
        let new = target.clamp(0, self.len(conn, parent)? - 1);
        if new > old {
            conn.execute(
                &format!(
                    "UPDATE {} SET position = position - 1 WHERE {} = ?1 AND position > ?2 AND position <= ?3",
                    self.table, self.parent
                ),
                params![parent, old, new],
            )?;
        } else if new < old {
            conn.execute(
                &format!(
                    "UPDATE {} SET position = position + 1 WHERE {} = ?1 AND position >= ?2 AND position < ?3",
                    self.table, self.parent
                ),
                params![parent, new, old],
            )?;
        }
        if new != old {
            conn.execute(
                &format!("UPDATE {} SET position = ?2 WHERE id = ?1", self.table),
                params![row_id, new],
            )?;
        }
        Ok(Some(new))
    }
}

fn source_from_column(raw: &str, table: &str, row_id: i64) -> Option<SourceType> {
    let parsed = SourceType::parse_lenient(raw);
    if parsed.is_none() {
        tracing::warn!(
            table,
            row_id,
            source = raw,
            "skipping row with an unknown source"
        );
    }
    parsed
}

// ---- STANDARD tracks ----

const TRACK_COLS: &str = "id, playlist_id, track_source, track_id, position, added_at, title, artist, album, duration_ms, image_url";

fn track_from_row(r: &Row<'_>) -> rusqlite::Result<Option<PlaylistTrackRecord>> {
    let id: i64 = r.get(0)?;
    let source: String = r.get(2)?;
    let Some(track_source) = source_from_column(&source, "playlist_tracks", id) else {
        return Ok(None);
    };
    Ok(Some(PlaylistTrackRecord {
        id,
        playlist_id: r.get(1)?,
        track_source,
        track_id: r.get(3)?,
        position: r.get(4)?,
        added_at: r.get(5)?,
        title: r.get(6)?,
        artist: r.get(7)?,
        album: r.get(8)?,
        duration_ms: r.get(9)?,
        image_url: r.get(10)?,
    }))
}

/// Entries of a STANDARD playlist in position order. Rows with an unknown source are skipped.
pub fn list_tracks(conn: &Connection, playlist_id: &str) -> Result<Vec<PlaylistTrackRecord>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {TRACK_COLS} FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position, id"
    ))?;
    let rows = stmt
        .query_map([playlist_id], track_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().flatten().collect())
}

fn insert_track(
    conn: &Connection,
    playlist_id: &str,
    track: &Track,
    position: i64,
    added_at: i64,
) -> Result<i64> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO playlist_tracks (playlist_id, track_source, track_id, position, added_at, title, artist, album,
                                      duration_ms, image_url)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;
    stmt.execute(params![
        playlist_id,
        track.source.serial_name(),
        track.id,
        position,
        added_at,
        track.title,
        track.artist,
        track.album,
        track.duration_ms,
        strip_opt(track.image_url.as_deref()),
    ])?;
    Ok(conn.last_insert_rowid())
}

fn recount(conn: &Connection, playlist_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE custom_playlists SET track_count = (SELECT COUNT(*) FROM playlist_tracks WHERE playlist_id = ?1)
          WHERE id = ?1",
        [playlist_id],
    )?;
    Ok(())
}

/// Appends `tracks` (duplicates allowed) to a STANDARD playlist; updates `track_count` and
/// `updated_at`. Returns the inserted rows.
pub fn add_tracks(
    conn: &Connection,
    playlist_id: &str,
    tracks: &[Track],
) -> Result<Vec<PlaylistTrackRecord>> {
    in_tx(conn, |conn| {
        require_playlist(conn, playlist_id, PlaylistType::Standard)?;
        let start = TRACKS.next_position(conn, playlist_id)?;
        let now = now_secs();
        let mut out = Vec::with_capacity(tracks.len());
        for (i, track) in tracks.iter().enumerate() {
            let position = start + to_i64(i);
            let id = insert_track(conn, playlist_id, track, position, now)?;
            out.push(PlaylistTrackRecord {
                id,
                playlist_id: playlist_id.to_owned(),
                track_source: track.source,
                track_id: track.id.clone(),
                position,
                added_at: now,
                title: track.title.clone(),
                artist: track.artist.clone(),
                album: track.album.clone(),
                duration_ms: track.duration_ms,
                image_url: strip_opt(track.image_url.as_deref()),
            });
        }
        recount(conn, playlist_id)?;
        touch(conn, playlist_id)?;
        Ok(out)
    })
}

/// Replaces every entry of a STANDARD playlist (imports/sync) and recomputes `track_count`.
/// `updated_at` is left alone so an imported timestamp survives.
pub fn replace_tracks(
    conn: &Connection,
    playlist_id: &str,
    tracks: &[NewPlaylistTrack],
) -> Result<()> {
    in_tx(conn, |conn| {
        require_playlist(conn, playlist_id, PlaylistType::Standard)?;
        conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            [playlist_id],
        )?;
        for (i, t) in tracks.iter().enumerate() {
            insert_track(conn, playlist_id, &t.track, to_i64(i), t.added_at)?;
        }
        recount(conn, playlist_id)
    })
}

/// Removes one entry by row id. `false` when it is not in this playlist.
pub fn remove_track(conn: &Connection, playlist_id: &str, row_id: i64) -> Result<bool> {
    in_tx(conn, |conn| {
        if !TRACKS.remove(conn, playlist_id, row_id)? {
            return Ok(false);
        }
        recount(conn, playlist_id)?;
        touch(conn, playlist_id)?;
        Ok(true)
    })
}

/// Moves one entry to `position` (clamped). Returns the final position, `None` when the row is
/// not in this playlist.
pub fn move_track(
    conn: &Connection,
    playlist_id: &str,
    row_id: i64,
    position: i64,
) -> Result<Option<i64>> {
    in_tx(conn, |conn| {
        let moved = TRACKS.move_to(conn, playlist_id, row_id, position)?;
        if moved.is_some() {
            touch(conn, playlist_id)?;
        }
        Ok(moved)
    })
}

// ---- UNION sources ----

const SOURCE_COLS: &str =
    "id, union_playlist_id, source_type, source_playlist_id, position, added_at";

fn union_source_from_row(r: &Row<'_>) -> rusqlite::Result<Option<UnionSourceRecord>> {
    let id: i64 = r.get(0)?;
    let raw: String = r.get(2)?;
    let Some(source_type) = source_from_column(&raw, "union_playlist_sources", id) else {
        return Ok(None);
    };
    Ok(Some(UnionSourceRecord {
        id,
        union_playlist_id: r.get(1)?,
        source_type,
        source_playlist_id: r.get(3)?,
        position: r.get(4)?,
        added_at: r.get(5)?,
    }))
}

/// Sources of a UNION playlist in position order.
pub fn list_union_sources(conn: &Connection, union_id: &str) -> Result<Vec<UnionSourceRecord>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {SOURCE_COLS} FROM union_playlist_sources WHERE union_playlist_id = ?1 ORDER BY position, id"
    ))?;
    let rows = stmt
        .query_map([union_id], union_source_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().flatten().collect())
}

fn insert_source(
    conn: &Connection,
    union_id: &str,
    s: &NewUnionSource,
    position: i64,
) -> Result<i64> {
    if s.source_type == SourceType::Custom && s.source_playlist_id == union_id {
        return Err(StoreError::Invalid(
            "a union playlist cannot contain itself".into(),
        ));
    }
    conn.prepare_cached(
        "INSERT INTO union_playlist_sources (union_playlist_id, source_type, source_playlist_id, position, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?
    .execute(params![union_id, s.source_type.serial_name(), s.source_playlist_id, position, s.added_at])?;
    Ok(conn.last_insert_rowid())
}

/// Appends a source playlist to a UNION playlist and bumps `updated_at`.
pub fn add_union_source(
    conn: &Connection,
    union_id: &str,
    source_type: SourceType,
    source_playlist_id: &str,
) -> Result<UnionSourceRecord> {
    in_tx(conn, |conn| {
        require_playlist(conn, union_id, PlaylistType::Union)?;
        let position = SOURCES.next_position(conn, union_id)?;
        let new = NewUnionSource {
            source_type,
            source_playlist_id: source_playlist_id.to_owned(),
            added_at: now_secs(),
        };
        let id = insert_source(conn, union_id, &new, position)?;
        touch(conn, union_id)?;
        Ok(UnionSourceRecord {
            id,
            union_playlist_id: union_id.to_owned(),
            source_type,
            source_playlist_id: new.source_playlist_id,
            position,
            added_at: new.added_at,
        })
    })
}

/// Replaces every source of a UNION playlist (imports/sync); `updated_at` is left alone.
pub fn replace_union_sources(
    conn: &Connection,
    union_id: &str,
    sources: &[NewUnionSource],
) -> Result<()> {
    in_tx(conn, |conn| {
        require_playlist(conn, union_id, PlaylistType::Union)?;
        conn.execute(
            "DELETE FROM union_playlist_sources WHERE union_playlist_id = ?1",
            [union_id],
        )?;
        for (i, s) in sources.iter().enumerate() {
            insert_source(conn, union_id, s, to_i64(i))?;
        }
        Ok(())
    })
}

/// Removes one source by row id. `false` when it is not in this union.
pub fn remove_union_source(conn: &Connection, union_id: &str, row_id: i64) -> Result<bool> {
    in_tx(conn, |conn| {
        let removed = SOURCES.remove(conn, union_id, row_id)?;
        if removed {
            touch(conn, union_id)?;
        }
        Ok(removed)
    })
}

/// Moves one source to `position` (clamped); returns the final position.
pub fn move_union_source(
    conn: &Connection,
    union_id: &str,
    row_id: i64,
    position: i64,
) -> Result<Option<i64>> {
    in_tx(conn, |conn| {
        let moved = SOURCES.move_to(conn, union_id, row_id, position)?;
        if moved.is_some() {
            touch(conn, union_id)?;
        }
        Ok(moved)
    })
}

// ---- provider playlist preferences ----

/// Whether a provider playlist plays deduplicated. Defaults to `false`.
pub fn provider_playlist_distinct(
    conn: &Connection,
    source: SourceType,
    playlist_id: &str,
) -> Result<bool> {
    Ok(conn
        .prepare_cached("SELECT is_distinct FROM provider_playlist_preferences WHERE source = ?1 AND playlist_id = ?2")?
        .query_row(params![source.serial_name(), playlist_id], |r| r.get::<_, Option<i64>>(0))
        .optional()?
        .flatten()
        .is_some_and(|v| v != 0))
}

/// Upserts the distinct flag of a provider playlist.
pub fn set_provider_playlist_distinct(
    conn: &Connection,
    source: SourceType,
    playlist_id: &str,
    is_distinct: bool,
) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO provider_playlist_preferences (source, playlist_id, is_distinct) VALUES (?1, ?2, ?3)
         ON CONFLICT(source, playlist_id) DO UPDATE SET is_distinct = excluded.is_distinct",
    )?
    .execute(params![source.serial_name(), playlist_id, is_distinct])?;
    Ok(())
}

/// Every stored provider playlist preference (rows with unknown sources are skipped).
pub fn list_provider_playlist_preferences(
    conn: &Connection,
) -> Result<Vec<ProviderPlaylistPreference>> {
    let mut stmt = conn.prepare_cached(
        "SELECT source, playlist_id, is_distinct FROM provider_playlist_preferences ORDER BY source, playlist_id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(raw, playlist_id, distinct)| {
            SourceType::parse_lenient(&raw).map(|source| ProviderPlaylistPreference {
                source,
                playlist_id,
                is_distinct: distinct.unwrap_or(0) != 0,
            })
        })
        .collect())
}

// ---- column_preferences ----

fn parse_widths(raw: Option<&str>) -> BTreeMap<String, i64> {
    let Some(raw) = raw else {
        return BTreeMap::new();
    };
    match serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(raw) {
        // Widths may have been written as floats by the webview; round them.
        #[expect(clippy::cast_possible_truncation, reason = "pixel widths are small")]
        Ok(map) => map
            .into_iter()
            .filter_map(|(k, v)| v.as_f64().map(|w| (k, w.round() as i64)))
            .collect(),
        Err(_) => BTreeMap::new(),
    }
}

impl ColumnLayout {
    /// Canonicalize the old webview's duration key; an explicit canonical width wins.
    fn normalize(&mut self) {
        for column in &mut self.columns {
            if column == "duration_ms" {
                *column = "duration".into();
            }
        }
        if let Some(width) = self.column_widths.remove("duration_ms") {
            self.column_widths.entry("duration".into()).or_insert(width);
        }
    }
}

/// The stored column layout, or the legacy default when unset or unreadable.
pub fn column_layout(conn: &Connection) -> Result<ColumnLayout> {
    let row = conn
        .query_row(
            "SELECT columns, column_order, column_widths FROM column_preferences WHERE id = 1",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((columns, order, widths)) = row else {
        return Ok(ColumnLayout::default());
    };
    if let (Ok(columns), Ok(column_order)) =
        (serde_json::from_str(&columns), serde_json::from_str(&order))
    {
        let mut layout = ColumnLayout {
            columns,
            column_order,
            column_widths: parse_widths(widths.as_deref()),
        };
        layout.normalize();
        Ok(layout)
    } else {
        tracing::warn!("unreadable column_preferences row; using the default layout");
        Ok(ColumnLayout::default())
    }
}

/// Stores the column layout (upsert of the singleton row).
pub fn set_column_layout(conn: &Connection, layout: &ColumnLayout) -> Result<()> {
    let mut layout = layout.clone();
    layout.normalize();
    conn.execute(
        "INSERT INTO column_preferences (id, columns, column_order, column_widths) VALUES (1, ?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET columns = excluded.columns, column_order = excluded.column_order,
           column_widths = excluded.column_widths",
        params![
            serde_json::to_string(&layout.columns)?,
            serde_json::to_string(&layout.column_order)?,
            serde_json::to_string(&layout.column_widths)?,
        ],
    )?;
    Ok(())
}
