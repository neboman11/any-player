//! Shared test helpers, including the legacy `playlists.db` fixture generator.
#![allow(dead_code, reason = "each test binary uses a subset")]

use std::path::Path;

use ap_model::{SourceType, Track};
use ap_store::rusqlite::{Connection, params};
use ap_store::{Db, Paths};

/// A temp dir with [`Paths`] rooted in it.
pub struct TestEnv {
    pub dir: tempfile::TempDir,
    pub paths: Paths,
}

impl TestEnv {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(dir.path());
        paths.ensure_dirs().unwrap();
        Self { dir, paths }
    }

    pub async fn db(&self) -> Db {
        Db::open(self.paths.db_path()).await.unwrap()
    }
}

pub fn track(id: &str, source: SourceType) -> Track {
    Track {
        album: Some(format!("Album {id}")),
        duration_ms: Some(180_000),
        ..Track::new(id, format!("Title {id}"), format!("Artist {id}"), source)
    }
}

/// Which legacy schema generation to emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacySchema {
    /// The last legacy schema (`database/mod.rs:132`).
    Current,
    /// An early legacy DB: `custom_playlists` without `playlist_type`/`is_distinct`, and no
    /// union or provider-preference tables yet.
    Early,
}

/// Exact legacy DDL (inventory §4).
const LEGACY_DDL: &str = "
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
  title TEXT NOT NULL, artist TEXT NOT NULL, album TEXT,
  duration_ms INTEGER, image_url TEXT,
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

/// The early schema before the ad-hoc column checks and union support existed.
const EARLY_DDL: &str = "
CREATE TABLE IF NOT EXISTS custom_playlists (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  description TEXT,
  image_url TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  track_count INTEGER DEFAULT 0
);
CREATE TABLE IF NOT EXISTS playlist_tracks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  playlist_id TEXT NOT NULL,
  track_source TEXT NOT NULL,
  track_id TEXT NOT NULL,
  position INTEGER NOT NULL,
  added_at INTEGER NOT NULL,
  title TEXT NOT NULL, artist TEXT NOT NULL, album TEXT,
  duration_ms INTEGER, image_url TEXT,
  FOREIGN KEY (playlist_id) REFERENCES custom_playlists(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS column_preferences (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  columns TEXT NOT NULL,
  column_order TEXT NOT NULL,
  column_widths TEXT
);
CREATE INDEX IF NOT EXISTS idx_playlist_tracks_playlist_id ON playlist_tracks(playlist_id);
CREATE INDEX IF NOT EXISTS idx_playlist_tracks_position ON playlist_tracks(playlist_id, position);
";

fn insert_track(
    c: &Connection,
    playlist: &str,
    source: &str,
    id: &str,
    position: i64,
    added_at: i64,
) {
    c.execute(
        "INSERT INTO playlist_tracks (playlist_id, track_source, track_id, position, added_at, title, artist, album,
                                      duration_ms, image_url)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            playlist,
            source,
            id,
            position,
            added_at,
            format!("Title {id}"),
            format!("Artist {id}"),
            format!("Album {id}"),
            200_000,
            format!("http://img/{id}.jpg"),
        ],
    )
    .unwrap();
}

