//! Offline demo adapters. Mutations are atomic and observable, with injectable failures.
use crate::vm::{
    Device, DjCommand, DjPort, DjSnapshot, LibraryCommand, LibraryPort, LibrarySnapshot,
    PlaybackCommand, PlaybackPort, PlaybackSnapshot, PlaylistCard, ProvidersCommand, ProvidersPort,
    ProvidersSnapshot, Result, SearchPort, SearchRequest, SearchSnapshot, ServiceError, Services,
    SettingsCommand, SettingsPort, SettingsSnapshot, SyncChoice, SyncCommand, SyncPort,
    SyncSnapshot, TrackOccurrence, TransferKind, TransferRequest,
};
use ap_model::{
    ColumnPreferences, Previous, ProviderConnectionProfile, QueueOps, QueueState, RepeatMode,
    SourceType, Track,
};
use async_trait::async_trait;
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::watch;
mod transfer;

/// Offline artwork supplied to the demo through the same loader as provider artwork.
pub struct DemoImageCache;

#[async_trait::async_trait]
impl crate::images::ImageCache for DemoImageCache {
    async fn load(&self, url: &str) -> crate::vm::Result<Vec<u8>> {
        let colors: ([u8; 3], [u8; 3]) = match url {
            "demo-art://0" => ([28, 58, 107], [234, 123, 84]),
            "demo-art://1" => ([44, 24, 84], [238, 160, 84]),
            "demo-art://2" => ([14, 73, 78], [227, 188, 104]),
            _ => return Err(crate::vm::ServiceError("Unknown demo artwork".into())),
        };
        let image = image::RgbaImage::from_fn(192, 192, |x, y| {
            let mix = x + y;
            let channel = |i: usize| {
                u8::try_from(
                    (u32::from(colors.0[i]) * (382 - mix) + u32::from(colors.1[i]) * mix) / 382,
                )
                .expect("interpolated channel fits in u8")
            };
            image::Rgba([channel(0), channel(1), channel(2), 255])
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .map_err(|error| crate::vm::ServiceError(error.to_string()))?;
        Ok(bytes.into_inner())
    }
}

pub struct DemoServices {
    playback: watch::Sender<PlaybackSnapshot>,
    library: watch::Sender<LibrarySnapshot>,
    search: watch::Sender<SearchSnapshot>,
    settings: watch::Sender<SettingsSnapshot>,
    sync: watch::Sender<SyncSnapshot>,
    dj: watch::Sender<DjSnapshot>,
    providers: watch::Sender<ProvidersSnapshot>,
    failure: Mutex<Option<String>>,
    failed_providers: Mutex<HashSet<SourceType>>,
    library_writer: Mutex<()>,
    search_writer: Mutex<()>,
    provider_seed: LibrarySnapshot,
}
impl Default for DemoServices {
    fn default() -> Self {
        Self::new()
    }
}
impl DemoServices {
    pub fn new() -> Self {
        let tracks: Vec<_> = (0..72)
            .map(|i| {
                let source = [SourceType::Jellyfin, SourceType::Spotify, SourceType::Plex][i % 3];
                let mut track = Track::new(
                    format!("demo-{i}"),
                    format!(
                        "{} {}",
                        [
                            "Northern Lights",
                            "Midnight Drive",
                            "First Light",
                            "Open Water"
                        ][i % 4],
                        i / 4 + 1
                    ),
                    ["The Satellites", "Mira", "Coastal Radio"][i % 3],
                    source,
                );
                track.album = Some("After Hours".into());
                track.duration_ms = Some(185_000 + i64::try_from(i).unwrap_or_default() * 1000);
                track.url = Some(format!("https://demo.invalid/audio/{i}.flac"));
                track.image_url = Some(format!("demo-art://{}", i % 3));
                track.bitrate_kbps = Some(980);
                track.sample_rate_hz = Some(44_100);
                track
            })
            .collect();
        let queue = QueueOps::replace(
            &QueueState::default(),
            tracks[..8].to_vec(),
            0,
            &mut rand::rng(),
            false,
        );
        let mut library = LibrarySnapshot::default();
        for (i, source) in [
            SourceType::Custom,
            SourceType::Spotify,
            SourceType::Jellyfin,
            SourceType::Plex,
        ]
        .into_iter()
        .enumerate()
        {
            let id = format!("playlist-{i}");
            library.catalog.push(PlaylistCard {
                id: id.clone(),
                name: [
                    "Late Night Favorites",
                    "Discover Weekly",
                    "Lossless Library",
                    "Road Trip",
                ][i]
                    .into(),
                description: "A selection for your next listening session".into(),
                owner: if i == 0 { "You" } else { "Any Player" }.into(),
                source,
                union: false,
                distinct: false,
                count: tracks.len(),
            });
            library.tracks.insert(
                id,
                tracks
                    .iter()
                    .enumerate()
                    .map(|(row, track)| TrackOccurrence {
                        key: format!("row-{i}-{row}"),
                        track: track.clone(),
                    })
                    .collect(),
            );
        }
        library.catalog.push(PlaylistCard {
            id: "union-1".into(),
            name: "All My Music".into(),
            description: "Combined favorites".into(),
            owner: "You".into(),
            source: SourceType::Custom,
            union: true,
            distinct: true,
            count: 144,
        });
        library.sources.insert(
            "union-1".into(),
            vec!["playlist-0".into(), "playlist-2".into()],
        );
        Self {
            provider_seed: library.clone(),
            playback: watch::channel(PlaybackSnapshot {
                queue,
                playing: false,
                position_ms: 0,
                at: Instant::now(),
                volume: 80,
                disabled_reason: String::new(),
                device: "This computer (demo)".into(),
            })
            .0,
            library: watch::channel(library).0,
            search: watch::channel(SearchSnapshot::default()).0,
            settings: watch::channel(SettingsSnapshot {
                normalize: true,
                output_device: "System default".into(),
                output_devices: vec!["System default".into(), "Demo headphones".into()],
                close_to_tray: false,
                tray_available: false,
                column_widths: BTreeMap::new(),
                columns: ["title", "artist", "album", "duration", "source"]
                    .iter()
                    .enumerate()
                    .map(|(i, key)| ColumnPreferences {
                        key: (*key).into(),
                        visible: true,
                        position: i32::try_from(i).unwrap_or_default(),
                    })
                    .collect(),
                sorts: BTreeMap::new(),
            })
            .0,
            sync: watch::channel(SyncSnapshot {
                domains: [true; 4],
                status: "Not connected (demo)".into(),
                ..SyncSnapshot::default()
            })
            .0,
            dj: watch::channel(DjSnapshot {
                enabled: false,
                show_queue: true,
                level: 1.0,
                voices: vec!["Aurora".into(), "River".into()],
                voice: "Aurora".into(),
                status: "Not started".into(),
                songs_away: 3,
                debug: false,
                logs: vec!["Demo DJ initialized".into()],
            })
            .0,
            providers: watch::channel(ProvidersSnapshot {
                profiles: [SourceType::Spotify, SourceType::Jellyfin, SourceType::Plex]
                    .into_iter()
                    .map(|source| ProviderConnectionProfile {
                        source,
                        connected: true,
                        server_url: Some("https://demo.invalid".into()),
                        username: Some("Demo listener".into()),
                        has_token: true,
                        is_premium: Some(true),
                        playback_ready: Some(true),
                        last_error: None,
                    })
                    .collect(),
                devices: vec![
                    Device {
                        id: "computer".into(),
                        name: "This computer".into(),
                    },
                    Device {
                        id: "speaker".into(),
                        name: "Living room".into(),
                    },
                ],
                preferred: "computer".into(),
                ..ProvidersSnapshot::default()
            })
            .0,
            failure: Mutex::new(None),
            failed_providers: Mutex::new(HashSet::new()),
            library_writer: Mutex::new(()),
            search_writer: Mutex::new(()),
        }
    }
    pub fn ports(self: &Arc<Self>) -> Services {
        Services {
            playback: self.clone(),
            library: self.clone(),
            search: self.clone(),
            settings: self.clone(),
            sync: self.clone(),
            dj: self.clone(),
            providers: self.clone(),
        }
    }
    pub fn fail_next(&self, message: impl Into<String>) {
        *self.failure.lock().expect("failure lock") = Some(message.into());
    }
    pub fn fail_provider(&self, source: SourceType) {
        self.failed_providers
            .lock()
            .expect("provider lock")
            .insert(source);
    }
    fn check(&self) -> Result {
        self.failure
            .lock()
            .expect("failure lock")
            .take()
            .map_or(Ok(()), |m| Err(ServiceError(m)))
    }
    /// Demo progression advances only at real boundaries; position is extrapolated by the UI.
    pub fn start_progression(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(std::time::Duration::from_millis(250));
            loop {
                timer.tick().await;
                let Some(service) = weak.upgrade() else { break };
                let state = service.playback.borrow().clone();
                if state.playing
                    && state
                        .queue
                        .current()
                        .and_then(|e| e.track.duration_ms)
                        .is_some_and(|d| state.position() >= d)
                {
                    if state.queue.repeat == RepeatMode::One {
                        let _ = service.playback(PlaybackCommand::Seek(0)).await;
                    } else {
                        let _ = service.playback(PlaybackCommand::Next).await;
                    }
                }
            }
        })
    }
}
#[async_trait]
impl PlaybackPort for DemoServices {
    fn subscribe(&self) -> watch::Receiver<PlaybackSnapshot> {
        self.playback.subscribe()
    }
    async fn playback(&self, command: PlaybackCommand) -> Result {
        self.check()?;
        self.playback.send_modify(|s| {
            s.position_ms = s.position();
            s.at = Instant::now();
            match command {
                PlaybackCommand::Toggle => s.playing = !s.playing,
                PlaybackCommand::Next => {
                    if let Some(queue) = QueueOps::advance(&s.queue, true) {
                        s.queue = queue;
                        s.position_ms = 0;
                    } else {
                        s.playing = false;
                    }
                }
                PlaybackCommand::Previous => {
                    if let Previous::MoveTo(pos) = QueueOps::previous(&s.queue, s.position_ms) {
                        s.queue = QueueOps::move_cursor(
                            &s.queue,
                            isize::try_from(pos).unwrap_or_default(),
                        );
                    }
                    s.position_ms = 0;
                }
                PlaybackCommand::Jump(uid) => {
                    s.queue = QueueOps::jump_to_uid(&s.queue, uid);
                    s.position_ms = 0;
                    s.playing = true;
                }
                PlaybackCommand::Seek(ms) => {
                    s.position_ms = ms.max(0).min(
                        s.queue
                            .current()
                            .and_then(|e| e.track.duration_ms)
                            .unwrap_or(i64::MAX),
                    );
                }
                PlaybackCommand::Volume(volume) => s.volume = volume.min(100),
                PlaybackCommand::Shuffle => {
                    s.queue = QueueOps::set_shuffle(&s.queue, !s.queue.shuffle, &mut rand::rng());
                }
                PlaybackCommand::Repeat => {
                    s.queue = QueueOps::set_repeat(
                        &s.queue,
                        match s.queue.repeat {
                            RepeatMode::Off => RepeatMode::All,
                            RepeatMode::All => RepeatMode::One,
                            RepeatMode::One => RepeatMode::Off,
                        },
                    );
                }
                PlaybackCommand::PlayNext(track) => s.queue = QueueOps::play_next(&s.queue, track),
                PlaybackCommand::SetQueue { tracks, start } => {
                    s.queue = QueueOps::replace(
                        &s.queue,
                        tracks,
                        isize::try_from(start).unwrap_or_default(),
                        &mut rand::rng(),
                        false,
                    );
                    s.position_ms = 0;
                    s.playing = true;
                }
            }
        });
        Ok(())
    }
}
#[async_trait]
impl LibraryPort for DemoServices {
    fn subscribe(&self) -> watch::Receiver<LibrarySnapshot> {
        self.library.subscribe()
    }
    async fn library(&self, command: LibraryCommand) -> Result {
        self.check()?;
        let _writer = self.library_writer.lock().expect("library writer");
        let mut state = self.library.borrow().clone();
        match command {
            LibraryCommand::Open(_) => {}
            LibraryCommand::Play { id, key } => {
                let rows = state.materialize(&id);
                let distinct = state
                    .catalog
                    .iter()
                    .find(|p| p.id == id)
                    .is_some_and(|p| p.distinct);
                let selected = key
                    .as_deref()
                    .or_else(|| rows.first().map(|r| r.key.as_str()));
                let (tracks, start) = selected
                    .and_then(|key| crate::vm::library::playback_selection(&rows, key, distinct))
                    .unwrap_or_default();
                self.playback.send_modify(|s| {
                    s.queue = QueueOps::replace(
                        &s.queue,
                        tracks,
                        isize::try_from(start).unwrap_or_default(),
                        &mut rand::rng(),
                        false,
                    );
                    s.position_ms = 0;
                    s.playing = !s.queue.entries.is_empty();
                });
            }
            LibraryCommand::PlayNext(track) => self.playback.send_modify(|s| {
                s.queue = QueueOps::play_next(&s.queue, track);
            }),
            LibraryCommand::Refresh => {
                for playlist in self
                    .provider_seed
                    .catalog
                    .iter()
                    .filter(|p| p.source != SourceType::Custom)
                {
                    if !state.catalog.iter().any(|p| p.id == playlist.id) {
                        state.catalog.push(playlist.clone());
                    }
                    if let Some(tracks) = self.provider_seed.tracks.get(&playlist.id) {
                        state.tracks.insert(playlist.id.clone(), tracks.clone());
                    }
                }
            }
            LibraryCommand::ClearCache => {
                let providers: HashSet<_> = state
                    .catalog
                    .iter()
                    .filter(|p| p.source != SourceType::Custom)
                    .map(|p| p.id.clone())
                    .collect();
                state.catalog.retain(|p| p.source == SourceType::Custom);
                state.tracks.retain(|id, _| !providers.contains(id));
            }
            LibraryCommand::Create { name, union } => {
                if name.trim().is_empty() {
                    return Err(ServiceError("Playlist name is required".into()));
                }
                let id = format!("custom-{}", state.revision + 1);
                state.catalog.push(PlaylistCard {
                    id: id.clone(),
                    name: name.trim().into(),
                    description: String::new(),
                    owner: "You".into(),
                    source: SourceType::Custom,
                    union,
                    distinct: false,
                    count: 0,
                });
                if union {
                    state.sources.insert(id, Vec::new());
                } else {
                    state.tracks.insert(id, Vec::new());
                }
            }
            LibraryCommand::Update {
                id,
                name,
                description,
            } => {
                let p = state
                    .catalog
                    .iter_mut()
                    .find(|p| p.id == id)
                    .ok_or_else(|| ServiceError("Playlist not found".into()))?;
                if name.trim().is_empty() {
                    return Err(ServiceError("Playlist name is required".into()));
                }
                p.name = name.trim().into();
                p.description = description;
            }
            LibraryCommand::Delete(id) => {
                state.catalog.retain(|p| p.id != id);
                state.tracks.remove(&id);
                state.sources.remove(&id);
                for sources in state.sources.values_mut() {
                    sources.retain(|s| s != &id);
                }
            }
            LibraryCommand::Distinct { id, enabled } => {
                if let Some(p) = state.catalog.iter_mut().find(|p| p.id == id) {
                    p.distinct = enabled;
                }
            }
            LibraryCommand::Add { id, track } => {
                if state.sources.contains_key(&id) {
                    return Err(ServiceError("Choose a STANDARD playlist".into()));
                }
                let key = format!("row-{}", state.revision);
                state
                    .tracks
                    .entry(id)
                    .or_default()
                    .push(TrackOccurrence { key, track });
            }
            LibraryCommand::Remove { id, key } => {
                if let Some(rows) = state.tracks.get_mut(&id) {
                    rows.retain(|r| r.key != key);
                }
            }
            LibraryCommand::Move { id, key, position } => {
                if let Some(rows) = state.tracks.get_mut(&id)
                    && let Some(index) = rows.iter().position(|r| r.key == key)
                {
                    let row = rows.remove(index);
                    rows.insert(position.min(rows.len()), row);
                }
            }
            LibraryCommand::AddSource { id, source } => {
                if !state.union_candidates(&id).iter().any(|p| p.id == source) {
                    return Err(ServiceError("Invalid union source".into()));
                }
                let sources = state.sources.entry(id).or_default();
                if !sources.contains(&source) {
                    sources.push(source);
                }
            }
            LibraryCommand::RemoveSource { id, source } => {
                if let Some(sources) = state.sources.get_mut(&id) {
                    sources.retain(|s| s != &source);
                }
            }
            LibraryCommand::MoveSource {
                id,
                source,
                position,
            } => {
                if let Some(sources) = state.sources.get_mut(&id)
                    && let Some(index) = sources.iter().position(|s| s == &source)
                {
                    let item = sources.remove(index);
                    sources.insert(position.min(sources.len()), item);
                }
            }
        }
        for index in 0..state.catalog.len() {
            let id = state.catalog[index].id.clone();
            state.catalog[index].count = state.materialize(&id).len();
        }
        state.revision += 1;
        self.library.send_replace(state);
        Ok(())
    }
}
#[async_trait]
impl SearchPort for DemoServices {
    fn subscribe(&self) -> watch::Receiver<SearchSnapshot> {
        self.search.subscribe()
    }
    async fn search(&self, request: SearchRequest) -> Result {
        self.check()?;
        tokio::task::yield_now().await;
        let _writer = self.search_writer.lock().expect("search writer");
        if self.search.borrow().generation > request.generation {
            return Ok(());
        }
        let failed = self.failed_providers.lock().expect("failed providers");
        let state = self.library.borrow();
        let query = request.query.trim().to_lowercase();
        let mut seen = HashSet::new();
        let tracks: Vec<_> = state
            .tracks
            .values()
            .flatten()
            .filter(|r| {
                (request.source == SourceType::All || r.track.source == request.source)
                    && !failed.contains(&r.track.source)
                    && format!("{} {}", r.track.title, r.track.artist)
                        .to_lowercase()
                        .contains(&query)
                    && seen.insert((r.track.source, r.track.id.clone()))
            })
            .map(|r| r.track.clone())
            .collect();
        let playlists: Vec<_> = state
            .catalog
            .iter()
            .filter(|p| {
                p.source != SourceType::Custom
                    && (request.source == SourceType::All || p.source == request.source)
                    && !failed.contains(&p.source)
                    && p.name.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
        let all_failed = if request.source == SourceType::All {
            [SourceType::Spotify, SourceType::Plex, SourceType::Jellyfin]
                .iter()
                .all(|s| failed.contains(s))
        } else {
            failed.contains(&request.source)
        };
        let total = if request.playlists {
            playlists.len()
        } else {
            tracks.len()
        };
        self.search.send_replace(SearchSnapshot {
            tracks: tracks
                .into_iter()
                .skip(request.page * 20)
                .take(20)
                .collect(),
            playlists: playlists
                .into_iter()
                .skip(request.page * 20)
                .take(20)
                .collect(),
            total,
            page: request.page,
            generation: request.generation,
            error: if all_failed {
                "Search failed".into()
            } else {
                String::new()
            },
            warnings: failed
                .iter()
                .map(|s| format!("{s:?} unavailable"))
                .collect(),
        });
        Ok(())
    }
}
#[async_trait]
impl SettingsPort for DemoServices {
    fn subscribe(&self) -> watch::Receiver<SettingsSnapshot> {
        self.settings.subscribe()
    }
    async fn settings(&self, command: SettingsCommand) -> Result {
        self.check()?;
        self.settings.send_modify(|s| match command {
            SettingsCommand::Normalize(v) => s.normalize = v,
            SettingsCommand::Output(v) => s.output_device = v,
            SettingsCommand::CloseToTray(v) => s.close_to_tray = v && s.tray_available,
            SettingsCommand::Columns(v) => s.columns = v,
            SettingsCommand::Sort { key, preference } => {
                s.sorts.insert(key, preference);
            }
        });
        Ok(())
    }
    async fn transfer(&self, request: TransferRequest) -> Result<String> {
        self.check()?;
        match request.kind {
            TransferKind::ExportState | TransferKind::ExportConfig => {
                let library = self.library.borrow().clone();
                let playback = self.playback.borrow().clone();
                let columns = self.settings.borrow().columns.clone();
                let kind = request.kind.clone();
                let private = request.private;
                let passphrase = request.passphrase.clone();
                let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
                    if matches!(kind, TransferKind::ExportConfig) {
                        return serde_json::to_vec_pretty(&transfer::config(&library))
                            .map_err(|e| ServiceError(e.to_string()));
                    }
                    let mut envelope = transfer::state(&library);
                    envelope.data.column_preferences =
                        columns.into_iter().map(|c| (c.key.clone(), c)).collect();
                    envelope.data.playback_state = Some(ap_model::PlaybackStatus {
                        state: if playback.playing {
                            ap_model::PlaybackStateType::Playing
                        } else {
                            ap_model::PlaybackStateType::Paused
                        },
                        shuffle: playback.queue.shuffle,
                        repeat_mode: playback.queue.repeat,
                        volume: i32::from(playback.volume),
                        current_track: playback.queue.current().map(|e| e.track.clone()),
                        position: playback.position(),
                        duration: playback
                            .queue
                            .current()
                            .and_then(|e| e.track.duration_ms)
                            .unwrap_or_default(),
                        queue: playback
                            .queue
                            .entries
                            .iter()
                            .map(|e| e.track.clone())
                            .collect(),
                        ordered_queue: playback.queue.play_order().cloned().collect(),
                        error_message: None,
                    });
                    envelope.integrity.sha256 = envelope.compute_integrity();
                    let text = envelope.to_json();
                    if private {
                        if passphrase.is_empty() {
                            return Err(ServiceError(
                                "Passphrase required for PRIVATE export".into(),
                            ));
                        }
                        let encrypted = ap_model::transfer_crypto::encrypt(
                            &text,
                            &passphrase,
                            ap_model::transfer_crypto::DEFAULT_KDF_ITERATIONS,
                            &mut rand::rng(),
                        )
                        .map_err(|e| ServiceError(e.to_string()))?;
                        serde_json::to_vec(&encrypted).map_err(|e| ServiceError(e.to_string()))
                    } else {
                        Ok(text.into_bytes())
                    }
                })
                .await
                .map_err(|e| ServiceError(e.to_string()))??;
                tokio::fs::write(&request.path, bytes)
                    .await
                    .map_err(|e| ServiceError(e.to_string()))?;
                Ok(format!("Saved to: {}", request.path.display()))
            }
            TransferKind::ImportState | TransferKind::ImportConfig => {
                let bytes = tokio::fs::read_to_string(&request.path)
                    .await
                    .map_err(|e| ServiceError(e.to_string()))?;
                let kind = request.kind.clone();
                let passphrase = request.passphrase.clone();
                let (incoming, playback, columns) =
                    tokio::task::spawn_blocking(move || -> Result<_> {
                        if matches!(kind, TransferKind::ImportConfig) {
                            let config = serde_json::from_str(&bytes)
                                .map_err(|e| ServiceError(e.to_string()))?;
                            return Ok((transfer::from_config(config)?, None, None));
                        }
                        let envelope = ap_model::wire::decode_and_verify(&bytes, Some(&passphrase))
                            .map_err(|e| ServiceError(e.to_string()))?;
                        let playback = envelope.data.playback_state.clone();
                        let columns = Some(
                            envelope
                                .data
                                .column_preferences
                                .values()
                                .cloned()
                                .collect::<Vec<_>>(),
                        );
                        Ok((transfer::library(envelope.data), playback, columns))
                    })
                    .await
                    .map_err(|e| ServiceError(e.to_string()))??;
                let count = incoming.catalog.len();
                if !request.dry_run {
                    let _writer = self.library_writer.lock().expect("library writer");
                    if let Some(playback) = playback {
                        self.playback.send_modify(|s| {
                            let tracks = playback.queue;
                            let start = playback
                                .current_track
                                .as_ref()
                                .and_then(|t| tracks.iter().position(|q| q.same_track(t)))
                                .unwrap_or_default();
                            let base = QueueState {
                                shuffle: playback.shuffle,
                                repeat: playback.repeat_mode,
                                ..QueueState::default()
                            };
                            s.queue = QueueOps::replace(
                                &base,
                                tracks,
                                isize::try_from(start).unwrap_or_default(),
                                &mut rand::rng(),
                                false,
                            );
                            s.playing = false;
                            s.position_ms = playback.position;
                            s.at = Instant::now();
                            s.volume = u8::try_from(playback.volume.clamp(0, 100)).unwrap_or(100);
                        });
                    }
                    if let Some(columns) = columns {
                        self.settings.send_modify(|s| s.columns = columns);
                    }
                    self.library.send_modify(|s| {
                        if request.replace {
                            let custom: HashSet<_> = s
                                .catalog
                                .iter()
                                .filter(|p| p.source == SourceType::Custom)
                                .map(|p| p.id.clone())
                                .collect();
                            s.catalog.retain(|p| p.source != SourceType::Custom);
                            s.tracks.retain(|id, _| !custom.contains(id));
                            s.sources.retain(|id, _| !custom.contains(id));
                        }
                        for playlist in incoming.catalog {
                            if let Some(current) =
                                s.catalog.iter_mut().find(|p| p.id == playlist.id)
                            {
                                *current = playlist;
                            } else {
                                s.catalog.push(playlist);
                            }
                        }
                        s.tracks.extend(incoming.tracks);
                        s.sources.extend(incoming.sources);
                        s.revision += 1;
                    });
                }
                Ok(format!(
                    "{}: {count} playlists",
                    if request.dry_run {
                        "Dry run"
                    } else {
                        "Imported"
                    }
                ))
            }
        }
    }
}
#[async_trait]
impl SyncPort for DemoServices {
    fn subscribe(&self) -> watch::Receiver<SyncSnapshot> {
        self.sync.subscribe()
    }
    async fn sync(&self, command: SyncCommand) -> Result {
        self.check()?;
        self.sync.send_modify(|s| match command {
            SyncCommand::Commit { url, token } => {
                let url = url.trim().trim_end_matches('/');
                s.url = if url.is_empty() || url.contains("://") {
                    url.into()
                } else {
                    format!("https://{url}")
                };
                s.token = token
                    .trim()
                    .strip_prefix("Bearer ")
                    .unwrap_or(token.trim())
                    .trim()
                    .into();
            }
            SyncCommand::Domain { index, enabled } => {
                if let Some(domain) = s.domains.get_mut(index) {
                    *domain = enabled;
                }
            }
            SyncCommand::Connect => {
                s.conflict = true;
                s.status = "Server already has synced data - choose which side to keep.".into();
            }
            SyncCommand::Pull => {
                s.overwrite = true;
                s.status = "Confirm playlist overwrite".into();
            }
            SyncCommand::Resolve(choice) => {
                s.conflict = false;
                s.overwrite = false;
                s.connected = !matches!(choice, SyncChoice::Cancel);
                s.status = match choice {
                    SyncChoice::Server => "Demo: server data selected",
                    SyncChoice::Device => "Demo: keeping this device's playlists",
                    SyncChoice::Cancel => "Cancelled",
                }
                .into();
            }
        });
        Ok(())
    }
}
#[async_trait]
impl DjPort for DemoServices {
    fn subscribe(&self) -> watch::Receiver<DjSnapshot> {
        self.dj.subscribe()
    }
    async fn dj(&self, command: DjCommand) -> Result {
        self.check()?;
        self.dj.send_modify(|s| {
            match command {
                DjCommand::Enable(v) => s.enabled = v,
                DjCommand::ShowQueue(v) => s.show_queue = v,
                DjCommand::Level(v) => s.level = v.clamp(0.5, 2.0),
                DjCommand::Voice(v) => s.voice = v,
                DjCommand::Debug(v) => s.debug = v,
                DjCommand::ClearLogs => s.logs.clear(),
            }
            s.status = if s.enabled {
                "Ready for playback"
            } else {
                "Not started"
            }
            .into();
            if s.debug {
                s.logs.push(format!("DJ settings updated: {}", s.status));
                if s.logs.len() > 200 {
                    s.logs.remove(0);
                }
            }
        });
        Ok(())
    }
}
#[async_trait]
impl ProvidersPort for DemoServices {
    fn subscribe(&self) -> watch::Receiver<ProvidersSnapshot> {
        self.providers.subscribe()
    }
    async fn providers(&self, command: ProvidersCommand) -> Result {
        self.check()?;
        if let ProvidersCommand::Connect(form) = &command
            && form.source != SourceType::Spotify
            && (form.url.trim().is_empty() || form.secret.trim().is_empty())
        {
            return Err(ServiceError(
                if form.source == SourceType::Plex {
                    "Please enter both URL and token"
                } else {
                    "Please enter both URL and API key"
                }
                .into(),
            ));
        }
        self.providers.send_modify(|s| match command {
            ProvidersCommand::Connect(form) => {
                if let Some(p) = s.profiles.iter_mut().find(|p| p.source == form.source) {
                    p.connected = true;
                    p.server_url = Some(form.url);
                    p.has_token = true;
                    p.last_error = None;
                }
            }
            ProvidersCommand::Disconnect(source) => {
                if let Some(p) = s.profiles.iter_mut().find(|p| p.source == source) {
                    p.connected = false;
                    p.has_token = false;
                }
            }
            ProvidersCommand::Preferred(id) => s.preferred = id,
            ProvidersCommand::CancelLogin => s.login_url.clear(),
        });
        Ok(())
    }
}
