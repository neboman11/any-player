//! Typed settings over the `settings` KV table (design §6.3). Values are JSON.
//!
//! [`Settings`] is the typed snapshot of every §6.3 key; [`SettingsStore`] loads it once,
//! publishes it on a `watch` channel and persists changes made through
//! [`SettingsStore::update`] (only the keys whose value changed are written, in one
//! transaction). Unreadable values use defaults; numeric values clamp to their documented ranges.
//! One-shot `migration.*` flags live in the same table but outside the snapshot.

use std::collections::BTreeMap;

use ap_model::RepeatMode;
use rusqlite::{Connection, OptionalExtension, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, watch};

use crate::db::{Db, in_tx};
use crate::error::Result;

/// Setting keys.
pub mod keys {
    pub const PLAYER_VOLUME: &str = "player.volume";
    pub const PLAYER_SHUFFLE: &str = "player.shuffle";
    pub const PLAYER_REPEAT: &str = "player.repeat";
    pub const PLAYER_POSITION_MS: &str = "player.position_ms";
    pub const PLAYER_NORMALIZATION_ENABLED: &str = "player.normalization_enabled";
    pub const PLAYER_NORMALIZATION_STRICT: &str = "player.normalization_strict";
    pub const DJ_ENABLED: &str = "dj.enabled";
    pub const DJ_SHOW_ENTRIES_IN_QUEUE: &str = "dj.show_entries_in_queue";
    pub const DJ_VOICE_ID: &str = "dj.voice_id";
    pub const DJ_VOICE_LEVEL: &str = "dj.voice_level";
    pub const DJ_DEBUG_LOGGING: &str = "dj.debug_logging";
    pub const SYNC_SERVER_TARGET: &str = "sync.server_target";
    pub const SYNC_CLIENT_ID: &str = "sync.client_id";
    pub const SYNC_DOMAINS: &str = "sync.domains";
    pub const SPOTIFY_PREFERRED_DEVICE: &str = "spotify.preferred_device";
    pub const UI_CLOSE_TO_TRAY: &str = "ui.close_to_tray";
    /// Prefix of `ui.track_sort.<tableKey>`.
    pub const UI_TRACK_SORT_PREFIX: &str = "ui.track_sort.";
    pub const UI_WINDOW: &str = "ui.window";
    pub const PROVIDERS_JELLYFIN_PAGE_SIZE: &str = "providers.jellyfin.page_size";
    pub const PROVIDERS_PLEX_PAGE_SIZE: &str = "providers.plex.page_size";
    pub const PROVIDERS_JELLYFIN_USER_ID: &str = "providers.jellyfin.user_id";
    /// Prefix of the one-shot `migration.<name>` flags.
    pub const MIGRATION_PREFIX: &str = "migration.";
}

pub const DEFAULT_VOLUME: i32 = 50;
pub const DEFAULT_VOICE_LEVEL: f64 = 1.0;
pub const VOICE_LEVEL_RANGE: std::ops::RangeInclusive<f64> = 0.5..=2.0;
pub const DEFAULT_PAGE_SIZE: u32 = 300;
pub const PAGE_SIZE_RANGE: std::ops::RangeInclusive<u32> = 1..=1000;

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSettings {
    /// 0..=100.
    pub volume: i32,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub position_ms: i64,
    pub normalization_enabled: bool,
    /// Not shown in the UI; kept so the sync `settings` namespace round-trips Android's key.
    pub normalization_strict: bool,
}

impl Default for PlayerSettings {
    fn default() -> Self {
        Self {
            volume: DEFAULT_VOLUME,
            shuffle: false,
            repeat: RepeatMode::Off,
            position_ms: 0,
            normalization_enabled: false,
            normalization_strict: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DjSettings {
    pub enabled: bool,
    pub show_entries_in_queue: bool,
    pub voice_id: Option<String>,
    /// 0.5..=2.0.
    pub voice_level: f64,
    pub debug_logging: bool,
}

impl Default for DjSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            show_entries_in_queue: false,
            voice_id: None,
            voice_level: DEFAULT_VOICE_LEVEL,
            debug_logging: false,
        }
    }
}

/// Which sync namespaces are enabled (`sync.domains`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncDomains {
    pub app_state: bool,
    pub playlists: bool,
    pub provider_configuration: bool,
    pub settings: bool,
}

