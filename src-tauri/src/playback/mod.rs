/// Playback management
use crate::models::{PlaybackInfo, PlaybackState, RepeatMode, Track};
use rodio::{Decoder, OutputStream, Sink, Source};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Shared playback state for the current audio stream
#[derive(Clone)]
pub struct PlaybackHandle {
    /// Global flag to stop the playback thread
    stop_flag: Arc<AtomicBool>,
    /// Current playback position in milliseconds
    position_ms: Arc<AtomicU64>,
    /// Total duration in milliseconds
    duration_ms: Arc<AtomicU64>,
    /// Whether playback is paused
    is_paused: Arc<AtomicBool>,
}

impl PlaybackHandle {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            position_ms: Arc::new(AtomicU64::new(0)),
            duration_ms: Arc::new(AtomicU64::new(0)),
            is_paused: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    pub fn pause(&self) {
        self.is_paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.is_paused.store(false, Ordering::SeqCst);
    }

    pub fn get_position(&self) -> u64 {
        self.position_ms.load(Ordering::SeqCst)
    }

    pub fn set_position(&self, ms: u64) {
        self.position_ms.store(ms, Ordering::SeqCst);
    }

    pub fn get_duration(&self) -> u64 {
        self.duration_ms.load(Ordering::SeqCst)
    }

    pub fn set_duration(&self, ms: u64) {
        self.duration_ms.store(ms, Ordering::SeqCst);
    }

    pub fn should_stop(&self) -> bool {
        self.stop_flag.load(Ordering::SeqCst)
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused.load(Ordering::SeqCst)
    }
}

impl Default for PlaybackHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Audio player for playback
pub struct AudioPlayer {
    current_handle: Arc<Mutex<Option<PlaybackHandle>>>,
}

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

