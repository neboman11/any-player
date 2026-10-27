#![allow(dead_code)]
use ap_model::{QueueState, SourceType, Track};
use ap_playback::ports::BoxError;
use ap_playback::{
    ConnectionGate, PlaybackPersistence, PlayerSettings, StreamRequest, StreamResolver,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
#[derive(Default)]
pub struct Memory {
    pub queue: Mutex<QueueState>,
    pub settings: Mutex<PlayerSettings>,
    pub gains: Mutex<HashMap<String, f32>>,
    pub writes: Mutex<Vec<QueueState>>,
    pub cursors: Mutex<Vec<(Option<u64>, u64)>>,
}
#[async_trait]
impl PlaybackPersistence for Memory {
    async fn load_queue(&self) -> Result<QueueState, BoxError> {
        Ok(self.queue.lock().unwrap().clone())
    }
    async fn save_queue_content(&self, q: &QueueState) -> Result<(), BoxError> {
        *self.queue.lock().unwrap() = q.clone();
        self.writes.lock().unwrap().push(q.clone());
        Ok(())
    }
    async fn save_cursor(&self, uid: Option<u64>, v: u64) -> Result<(), BoxError> {
        self.cursors.lock().unwrap().push((uid, v));
        Ok(())
    }
    async fn load_settings(&self) -> Result<PlayerSettings, BoxError> {
        Ok(self.settings.lock().unwrap().clone())
    }
    async fn save_settings(&self, s: &PlayerSettings) -> Result<(), BoxError> {
        *self.settings.lock().unwrap() = s.clone();
        Ok(())
    }
    async fn track_gain(&self, key: &str) -> Option<f32> {
        self.gains.lock().unwrap().get(key).copied()
    }
    async fn put_track_gain(&self, key: &str, gain: f32) {
        self.gains.lock().unwrap().insert(key.into(), gain);
    }
}
pub struct Gate {
    pub disabled: Mutex<Option<String>>,
    pub tx: watch::Sender<()>,
}
impl Gate {
    pub fn new() -> Arc<Self> {
        let (tx, _) = watch::channel(());
        Arc::new(Self {
            disabled: Mutex::new(None),
            tx,
        })
    }
}
impl ConnectionGate for Gate {
    fn disabled_name(&self, _: SourceType) -> Option<String> {
        self.disabled.lock().unwrap().clone()
    }
    fn subscribe(&self) -> watch::Receiver<()> {
        self.tx.subscribe()
    }
}
pub struct Resolver;
#[async_trait]
impl StreamResolver for Resolver {
    async fn resolve(&self, track: &Track) -> Result<StreamRequest, BoxError> {
        Ok(StreamRequest {
            url: track.url.as_ref().unwrap().parse()?,
            headers: reqwest::header::HeaderMap::default(),
            allow_invalid_certs: false,
        })
    }
}
pub fn track(id: &str, source: SourceType) -> Track {
    Track {
        url: Some(format!("http://127.0.0.1/{id}")),
        duration_ms: Some(60_000),
        ..Track::new(id, id, "Artist", source)
    }
}
pub async fn settle() {
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
}
