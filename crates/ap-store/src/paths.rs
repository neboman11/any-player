//! Data and cache locations (design §6.1), unchanged from the legacy desktop.
//!
//! - `data_dir()/any-player/`: `playlists.db`, `logs/`, `dj_filler/<breakId>.wav`, `instance.lock`
//!   (Linux `~/.local/share/any-player`, macOS `~/Library/Application Support/any-player`,
//!   Windows `%APPDATA%\any-player`).
//! - `cache_dir()/any-player/`: `audio_cache/` (LRU 500 MiB) and `images/` (LRU 100 MiB).
//!
//! Tests use [`Paths::rooted`] so nothing touches the real user directories.

use std::io;
use std::path::{Path, PathBuf};

use crate::error::{Result, StoreError};

/// Directory name under the platform data and cache dirs.
pub const APP_DIR_NAME: &str = "any-player";
pub const DB_FILE_NAME: &str = "playlists.db";
pub const LOGS_DIR_NAME: &str = "logs";
pub const DJ_FILLER_DIR_NAME: &str = "dj_filler";
pub const INSTANCE_LOCK_FILE_NAME: &str = "instance.lock";
pub const AUDIO_CACHE_DIR_NAME: &str = "audio_cache";
pub const IMAGES_CACHE_DIR_NAME: &str = "images";
/// Legacy playback state file in the data dir (design §11.2 step 2).
pub const LEGACY_PLAYBACK_STATE_FILE_NAME: &str = "playback_state.json";

/// LRU budget of `audio_cache/`.
pub const AUDIO_CACHE_MAX_BYTES: u64 = 500 * 1024 * 1024;
/// LRU budget of `images/`.
pub const IMAGE_CACHE_MAX_BYTES: u64 = 100 * 1024 * 1024;
/// Rolling log files are kept this many days.
pub const LOG_RETENTION_DAYS: u32 = 7;

/// Resolved app directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    data: PathBuf,
    cache: PathBuf,
    temp: PathBuf,
}

impl Paths {
    /// The real per-user locations from `dirs` plus the OS temp dir.
    pub fn system() -> Result<Self> {
        let data = dirs::data_dir().ok_or(StoreError::NoPlatformDir("data"))?;
        let cache = dirs::cache_dir().ok_or(StoreError::NoPlatformDir("cache"))?;
        Ok(Self::new(
            data.join(APP_DIR_NAME),
            cache.join(APP_DIR_NAME),
            std::env::temp_dir(),
        ))
    }

    /// Explicit app directories (`…/any-player` already included).
    pub fn new(
        data: impl Into<PathBuf>,
        cache: impl Into<PathBuf>,
        temp: impl Into<PathBuf>,
    ) -> Self {
        Self {
            data: data.into(),
            cache: cache.into(),
            temp: temp.into(),
        }
    }

    /// Everything below `root`: `root/data/any-player`, `root/cache/any-player`, `root/tmp`.
    pub fn rooted(root: &Path) -> Self {
        Self::new(
            root.join("data").join(APP_DIR_NAME),
            root.join("cache").join(APP_DIR_NAME),
            root.join("tmp"),
        )
    }

    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache
    }

    /// Where the legacy app wrote `any-player-audio-*.mp3` temp files.
    pub fn temp_dir(&self) -> &Path {
        &self.temp
    }

    pub fn db_path(&self) -> PathBuf {
        self.data.join(DB_FILE_NAME)
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data.join(LOGS_DIR_NAME)
    }

    pub fn dj_filler_dir(&self) -> PathBuf {
        self.data.join(DJ_FILLER_DIR_NAME)
    }

    /// `dj_filler/<breakId>.wav`.
    pub fn dj_filler_wav(&self, break_id: &str) -> PathBuf {
        self.dj_filler_dir().join(format!("{break_id}.wav"))
    }

    pub fn instance_lock(&self) -> PathBuf {
        self.data.join(INSTANCE_LOCK_FILE_NAME)
    }

    pub fn audio_cache_dir(&self) -> PathBuf {
        self.cache.join(AUDIO_CACHE_DIR_NAME)
    }

    pub fn images_cache_dir(&self) -> PathBuf {
        self.cache.join(IMAGES_CACHE_DIR_NAME)
    }

    pub fn legacy_playback_state(&self) -> PathBuf {
        self.data.join(LEGACY_PLAYBACK_STATE_FILE_NAME)
    }

    /// Creates the data, logs, DJ filler and cache directories.
    pub fn ensure_dirs(&self) -> io::Result<()> {
        for dir in [
            self.data.clone(),
            self.logs_dir(),
            self.dj_filler_dir(),
            self.audio_cache_dir(),
            self.images_cache_dir(),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_design() {
        let p = Paths::rooted(Path::new("/r"));
        assert_eq!(p.db_path(), Path::new("/r/data/any-player/playlists.db"));
        assert_eq!(p.logs_dir(), Path::new("/r/data/any-player/logs"));
        assert_eq!(
            p.dj_filler_wav("b1"),
            Path::new("/r/data/any-player/dj_filler/b1.wav")
        );
        assert_eq!(
            p.instance_lock(),
            Path::new("/r/data/any-player/instance.lock")
        );
        assert_eq!(
            p.audio_cache_dir(),
            Path::new("/r/cache/any-player/audio_cache")
        );
        assert_eq!(
            p.images_cache_dir(),
            Path::new("/r/cache/any-player/images")
        );
        assert_eq!(
            p.legacy_playback_state(),
            Path::new("/r/data/any-player/playback_state.json")
        );
    }

    #[test]
    fn system_paths_end_in_app_dir() {
        if let Ok(p) = Paths::system() {
            assert!(p.data_dir().ends_with(APP_DIR_NAME));
            assert!(p.cache_dir().ends_with(APP_DIR_NAME));
            assert_eq!(p.data_dir().parent(), dirs::data_dir().as_deref());
        }
    }

    #[test]
    fn ensure_dirs_creates_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::rooted(tmp.path());
        p.ensure_dirs().unwrap();
        assert!(p.dj_filler_dir().is_dir() && p.logs_dir().is_dir());
        assert!(p.audio_cache_dir().is_dir() && p.images_cache_dir().is_dir());
    }
}
