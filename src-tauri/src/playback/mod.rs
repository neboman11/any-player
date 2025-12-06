/// Playback management
use crate::models::{PlaybackInfo, PlaybackState, RepeatMode, Track};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Audio player for playback
pub struct AudioPlayer;

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
        Self {}
    }

    pub async fn play_url(&self, url: &str) -> Result<(), String> {
        let url = url.to_string();

        // Spawn a background task to play audio without blocking
        tokio::spawn(async move {
            tracing::info!("Starting audio playback from URL: {}", url);

            // Spawn blocking task since rodio is not async-aware
            let result = tokio::task::spawn_blocking({
                let url = url.clone();
                move || Self::play_audio_blocking(&url)
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

        Ok(())
    }

    fn play_audio_blocking(url: &str) -> Result<(), String> {
        use rodio::{Decoder, OutputStream, Source};
        use std::io::Cursor;

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

        // Get duration for logging
        let duration_secs = source.total_duration().map(|d| d.as_secs()).unwrap_or(0);

        tracing::info!("Playing audio (duration: ~{}s)", duration_secs);

        // Convert to f32 samples and play
        let source = source.convert_samples::<f32>();
        let _sink = stream_handle
            .play_raw(source)
            .map_err(|e| format!("Failed to play audio: {}", e))?;

        // Sleep until audio finishes (or for a maximum duration)
        // For Spotify preview URLs, this is typically 30 seconds
        let max_duration = std::time::Duration::from_secs(35);
        std::thread::sleep(max_duration);

        Ok(())
    }

    pub async fn pause(&self) -> Result<(), String> {
        tracing::info!("Pausing playback");
        Ok(())
    }

    pub async fn resume(&self) -> Result<(), String> {
        tracing::info!("Resuming playback");
        Ok(())
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
            if let Err(e) = self.audio_player.play_url(url).await {
                tracing::error!("Failed to play audio: {}", e);
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