impl AudioPlayer {
    pub fn new() -> Self {
        Self {
            current_handle: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn play_url(&self, url: &str) -> Result<PlaybackHandle, String> {
        let url = url.to_string();
        let handle = PlaybackHandle::new();
        let handle_clone = handle.clone();

        // Store the handle so we can control playback
        {
            let mut current = self.current_handle.lock().await;
            if let Some(old_handle) = current.take() {
                old_handle.stop();
            }
            *current = Some(handle.clone());
        }

        // Spawn a background task to play audio without blocking
        tokio::spawn(async move {
            tracing::info!("Starting audio playback from URL: {}", url);

            // Spawn blocking task since rodio is not async-aware
            let result = tokio::task::spawn_blocking({
                let url = url.clone();
                let handle = handle_clone.clone();
                move || Self::play_audio_blocking(&url, &handle)
            })
            .await;

            match result {
                Ok(Ok(())) => {
                    tracing::info!("Audio playback completed successfully");
                }
                Ok(Err(e)) => {
                    tracing::error!("Audio playback error: {}", e);
                }
                Err(e) => {
                    tracing::error!("Task join error: {}", e);
                }
            }
        });

        Ok(handle)
    }

    fn play_audio_blocking(url: &str, handle: &PlaybackHandle) -> Result<(), String> {
        // Check if URL is valid (should be HTTP(S))
        if !url.starts_with("http") {
            return Err(format!(
                "Invalid playback URL format. Expected HTTP URL, got: {}",
                url
            ));
        }

        Self::play_http_audio(url, handle)
    }

    fn play_http_audio(url: &str, handle: &PlaybackHandle) -> Result<(), String> {
        // Get audio output stream
        let (_stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| format!("Failed to get audio output: {}", e))?;

        // Fetch audio data from URL
        let client = reqwest::blocking::Client::new();
        let response = client
            .get(url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .send()
            .map_err(|e| format!("Failed to fetch audio: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Failed to fetch audio: HTTP {}", response.status()));
        }

        let bytes = response
            .bytes()
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        // Decode audio data
        let cursor = Cursor::new(bytes.to_vec());
        let source = Decoder::new(cursor).map_err(|e| format!("Failed to decode audio: {}", e))?;

        // Get duration
        let duration_secs = source
            .total_duration()
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        handle.set_duration(duration_secs);

        tracing::info!("Playing preview audio (duration: {}ms)", duration_secs);

        // Create sink for playback control
        let sink =
            Sink::try_new(&stream_handle).map_err(|e| format!("Failed to create sink: {}", e))?;

        // Convert to f32 samples and add to sink
        let source = source.convert_samples::<f32>();
        sink.append(source);

        // Track playback progress
        let start = Instant::now();
        let mut last_update = Instant::now();

        loop {
            if handle.should_stop() {
                break;
            }

            // Update position
            let elapsed = start.elapsed().as_millis() as u64;
            if elapsed != handle.get_position() {
                handle.set_position(elapsed);
            }

            // Handle pause/resume
            if handle.is_paused() {
                sink.pause();
            } else {
                sink.play();
            }

            std::thread::sleep(Duration::from_millis(100));

            // Log progress periodically
            if last_update.elapsed() > Duration::from_secs(1) {
                tracing::debug!(
                    "Playback progress: {}/{} ms",
                    handle.get_position(),
                    duration_secs
                );
                last_update = Instant::now();
            }

            // Stop if we've reached the end or duration is exceeded
            if elapsed >= duration_secs && duration_secs > 0 {
                break;
            }
        }

        sink.stop();
        Ok(())
    }

    pub async fn pause(&self) -> Result<(), String> {
        if let Some(handle) = &*self.current_handle.lock().await {
            handle.pause();
            tracing::info!("Pausing playback");
            Ok(())
        } else {
            Err("No playback in progress".to_string())
        }
    }

    pub async fn resume(&self) -> Result<(), String> {
        if let Some(handle) = &*self.current_handle.lock().await {
            handle.resume();
            tracing::info!("Resuming playback");
            Ok(())
        } else {
            Err("No playback in progress".to_string())
        }
    }

    pub async fn stop(&self) -> Result<(), String> {
        if let Some(handle) = self.current_handle.lock().await.take() {
            handle.stop();
            tracing::info!("Stopping playback");
            Ok(())
        } else {
            Err("No playback in progress".to_string())
        }
    }

    pub async fn get_current_handle(&self) -> Option<PlaybackHandle> {
        self.current_handle.lock().await.clone()
    }
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Playback manager - handles playback state and queue
pub struct PlaybackManager {
    queue: Arc<Mutex<PlaybackQueue>>,
    info: Arc<Mutex<PlaybackInfo>>,
    audio_player: Arc<AudioPlayer>,
}

impl PlaybackManager {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(Mutex::new(PlaybackQueue::new())),
            info: Arc::new(Mutex::new(PlaybackInfo::default())),
            audio_player: Arc::new(AudioPlayer::new()),
        }
    }

    /// Set current track and start playing
    pub async fn play_track(&self, track: Track) {
        let mut info = self.info.lock().await;
        info.current_track = Some(track.clone());
        info.state = PlaybackState::Playing;
        info.position_ms = 0;
        drop(info); // Release the lock

        // Attempt to play the audio
        if let Some(url) = &track.url {
            match self.audio_player.play_url(url).await {
                Ok(handle) => {
                    // Spawn a task to update playback position from the audio player
                    let info_clone = self.info.clone();
                    tokio::spawn(async move {
                        loop {
                            let position = handle.get_position();
                            let duration = handle.get_duration();
                            let should_stop = handle.should_stop();

                            let mut info = info_clone.lock().await;
                            info.position_ms = position;
                            if duration > 0 && info.current_track.is_some() {
                                info.current_track.as_mut().unwrap().duration_ms = duration;
                            }

                            if should_stop || duration == 0 {
                                break;
                            }

                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to play audio: {}", e);
                }
            }
        } else {
            tracing::warn!("No playback URL available for track: {}", track.title);
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
        drop(info);

        // Resume audio playback
        if let Err(e) = self.audio_player.resume().await {
            tracing::warn!("Failed to resume playback: {}", e);
        }
    }

    /// Pause playback
    pub async fn pause(&self) {
        let mut info = self.info.lock().await;
        info.state = PlaybackState::Paused;
        drop(info);

        // Pause audio playback
        if let Err(e) = self.audio_player.pause().await {
            tracing::warn!("Failed to pause playback: {}", e);
        }
    }

    /// Toggle play/pause
    pub async fn toggle_play_pause(&self) {
        let info_arc = self.info.clone();
        let player = self.audio_player.clone();

        // Determine new state based on current state
        let new_state = {
            let info = info_arc.lock().await;
            match info.state {
                PlaybackState::Playing => PlaybackState::Paused,
                PlaybackState::Paused | PlaybackState::Stopped => PlaybackState::Playing,
            }
        };

        // Update audio player
        match new_state {
            PlaybackState::Playing => {
                if let Err(e) = player.resume().await {
                    tracing::warn!("Failed to resume playback: {}", e);
                }
            }
            PlaybackState::Paused => {
                if let Err(e) = player.pause().await {
                    tracing::warn!("Failed to pause playback: {}", e);
                }
            }
            PlaybackState::Stopped => {}
        }

        // Update playback state
        let mut info = info_arc.lock().await;
        info.state = new_state;
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
