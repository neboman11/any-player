//! Remote DJ passage state per account (`dj_passage_state`). The JSON shape is owned by
//! `ap-dj`; this module stores it opaquely.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::Result;

/// The stored state of `account_key`. A row that no longer decodes reads as absent.
pub fn load<T: DeserializeOwned>(conn: &Connection, account_key: &str) -> Result<Option<T>> {
    let raw: Option<String> = conn
        .prepare_cached("SELECT state_json FROM dj_passage_state WHERE account_key = ?1")?
        .query_row([account_key], |r| r.get(0))
        .optional()?;
    Ok(raw.and_then(|raw| match serde_json::from_str(&raw) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(account_key, error = %e, "discarding unreadable DJ passage state");
            None
        }
    }))
}

pub fn save<T: Serialize>(conn: &Connection, account_key: &str, state: &T) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO dj_passage_state (account_key, state_json) VALUES (?1, ?2)
         ON CONFLICT(account_key) DO UPDATE SET state_json = excluded.state_json",
    )?
    .execute(params![account_key, serde_json::to_string(state)?])?;
    Ok(())
}

pub fn delete(conn: &Connection, account_key: &str) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM dj_passage_state WHERE account_key = ?1",
        [account_key],
    )? > 0)
}
