//! UI wiring contracts. Services own state; each command returns only after acceptance.
//! Receivers publish complete snapshots. No Slint types cross a service boundary.
use ap_model::{ColumnPreferences, ProviderConnectionProfile, QueueState, SourceType, Track};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc, time::Instant};
use tokio::sync::watch;

pub mod dj;
pub mod library;
pub mod playback;
pub mod providers;
pub mod search;
pub mod settings;
pub mod sync;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ServiceError(pub String);
pub type Result<T = ()> = std::result::Result<T, ServiceError>;

#[derive(Clone, Debug)]
pub struct PlaybackSnapshot {
    pub queue: QueueState,
    pub playing: bool,
    pub buffering: bool,
    pub error: String,
    pub position_ms: i64,
    pub at: Instant,
    pub volume: u8,
    pub disabled_reason: String,
    pub device: String,
}
impl PlaybackSnapshot {
    pub fn position(&self) -> i64 {
        let elapsed = if self.playing {
            i64::try_from(self.at.elapsed().as_millis()).unwrap_or(i64::MAX)
        } else {
            0
        };
        (self.position_ms.saturating_add(elapsed)).min(
            self.queue
                .current()
                .and_then(|e| e.track.duration_ms)
                .unwrap_or(i64::MAX),
        )
    }
}
#[derive(Clone, Debug)]
pub enum PlaybackCommand {
    DismissError,
    Toggle,
    Next,
    Previous,
    Jump(u64),
    Seek(i64),
    Volume(u8),
    Shuffle,
    Repeat,
    PlayNext(Track),
    SetQueue { tracks: Vec<Track>, start: usize },
}
#[async_trait]
pub trait PlaybackPort: Send + Sync {
    fn subscribe(&self) -> watch::Receiver<PlaybackSnapshot>;
    fn errors(&self) -> tokio::sync::broadcast::Receiver<String>;
    async fn playback(&self, command: PlaybackCommand) -> Result;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaylistCard {
    pub id: String,
    pub name: String,
    pub description: String,
    pub owner: String,
    pub source: SourceType,
    pub union: bool,
    pub distinct: bool,
    pub count: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackOccurrence {
    pub key: String,
    pub track: Track,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LibrarySnapshot {
    pub catalog: Vec<PlaylistCard>,
    pub tracks: BTreeMap<String, Vec<TrackOccurrence>>,
    pub sources: BTreeMap<String, Vec<String>>,
    pub revision: u64,
}
impl LibrarySnapshot {
    pub fn union_candidates(&self, id: &str) -> Vec<&PlaylistCard> {
        self.catalog
            .iter()
            .filter(|p| p.id != id && !p.union)
            .collect()
    }
    pub fn materialize(&self, id: &str) -> Vec<TrackOccurrence> {
        if let Some(rows) = self.tracks.get(id) {
            return rows.clone();
        }
        if let Some(sources) = self.sources.get(id) {
            sources
                .iter()
                .flat_map(|source| {
                    self.tracks
                        .get(source)
                        .into_iter()
                        .flatten()
                        .map(move |row| TrackOccurrence {
                            key: format!("{source}:{}", row.key),
                            track: row.track.clone(),
                        })
                })
                .collect()
        } else {
            self.tracks.get(id).cloned().unwrap_or_default()
        }
    }
}
#[derive(Clone, Debug)]
pub enum LibraryCommand {
    Refresh,
    Open(String),
    Play {
        id: String,
        key: Option<String>,
    },
    PlayNext(Track),
    Create {
        name: String,
        union: bool,
    },
    Update {
        id: String,
        name: String,
        description: String,
    },
    Delete(String),
    Distinct {
        id: String,
        enabled: bool,
    },
    Add {
        id: String,
        track: Track,
    },
    Remove {
        id: String,
        key: String,
    },
    Move {
        id: String,
        key: String,
        position: usize,
    },
    AddSource {
        id: String,
        source: String,
    },
    RemoveSource {
        id: String,
        source: String,
    },
    MoveSource {
        id: String,
        source: String,
        position: usize,
    },
    ClearCache,
}
#[async_trait]
pub trait LibraryPort: Send + Sync {
    fn subscribe(&self) -> watch::Receiver<LibrarySnapshot>;
    async fn library(&self, command: LibraryCommand) -> Result;
}
#[derive(Clone, Debug)]
pub struct SearchRequest {
    pub query: String,
    pub source: SourceType,
    pub playlists: bool,
    pub page: usize,
    pub generation: u64,
}
#[derive(Clone, Debug, Default)]
pub struct SearchSnapshot {
    pub tracks: Vec<Track>,
    pub playlists: Vec<PlaylistCard>,
    pub total: usize,
    pub page: usize,
    pub generation: u64,
    pub error: String,
    pub warnings: Vec<String>,
}
#[async_trait]
pub trait SearchPort: Send + Sync {
    fn subscribe(&self) -> watch::Receiver<SearchSnapshot>;
    async fn search(&self, request: SearchRequest) -> Result;
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SortPreference {
    pub column: String,
    pub descending: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SettingsSnapshot {
    pub normalize: bool,
    pub output_device: String,
    pub output_devices: Vec<String>,
    pub close_to_tray: bool,
    pub tray_available: bool,
    pub columns: Vec<ColumnPreferences>,
    /// Logical pixel widths. Legacy `duration_ms` is accepted as an alias for `duration`.
    #[serde(default)]
    pub column_widths: BTreeMap<String, f32>,
    pub sorts: BTreeMap<String, SortPreference>,
}
#[derive(Clone, Debug)]
pub enum SettingsCommand {
    Normalize(bool),
    Output(String),
    CloseToTray(bool),
    Columns(Vec<ColumnPreferences>),
    Sort {
        key: String,
        preference: SortPreference,
    },
}
#[derive(Clone, Debug)]
pub enum TransferKind {
    ExportState,
    ImportState,
    ExportConfig,
    ImportConfig,
}
#[derive(Clone, Debug)]
pub struct TransferRequest {
    pub kind: TransferKind,
    pub path: std::path::PathBuf,
    pub private: bool,
    pub passphrase: String,
    pub replace: bool,
    pub dry_run: bool,
}
#[async_trait]
pub trait SettingsPort: Send + Sync {
    fn subscribe(&self) -> watch::Receiver<SettingsSnapshot>;
    async fn settings(&self, command: SettingsCommand) -> Result;
    async fn transfer(&self, request: TransferRequest) -> Result<String>;
}
#[derive(Clone, Debug, Default)]
pub struct SyncSnapshot {
    pub url: String,
    pub token: String,
    pub domains: [bool; 4],
    pub connected: bool,
    pub conflict: bool,
    pub overwrite: bool,
    pub status: String,
    /// Last sync import outcome, shown on the Settings transfer status line.
    pub import_status: String,
}
#[derive(Clone, Debug)]
pub enum SyncChoice {
    Server,
    Device,
    Cancel,
}
#[derive(Clone, Debug)]
pub enum SyncCommand {
    Commit { url: String, token: String },
    Domain { index: usize, enabled: bool },
    Connect,
    Pull,
    Resolve(SyncChoice),
}
#[async_trait]
pub trait SyncPort: Send + Sync {
    fn subscribe(&self) -> watch::Receiver<SyncSnapshot>;
    async fn sync(&self, command: SyncCommand) -> Result;
}
#[derive(Clone, Debug)]
pub struct DjSnapshot {
    pub enabled: bool,
    pub show_queue: bool,
    pub level: f32,
    pub voices: Vec<String>,
    pub voice: String,
    pub status: String,
    pub songs_away: usize,
    pub debug: bool,
    pub logs: Vec<String>,
}
#[derive(Clone, Debug)]
pub enum DjCommand {
    Enable(bool),
    ShowQueue(bool),
    Level(f32),
    Voice(String),
    Debug(bool),
    ClearLogs,
}
#[async_trait]
pub trait DjPort: Send + Sync {
    fn subscribe(&self) -> watch::Receiver<DjSnapshot>;
    async fn dj(&self, command: DjCommand) -> Result;
}
#[derive(Clone, Debug)]
pub struct ProviderForm {
    pub source: SourceType,
    pub url: String,
    pub secret: String,
    pub page_size: u16,
    pub self_signed: bool,
}
#[derive(Clone, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Default)]
pub struct ProvidersSnapshot {
    pub profiles: Vec<ProviderConnectionProfile>,
    pub login_url: String,
    pub devices: Vec<Device>,
    pub preferred: String,
}
#[derive(Clone, Debug)]
pub enum ProvidersCommand {
    Connect(ProviderForm),
    Disconnect(SourceType),
    Preferred(String),
    CancelLogin,
}
#[async_trait]
pub trait ProvidersPort: Send + Sync {
    fn subscribe(&self) -> watch::Receiver<ProvidersSnapshot>;
    async fn providers(&self, command: ProvidersCommand) -> Result;
}

#[derive(Clone, Debug, Default)]
pub struct TrackTable {
    pub filter: String,
    pub sort: SortPreference,
}
impl TrackTable {
    pub fn can_reorder(&self) -> bool {
        self.filter.trim().is_empty() && self.sort.column.is_empty()
    }
    pub fn cycle_sort(&mut self, column: &str) {
        if self.sort.column != column {
            self.sort = SortPreference {
                column: column.into(),
                descending: false,
            };
        } else if !self.sort.descending {
            self.sort.descending = true;
        } else {
            self.sort = SortPreference::default();
        }
    }
    pub fn rows(&self, rows: &[TrackOccurrence]) -> Vec<TrackOccurrence> {
        let query = self.filter.to_lowercase();
        let mut rows: Vec<_> = rows
            .iter()
            .filter(|r| {
                format!(
                    "{} {} {} {:?}",
                    r.track.title,
                    r.track.artist,
                    r.track.album.as_deref().unwrap_or_default(),
                    r.track.source
                )
                .to_lowercase()
                .contains(&query)
            })
            .cloned()
            .collect();
        if !self.sort.column.is_empty() {
            if self.sort.column == "duration" {
                rows.sort_by_key(|r| r.track.duration_ms);
            } else {
                rows.sort_by_cached_key(|r| match self.sort.column.as_str() {
                    "artist" => r.track.artist.to_lowercase(),
                    "album" => r.track.album.as_deref().unwrap_or_default().to_lowercase(),
                    "source" => format!("{:?}", r.track.source),
                    _ => r.track.title.to_lowercase(),
                });
            }
            if self.sort.descending {
                rows.reverse();
            }
        }
        rows
    }
}

pub struct Services {
    pub playback: Arc<dyn PlaybackPort>,
    pub library: Arc<dyn LibraryPort>,
    pub search: Arc<dyn SearchPort>,
    pub settings: Arc<dyn SettingsPort>,
    pub sync: Arc<dyn SyncPort>,
    pub dj: Arc<dyn DjPort>,
    pub providers: Arc<dyn ProvidersPort>,
}