/// Writes a legacy DB as the Tauri app left it: no `user_version`, no foreign keys enforced,
/// orphan rows, position gaps and duplicate positions, duplicate tracks and stale counters.
///
/// Contents (`Current`):
/// - `road` STANDARD, `track_count` 99 (stale). Tracks at positions 0, 2, 2, 7 with ids
///   `a`, `b`, `a` (duplicate track), `c` (row ids 1..=4).
/// - `mix` UNION, `track_count` 3 (kept: unions are not recounted). Sources at positions 1, 4,
///   4 (`spotify:sp1`, `jellyfin:jf1`, `custom:road`).
/// - `empty` STANDARD, `track_count` 5 (stale), no tracks.
/// - `dist` STANDARD distinct, tracks at positions 5 then 3 (`e` row 5, `d` row 6).
/// - Orphans: two `playlist_tracks` and one `union_playlist_sources` row for `gone`.
/// - `provider_playlist_preferences` (`spotify`, `sp1`, distinct) and a `column_preferences` row.
///
/// `Early`: only `road`, `empty` (+ the `gone` track orphans) and the column row.
pub fn write_legacy_db(path: &Path, schema: LegacySchema) {
    let c = Connection::open(path).unwrap();
    // The legacy app left foreign keys unenforced; rusqlite enables them per connection.
    c.pragma_update(None, "foreign_keys", false).unwrap();
    c.execute_batch(if schema == LegacySchema::Current {
        LEGACY_DDL
    } else {
        EARLY_DDL
    })
    .unwrap();
    let playlist = |id: &str, name: &str, count: i64, kind: &str, distinct: bool| {
        if schema == LegacySchema::Current {
            c.execute(
                "INSERT INTO custom_playlists (id, name, description, image_url, created_at, updated_at, track_count,
                                               playlist_type, is_distinct)
                 VALUES (?1, ?2, 'desc', NULL, 1700000000, 1700000100, ?3, ?4, ?5)",
                params![id, name, count, kind, distinct],
            )
            .unwrap();
        } else {
            c.execute(
                "INSERT INTO custom_playlists (id, name, description, image_url, created_at, updated_at, track_count)
                 VALUES (?1, ?2, 'desc', NULL, 1700000000, 1700000100, ?3)",
                params![id, name, count],
            )
            .unwrap();
        }
    };
    playlist("road", "Road Trip", 99, "standard", false);
    playlist("empty", "Empty", 5, "standard", false);
    insert_track(&c, "road", "spotify", "a", 0, 1_700_000_001);
    insert_track(&c, "road", "jellyfin", "b", 2, 1_700_000_002);
    insert_track(&c, "road", "spotify", "a", 2, 1_700_000_003);
    insert_track(&c, "road", "plex", "c", 7, 1_700_000_004);
    if schema == LegacySchema::Current {
        playlist("mix", "Mix", 3, "union", false);
        playlist("dist", "Distinct", 2, "standard", true);
        insert_track(&c, "dist", "spotify", "e", 5, 1_700_000_005);
        insert_track(&c, "dist", "spotify", "d", 3, 1_700_000_006);
        for (source, id, pos) in [
            ("spotify", "sp1", 1),
            ("jellyfin", "jf1", 4),
            ("custom", "road", 4),
        ] {
            c.execute(
                "INSERT INTO union_playlist_sources (union_playlist_id, source_type, source_playlist_id, position, added_at)
                 VALUES ('mix', ?1, ?2, ?3, 1700000010)",
                params![source, id, pos],
            )
            .unwrap();
        }
        c.execute(
            "INSERT INTO union_playlist_sources (union_playlist_id, source_type, source_playlist_id, position, added_at)
             VALUES ('gone', 'plex', 'px', 0, 1700000011)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO provider_playlist_preferences (source, playlist_id, is_distinct) VALUES ('spotify', 'sp1', 1)",
            [],
        )
        .unwrap();
    }
    insert_track(&c, "gone", "spotify", "x", 0, 1_700_000_020);
    insert_track(&c, "gone", "spotify", "y", 1, 1_700_000_021);
    c.execute(
        "INSERT INTO column_preferences (id, columns, column_order, column_widths)
         VALUES (1, '[\"title\",\"artist\",\"source\"]', '[2,0,1]', '{\"title\":320,\"artist\":180.4}')",
        [],
    )
    .unwrap();
}

/// Every row of every table except `sqlite_*`, as text, for whole-DB comparisons.
pub fn dump(path: &Path) -> Vec<String> {
    let c = Connection::open(path).unwrap();
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut out = Vec::new();
    for t in tables {
        let column_count = c
            .prepare(&format!("SELECT * FROM {t}"))
            .unwrap()
            .column_count();
        let order_by = if column_count > 1 { "1, 2" } else { "1" };
        let mut stmt = c
            .prepare(&format!("SELECT * FROM {t} ORDER BY {order_by}"))
            .unwrap();
        let n = stmt.column_count();
        let rows = stmt
            .query_map([], |r| {
                let cells: Vec<String> = (0..n)
                    .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                    .collect();
                Ok(format!("{t}: {}", cells.join(" | ")))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        out.extend(rows);
    }
    out
}
