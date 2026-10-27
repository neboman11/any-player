//! Production library/search ports. UI observations never fetch; Open and Play explicitly do.
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex as StdMutex},
};

use ap_library::{CatalogRepository, PlaylistRepository, SearchResult, SearchType};
use ap_model::{Playlist, PlaylistType, SourceType, Track, dedupe::DuplicateGroup};
use ap_playback::{ChangeOrigin, Engine};
use async_trait::async_trait;
use tokio::sync::{Mutex, watch};

use crate::vm::{
    LibraryCommand, LibraryPort, LibrarySnapshot, PlaylistCard, Result, SearchPort, SearchRequest,
    SearchSnapshot, ServiceError, TrackOccurrence,
};

#[derive(Clone)]
pub struct LibraryAdapter {
    inner: Arc<LibraryInner>,
}
struct LibraryInner {
    catalog: CatalogRepository,
    playlists: PlaylistRepository,
    engine: Engine,
    state: watch::Sender<LibrarySnapshot>,
    rebuild: Mutex<()>,
}

impl LibraryAdapter {
    pub async fn new(
        catalog: CatalogRepository,
        playlists: PlaylistRepository,
        engine: Engine,
    ) -> Result<Self> {
        let mut listings = catalog.observe_playlists();
        let mut provider_tracks = catalog.subscribe();
        let mut custom = playlists.subscribe();
        let (state, _) = watch::channel(LibrarySnapshot::default());
        let this = Self {
            inner: Arc::new(LibraryInner {
                catalog,
                playlists,
                engine,
                state,
                rebuild: Mutex::new(()),
            }),
        };
        this.rebuild().await?;
        let weak = Arc::downgrade(&this.inner);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = listings.changed() => if changed.is_err() { break; },
                    changed = provider_tracks.recv() => if matches!(changed, Err(tokio::sync::broadcast::error::RecvError::Closed)) { break; },
                    changed = custom.recv() => if matches!(changed, Err(tokio::sync::broadcast::error::RecvError::Closed)) { break; },
                }
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                if let Err(error) = (Self { inner }).rebuild().await {
                    tracing::warn!(%error, "failed to rebuild library snapshot");
                }
            }
        });
        Ok(this)
    }

    /// Provider ids include their source, so Plex/Jellyfin playlist ids cannot collide in UI maps.
    pub fn provider_id(source: SourceType, id: &str) -> String {
        format!("p:{}:{id}", source.name())
    }

    async fn rebuild(&self) -> Result {
        let _guard = self.inner.rebuild.lock().await;
        let providers = self
            .inner
            .catalog
            .cached_playlists()
            .await
            .map_err(service_error)?;
        let custom = self.inner.playlists.list().await.map_err(service_error)?;
        let mut custom_rows = Vec::with_capacity(custom.len());
        for playlist in custom {
            let saved = if playlist.playlist_type == PlaylistType::Standard {
                self.inner
                    .playlists
                    .track_rows(&playlist.id)
                    .await
                    .map_err(service_error)?
            } else {
                vec![]
            };
            let sources = if playlist.playlist_type == PlaylistType::Union {
                self.inner
                    .playlists
                    .union_sources(&playlist.id)
                    .await
                    .map_err(service_error)?
            } else {
                vec![]
            };
            let tracks = self
                .inner
                .playlists
                .cached_tracks(&playlist.id)
                .await
                .map_err(service_error)?;
            custom_rows.push(CustomRows {
                playlist,
                saved,
                sources,
                tracks,
            });
        }
        let mut provider_rows = Vec::with_capacity(providers.len());
        for playlist in providers {
            let distinct = self
                .inner
                .catalog
                .is_distinct(playlist.source, &playlist.id)
                .await
                .map_err(service_error)?;
            let tracks = self
                .inner
                .catalog
                .cached_tracks(playlist.source, &playlist.id)
                .await
                .map_err(service_error)?;
            provider_rows.push((playlist, distinct, tracks));
        }
        let revision = self.inner.state.borrow().revision.wrapping_add(1);
        // Snapshot ownership and large row transforms stay off the executor's cooperative workers.
        let snapshot = tokio::task::spawn_blocking(move || {
            build_snapshot(custom_rows, provider_rows, revision)
        })
        .await
        .map_err(service_error)?;
        self.inner.state.send_replace(snapshot);
        Ok(())
    }

    pub async fn open(&self, id: &str) -> Result {
        if let Some((source, provider_id)) = provider_ref(id) {
            self.inner
                .catalog
                .tracks(source, provider_id, false)
                .await
                .map_err(service_error)?;
        } else {
            let playlist = self
                .inner
                .playlists
                .get(id)
                .await
                .map_err(service_error)?
                .ok_or_else(|| ServiceError("Playlist no longer exists".into()))?;
            if playlist.playlist_type == PlaylistType::Union {
                self.inner
                    .playlists
                    .materialize_union(id, false)
                    .await
                    .map_err(service_error)?;
            }
        }
        self.rebuild().await
    }

    pub async fn play_selection(&self, id: &str, index: usize) -> Result {
        let selection = if let Some((source, id)) = provider_ref(id) {
            self.inner
                .playlists
                .provider_playback_selection(source, id, Some(index))
                .await
        } else {
            self.inner
                .playlists
                .playback_selection(id, Some(index))
                .await
        }
        .map_err(service_error)?;
        self.inner.engine.set_queue(
            selection.tracks,
            isize::try_from(selection.start_index).unwrap_or_default(),
            ChangeOrigin::User,
            true,
        );
        self.rebuild().await
    }
    pub async fn play_from_occurrence(&self, id: &str, key: &str) -> Result {
        // Materialize first, then resolve the stable occurrence key against the current source list.
        self.open(id).await?;
        let index = self
            .inner
            .state
            .borrow()
            .tracks
            .get(id)
            .and_then(|rows| rows.iter().position(|row| row.key == key))
            .ok_or_else(|| ServiceError("Track no longer exists".into()))?;
        self.play_selection(id, index).await
    }
    pub fn play_next(&self, track: Track) {
        self.inner.engine.play_next(vec![track]);
    }
    pub async fn duplicate_groups(&self, id: &str) -> Result<Vec<DuplicateGroup>> {
        let tracks = if let Some((source, id)) = provider_ref(id) {
            self.inner.catalog.cached_tracks(source, id).await
        } else {
            self.inner.playlists.cached_tracks(id).await
        }
        .map_err(service_error)?;
        tokio::task::spawn_blocking(move || {
            ap_model::dedupe::deduplicate_tracks(tracks).duplicate_groups
        })
        .await
        .map_err(service_error)
    }
}

