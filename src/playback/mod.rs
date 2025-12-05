/// Playback management
use crate::models::{PlaybackInfo, PlaybackState, RepeatMode, Track};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Queue for managing playback
#[derive(Debug, Clone)]
pub struct PlaybackQueue {
    /// All tracks in the queue
    pub tracks: Vec<Track>,
    /// Current position in queue
    pub current_index: usize,
}

impl PlaybackQueue {
    pub fn new() -> Self {
        Self {
            tracks: Vec::new(),
            current_index: 0,
        }
    }

    pub fn add_track(&mut self, track: Track) {
        self.tracks.push(track);
    }

    pub fn add_tracks(&mut self, tracks: Vec<Track>) {
        self.tracks.extend(tracks);
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current_index = 0;
    }

    pub fn current_track(&self) -> Option<&Track> {
        if self.current_index < self.tracks.len() {
            Some(&self.tracks[self.current_index])
        } else {
            None
        }
    }

    pub fn next(&mut self) -> Option<&Track> {
        if self.current_index < self.tracks.len() - 1 {
            self.current_index += 1;
            self.current_track()
        } else {
            None
        }
    }

    pub fn previous(&mut self) -> Option<&Track> {
        if self.current_index > 0 {
            self.current_index -= 1;
            self.current_track()
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}

impl Default for PlaybackQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Playback manager - handles playback state and queue
pub struct PlaybackManager {
    queue: Arc<Mutex<PlaybackQueue>>,
    info: Arc<Mutex<PlaybackInfo>>,
    // TODO: Add audio sink/stream for actual playback
}

impl PlaybackManager {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(Mutex::new(PlaybackQueue::new())),
            info: Arc::new(Mutex::new(PlaybackInfo::default())),
        }
    }

    /// Add a track to the queue
    pub async fn queue_track(&self, track: Track) {
        let mut queue = self.queue.lock().await;
        queue.add_track(track);
    }

    /// Add multiple tracks to the queue
    pub async fn queue_tracks(&self, tracks: Vec<Track>) {
        let mut queue = self.queue.lock().await;
        queue.add_tracks(tracks);
    }

    /// Clear the playback queue
    pub async fn clear_queue(&self) {
        let mut queue = self.queue.lock().await;
        queue.clear();
        let mut info = self.info.lock().await;
        info.state = PlaybackState::Stopped;
        info.current_track = None;
    }

    /// Play a track (start playback)
    pub async fn play(&self) {
        let mut info = self.info.lock().await;
        info.state = PlaybackState::Playing;
    }

    /// Pause playback
    pub async fn pause(&self) {
        let mut info = self.info.lock().await;
        info.state = PlaybackState::Paused;
    }

    /// Toggle play/pause
    pub async fn toggle_play_pause(&self) {
        let mut info = self.info.lock().await;
        info.state = match info.state {
            PlaybackState::Playing => PlaybackState::Paused,
            PlaybackState::Paused | PlaybackState::Stopped => PlaybackState::Playing,
        };
    }

    /// Play next track
    pub async fn next_track(&self) -> Option<Track> {
        let mut queue = self.queue.lock().await;
        if let Some(track) = queue.next() {
            let mut info = self.info.lock().await;
            info.current_track = Some(track.clone());
            info.position_ms = 0;
            Some(track.clone())
        } else {
            None
        }
    }

    /// Play previous track
    pub async fn previous_track(&self) -> Option<Track> {
        let mut queue = self.queue.lock().await;
        if let Some(track) = queue.previous() {
            let mut info = self.info.lock().await;
            info.current_track = Some(track.clone());
            info.position_ms = 0;
            Some(track.clone())
        } else {
            None
        }
    }

    /// Seek to a position in the current track
    pub async fn seek(&self, position_ms: u64) {
        let mut info = self.info.lock().await;
        info.position_ms = position_ms;
    }

    /// Set volume (0-100)
    pub async fn set_volume(&self, volume: u32) {
        let mut info = self.info.lock().await;
        info.volume = volume.min(100);
    }

    /// Toggle shuffle mode
    pub async fn toggle_shuffle(&self) {
        let mut info = self.info.lock().await;
        info.shuffle = !info.shuffle;
    }

    /// Set repeat mode
    pub async fn set_repeat_mode(&self, mode: RepeatMode) {
        let mut info = self.info.lock().await;
        info.repeat_mode = mode;
    }

    /// Get current playback info
    pub async fn get_info(&self) -> PlaybackInfo {
        self.info.lock().await.clone()
    }

    /// Get current queue length
    pub async fn queue_length(&self) -> usize {
        self.queue.lock().await.len()
    }

    /// Get current track
    pub async fn current_track(&self) -> Option<Track> {
        self.queue.lock().await.current_track().cloned()
    }
}

impl Default for PlaybackManager {
    fn default() -> Self {
        Self::new()
    }
}
