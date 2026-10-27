//! One-shot upgrades from the Tauri desktop 0.1.x (design §11.2 steps 2 and 4). Each step is
//! guarded by a `migration.*` flag and is idempotent: re-running after a crash at any point
//! completes the step without duplicating or losing data.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ap_model::{QueueEntry, QueueState, RepeatMode, Track};
use rusqlite::Connection;
use serde::Deserialize;

use crate::db::{Db, in_tx};
use crate::error::Result;
use crate::paths::Paths;
use crate::queue;
use crate::settings::{self, keys};

/// Flag (`migration.<name>`) of the `playback_state.json` import.
pub const PLAYBACK_STATE_MIGRATION: &str = "playback_state_json";
/// Flag of the legacy cache-file sweep.
pub const CACHE_SWEEP_MIGRATION: &str = "legacy_cache_sweep";
/// Suffix of the retired state file.
pub const MIGRATED_SUFFIX: &str = ".migrated-v2";

/// The legacy `PersistentPlaybackState` (`serde_json::to_string_pretty`, snake_case keys,
/// PascalCase `Source`, `repeat_mode` `"Off"|"One"|"All"`). Unknown keys (`state`,
/// `audio_normalization_target`, `current_track`) are ignored; missing ones take legacy defaults.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LegacyPlaybackState {
    pub queue: Vec<Track>,
    /// A position in `shuffle_order` when shuffle is on and the order is non-empty, otherwise an
    /// index into `queue`.
    pub current_index: usize,
    pub position_ms: u64,
    pub shuffle: bool,
    pub repeat_mode: RepeatMode,
    pub volume: u32,
    pub audio_normalization_enabled: bool,
    pub shuffle_order: Vec<usize>,
}

impl Default for LegacyPlaybackState {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            current_index: 0,
            position_ms: 0,
            shuffle: false,
            repeat_mode: RepeatMode::Off,
            volume: 50,
            audio_normalization_enabled: false,
            shuffle_order: Vec::new(),
        }
    }
}

impl LegacyPlaybackState {
    /// Maps the legacy queue to a [`QueueState`] (`None` for an empty queue).
    ///
    /// Entries keep insertion order with uids `1..=n`. With `shuffle` and a non-empty
    /// `shuffle_order`, the play order is `shuffle_order` (out-of-range and repeated indices
    /// dropped, missing ones appended) and the cursor is the entry `current_index` pointed at;
    /// otherwise the order is the identity and the cursor is `current_index`. Out-of-range
    /// cursors clamp to the last entry.
    pub fn to_queue(&self) -> Option<QueueState> {
        let n = self.queue.len();
        if n == 0 {
            return None;
        }
        let entries: Vec<QueueEntry> = self
            .queue
            .iter()
            .zip(1u64..)
            .map(|(track, uid)| QueueEntry {
                uid,
                track: track.clone(),
            })
            .collect();
        let (order, cursor) = if self.shuffle && !self.shuffle_order.is_empty() {
            let mut seen = vec![false; n];
            let mut order = Vec::with_capacity(n);
            for &i in &self.shuffle_order {
                if i < n && !std::mem::replace(&mut seen[i], true) {
                    order.push(i);
                }
            }
            order.extend((0..n).filter(|&i| !seen[i]));
            let target = self
                .shuffle_order
                .get(self.current_index)
                .copied()
                .filter(|&i| i < n);
            let cursor = target
                .and_then(|t| order.iter().position(|&i| i == t))
                .unwrap_or_else(|| self.current_index.min(n - 1));
            (order, cursor)
        } else {
            ((0..n).collect(), self.current_index.min(n - 1))
        };
        Some(QueueState {
            entries: Arc::from(entries),
            order: Arc::from(order),
            cursor: isize::try_from(cursor).expect("queue length fits isize"),
            shuffle: self.shuffle,
            repeat: self.repeat_mode,
            play_next_offset: 0,
            content_version: 1,
            next_uid: n as u64 + 1,
        })
    }
}

/// Result of [`migrate_playback_state`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackStateOutcome {
    /// The flag was already set (a leftover file is renamed now).
    AlreadyDone,
    /// No legacy file.
    NoFile,
    /// The file could not be decoded; it is left in place and the step is marked done.
    Undecodable(String),
    /// Settings copied; `queue_entries` entries written (0 for an empty queue or when a v2
    /// queue already existed).
    Migrated { queue_entries: usize },
}

fn retired_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(MIGRATED_SUFFIX);
    PathBuf::from(name)
}

fn retire(file: &Path) {
    if let Err(e) = std::fs::rename(file, retired_path(file)) {
        tracing::warn!(file = %file.display(), error = %e, "could not rename the legacy playback state");
    }
}