#[async_trait]
impl LibraryPort for LibraryAdapter {
    fn subscribe(&self) -> watch::Receiver<LibrarySnapshot> {
        self.inner.state.subscribe()
    }
    async fn library(&self, command: LibraryCommand) -> Result {
        match command {
            LibraryCommand::Open(id) => return self.open(&id).await,
            LibraryCommand::Play { id, key } => {
                return if let Some(key) = key {
                    self.play_from_occurrence(&id, &key).await
                } else {
                    self.play_selection(&id, 0).await
                };
            }
            LibraryCommand::PlayNext(track) => {
                self.play_next(track);
                return Ok(());
            }
            LibraryCommand::Refresh => {
                let failures = self
                    .inner
                    .catalog
                    .refresh_all()
                    .await
                    .map_err(service_error)?;
                for playlist in self.inner.playlists.list().await.map_err(service_error)? {
                    if playlist.playlist_type == PlaylistType::Union {
                        self.inner
                            .playlists
                            .materialize_union(&playlist.id, false)
                            .await
                            .map_err(service_error)?;
                    }
                }
                self.rebuild().await?;
                if !failures.is_empty() {
                    return Err(ServiceError(
                        failures
                            .into_iter()
                            .map(|f| format!("{}: {}", f.source.name(), f.message))
                            .collect::<Vec<_>>()
                            .join("; "),
                    ));
                }
                return Ok(());
            }
            LibraryCommand::Create { name, union } => {
                self.inner
                    .playlists
                    .create(
                        &name,
                        if union {
                            PlaylistType::Union
                        } else {
                            PlaylistType::Standard
                        },
                    )
                    .await
                    .map_err(service_error)?;
            }
            LibraryCommand::Update {
                id,
                name,
                description,
            } => {
                require_custom(&id)?;
                self.inner
                    .playlists
                    .update(
                        &id,
                        ap_store::playlists::PlaylistPatch {
                            name: Some(name),
                            description: Some(if description.is_empty() {
                                None
                            } else {
                                Some(description)
                            }),
                            image_url: None,
                        },
                    )
                    .await
                    .map_err(service_error)?;
            }
            LibraryCommand::Delete(id) => {
                require_custom(&id)?;
                self.inner
                    .playlists
                    .delete(&id)
                    .await
                    .map_err(service_error)?;
            }
            LibraryCommand::Distinct { id, enabled } => {
                if let Some((source, id)) = provider_ref(&id) {
                    self.inner
                        .catalog
                        .set_distinct(source, id, enabled)
                        .await
                        .map_err(service_error)?;
                } else {
                    self.inner
                        .playlists
                        .set_distinct(&id, enabled)
                        .await
                        .map_err(service_error)?;
                }
            }
            LibraryCommand::Add { id, track } => {
                require_custom(&id)?;
                self.inner
                    .playlists
                    .add_tracks(&id, vec![track])
                    .await
                    .map_err(service_error)?;
            }
            LibraryCommand::Remove { id, key } => {
                require_custom(&id)?;
                self.inner
                    .playlists
                    .remove_track(&id, row_id(&key)?)
                    .await
                    .map_err(service_error)?;
            }
            LibraryCommand::Move { id, key, position } => {
                require_custom(&id)?;
                let mut rows = self
                    .inner
                    .playlists
                    .track_rows(&id)
                    .await
                    .map_err(service_error)?
                    .into_iter()
                    .map(|r| r.id)
                    .collect::<Vec<_>>();
                move_item(&mut rows, &row_id(&key)?, position)?;
                self.inner
                    .playlists
                    .reorder_tracks(&id, rows)
                    .await
                    .map_err(service_error)?;
            }
            LibraryCommand::AddSource { id, source } => {
                require_custom(&id)?;
                if id == source {
                    return Err(ServiceError("A union cannot include itself".into()));
                }
                let (kind, source_id) =
                    provider_ref(&source).unwrap_or((SourceType::Custom, source.as_str()));
                self.inner
                    .playlists
                    .add_union_source(&id, kind, source_id)
                    .await
                    .map_err(service_error)?;
            }
            LibraryCommand::RemoveSource { id, source } => {
                require_custom(&id)?;
                let rows = self
                    .inner
                    .playlists
                    .union_sources(&id)
                    .await
                    .map_err(service_error)?;
                let row = rows
                    .iter()
                    .find(|row| playlist_key(row.source_type, &row.source_playlist_id) == source)
                    .ok_or_else(|| ServiceError("Union source no longer exists".into()))?;
                self.inner
                    .playlists
                    .remove_union_source(&id, row.id)
                    .await
                    .map_err(service_error)?;
            }
            LibraryCommand::MoveSource {
                id,
                source,
                position,
            } => {
                require_custom(&id)?;
                let rows = self
                    .inner
                    .playlists
                    .union_sources(&id)
                    .await
                    .map_err(service_error)?;
                let target = rows
                    .iter()
                    .find(|row| playlist_key(row.source_type, &row.source_playlist_id) == source)
                    .ok_or_else(|| ServiceError("Union source no longer exists".into()))?
                    .id;
                let mut order = rows.into_iter().map(|row| row.id).collect::<Vec<_>>();
                move_item(&mut order, &target, position)?;
                self.inner
                    .playlists
                    .reorder_union_sources(&id, order)
                    .await
                    .map_err(service_error)?;
            }
            LibraryCommand::ClearCache => {
                self.inner
                    .catalog
                    .clear_cache()
                    .await
                    .map_err(service_error)?;
            }
        }
        self.rebuild().await
    }
}

