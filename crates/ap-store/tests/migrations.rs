mod common;

use std::path::Path;

use ap_model::{PlaylistType, SourceType};
use ap_store::migrations::SCHEMA_VERSION;
use ap_store::playlists;
use ap_store::rusqlite::Connection;
use ap_store::{Db, StoreError};
use common::{LegacySchema, TestEnv, dump, write_legacy_db};

const NEW_TABLES: [&str; 10] = [
    "settings",
    "queue_entries",
    "queue_meta",
    "cached_playlists",
    "cached_track_owners",
    "cached_tracks",
    "sync_base",
    "sync_version",
    "dj_passage_state",
    "track_gain",
];

fn pragma_i64(db: &Path, pragma: &str) -> i64 {
    Connection::open(db)
        .unwrap()
        .query_row(&format!("PRAGMA {pragma}"), [], |r| r.get(0))
        .unwrap()
}

fn table_names(db: &Path) -> Vec<String> {
    Connection::open(db)
        .unwrap()
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn count(db: &Db, sql: &'static str) -> impl Future<Output = i64> {
    let db = db.clone();
    async move {
        db.read(move |c| Ok(c.query_row(sql, [], |r| r.get(0))?))
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn fresh_empty_db_gets_full_v1_schema() {
    let env = TestEnv::new();
    let db = env.db().await;
    let path = env.paths.db_path();
    assert_eq!(pragma_i64(&path, "user_version"), SCHEMA_VERSION);
    let tables = table_names(&path);
    for t in NEW_TABLES.iter().chain(&[
        "custom_playlists",
        "provider_playlist_preferences",
        "playlist_tracks",
        "union_playlist_sources",
        "column_preferences",
    ]) {
        assert!(tables.iter().any(|n| n == t), "missing {t}: {tables:?}");
    }
    let (fk, mode): (i64, String) = db
        .call(|c| {
            Ok((
                c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?,
                c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(fk, 1);
    assert_eq!(mode, "wal");
    assert!(
        db.call(|c| playlists::list_playlists(c))
            .await
            .unwrap()
            .is_empty()
    );
}

async fn assert_migrated_current_fixture(db: &Db, path: &Path) {
    assert_eq!(pragma_i64(path, "user_version"), 1);
    // Orphans gone.
    assert_eq!(
        count(
            db,
            "SELECT COUNT(*) FROM playlist_tracks WHERE playlist_id = 'gone'"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            db,
            "SELECT COUNT(*) FROM union_playlist_sources WHERE union_playlist_id = 'gone'"
        )
        .await,
        0
    );

    let lists = db.read(playlists::list_playlists).await.unwrap();
    let by_id = |id: &str| lists.iter().find(|p| p.id == id).unwrap().clone();
    // STANDARD counters recomputed; the UNION counter is left alone.
    assert_eq!(by_id("road").track_count, 4);
    assert_eq!(by_id("empty").track_count, 0);
    assert_eq!(by_id("dist").track_count, 2);
    assert_eq!(by_id("mix").track_count, 3);
    assert_eq!(by_id("mix").playlist_type, PlaylistType::Union);
    assert!(by_id("dist").is_distinct);
    // Legacy timestamps (epoch seconds) untouched.
    assert_eq!(
        (by_id("road").created_at, by_id("road").updated_at),
        (1_700_000_000, 1_700_000_100)
    );

    // Dense positions; ties broken by row id; duplicates kept.
    let road = db
        .read(|c| playlists::list_tracks(c, "road"))
        .await
        .unwrap();
    assert_eq!(
        road.iter().map(|t| t.position).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert_eq!(road.iter().map(|t| t.id).collect::<Vec<_>>(), [1, 2, 3, 4]);
    assert_eq!(
        road.iter().map(|t| t.track_id.as_str()).collect::<Vec<_>>(),
        ["a", "b", "a", "c"]
    );
    assert_eq!(road[1].track_source, SourceType::Jellyfin);
    let dist = db
        .read(|c| playlists::list_tracks(c, "dist"))
        .await
        .unwrap();
    assert_eq!(
        dist.iter()
            .map(|t| (t.track_id.as_str(), t.position))
            .collect::<Vec<_>>(),
        [("d", 0), ("e", 1)]
    );

    let sources = db
        .read(|c| playlists::list_union_sources(c, "mix"))
        .await
        .unwrap();
    assert_eq!(
        sources
            .iter()
            .map(|s| (s.source_type, s.source_playlist_id.as_str(), s.position))
            .collect::<Vec<_>>(),
        [
            (SourceType::Spotify, "sp1", 0),
            (SourceType::Jellyfin, "jf1", 1),
            (SourceType::Custom, "road", 2)
        ]
    );
    assert!(
        db.read(|c| playlists::provider_playlist_distinct(c, SourceType::Spotify, "sp1"))
            .await
            .unwrap()
    );
    let layout = db.read(playlists::column_layout).await.unwrap();
    assert_eq!(layout.columns, ["title", "artist", "source"]);
    assert_eq!(layout.column_order, [2, 0, 1]);
    assert_eq!(layout.column_widths.get("artist"), Some(&180));

    // Foreign keys are enforced now: deleting a playlist cascades.
    assert!(
        db.call(|c| playlists::delete_playlist(c, "road"))
            .await
            .unwrap()
    );
    assert_eq!(
        count(
            db,
            "SELECT COUNT(*) FROM playlist_tracks WHERE playlist_id = 'road'"
        )
        .await,
        0
    );
    let fk_err = db
        .call(|c| {
            Ok(c.execute(
                "INSERT INTO playlist_tracks (playlist_id, track_source, track_id, position, added_at, title, artist)
                 VALUES ('nope', 'spotify', 'z', 0, 0, 't', 'a')",
                [],
            )?)
        })
        .await;
    assert!(matches!(fk_err, Err(StoreError::Sqlite(_))), "{fk_err:?}");
    for t in NEW_TABLES {
        assert!(table_names(path).iter().any(|n| n == t), "missing {t}");
    }
}

#[tokio::test]
async fn migrates_generated_legacy_db() {
    let env = TestEnv::new();
    write_legacy_db(&env.paths.db_path(), LegacySchema::Current);
    assert_eq!(pragma_i64(&env.paths.db_path(), "user_version"), 0);
    let db = env.db().await;
    assert_migrated_current_fixture(&db, &env.paths.db_path()).await;
}

#[tokio::test]
async fn migrates_early_legacy_db_missing_columns_and_tables() {
    let env = TestEnv::new();
    write_legacy_db(&env.paths.db_path(), LegacySchema::Early);
    let db = env.db().await;
    let cols: Vec<String> = db
        .read(|c| {
            Ok(
                c.prepare("SELECT name FROM pragma_table_info('custom_playlists')")?
                    .query_map([], |r| r.get(0))?
                    .collect::<Result<_, _>>()?,
            )
        })
        .await
        .unwrap();
    assert!(cols.contains(&"playlist_type".to_owned()) && cols.contains(&"is_distinct".to_owned()));
    let lists = db.read(playlists::list_playlists).await.unwrap();
    assert_eq!(lists.len(), 2);
    assert!(
        lists
            .iter()
            .all(|p| p.playlist_type == PlaylistType::Standard && !p.is_distinct)
    );
    assert_eq!(
        lists.iter().find(|p| p.id == "road").unwrap().track_count,
        4
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM playlist_tracks WHERE playlist_id = 'gone'"
        )
        .await,
        0
    );
    // Tables the early schema lacked now exist and work.
    let union = db
        .call(|c| {
            playlists::create_playlist(c, &playlists::NewPlaylist::new("U", PlaylistType::Union))
        })
        .await
        .unwrap();
    db.call(move |c| playlists::add_union_source(c, &union.id, SourceType::Plex, "p1"))
        .await
        .unwrap();
    db.call(|c| playlists::set_provider_playlist_distinct(c, SourceType::Plex, "p1", true))
        .await
        .unwrap();
}

#[tokio::test]
async fn reopening_is_a_no_op() {
    let env = TestEnv::new();
    write_legacy_db(&env.paths.db_path(), LegacySchema::Current);
    drop(env.db().await);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let before = dump(&env.paths.db_path());
    let db = env.db().await;
    assert_eq!(db.read(playlists::list_playlists).await.unwrap().len(), 4);
    drop(db);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(dump(&env.paths.db_path()), before);
}

#[tokio::test]
async fn newer_schema_is_refused_and_untouched() {
    let env = TestEnv::new();
    let path = env.paths.db_path();
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE future (x); PRAGMA user_version = 99;")
        .unwrap();
    let err = Db::open(&path).await.unwrap_err();
    assert!(
        matches!(
            err,
            StoreError::SchemaTooNew {
                found: 99,
                supported: 1
            }
        ),
        "{err:?}"
    );
    assert_eq!(pragma_i64(&path, "user_version"), 99);
}

#[tokio::test]
async fn failed_migration_rolls_back_completely() {
    let env = TestEnv::new();
    let path = env.paths.db_path();
    write_legacy_db(&path, LegacySchema::Current);
    // A pre-existing `settings` table makes step 6 fail after steps 2–5 already ran.
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE settings (k TEXT)")
        .unwrap();
    let before = dump(&path);
    let err = Db::open(&path).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Migration { version: 1, .. }),
        "{err:?}"
    );
    assert_eq!(pragma_i64(&path, "user_version"), 0);
    assert_eq!(
        dump(&path),
        before,
        "a failed step must leave the legacy data untouched"
    );
}
