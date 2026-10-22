//! Queue persistence (`queue_entries`, `queue_meta`), a port of Android `QueueDao` /
//! `RoomPlaybackPersistence`.
//!
//! - [`write_content`] replaces entries and meta in one transaction; the engine calls it
//!   (debounced) after a `content_version` change.
//! - [`write_cursor`] moves only the cursor and is qualified by `content_version`: it is a no-op
//!   unless the stored content is the version the cursor belongs to, so a late cursor write
//!   can never point into a different queue.

use std::collections::HashMap;
use std::sync::Arc;

use ap_model::{QueueEntry, QueueState, RepeatMode, Track};
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::in_tx;
use crate::error::{Result, StoreError};
use crate::sanitize::persisted_track_json;
use crate::settings::{self, keys};
use crate::util::to_i64;

fn uid_to_sql(uid: u64) -> Result<i64> {
    i64::try_from(uid).map_err(|_| StoreError::Invalid(format!("queue uid {uid} exceeds i64")))
}

fn version_to_sql(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(|_| StoreError::Invalid(format!("content version {v} exceeds i64")))
}

/// Whether a persisted queue exists.
pub fn has_queue(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM queue_meta WHERE id = 0)",
        [],
        |r| r.get(0),
    )?)
}

/// Replaces the persisted queue with `queue` (entries in insertion order, play order as uids,
/// cursor uid and `content_version`).
pub fn write_content(conn: &Connection, queue: &QueueState) -> Result<()> {
    let order_uids = queue
        .order
        .iter()
        .map(|&i| queue.entries[i].uid.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let cursor_uid = queue.current().map(|e| uid_to_sql(e.uid)).transpose()?;
    let version = version_to_sql(queue.content_version)?;
    in_tx(conn, |conn| {
        conn.execute("DELETE FROM queue_entries", [])?;
        let mut insert = conn.prepare_cached(
            "INSERT INTO queue_entries (uid, seq, track_json) VALUES (?1, ?2, ?3)",
        )?;
        for (seq, entry) in queue.entries.iter().enumerate() {
            insert.execute(params![
                uid_to_sql(entry.uid)?,
                to_i64(seq),
                persisted_track_json(&entry.track)?
            ])?;
        }
        conn.execute(
            "INSERT INTO queue_meta (id, order_uids, cursor_uid, content_version) VALUES (0, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET order_uids = excluded.order_uids, cursor_uid = excluded.cursor_uid,
               content_version = excluded.content_version",
            params![order_uids, cursor_uid, version],
        )?;
        Ok(())
    })
}

/// Persists `cursor_uid` when the stored content has the same `content_version` and the uid
/// exists. `None` clears the cursor. Returns whether a row changed.
pub fn write_cursor(
    conn: &Connection,
    cursor_uid: Option<u64>,
    content_version: u64,
) -> Result<bool> {
    let uid = cursor_uid.map(uid_to_sql).transpose()?;
    let version = version_to_sql(content_version)?;
    Ok(conn
        .prepare_cached(
            "UPDATE queue_meta SET cursor_uid = ?1
          WHERE id = 0 AND content_version = ?2
            AND (?1 IS NULL OR EXISTS(SELECT 1 FROM queue_entries WHERE uid = ?1))",
        )?
        .execute(params![uid, version])?
        > 0)
}

/// Deletes the persisted queue.
pub fn clear(conn: &Connection) -> Result<()> {
    in_tx(conn, |conn| {
        conn.execute("DELETE FROM queue_entries", [])?;
        conn.execute("DELETE FROM queue_meta", [])?;
        Ok(())
    })
}

/// Restores the persisted queue; the empty default when none is stored. `shuffle` and
/// `repeat` come from the `player.*` settings. A stored play order that does not cover every
/// entry exactly once is reported as [`StoreError::Corrupt`] (Android parity).
pub fn load(conn: &Connection) -> Result<QueueState> {
    let shuffle = settings::get::<bool>(conn, keys::PLAYER_SHUFFLE)?.unwrap_or(false);
    let repeat = settings::get::<RepeatMode>(conn, keys::PLAYER_REPEAT)?.unwrap_or_default();
    let meta = conn
        .query_row(
            "SELECT order_uids, cursor_uid, content_version FROM queue_meta WHERE id = 0",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((order_uids, cursor_uid, content_version)) = meta else {
        return Ok(QueueState {
            shuffle,
            repeat,
            ..QueueState::default()
        });
    };

    let mut stmt = conn.prepare_cached("SELECT uid, track_json FROM queue_entries ORDER BY seq")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut entries = Vec::with_capacity(rows.len());
    for (uid, json) in rows {
        let uid = u64::try_from(uid)
            .map_err(|_| StoreError::Corrupt(format!("negative queue uid {uid}")))?;
        let track: Track = serde_json::from_str(&json)?;
        entries.push(QueueEntry { uid, track });
    }

    let index_of: HashMap<u64, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.uid, i))
        .collect();
    let mut seen = vec![false; entries.len()];
    let mut order = Vec::with_capacity(entries.len());
    for part in order_uids.split(',').filter(|p| !p.is_empty()) {
        let Some(&i) = part.parse::<u64>().ok().and_then(|uid| index_of.get(&uid)) else {
            continue;
        };
        if !std::mem::replace(&mut seen[i], true) {
            order.push(i);
        }
    }
    if order.len() != entries.len() {
        return Err(StoreError::Corrupt(
            "persisted queue order is incomplete".into(),
        ));
    }
    let cursor = cursor_uid
        .and_then(|uid| u64::try_from(uid).ok())
        .and_then(|uid| order.iter().position(|&i| entries[i].uid == uid))
        .map_or(-1, |p| isize::try_from(p).expect("queue length fits isize"));
    let next_uid = entries.iter().map(|e| e.uid).max().unwrap_or(0) + 1;
    Ok(QueueState {
        entries: Arc::from(entries),
        order: Arc::from(order),
        cursor,
        shuffle,
        repeat,
        play_next_offset: 0,
        content_version: u64::try_from(content_version).unwrap_or(0),
        next_uid,
    })
}
