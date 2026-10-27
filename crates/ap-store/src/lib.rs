//! Persistence for Any Player desktop (design §6, §11): data/cache paths, the SQLite database
//! (`playlists.db`) with its versioned migrations, synchronous transactional repositories, the
//! typed `settings` store, OS-keyring secrets and the one-shot legacy (Tauri 0.1.x) migrations.
//!
//! # Usage
//!
//! - [`Db::open`] starts the DB thread, which owns the only writer connection, and runs the
//!   schema migrations before returning. Writes go through [`Db::call`]; multi-statement reads
//!   can use [`Db::read`], which runs on a small pool of read-only WAL connections.
//! - Repository modules ([`playlists`], [`catalog`], [`queue`], [`settings`], [`sync_state`],
//!   [`dj_state`], [`track_gain`]) expose plain functions over `&rusqlite::Connection`. Every
//!   multi-statement mutation runs in its own transaction (a savepoint, so callers can compose
//!   several repository calls inside an outer [`in_tx`]).
//! - [`SettingsStore`] and [`Secrets`] are the observable services built on top.
//! - [`legacy::run_first_run_migrations`] performs design §11.2 steps 2 and 4.
//!
//! Cursor-only adapters use [`queue::write_cursor`] with the uid and content version directly;
//! no queue clone or dependency on the playback crate is needed. Column layouts canonicalize
//! legacy `duration_ms` keys to `duration` on read and write (canonical widths take precedence).
//! Settings clamp provider page sizes to 1–1000 and voice levels to 0.5–2.0.
//! A legacy import never mixes old player settings with an already-persisted v2 queue.

pub mod catalog;
pub mod db;
pub mod dj_state;
mod error;
pub mod legacy;
pub mod migrations;
pub mod paths;
pub mod playlists;
pub mod queue;
mod sanitize;
pub mod secrets;
pub mod settings;
pub mod sync_state;
pub mod track_gain;
mod util;

pub use db::{Db, in_tx};
pub use error::{Result, StoreError};
pub use paths::Paths;
pub use sanitize::{persisted_track_json, strip_plex_token};
pub use secrets::{
    KeyringBackend, MemoryBackend, SecretAccount, SecretBackend, Secrets, SecretsError,
    SecretsState,
};
pub use settings::{Settings, SettingsStore};
pub use util::{now_ms, now_secs};

/// Re-exported so dependants use the same SQLite binding version.
pub use rusqlite;
