//! Sync merge bases (`sync_base`) and last-seen server versions (`sync_version`), a port of
//! Android `SyncBaseStore`. One agreed target per domain: a base recorded for another target
//! reads as absent.

use rusqlite::{Connection, OptionalExtension, params};

use crate::db::in_tx;
use crate::error::Result;

/// A sync namespace with a merge base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SyncDomain {
    AppState,
    Playlists,
    ProviderConfiguration,
    Settings,
}

impl SyncDomain {
    pub const ALL: [Self; 4] = [
        Self::AppState,
        Self::Playlists,
        Self::ProviderConfiguration,
        Self::Settings,
    ];

    /// The `sync_base.domain` value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AppState => "app_state",
            Self::Playlists => "playlists",
            Self::ProviderConfiguration => "provider_configuration",
            Self::Settings => "settings",
        }
    }
}

/// Signatures agreed with a target: what the server held and what this device had locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSignatures {
    pub remote: String,
    pub local: String,
}

/// The base of `domain` if it was recorded for `target`.
pub fn base(conn: &Connection, domain: SyncDomain, target: &str) -> Result<Option<SyncSignatures>> {
    Ok(conn
        .prepare_cached(
            "SELECT remote_sig, local_sig FROM sync_base WHERE domain = ?1 AND target = ?2",
        )?
        .query_row(params![domain.as_str(), target], |r| {
            Ok(SyncSignatures {
                remote: r.get(0)?,
                local: r.get(1)?,
            })
        })
        .optional()?)
}

/// Records the base of `domain`, replacing any base for another target.
pub fn set_base(
    conn: &Connection,
    domain: SyncDomain,
    target: &str,
    remote: &str,
    local: &str,
) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO sync_base (domain, target, remote_sig, local_sig) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(domain) DO UPDATE SET target = excluded.target, remote_sig = excluded.remote_sig,
           local_sig = excluded.local_sig",
    )?
    .execute(params![domain.as_str(), target, remote, local])?;
    Ok(())
}

/// The last server version seen for `target`.
pub fn version(conn: &Connection, target: &str) -> Result<Option<i64>> {
    Ok(conn
        .prepare_cached("SELECT version FROM sync_version WHERE target = ?1")?
        .query_row([target], |r| r.get(0))
        .optional()?)
}

pub fn record_version(conn: &Connection, target: &str, version: i64) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO sync_version (target, version) VALUES (?1, ?2)
         ON CONFLICT(target) DO UPDATE SET version = excluded.version",
    )?
    .execute(params![target, version])?;
    Ok(())
}

/// Forgets the version and every base recorded for `target`.
pub fn clear_target(conn: &Connection, target: &str) -> Result<()> {
    in_tx(conn, |conn| {
        conn.execute("DELETE FROM sync_version WHERE target = ?1", [target])?;
        conn.execute("DELETE FROM sync_base WHERE target = ?1", [target])?;
        Ok(())
    })
}