impl Default for SyncDomains {
    fn default() -> Self {
        Self {
            app_state: true,
            playlists: true,
            provider_configuration: true,
            settings: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncSettings {
    /// `""` = not configured.
    pub server_target: String,
    /// `desktop-<uuid>`, generated on first open.
    pub client_id: String,
    pub domains: SyncDomains,
}

/// `spotify.preferred_device`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreferredDevice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpotifySettings {
    pub preferred_device: Option<PreferredDevice>,
}

/// `ui.track_sort.<tableKey>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackSort {
    pub column: String,
    pub ascending: bool,
}

/// `ui.window`: logical size and position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub maximized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSettings {
    pub close_to_tray: bool,
    /// Keyed by table key.
    pub track_sort: BTreeMap<String, TrackSort>,
    pub window: Option<WindowGeometry>,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            track_sort: BTreeMap::new(),
            window: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSettings {
    /// 1..=1000.
    pub jellyfin_page_size: u32,
    /// 1..=1000.
    pub plex_page_size: u32,
    pub jellyfin_user_id: Option<String>,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            jellyfin_page_size: DEFAULT_PAGE_SIZE,
            plex_page_size: DEFAULT_PAGE_SIZE,
            jellyfin_user_id: None,
        }
    }
}

/// Typed snapshot of every §6.3 setting.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub player: PlayerSettings,
    pub dj: DjSettings,
    pub sync: SyncSettings,
    pub spotify: SpotifySettings,
    pub ui: UiSettings,
    pub providers: ProviderSettings,
}

fn json<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("settings values serialize")
}

fn decode<T: DeserializeOwned>(key: &str, value: Value) -> Option<T> {
    match serde_json::from_value(value) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(key, error = %e, "ignoring unreadable setting");
            None
        }
    }
}

fn set<T: DeserializeOwned>(slot: &mut T, key: &str, value: Value) {
    if let Some(v) = decode(key, value) {
        *slot = v;
    }
}

impl Settings {
    /// Clamps out-of-range values back into their documented ranges.
    pub fn normalize(&mut self) {
        self.player.volume = self.player.volume.clamp(0, 100);
        self.player.position_ms = self.player.position_ms.max(0);
        let level = self.dj.voice_level;
        self.dj.voice_level = if level.is_finite() {
            level.clamp(*VOICE_LEVEL_RANGE.start(), *VOICE_LEVEL_RANGE.end())
        } else {
            DEFAULT_VOICE_LEVEL
        };
        for size in [
            &mut self.providers.jellyfin_page_size,
            &mut self.providers.plex_page_size,
        ] {
            *size = (*size).clamp(*PAGE_SIZE_RANGE.start(), *PAGE_SIZE_RANGE.end());
        }
    }

