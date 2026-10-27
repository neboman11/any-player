//! Schema migrations, versioned by `PRAGMA user_version` (design §6.2).
//!
//! Version 0 is the legacy desktop schema (or an empty file). Each step runs in one
//! transaction with `foreign_keys` off; `user_version` is bumped inside that transaction and
//! foreign keys are switched back on afterwards. A failed step leaves the file at the previous
//! version — there is never a destructive fallback.

use rusqlite::{Connection, TransactionBehavior};

use crate::error::{Result, StoreError};

/// The schema version this build reads and writes.
pub const SCHEMA_VERSION: i64 = 2;

type Step = fn(&Connection) -> Result<()>;

/// `STEPS[n]` migrates version `n` to `n + 1`.
const STEPS: [Step; 2] = [migrate_0_to_1, migrate_1_to_2];

/// Current `PRAGMA user_version`.
pub fn user_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Brings the database to [`SCHEMA_VERSION`] and leaves `foreign_keys=ON`.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    let mut version = user_version(conn)?;
    if version > SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    while version < SCHEMA_VERSION {
        let next = version + 1;
        let wrap = |e: StoreError| StoreError::Migration {
            version: next,
            source: Box::new(e),
        };
        // PRAGMA foreign_keys is a no-op inside a transaction, so it is set before BEGIN.
        conn.pragma_update(None, "foreign_keys", false)
            .map_err(|e| wrap(e.into()))?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| wrap(e.into()))?;
        let step = STEPS[usize::try_from(version).expect("version is non-negative")];
        step(&tx).map_err(wrap)?;
        tx.pragma_update(None, "user_version", next)
            .map_err(|e| wrap(e.into()))?;
        tx.commit().map_err(|e| wrap(e.into()))?;
        tracing::info!(version = next, "database migrated");
        version = next;
    }
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(())
}

/// The legacy desktop tables (exact legacy DDL, `IF NOT EXISTS` so a legacy file keeps its
/// own definitions). An empty file gets the full legacy baseline.
const LEGACY_TABLES: &str = "
CREATE TABLE IF NOT EXISTS custom_playlists (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  description TEXT,
  image_url TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  track_count INTEGER DEFAULT 0,
  playlist_type TEXT DEFAULT 'standard',
  is_distinct INTEGER DEFAULT 0
);
CREATE TABLE IF NOT EXISTS provider_playlist_preferences (
  source TEXT NOT NULL,
  playlist_id TEXT NOT NULL,
  is_distinct INTEGER DEFAULT 0,
  PRIMARY KEY (source, playlist_id)
);
CREATE TABLE IF NOT EXISTS playlist_tracks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  playlist_id TEXT NOT NULL,
  track_source TEXT NOT NULL,
  track_id TEXT NOT NULL,
  position INTEGER NOT NULL,
  added_at INTEGER NOT NULL,
  title TEXT NOT NULL,
  artist TEXT NOT NULL,
  album TEXT,
  duration_ms INTEGER,
  image_url TEXT,
  FOREIGN KEY (playlist_id) REFERENCES custom_playlists(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS union_playlist_sources (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  union_playlist_id TEXT NOT NULL,
  source_type TEXT NOT NULL,
  source_playlist_id TEXT NOT NULL,
  position INTEGER NOT NULL,
  added_at INTEGER NOT NULL,
  FOREIGN KEY (union_playlist_id) REFERENCES custom_playlists(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS column_preferences (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  columns TEXT NOT NULL,
  column_order TEXT NOT NULL,
  column_widths TEXT
);
CREATE INDEX IF NOT EXISTS idx_playlist_tracks_playlist_id ON playlist_tracks(playlist_id);
CREATE INDEX IF NOT EXISTS idx_playlist_tracks_position ON playlist_tracks(playlist_id, position);
CREATE INDEX IF NOT EXISTS idx_union_playlist_sources_union_id ON union_playlist_sources(union_playlist_id);
CREATE INDEX IF NOT EXISTS idx_union_playlist_sources_position ON union_playlist_sources(union_playlist_id, position);
";

/// Version 1 additions (design §6.2 "New tables").
const V1_TABLES: &str = "
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE queue_entries (uid INTEGER PRIMARY KEY, seq INTEGER NOT NULL, track_json TEXT NOT NULL);
CREATE TABLE queue_meta (id INTEGER PRIMARY KEY CHECK (id = 0), order_uids TEXT NOT NULL,
                         cursor_uid INTEGER, content_version INTEGER NOT NULL);
CREATE TABLE cached_playlists (source TEXT NOT NULL, id TEXT NOT NULL, name TEXT NOT NULL, owner TEXT NOT NULL,
                               description TEXT, image_url TEXT, track_count INTEGER NOT NULL,
                               sort_index INTEGER NOT NULL, fetched_at_ms INTEGER NOT NULL,
                               PRIMARY KEY (source, id));
CREATE TABLE cached_track_owners (owner_key TEXT PRIMARY KEY, updated_at_ms INTEGER NOT NULL, track_count INTEGER NOT NULL);
CREATE TABLE cached_tracks (owner_key TEXT NOT NULL REFERENCES cached_track_owners(owner_key) ON DELETE CASCADE,
                            position INTEGER NOT NULL, track_json TEXT NOT NULL, PRIMARY KEY (owner_key, position));
CREATE TABLE sync_base (domain TEXT NOT NULL, target TEXT NOT NULL, remote_sig TEXT NOT NULL, local_sig TEXT NOT NULL,
                        PRIMARY KEY (domain));
CREATE TABLE sync_version (target TEXT PRIMARY KEY, version INTEGER NOT NULL);
CREATE TABLE dj_passage_state (account_key TEXT PRIMARY KEY, state_json TEXT NOT NULL);
CREATE TABLE track_gain (track_key TEXT PRIMARY KEY, gain REAL NOT NULL, analyzed_at_ms INTEGER NOT NULL);
";

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        [table, column],
        |r| r.get(0),
    )?)
}