fn service_error(error: impl std::fmt::Display) -> ServiceError {
    ServiceError(error.to_string())
}
fn require_custom(id: &str) -> Result {
    if provider_ref(id).is_some() {
        Err(ServiceError("Provider playlists are read-only".into()))
    } else {
        Ok(())
    }
}
fn provider_ref(id: &str) -> Option<(SourceType, &str)> {
    let (source, id) = id.strip_prefix("p:")?.split_once(':')?;
    let source = SourceType::parse_lenient(source)?;
    matches!(
        source,
        SourceType::Jellyfin | SourceType::Plex | SourceType::Spotify
    )
    .then_some((source, id))
}
fn playlist_key(source: SourceType, id: &str) -> String {
    if source == SourceType::Custom {
        id.to_owned()
    } else {
        LibraryAdapter::provider_id(source, id)
    }
}
fn row_id(key: &str) -> Result<i64> {
    key.parse()
        .map_err(|_| ServiceError("Track occurrence is invalid".into()))
}
fn move_item<T: PartialEq>(items: &mut Vec<T>, key: &T, position: usize) -> Result {
    let index = items
        .iter()
        .position(|item| item == key)
        .ok_or_else(|| ServiceError("Item no longer exists".into()))?;
    let item = items.remove(index);
    items.insert(position.min(items.len()), item);
    Ok(())
}
fn provider_card(p: Playlist, distinct: bool) -> PlaylistCard {
    PlaylistCard {
        id: LibraryAdapter::provider_id(p.source, &p.id),
        name: p.name,
        description: p.description.unwrap_or_default(),
        owner: p.owner,
        source: p.source,
        union: false,
        distinct,
        count: usize::try_from(p.track_count).unwrap_or_default(),
    }
}
fn occurrences(tracks: Vec<Track>) -> Vec<TrackOccurrence> {
    let mut counts = HashMap::<(SourceType, String), usize>::new();
    tracks
        .into_iter()
        .map(|track| {
            let n = counts.entry((track.source, track.id.clone())).or_default();
            let key = format!("{}:{}:{}", track.source.name(), track.id, *n);
            *n += 1;
            TrackOccurrence { key, track }
        })
        .collect()
}