    /// Every key with its JSON value.
    pub fn to_entries(&self) -> BTreeMap<String, Value> {
        use keys::{
            DJ_DEBUG_LOGGING, DJ_ENABLED, DJ_SHOW_ENTRIES_IN_QUEUE, DJ_VOICE_ID, DJ_VOICE_LEVEL,
            PLAYER_NORMALIZATION_ENABLED, PLAYER_NORMALIZATION_STRICT, PLAYER_POSITION_MS,
            PLAYER_REPEAT, PLAYER_SHUFFLE, PLAYER_VOLUME, PROVIDERS_JELLYFIN_PAGE_SIZE,
            PROVIDERS_JELLYFIN_USER_ID, PROVIDERS_PLEX_PAGE_SIZE, SPOTIFY_PREFERRED_DEVICE,
            SYNC_CLIENT_ID, SYNC_DOMAINS, SYNC_SERVER_TARGET, UI_CLOSE_TO_TRAY,
            UI_TRACK_SORT_PREFIX, UI_WINDOW,
        };
        let mut m = BTreeMap::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_owned(), v);
        };
        put(PLAYER_VOLUME, json(&self.player.volume));
        put(PLAYER_SHUFFLE, json(&self.player.shuffle));
        put(PLAYER_REPEAT, json(&self.player.repeat));
        put(PLAYER_POSITION_MS, json(&self.player.position_ms));
        put(
            PLAYER_NORMALIZATION_ENABLED,
            json(&self.player.normalization_enabled),
        );
        put(
            PLAYER_NORMALIZATION_STRICT,
            json(&self.player.normalization_strict),
        );
        put(DJ_ENABLED, json(&self.dj.enabled));
        put(
            DJ_SHOW_ENTRIES_IN_QUEUE,
            json(&self.dj.show_entries_in_queue),
        );
        put(DJ_VOICE_ID, json(&self.dj.voice_id));
        put(DJ_VOICE_LEVEL, json(&self.dj.voice_level));
        put(DJ_DEBUG_LOGGING, json(&self.dj.debug_logging));
        put(SYNC_SERVER_TARGET, json(&self.sync.server_target));
        put(SYNC_CLIENT_ID, json(&self.sync.client_id));
        put(SYNC_DOMAINS, json(&self.sync.domains));
        put(
            SPOTIFY_PREFERRED_DEVICE,
            json(&self.spotify.preferred_device),
        );
        put(UI_CLOSE_TO_TRAY, json(&self.ui.close_to_tray));
        put(UI_WINDOW, json(&self.ui.window));
        put(
            PROVIDERS_JELLYFIN_PAGE_SIZE,
            json(&self.providers.jellyfin_page_size),
        );
        put(
            PROVIDERS_PLEX_PAGE_SIZE,
            json(&self.providers.plex_page_size),
        );
        put(
            PROVIDERS_JELLYFIN_USER_ID,
            json(&self.providers.jellyfin_user_id),
        );
        for (table, sort) in &self.ui.track_sort {
            m.insert(format!("{UI_TRACK_SORT_PREFIX}{table}"), json(sort));
        }
        m
    }

    /// Applies one stored entry. Unknown keys and unreadable values are ignored.
    pub fn apply_entry(&mut self, key: &str, value: Value) {
        use keys::{
            DJ_DEBUG_LOGGING, DJ_ENABLED, DJ_SHOW_ENTRIES_IN_QUEUE, DJ_VOICE_ID, DJ_VOICE_LEVEL,
            MIGRATION_PREFIX, PLAYER_NORMALIZATION_ENABLED, PLAYER_NORMALIZATION_STRICT,
            PLAYER_POSITION_MS, PLAYER_REPEAT, PLAYER_SHUFFLE, PLAYER_VOLUME,
            PROVIDERS_JELLYFIN_PAGE_SIZE, PROVIDERS_JELLYFIN_USER_ID, PROVIDERS_PLEX_PAGE_SIZE,
            SPOTIFY_PREFERRED_DEVICE, SYNC_CLIENT_ID, SYNC_DOMAINS, SYNC_SERVER_TARGET,
            UI_CLOSE_TO_TRAY, UI_TRACK_SORT_PREFIX, UI_WINDOW,
        };
        match key {
            PLAYER_VOLUME => set(&mut self.player.volume, key, value),
            PLAYER_SHUFFLE => set(&mut self.player.shuffle, key, value),
            PLAYER_REPEAT => set(&mut self.player.repeat, key, value),
            PLAYER_POSITION_MS => set(&mut self.player.position_ms, key, value),
            PLAYER_NORMALIZATION_ENABLED => set(&mut self.player.normalization_enabled, key, value),
            PLAYER_NORMALIZATION_STRICT => set(&mut self.player.normalization_strict, key, value),
            DJ_ENABLED => set(&mut self.dj.enabled, key, value),
            DJ_SHOW_ENTRIES_IN_QUEUE => set(&mut self.dj.show_entries_in_queue, key, value),
            DJ_VOICE_ID => set(&mut self.dj.voice_id, key, value),
            DJ_VOICE_LEVEL => set(&mut self.dj.voice_level, key, value),
            DJ_DEBUG_LOGGING => set(&mut self.dj.debug_logging, key, value),
            SYNC_SERVER_TARGET => set(&mut self.sync.server_target, key, value),
            SYNC_CLIENT_ID => set(&mut self.sync.client_id, key, value),
            SYNC_DOMAINS => set(&mut self.sync.domains, key, value),
            SPOTIFY_PREFERRED_DEVICE => set(&mut self.spotify.preferred_device, key, value),
            UI_CLOSE_TO_TRAY => set(&mut self.ui.close_to_tray, key, value),
            UI_WINDOW => set(&mut self.ui.window, key, value),
            PROVIDERS_JELLYFIN_PAGE_SIZE => set(&mut self.providers.jellyfin_page_size, key, value),
            PROVIDERS_PLEX_PAGE_SIZE => set(&mut self.providers.plex_page_size, key, value),
            PROVIDERS_JELLYFIN_USER_ID => set(&mut self.providers.jellyfin_user_id, key, value),
            _ => {
                if let Some(table) = key.strip_prefix(UI_TRACK_SORT_PREFIX) {
                    match decode::<Option<TrackSort>>(key, value) {
                        Some(Some(sort)) => {
                            self.ui.track_sort.insert(table.to_owned(), sort);
                        }
                        Some(None) => {
                            self.ui.track_sort.remove(table);
                        }
                        None => {}
                    }
                } else if !key.starts_with(MIGRATION_PREFIX) {
                    tracing::debug!(key, "ignoring unknown setting");
                }
            }
        }
    }
}

// ---- raw KV access (usable inside any transaction) ----

/// The stored JSON value of `key`. Unparseable rows read as absent.
pub fn get_json(conn: &Connection, key: &str) -> Result<Option<Value>> {
    let raw: Option<String> = conn
        .prepare_cached("SELECT value FROM settings WHERE key = ?1")?
        .query_row([key], |r| r.get(0))
        .optional()?;
    Ok(raw.and_then(|raw| serde_json::from_str(&raw).ok()))
}