fn add_column_if_missing(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    if !has_column(conn, table, column)? {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))?;
        tracing::info!(table, column, "added missing legacy column");
    }
    Ok(())
}

/// Legacy → v2 baseline (design §6.2 steps 2–6; the caller does 1 and 7).
fn migrate_0_to_1(conn: &Connection) -> Result<()> {
    conn.execute_batch(LEGACY_TABLES)?;
    // 2. The legacy ad-hoc column checks.
    add_column_if_missing(
        conn,
        "custom_playlists",
        "playlist_type",
        "TEXT DEFAULT 'standard'",
    )?;
    add_column_if_missing(conn, "custom_playlists", "is_distinct", "INTEGER DEFAULT 0")?;

    // 3. Orphans (the legacy app never enabled foreign keys).
    let orphan_tracks = conn.execute(
        "DELETE FROM playlist_tracks WHERE playlist_id NOT IN (SELECT id FROM custom_playlists)",
        [],
    )?;
    let orphan_sources = conn.execute(
        "DELETE FROM union_playlist_sources WHERE union_playlist_id NOT IN (SELECT id FROM custom_playlists)",
        [],
    )?;

    // 4. Denormalized counters of STANDARD playlists.
    conn.execute(
        "UPDATE custom_playlists
            SET track_count = (SELECT COUNT(*) FROM playlist_tracks t WHERE t.playlist_id = custom_playlists.id)
          WHERE lower(COALESCE(playlist_type, 'standard')) = 'standard'",
        [],
    )?;

    // 5. Dense 0..n positions per playlist (ties keep row-id order).
    conn.execute(
        "UPDATE playlist_tracks SET position = r.rn
           FROM (SELECT id, ROW_NUMBER() OVER (PARTITION BY playlist_id ORDER BY position, id) - 1 AS rn
                   FROM playlist_tracks) AS r
          WHERE playlist_tracks.id = r.id AND playlist_tracks.position IS NOT r.rn",
        [],
    )?;
    conn.execute(
        "UPDATE union_playlist_sources SET position = r.rn
           FROM (SELECT id, ROW_NUMBER() OVER (PARTITION BY union_playlist_id ORDER BY position, id) - 1 AS rn
                   FROM union_playlist_sources) AS r
          WHERE union_playlist_sources.id = r.id AND union_playlist_sources.position IS NOT r.rn",
        [],
    )?;

    // 6. New tables.
    conn.execute_batch(V1_TABLES)?;

    // The result must satisfy every declared foreign key before it is committed.
    let violations: i64 =
        conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })?;
    if violations > 0 {
        return Err(StoreError::Corrupt(format!(
            "{violations} foreign key violations remain after cleanup"
        )));
    }
    if orphan_tracks + orphan_sources > 0 {
        tracing::info!(orphan_tracks, orphan_sources, "removed legacy orphan rows");
    }
    Ok(())
}

/// Wire metadata for exact state-file round trips (design §6.2, migration 1 → 2).
///
/// State files from Android carry string row ids, ISO timestamps with fractional seconds and
/// track URLs that the legacy integer-keyed, seconds-precision rows cannot represent. The columns
/// are nullable: rows created locally have no wire id until first exported, and the legacy app
/// ignores unknown columns.
fn migrate_1_to_2(conn: &Connection) -> Result<()> {
    for (table, column) in [
        ("custom_playlists", "created_at_iso"),
        ("custom_playlists", "updated_at_iso"),
        ("playlist_tracks", "wire_id"),
        ("playlist_tracks", "added_at_iso"),
        ("playlist_tracks", "url"),
        ("union_playlist_sources", "wire_id"),
        ("union_playlist_sources", "added_at_iso"),
    ] {
        add_column_if_missing(conn, table, column, "TEXT")?;
    }
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_playlist_tracks_wire_id
             ON playlist_tracks(wire_id) WHERE wire_id IS NOT NULL;
         CREATE UNIQUE INDEX IF NOT EXISTS idx_union_playlist_sources_wire_id
             ON union_playlist_sources(wire_id) WHERE wire_id IS NOT NULL;",
    )?;
    Ok(())
}