#[derive(Clone)]
pub struct SearchAdapter {
    inner: Arc<SearchInner>,
}
struct SearchInner {
    catalog: CatalogRepository,
    state: watch::Sender<SearchSnapshot>,
    cancelled: watch::Sender<u64>,
    control: StdMutex<SearchControl>,
}
#[derive(Default)]
struct SearchControl {
    sequence: u64,
    generation: u64,
    cache: Option<SearchCache>,
}
struct SearchCache {
    query: String,
    source: SourceType,
    playlists: bool,
    result: Arc<SearchResult>,
}
impl SearchAdapter {
    pub fn new(catalog: CatalogRepository) -> Self {
        let (state, _) = watch::channel(SearchSnapshot::default());
        let (cancelled, _) = watch::channel(0);
        Self {
            inner: Arc::new(SearchInner {
                catalog,
                state,
                cancelled,
                control: StdMutex::new(SearchControl::default()),
            }),
        }
    }
}
#[async_trait]
impl SearchPort for SearchAdapter {
    fn subscribe(&self) -> watch::Receiver<SearchSnapshot> {
        self.inner.state.subscribe()
    }
    async fn search(&self, request: SearchRequest) -> Result {
        let query = request.query.trim().to_owned();
        let mut cancelled = self.inner.cancelled.subscribe();
        let (sequence, cached) = {
            let mut control = self.inner.control.lock().map_err(service_error)?;
            if control.generation > request.generation {
                return Ok(());
            }
            control.generation = request.generation;
            control.sequence = control.sequence.wrapping_add(1);
            let sequence = control.sequence;
            self.inner.cancelled.send_replace(sequence);
            let cached = control
                .cache
                .as_ref()
                .filter(|c| {
                    c.query == query
                        && c.source == request.source
                        && c.playlists == request.playlists
                })
                .map(|c| c.result.clone());
            (sequence, cached)
        };
        let result = if let Some(cached) = cached {
            Ok(cached)
        } else {
            let kind = if request.playlists {
                SearchType::Playlists
            } else {
                SearchType::Tracks
            };
            let superseded = async {
                while *cancelled.borrow() == sequence {
                    if cancelled.changed().await.is_err() {
                        break;
                    }
                }
            };
            tokio::select! {
                biased;
                () = superseded => return Ok(()),
                result = self.inner.catalog.search(&query, request.source, kind) => result.map(Arc::new),
            }
        };
        let (snapshot, result) = tokio::task::spawn_blocking(move || {
            let mut snapshot = SearchSnapshot {
                page: request.page,
                generation: request.generation,
                ..Default::default()
            };
            match result {
                Ok(result) => {
                    snapshot.total = if request.playlists {
                        result.playlists.len()
                    } else {
                        result.tracks.len()
                    };
                    let offset = request.page.saturating_mul(20);
                    if request.playlists {
                        snapshot.playlists = result
                            .playlists
                            .iter()
                            .skip(offset)
                            .take(20)
                            .cloned()
                            .map(|p| provider_card(p, false))
                            .collect();
                    } else {
                        snapshot.tracks = result
                            .tracks
                            .iter()
                            .skip(offset)
                            .take(20)
                            .cloned()
                            .collect();
                    }
                    (snapshot, Some(result))
                }
                Err(error) => {
                    snapshot.error = "Search failed".into();
                    snapshot.warnings.push(error.to_string());
                    (snapshot, None)
                }
            }
        })
        .await
        .map_err(service_error)?;
        let mut control = self.inner.control.lock().map_err(service_error)?;
        if control.sequence == sequence {
            control.cache = result.map(|result| SearchCache {
                query,
                source: request.source,
                playlists: request.playlists,
                result,
            });
            self.inner.state.send_replace(snapshot);
        }
        Ok(())
    }
}