/// Upserts `key`.
pub fn set_json(conn: &Connection, key: &str, value: &Value) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )?
    .execute(params![key, value.to_string()])?;
    Ok(())
}

pub fn delete_key(conn: &Connection, key: &str) -> Result<bool> {
    Ok(conn.execute("DELETE FROM settings WHERE key = ?1", [key])? > 0)
}

/// Typed read of one key.
pub fn get<T: DeserializeOwned>(conn: &Connection, key: &str) -> Result<Option<T>> {
    Ok(get_json(conn, key)?.and_then(|v| decode(key, v)))
}

/// Typed write of one key.
pub fn put<T: Serialize>(conn: &Connection, key: &str, value: &T) -> Result<()> {
    set_json(conn, key, &serde_json::to_value(value)?)
}

/// Whether the one-shot migration `migration.<name>` has completed.
pub fn migration_done(conn: &Connection, name: &str) -> Result<bool> {
    Ok(get::<bool>(conn, &format!("{}{name}", keys::MIGRATION_PREFIX))?.unwrap_or(false))
}

pub fn mark_migration_done(conn: &Connection, name: &str) -> Result<()> {
    put(conn, &format!("{}{name}", keys::MIGRATION_PREFIX), &true)
}

/// Reads the full snapshot (defaults for missing keys, normalized).
pub fn load(conn: &Connection) -> Result<Settings> {
    let mut stmt = conn.prepare_cached("SELECT key, value FROM settings")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut s = Settings::default();
    for (key, raw) in rows {
        match serde_json::from_str::<Value>(&raw) {
            Ok(value) => s.apply_entry(&key, value),
            Err(e) => tracing::warn!(key, error = %e, "ignoring non-JSON setting"),
        }
    }
    s.normalize();
    Ok(s)
}

/// Writes the keys whose value differs between `old` and `new` (removed track sorts are
/// deleted) in one transaction. Returns the number of keys touched.
pub fn write_changes(conn: &Connection, old: &Settings, new: &Settings) -> Result<usize> {
    let before = old.to_entries();
    let after = new.to_entries();
    in_tx(conn, |conn| {
        let mut touched = 0;
        for (key, value) in &after {
            if before.get(key) != Some(value) {
                set_json(conn, key, value)?;
                touched += 1;
            }
        }
        for key in before.keys().filter(|k| !after.contains_key(*k)) {
            delete_key(conn, key)?;
            touched += 1;
        }
        Ok(touched)
    })
}

fn new_client_id() -> String {
    format!("desktop-{}", uuid::Uuid::new_v4())
}

/// The settings service: one in-memory snapshot, persisted on every change.
pub struct SettingsStore {
    db: Db,
    state: watch::Sender<Settings>,
    write: Mutex<()>,
}

impl std::fmt::Debug for SettingsStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsStore")
            .field("settings", &*self.state.borrow())
            .finish_non_exhaustive()
    }
}

impl SettingsStore {
    /// Loads the snapshot. A missing `sync.client_id` is generated and persisted.
    pub async fn open(db: Db) -> Result<Self> {
        let settings = db
            .call(|conn| {
                let mut s = load(conn)?;
                if s.sync.client_id.trim().is_empty() {
                    s.sync.client_id = new_client_id();
                    put(conn, keys::SYNC_CLIENT_ID, &s.sync.client_id)?;
                }
                Ok(s)
            })
            .await?;
        Ok(Self {
            db,
            state: watch::Sender::new(settings),
            write: Mutex::new(()),
        })
    }

    /// The current snapshot.
    pub fn get(&self) -> Settings {
        self.state.borrow().clone()
    }

    /// Observes every committed change.
    pub fn subscribe(&self) -> watch::Receiver<Settings> {
        self.state.subscribe()
    }

    /// Applies `f`, normalizes, persists the changed keys and publishes the new snapshot.
    /// Updates are serialized; a failed write publishes nothing.
    pub async fn update(&self, f: impl FnOnce(&mut Settings)) -> Result<Settings> {
        let _guard = self.write.lock().await;
        let old = self.get();
        let mut new = old.clone();
        f(&mut new);
        new.normalize();
        if new == old {
            return Ok(new);
        }
        let (o, n) = (old, new.clone());
        self.db
            .call(move |conn| write_changes(conn, &o, &n))
            .await?;
        self.state.send_replace(new.clone());
        Ok(new)
    }

    /// Re-reads the table (after an out-of-band write such as an import) and publishes it.
    pub async fn reload(&self) -> Result<Settings> {
        let _guard = self.write.lock().await;
        let s = self.db.call(|conn| load(conn)).await?;
        self.state.send_replace(s.clone());
        Ok(s)
    }
}