/// §11.2 step 2: `playback_state.json` → queue tables and `player.*` settings. One
/// transaction commits the settings, the queue (only if no v2 queue exists yet) and the flag;
/// the file is renamed to `playback_state.json.migrated-v2` afterwards.
pub fn migrate_playback_state(conn: &Connection, paths: &Paths) -> Result<PlaybackStateOutcome> {
    let file = paths.legacy_playback_state();
    if settings::migration_done(conn, PLAYBACK_STATE_MIGRATION)? {
        if file.exists() {
            retire(&file);
        }
        return Ok(PlaybackStateOutcome::AlreadyDone);
    }
    let raw = match std::fs::read_to_string(&file) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            settings::mark_migration_done(conn, PLAYBACK_STATE_MIGRATION)?;
            return Ok(PlaybackStateOutcome::NoFile);
        }
        Err(e) => return Err(e.into()),
    };
    let legacy: LegacyPlaybackState = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "legacy playback state is unreadable; skipping it");
            settings::mark_migration_done(conn, PLAYBACK_STATE_MIGRATION)?;
            return Ok(PlaybackStateOutcome::Undecodable(e.to_string()));
        }
    };
    let written = in_tx(conn, |conn| {
        let mut written = 0;
        // Queue and player settings describe one snapshot. Preserve both when v2 already
        // persisted a queue; mixing a legacy cursor's shuffle/position with it is unsafe.
        let fresh = if queue::has_queue(conn)? {
            None
        } else {
            settings::put(conn, keys::PLAYER_VOLUME, &legacy.volume.min(100))?;
            settings::put(conn, keys::PLAYER_SHUFFLE, &legacy.shuffle)?;
            settings::put(conn, keys::PLAYER_REPEAT, &legacy.repeat_mode)?;
            settings::put(conn, keys::PLAYER_POSITION_MS, &legacy.position_ms)?;
            settings::put(
                conn,
                keys::PLAYER_NORMALIZATION_ENABLED,
                &legacy.audio_normalization_enabled,
            )?;
            legacy.to_queue()
        };
        if let Some(q) = fresh {
            queue::write_content(conn, &q)?;
            written = q.entries.len();
        }
        settings::mark_migration_done(conn, PLAYBACK_STATE_MIGRATION)?;
        Ok(written)
    })?;
    retire(&file);
    Ok(PlaybackStateOutcome::Migrated {
        queue_entries: written,
    })
}

/// Result of [`sweep_legacy_cache`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub already_done: bool,
    pub removed: usize,
    /// Files that could not be deleted; the flag stays unset so the sweep retries next start.
    pub failed: usize,
}

const CACHE_FILES: [&str; 2] = ["playlists_cache.json", "custom_playlists_cache.json"];
const CACHE_PREFIXES: [&str; 4] = [
    "custom_playlist_tracks_",
    "union_playlist_tracks_",
    "provider_playlist_",
    "track_metadata_spotify_",
];

fn is_legacy_cache_file(name: &str) -> bool {
    CACHE_FILES.contains(&name) || CACHE_PREFIXES.iter().any(|p| name.starts_with(p))
}

fn is_legacy_temp_audio(name: &str) -> bool {
    name.starts_with("any-player-audio-")
        && Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"))
}

fn sweep_dir(dir: &Path, matches: fn(&str) -> bool, report: &mut SweepReport) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "cannot list directory for the legacy sweep");
            report.failed += 1;
            return;
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !matches(name) || !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => report.removed += 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::warn!(file = %entry.path().display(), error = %e, "cannot delete legacy cache file");
                report.failed += 1;
            }
        }
    }
}

/// §11.2 step 4: deletes the legacy frontend cache blobs and Spotify metadata files from the
/// cache dir and `any-player-audio-*.mp3` from the temp dir. The new cache subdirectories are
/// never touched.
pub fn sweep_legacy_cache(conn: &Connection, paths: &Paths) -> Result<SweepReport> {
    if settings::migration_done(conn, CACHE_SWEEP_MIGRATION)? {
        return Ok(SweepReport {
            already_done: true,
            ..SweepReport::default()
        });
    }
    let mut report = SweepReport::default();
    sweep_dir(paths.cache_dir(), is_legacy_cache_file, &mut report);
    sweep_dir(paths.temp_dir(), is_legacy_temp_audio, &mut report);
    if report.failed == 0 {
        settings::mark_migration_done(conn, CACHE_SWEEP_MIGRATION)?;
    }
    Ok(report)
}

/// Outcome of [`run_first_run_migrations`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstRunReport {
    pub playback_state: PlaybackStateOutcome,
    pub cache_sweep: SweepReport,
}

/// Runs §11.2 steps 2 and 4 on the DB thread (step 1 already ran in [`Db::open`]). Call
/// before any service loads settings or the queue.
pub async fn run_first_run_migrations(db: &Db, paths: &Paths) -> Result<FirstRunReport> {
    let paths = paths.clone();
    db.call(move |conn| {
        Ok(FirstRunReport {
            playback_state: migrate_playback_state(conn, &paths)?,
            cache_sweep: sweep_legacy_cache(conn, &paths)?,
        })
    })
    .await
}