struct CustomRows {
    playlist: ap_store::playlists::PlaylistRecord,
    saved: Vec<ap_store::playlists::PlaylistTrackRecord>,
    sources: Vec<ap_store::playlists::UnionSourceRecord>,
    tracks: Vec<Track>,
}
fn build_snapshot(
    custom: Vec<CustomRows>,
    providers: Vec<(Playlist, bool, Vec<Track>)>,
    revision: u64,
) -> LibrarySnapshot {
    let mut cards = Vec::with_capacity(custom.len() + providers.len());
    let mut tracks = BTreeMap::new();
    let mut sources = BTreeMap::new();
    for item in custom {
        let p = item.playlist;
        let id = p.id;
        let rows = if p.playlist_type == PlaylistType::Standard {
            item.saved
                .into_iter()
                .enumerate()
                .map(|(index, row)| {
                    let mut track = row.to_track();
                    if let Some(cached) = item
                        .tracks
                        .get(index)
                        .filter(|t| t.id == row.track_id && t.source == row.track_source)
                    {
                        track.url.clone_from(&cached.url);
                    }
                    TrackOccurrence {
                        key: row.id.to_string(),
                        track,
                    }
                })
                .collect()
        } else {
            sources.insert(
                id.clone(),
                item.sources
                    .into_iter()
                    .map(|s| playlist_key(s.source_type, &s.source_playlist_id))
                    .collect(),
            );
            occurrences(item.tracks)
        };
        cards.push(PlaylistCard {
            id: id.clone(),
            name: p.name,
            description: p.description.unwrap_or_default(),
            owner: "You".into(),
            source: SourceType::Custom,
            union: p.playlist_type == PlaylistType::Union,
            distinct: p.is_distinct,
            count: usize::try_from(p.track_count).unwrap_or_default(),
        });
        tracks.insert(id, rows);
    }
    for (p, distinct, rows) in providers {
        tracks.insert(
            LibraryAdapter::provider_id(p.source, &p.id),
            occurrences(rows),
        );
        cards.push(provider_card(p, distinct));
    }
    LibrarySnapshot {
        catalog: cards,
        tracks,
        sources,
        revision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ap_model::QueueState;
    use ap_playback::{
        EngineDeps, PlayerSettings,
        ports::{BoxError, ConnectionGate, PlaybackPersistence},
    };
    use ap_providers::{MediaRequest, ProviderClient, ProviderError, Validation};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::Semaphore;

    #[derive(Default)]
    struct MemoryPlayback {
        queue: StdMutex<QueueState>,
        settings: StdMutex<PlayerSettings>,
        gains: StdMutex<HashMap<String, f32>>,
    }
    #[async_trait]
    impl PlaybackPersistence for MemoryPlayback {
        async fn load_queue(&self) -> std::result::Result<QueueState, BoxError> {
            Ok(self.queue.lock().unwrap().clone())
        }
        async fn save_queue_content(
            &self,
            queue: &QueueState,
        ) -> std::result::Result<(), BoxError> {
            *self.queue.lock().unwrap() = queue.clone();
            Ok(())
        }
        async fn save_cursor(
            &self,
            uid: Option<u64>,
            version: u64,
        ) -> std::result::Result<(), BoxError> {
            let mut queue = self.queue.lock().unwrap();
            if queue.content_version == version {
                queue.cursor = uid
                    .and_then(|uid| {
                        queue.order.iter().position(|&index| {
                            queue.entries.get(index).is_some_and(|e| e.uid == uid)
                        })
                    })
                    .and_then(|i| isize::try_from(i).ok())
                    .unwrap_or(-1);
            }
            Ok(())
        }
        async fn load_settings(&self) -> std::result::Result<PlayerSettings, BoxError> {
            Ok(self.settings.lock().unwrap().clone())
        }
        async fn save_settings(
            &self,
            settings: &PlayerSettings,
        ) -> std::result::Result<(), BoxError> {
            *self.settings.lock().unwrap() = settings.clone();
            Ok(())
        }
        async fn track_gain(&self, key: &str) -> Option<f32> {
            self.gains.lock().unwrap().get(key).copied()
        }
        async fn put_track_gain(&self, key: &str, gain: f32) {
            self.gains.lock().unwrap().insert(key.into(), gain);
        }
    }
    struct Gate(watch::Sender<()>);
    impl ConnectionGate for Gate {
        fn disabled_name(&self, _: SourceType) -> Option<String> {
            None
        }
        fn subscribe(&self) -> watch::Receiver<()> {
            self.0.subscribe()
        }
    }
    fn engine() -> Engine {
        let (gate, _) = watch::channel(());
        Engine::spawn(EngineDeps {
            backends: vec![],
            persistence: Arc::new(MemoryPlayback::default()),
            gate: Arc::new(Gate(gate)),
        })
    }
    struct Active(Arc<AtomicUsize>);
    impl Drop for Active {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    struct Provider {
        source: SourceType,
        fail: AtomicBool,
        tracks: StdMutex<Vec<Track>>,
        fetches: AtomicUsize,
        searches: AtomicUsize,
        playlist_searches: AtomicUsize,
        active: Arc<AtomicUsize>,
        started: Semaphore,
        gate: Semaphore,
    }
    impl Provider {
        fn new(source: SourceType, count: usize) -> Self {
            Self {
                source,
                fail: AtomicBool::new(false),
                tracks: StdMutex::new(
                    (0..count)
                        .map(|i| Track::new(format!("{i}"), format!("Song {i}"), "Artist", source))
                        .collect(),
                ),
                fetches: AtomicUsize::new(0),
                searches: AtomicUsize::new(0),
                playlist_searches: AtomicUsize::new(0),
                active: Arc::new(AtomicUsize::new(0)),
                started: Semaphore::new(0),
                gate: Semaphore::new(0),
            }
        }
        fn check(&self) -> std::result::Result<(), ProviderError> {
            if self.fail.load(Ordering::SeqCst) {
                Err(ProviderError::InvalidConfig("offline".into()))
            } else {
                Ok(())
            }
        }
        fn playlist(&self) -> Playlist {
            Playlist {
                id: "shared".into(),
                name: self.source.name().into(),
                owner: "Provider".into(),
                track_count: i32::try_from(self.tracks.lock().unwrap().len()).unwrap(),
                source: self.source,
                image_url: None,
                tracks: None,
                description: None,
            }
        }
    }
    #[async_trait]
    impl ProviderClient for Provider {
        fn source(&self) -> SourceType {
            self.source
        }
        async fn validate(&self) -> std::result::Result<Validation, ProviderError> {
            self.check()?;
            Ok(Validation::default())
        }
        async fn playlists(&self) -> std::result::Result<Vec<Playlist>, ProviderError> {
            self.check()?;
            Ok(vec![self.playlist()])
        }
        async fn playlist_tracks(&self, _: &str) -> std::result::Result<Vec<Track>, ProviderError> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            self.check()?;
            Ok(self.tracks.lock().unwrap().clone())
        }
        async fn search_tracks(
            &self,
            query: &str,
        ) -> std::result::Result<Vec<Track>, ProviderError> {
            self.searches.fetch_add(1, Ordering::SeqCst);
            self.check()?;
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(self.active.clone());
            if query == "slow" {
                self.started.add_permits(1);
                let _permit = self.gate.acquire().await.unwrap();
            }
            Ok(self.tracks.lock().unwrap().clone())
        }
        async fn search_playlists(
            &self,
            _: &str,
        ) -> std::result::Result<Vec<Playlist>, ProviderError> {
            self.playlist_searches.fetch_add(1, Ordering::SeqCst);
            self.check()?;
            Ok(vec![self.playlist()])
        }
        async fn stream_request(
            &self,
            _: &Track,
        ) -> std::result::Result<MediaRequest, ProviderError> {
            Err(ProviderError::InvalidConfig(
                "No audio in adapter tests".into(),
            ))
        }
        fn image_request(&self, _: &str) -> Option<MediaRequest> {
            None
        }
    }
    async fn database() -> (tempfile::TempDir, ap_store::Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = ap_store::Db::open(dir.path().join("playlists.db"))
            .await
            .unwrap();
        (dir, db)
    }
    fn search_request(query: &str, page: usize, generation: u64) -> SearchRequest {
        SearchRequest {
            query: query.into(),
            source: SourceType::All,
            playlists: false,
            page,
            generation,
        }
    }

    #[tokio::test]
    async fn library_observes_cache_only_and_open_fetches_namespaced_provider_playlist() {
        let (_dir, db) = database().await;
        let jellyfin = Arc::new(Provider::new(SourceType::Jellyfin, 3));
        let plex = Arc::new(Provider::new(SourceType::Plex, 2));
        let catalog = CatalogRepository::new(db.clone(), vec![jellyfin.clone(), plex.clone()])
            .await
            .unwrap();
        catalog.refresh_playlists().await.unwrap();
        let playlists = PlaylistRepository::new(db, catalog.clone()).await.unwrap();
        let adapter = LibraryAdapter::new(catalog, playlists, engine())
            .await
            .unwrap();
        let state = LibraryPort::subscribe(&adapter);
        assert_eq!(state.borrow().catalog.len(), 2);
        assert!(state.borrow().tracks.values().all(Vec::is_empty));
        assert_eq!(jellyfin.fetches.load(Ordering::SeqCst), 0);
        assert_eq!(plex.fetches.load(Ordering::SeqCst), 0);
        assert_ne!(state.borrow().catalog[0].id, state.borrow().catalog[1].id);
        adapter
            .library(LibraryCommand::Open(LibraryAdapter::provider_id(
                SourceType::Jellyfin,
                "shared",
            )))
            .await
            .unwrap();
        assert_eq!(state.borrow().tracks["p:JELLYFIN:shared"].len(), 3);
        assert_eq!(state.borrow().tracks["p:PLEX:shared"].len(), 0);
        adapter
            .library(LibraryCommand::Distinct {
                id: "p:JELLYFIN:shared".into(),
                enabled: true,
            })
            .await
            .unwrap();
        assert!(
            state
                .borrow()
                .catalog
                .iter()
                .find(|p| p.source == SourceType::Jellyfin)
                .unwrap()
                .distinct
        );
    }

    #[tokio::test]
    async fn standard_commands_preserve_occurrences_and_distinct_play_maps_clicked_duplicate() {
        let (_dir, db) = database().await;
        let catalog = CatalogRepository::new(db.clone(), vec![]).await.unwrap();
        let playlists = PlaylistRepository::new(db, catalog.clone()).await.unwrap();
        let player = engine();
        let adapter = LibraryAdapter::new(catalog, playlists.clone(), player.clone())
            .await
            .unwrap();
        adapter
            .library(LibraryCommand::Create {
                name: "Standard".into(),
                union: false,
            })
            .await
            .unwrap();
        let id = LibraryPort::subscribe(&adapter).borrow().catalog[0]
            .id
            .clone();
        let a = Track::new("a", "Same", "Artist", SourceType::Plex);
        let b = Track::new("b", "same", "Artist", SourceType::Spotify);
        for track in [
            a,
            Track::new("other", "Other", "Artist", SourceType::Jellyfin),
            b,
        ] {
            adapter
                .library(LibraryCommand::Add {
                    id: id.clone(),
                    track,
                })
                .await
                .unwrap();
        }
        let rows = LibraryPort::subscribe(&adapter).borrow().tracks[&id].clone();
        assert_ne!(rows[0].key, rows[2].key);
        assert_eq!(adapter.duplicate_groups(&id).await.unwrap().len(), 1);
        adapter
            .library(LibraryCommand::Distinct {
                id: id.clone(),
                enabled: true,
            })
            .await
            .unwrap();
        adapter
            .library(LibraryCommand::Play {
                id: id.clone(),
                key: Some(rows[2].key.clone()),
            })
            .await
            .unwrap();
        player.flush().await;
        assert_eq!(player.queue().borrow().entries.len(), 2);
        assert_eq!(player.queue().borrow().current().unwrap().track.id, "a");
        adapter
            .library(LibraryCommand::PlayNext(Track::new(
                "next",
                "Next",
                "Artist",
                SourceType::Plex,
            )))
            .await
            .unwrap();
        player.flush().await;
        assert_eq!(player.queue().borrow().entries.len(), 3);
        adapter
            .library(LibraryCommand::Move {
                id: id.clone(),
                key: rows[2].key.clone(),
                position: 0,
            })
            .await
            .unwrap();
        assert_eq!(
            LibraryPort::subscribe(&adapter).borrow().tracks[&id][0]
                .track
                .id,
            "b"
        );
        adapter
            .library(LibraryCommand::Remove {
                id: id.clone(),
                key: rows[0].key.clone(),
            })
            .await
            .unwrap();
        assert_eq!(playlists.track_rows(&id).await.unwrap().len(), 2);
        adapter
            .library(LibraryCommand::Update {
                id: id.clone(),
                name: "Renamed".into(),
                description: "Description".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            LibraryPort::subscribe(&adapter).borrow().catalog[0].name,
            "Renamed"
        );
        adapter
            .library(LibraryCommand::Delete(id.clone()))
            .await
            .unwrap();
        assert!(
            !LibraryPort::subscribe(&adapter)
                .borrow()
                .tracks
                .contains_key(&id)
        );
    }

    #[tokio::test]
    async fn union_source_editor_and_play_all_use_repository_materialization() {
        let (_dir, db) = database().await;
        let provider = Arc::new(Provider::new(SourceType::Plex, 2));
        let catalog = CatalogRepository::new(db.clone(), vec![provider])
            .await
            .unwrap();
        catalog.refresh_playlists().await.unwrap();
        let playlists = PlaylistRepository::new(db, catalog.clone()).await.unwrap();
        let player = engine();
        let standard = playlists
            .create("Standard", PlaylistType::Standard)
            .await
            .unwrap();
        playlists
            .add_tracks(
                &standard.id,
                vec![
                    Track::new("0", "Song 0", "Artist", SourceType::Plex),
                    Track::new("custom", "Custom", "Artist", SourceType::Custom),
                ],
            )
            .await
            .unwrap();
        let union = playlists
            .create("Union", PlaylistType::Union)
            .await
            .unwrap();
        let adapter = LibraryAdapter::new(catalog, playlists, player.clone())
            .await
            .unwrap();
        for source in ["p:PLEX:shared".to_owned(), standard.id.clone()] {
            adapter
                .library(LibraryCommand::AddSource {
                    id: union.id.clone(),
                    source,
                })
                .await
                .unwrap();
        }
        let state = LibraryPort::subscribe(&adapter);
        assert_eq!(state.borrow().materialize(&union.id).len(), 3);
        adapter
            .library(LibraryCommand::Play {
                id: union.id.clone(),
                key: None,
            })
            .await
            .unwrap();
        player.flush().await;
        assert_eq!(player.queue().borrow().entries.len(), 3);
        adapter
            .library(LibraryCommand::MoveSource {
                id: union.id.clone(),
                source: standard.id.clone(),
                position: 0,
            })
            .await
            .unwrap();
        assert_eq!(state.borrow().sources[&union.id][0], standard.id);
        adapter
            .library(LibraryCommand::RemoveSource {
                id: union.id.clone(),
                source: "p:PLEX:shared".into(),
            })
            .await
            .unwrap();
        assert_eq!(state.borrow().materialize(&union.id).len(), 2);
    }

    #[tokio::test]
    async fn search_paginates_cached_typed_results_in_provider_order_and_isolates_failures() {
        let (_dir, db) = database().await;
        let jellyfin = Arc::new(Provider::new(SourceType::Jellyfin, 25));
        let plex = Arc::new(Provider::new(SourceType::Plex, 25));
        let catalog = CatalogRepository::new(db, vec![plex.clone(), jellyfin.clone()])
            .await
            .unwrap();
        let adapter = SearchAdapter::new(catalog);
        let state = SearchPort::subscribe(&adapter);
        adapter.search(search_request("query", 0, 1)).await.unwrap();
        assert_eq!(state.borrow().total, 50);
        assert_eq!(state.borrow().tracks.len(), 20);
        assert_eq!(state.borrow().tracks[0].source, SourceType::Jellyfin);
        adapter.search(search_request("query", 1, 2)).await.unwrap();
        assert_eq!(state.borrow().tracks.len(), 20);
        assert_eq!(state.borrow().tracks[5].source, SourceType::Plex);
        assert_eq!(jellyfin.searches.load(Ordering::SeqCst), 1);
        assert_eq!(plex.searches.load(Ordering::SeqCst), 1);
        plex.fail.store(true, Ordering::SeqCst);
        adapter.search(search_request("other", 0, 3)).await.unwrap();
        assert_eq!(state.borrow().total, 25);
        assert!(state.borrow().error.is_empty());
        jellyfin.fail.store(true, Ordering::SeqCst);
        adapter
            .search(search_request("all fail", 0, 4))
            .await
            .unwrap();
        assert_eq!(state.borrow().error, "Search failed");
        jellyfin.fail.store(false, Ordering::SeqCst);
        let mut request = search_request("playlists", 0, 5);
        request.playlists = true;
        adapter.search(request).await.unwrap();
        assert_eq!(state.borrow().playlists.len(), 1);
        assert_eq!(state.borrow().tracks.len(), 0);
        assert_eq!(jellyfin.playlist_searches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn newer_search_cancels_network_future_and_stale_generations_cannot_replace_it() {
        let (_dir, db) = database().await;
        let provider = Arc::new(Provider::new(SourceType::Jellyfin, 1));
        let catalog = CatalogRepository::new(db, vec![provider.clone()])
            .await
            .unwrap();
        let adapter = SearchAdapter::new(catalog);
        let slow = tokio::spawn({
            let adapter = adapter.clone();
            async move { adapter.search(search_request("slow", 0, 10)).await }
        });
        provider.started.acquire().await.unwrap().forget();
        adapter.search(search_request("fast", 0, 11)).await.unwrap();
        slow.await.unwrap().unwrap();
        assert_eq!(provider.active.load(Ordering::SeqCst), 0);
        adapter.search(search_request("stale", 0, 9)).await.unwrap();
        assert_eq!(SearchPort::subscribe(&adapter).borrow().generation, 11);
        assert_eq!(provider.searches.load(Ordering::SeqCst), 2);
    }
}
