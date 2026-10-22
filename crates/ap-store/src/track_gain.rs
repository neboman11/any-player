//! Per-track loudness gain cache (`track_gain`, design §8.4), keyed by `${SOURCE}:${trackId}`.

use ap_model::SourceType;
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;

/// Cache key of a track: the Kotlin constant name of the source, `:`, the track id.
pub fn track_gain_key(source: SourceType, track_id: &str) -> String {
    format!("{}:{track_id}", source.name())
}

/// A cached analysis result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackGain {
    pub gain: f32,
    pub analyzed_at_ms: i64,
}

pub fn get(conn: &Connection, track_key: &str) -> Result<Option<TrackGain>> {
    Ok(conn
        .prepare_cached("SELECT gain, analyzed_at_ms FROM track_gain WHERE track_key = ?1")?
        .query_row([track_key], |r| {
            #[expect(clippy::cast_possible_truncation, reason = "stored from an f32")]
            let gain = r.get::<_, f64>(0)? as f32;
            Ok(TrackGain {
                gain,
                analyzed_at_ms: r.get(1)?,
            })
        })
        .optional()?)
}

pub fn put(conn: &Connection, track_key: &str, gain: TrackGain) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO track_gain (track_key, gain, analyzed_at_ms) VALUES (?1, ?2, ?3)
         ON CONFLICT(track_key) DO UPDATE SET gain = excluded.gain, analyzed_at_ms = excluded.analyzed_at_ms",
    )?
    .execute(params![track_key, f64::from(gain.gain), gain.analyzed_at_ms])?;
    Ok(())
}

pub fn delete(conn: &Connection, track_key: &str) -> Result<bool> {
    Ok(conn.execute("DELETE FROM track_gain WHERE track_key = ?1", [track_key])? > 0)
}
