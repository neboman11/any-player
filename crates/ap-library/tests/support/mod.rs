#![allow(dead_code)]
use ap_library::{
    Result,
    transfer::{ConnectionStore, StoredConnection},
};
use ap_model::{Playlist, SourceType, Track};
use ap_providers::{MediaRequest, ProviderClient, ProviderError, Validation};
use async_trait::async_trait;
use indexmap::IndexMap;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::Semaphore;

#[derive(Default)]
pub struct Connections(pub Mutex<IndexMap<SourceType, StoredConnection>>);
#[async_trait]
impl ConnectionStore for Connections {
    async fn snapshot(&self) -> Result<IndexMap<SourceType, StoredConnection>> {
        Ok(self.0.lock().unwrap().clone())
    }
    async fn put(&self, connection: StoredConnection) -> Result<()> {
        self.0.lock().unwrap().insert(connection.source, connection);
        Ok(())
    }
}

pub struct Fake {
    pub source: SourceType,
    pub listing: Mutex<Vec<Playlist>>,
    pub tracks: Mutex<HashMap<String, Vec<Track>>>,
    pub fail: AtomicBool,
    pub calls: AtomicUsize,
    pub active: Arc<AtomicUsize>,
    pub peak: AtomicUsize,
    pub started: Semaphore,
    pub gate: Option<Arc<Semaphore>>,
}
struct Guard(Arc<AtomicUsize>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Fake {
    pub fn new(source: SourceType) -> Self {
        Self {
            source,
            listing: Mutex::new(vec![]),
            tracks: Mutex::new(HashMap::new()),
            fail: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            active: Arc::new(AtomicUsize::new(0)),
            peak: AtomicUsize::new(0),
            started: Semaphore::new(0),
            gate: None,
        }
    }
    async fn request(&self) -> ap_library::Result<(), ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        let _guard = Guard(self.active.clone());
        self.started.add_permits(1);
        if let Some(gate) = &self.gate {
            let _permit = gate.acquire().await.unwrap();
        }
        tokio::task::yield_now().await;
        if self.fail.load(Ordering::SeqCst) {
            Err(ProviderError::InvalidConfig("offline".into()))
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl ProviderClient for Fake {
    fn source(&self) -> SourceType {
        self.source
    }
    async fn validate(&self) -> ap_library::Result<Validation, ProviderError> {
        self.request().await?;
        Ok(Validation::default())
    }
    async fn playlists(&self) -> ap_library::Result<Vec<Playlist>, ProviderError> {
        self.request().await?;
        Ok(self.listing.lock().unwrap().clone())
    }
    async fn playlist_tracks(&self, id: &str) -> ap_library::Result<Vec<Track>, ProviderError> {
        self.request().await?;
        Ok(self
            .tracks
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or_default())
    }
    async fn search_tracks(&self, _: &str) -> ap_library::Result<Vec<Track>, ProviderError> {
        self.request().await?;
        Ok(vec![track(self.source.serial_name(), self.source)])
    }
    async fn search_playlists(&self, _: &str) -> ap_library::Result<Vec<Playlist>, ProviderError> {
        self.playlists().await
    }
    async fn stream_request(&self, _: &Track) -> ap_library::Result<MediaRequest, ProviderError> {
        Err(ProviderError::InvalidConfig("not used by library".into()))
    }
    fn image_request(&self, _: &str) -> Option<MediaRequest> {
        None
    }
}
pub fn track(id: &str, source: SourceType) -> Track {
    Track::new(id, format!("Song {id}"), "Artist", source)
}
pub fn playlist(id: &str, source: SourceType) -> Playlist {
    Playlist {
        id: id.into(),
        name: id.into(),
        owner: "Owner".into(),
        track_count: 0,
        source,
        image_url: None,
        tracks: None,
        description: None,
    }
}
pub async fn database() -> (tempfile::TempDir, ap_store::Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = ap_store::Db::open(dir.path().join("playlists.db"))
        .await
        .unwrap();
    (dir, db)
}
