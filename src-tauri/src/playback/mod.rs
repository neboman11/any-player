/// Playback management
use crate::models::{PlaybackInfo, PlaybackState, RepeatMode, Track};
use crate::providers::{spotify::SPOTIFY_CLIENT_ID, ProviderRegistry};
use rodio::{Decoder, OutputStream, Sink, Source};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};

// Librespot imports for premium Spotify streaming via session-based OAuth
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::config::SessionConfig;
use librespot_core::session::Session;
use librespot_core::spotify_id::SpotifyId;
use librespot_playback::audio_backend::Sink as LibrespotSink;
use librespot_playback::config::PlayerConfig;
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::mixer::VolumeGetter;
use librespot_playback::player::Player as LibrespotPlayer;
// Use conservative defaults for sample rate and channels when converting to rodio
const LIBRESPOT_FALLBACK_SAMPLE_RATE: u32 = 44100;
const LIBRESPOT_FALLBACK_CHANNELS: u16 = 2;

pub mod spotify_session;
pub use spotify_session::SpotifySessionManager;

// Simple volume getter that returns 1.0 (no attenuation)
struct NoOpVolume {}

impl VolumeGetter for NoOpVolume {
    fn attenuation_factor(&self) -> f64 {
        1.0
    }
}

/// A minimal librespot Sink implementation that writes PCM bytes into a `rodio::Sink`.
struct RodioSink {
    // Keep the stream alive
    _stream: OutputStream,
    // Wrap sink in Arc<Mutex> so we can share it with PlaybackHandle for direct control
    sink: Arc<Mutex<Sink>>,
}

impl RodioSink {
    fn new() -> Result<Self, String> {
        let (stream, _handle) = OutputStream::try_default()
            .map_err(|e| format!("Failed to open audio output: {}", e))?;
        let handle = _handle;
        let sink = Sink::try_new(&handle).map_err(|e| format!("Failed to create sink: {}", e))?;
        Ok(Self {
            _stream: stream,
            sink: Arc::new(Mutex::new(sink)),
        })
    }

    /// Get a cloneable handle to the sink for direct control
    fn get_sink_handle(&self) -> Arc<Mutex<Sink>> {
        Arc::clone(&self.sink)
    }
}

impl LibrespotSink for RodioSink {
    fn write(
        &mut self,
        packet: AudioPacket,
        _converter: &mut Converter,
    ) -> librespot_playback::audio_backend::SinkResult<()> {
        match packet {
            AudioPacket::Samples(samples_f64) => {
                // Convert f64 samples [-1.0, 1.0] to i16 PCM
                let mut samples_i16: Vec<i16> = Vec::with_capacity(samples_f64.len());
                for &s in samples_f64.iter() {
                    let scaled = (s * 32767.0).round();
                    let clamped = if scaled < i16::MIN as f64 {
                        i16::MIN
                    } else if scaled > i16::MAX as f64 {
                        i16::MAX
                    } else {
                        scaled as i16
                    };
                    samples_i16.push(clamped);
                }

                let source = rodio::buffer::SamplesBuffer::new(
                    LIBRESPOT_FALLBACK_CHANNELS,
                    LIBRESPOT_FALLBACK_SAMPLE_RATE,
                    samples_i16,
                );
                // Lock the sink to append samples
                if let Ok(sink) = self.sink.try_lock() {
                    sink.append(source);
                }
            }
            _ => {
                // Non-sample packets (e.g. encoded/ogg data) would require decoding;
                // skip for now to keep the sink implementation simple.
            }
        }

        Ok(())
    }

    fn start(&mut self) -> librespot_playback::audio_backend::SinkResult<()> {
        tracing::debug!("RodioSink::start() called - resuming playback");
        if let Ok(sink) = self.sink.try_lock() {
            sink.play();
        }
        Ok(())
    }

    fn stop(&mut self) -> librespot_playback::audio_backend::SinkResult<()> {
        tracing::debug!("RodioSink::stop() called - pausing playback");
        if let Ok(sink) = self.sink.try_lock() {
            sink.pause();
        }
        Ok(())
    }
}

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
    /// Direct reference to rodio sink for immediate pause/play control
    /// Using Arc<Mutex<Option<...>>> for interior mutability
    sink: Arc<Mutex<Option<Arc<Mutex<Sink>>>>>,
}

impl PlaybackHandle {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            position_ms: Arc::new(AtomicU64::new(0)),
            duration_ms: Arc::new(AtomicU64::new(0)),
            is_paused: Arc::new(AtomicBool::new(false)),
            sink: Arc::new(Mutex::new(None)),
        }
    }

    /// Set the sink handle for direct pause/play control
    pub async fn set_sink(&self, sink: Arc<Mutex<Sink>>) {
        let mut sink_opt = self.sink.lock().await;
        *sink_opt = Some(sink);
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    pub fn pause(&self) {
        self.is_paused.store(true, Ordering::SeqCst);
        // Directly pause the rodio sink for immediate effect
        let sink_arc = self.sink.clone();
        tokio::spawn(async move {
            let sink_opt = sink_arc.lock().await;
            if let Some(sink_handle) = sink_opt.as_ref() {
                if let Ok(s) = sink_handle.try_lock() {
                    tracing::warn!("PlaybackHandle::pause() - directly pausing rodio sink");
                    s.pause();
                }
            }
        });
    }

    pub fn resume(&self) {
        self.is_paused.store(false, Ordering::SeqCst);
        // Directly resume the rodio sink for immediate effect
        let sink_arc = self.sink.clone();
        tokio::spawn(async move {
            let sink_opt = sink_arc.lock().await;
            if let Some(sink_handle) = sink_opt.as_ref() {
                if let Ok(s) = sink_handle.try_lock() {
                    tracing::warn!("PlaybackHandle::resume() - directly resuming rodio sink");
                    s.play();
                }
            }
        });
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
#[derive(Clone)]
pub struct AudioPlayer {
    current_handle: Arc<Mutex<Option<PlaybackHandle>>>,
    /// Store the active librespot player to keep it alive during playback
    active_player: Arc<Mutex<Option<Arc<LibrespotPlayer>>>>,
}

/// Queue for managing playback
#[derive(Debug, Clone)]
pub struct PlaybackQueue {
    /// All tracks in the queue
    pub tracks: Vec<Track>,
    /// Current position in queue
    pub current_index: usize,
    /// Shuffle order: maps shuffle position to original queue index
    /// When shuffle is enabled, this array defines the play order
    pub shuffle_order: Vec<usize>,
}

impl PlaybackQueue {
    pub fn new() -> Self {
        Self {
            tracks: Vec::new(),
            current_index: 0,
            shuffle_order: Vec::new(),
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
        self.shuffle_order.clear();
    }

    pub fn current_track(&self) -> Option<&Track> {
        if self.current_index < self.tracks.len() {
            Some(&self.tracks[self.current_index])
        } else {
            None
        }
    }

    pub fn next_track(&mut self) -> Option<&Track> {
        if !self.tracks.is_empty() && self.current_index < self.tracks.len() - 1 {
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

    /// Generate a new shuffle order for the current tracks
    /// This creates a randomized order of indices from 0..tracks.len()
    pub fn generate_shuffle_order(&mut self) {
        use rand::seq::SliceRandom;
        use rand::thread_rng;

        let track_count = self.tracks.len();
        if track_count == 0 {
            self.shuffle_order.clear();
            return;
        }

        // Create a vector of indices
        let mut indices: Vec<usize> = (0..track_count).collect();

        // Shuffle the indices
        let mut rng = thread_rng();
        indices.shuffle(&mut rng);

        self.shuffle_order = indices;
        tracing::info!("Generated shuffle order: {:?}", self.shuffle_order);
    }

    /// Clear the shuffle order (used when shuffle is disabled)
    pub fn clear_shuffle_order(&mut self) {
        self.shuffle_order.clear();
    }

    /// Validate that all indices in shuffle_order are within bounds
    /// and regenerate if invalid. This ensures robustness if tracks are
    /// modified after shuffle order is generated.
    fn validate_shuffle_order(&mut self) {
        if self.shuffle_order.is_empty() {
            return;
        }
        
        let track_count = self.tracks.len();
        let has_invalid = self.shuffle_order.iter().any(|&idx| idx >= track_count);
        
        if has_invalid {
            tracing::warn!(
                "Shuffle order contains invalid indices (track count: {}), regenerating",
                track_count
            );
            self.generate_shuffle_order();
        }
    }

    /// Get the current track respecting shuffle mode
    pub fn current_track_shuffled(&self, shuffle_enabled: bool) -> Option<&Track> {
        if shuffle_enabled && !self.shuffle_order.is_empty() {
            // In shuffle mode, map current_index through shuffle_order
            if self.current_index < self.shuffle_order.len() {
                let actual_index = self.shuffle_order[self.current_index];
                // Bounds check to handle edge cases where shuffle_order may be stale
                if actual_index < self.tracks.len() {
                    return Some(&self.tracks[actual_index]);
                } else {
                    tracing::warn!(
                        "Shuffle order index {} out of bounds (track count: {})",
                        actual_index,
                        self.tracks.len()
                    );
                }
            }
            None
        } else {
            // Normal mode
            self.current_track()
        }
    }

    /// Move to the next track respecting shuffle mode
    pub fn next_track_shuffled(&mut self, shuffle_enabled: bool) -> Option<&Track> {
        if shuffle_enabled && !self.shuffle_order.is_empty() {
            self.validate_shuffle_order();
            // In shuffle mode, navigate through shuffle_order
            if self.current_index < self.shuffle_order.len() - 1 {
                self.current_index += 1;
                let actual_index = self.shuffle_order[self.current_index];
                // Bounds check to handle edge cases
                if actual_index < self.tracks.len() {
                    return Some(&self.tracks[actual_index]);
                } else {
                    tracing::warn!(
                        "Shuffle order index {} out of bounds (track count: {})",
                        actual_index,
                        self.tracks.len()
                    );
                }
            }
            None
        } else {
            // Normal mode
            self.next_track()
        }
    }

    /// Move to the previous track respecting shuffle mode
    pub fn previous_shuffled(&mut self, shuffle_enabled: bool) -> Option<&Track> {
        if shuffle_enabled && !self.shuffle_order.is_empty() {
            self.validate_shuffle_order();
            // In shuffle mode, navigate through shuffle_order
            if self.current_index > 0 {
                self.current_index -= 1;
                let actual_index = self.shuffle_order[self.current_index];
                // Bounds check to handle edge cases
                if actual_index < self.tracks.len() {
                    return Some(&self.tracks[actual_index]);
                } else {
                    tracing::warn!(
                        "Shuffle order index {} out of bounds (track count: {})",
                        actual_index,
                        self.tracks.len()
                    );
                }
            }
            None
        } else {
            // Normal mode
            self.previous()
        }
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
            active_player: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn play_url(
        &self,
        url: &str,
        auth_headers: Option<Vec<(String, String)>>,
    ) -> Result<PlaybackHandle, String> {
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
                move || Self::play_audio_blocking(&url, &handle, auth_headers)
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

    fn play_audio_blocking(
        url: &str,
        handle: &PlaybackHandle,
        auth_headers: Option<Vec<(String, String)>>,
    ) -> Result<(), String> {
        // Check if URL is a spotify: URI - would require session for full playback
        if url.starts_with("spotify:track:") {
            return Err(
                "Session not available for Spotify track. Ensure spotify_session is initialized."
                    .to_string(),
            );
        }

        // Check if URL is valid (should be HTTP(S))
        if !url.starts_with("http") {
            return Err(format!(
                "Invalid playback URL format. Expected HTTP URL or spotify: URI, got: {}",
                url
            ));
        }

        Self::play_http_audio(url, handle, auth_headers)
    }

    fn play_http_audio(
        url: &str,
        handle: &PlaybackHandle,
        auth_headers: Option<Vec<(String, String)>>,
    ) -> Result<(), String> {
        // Get audio output stream
        let (_stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| format!("Failed to get audio output: {}", e))?;

        // Fetch audio data from URL
        let client = reqwest::blocking::Client::new();
        let mut request = client
            .get(url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)");

        // Add authentication headers if provided (e.g., for Jellyfin)
        if let Some(headers) = auth_headers {
            for (key, value) in headers {
                request = request.header(key, value);
            }
        }

        let response = request
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
        let mut pause_time: Option<Instant> = None;
        let mut accumulated_pause_duration = Duration::from_secs(0);

        loop {
            if handle.should_stop() {
                break;
            }

            // Handle pause/resume and track pause duration
            let is_paused = handle.is_paused();
            if is_paused {
                sink.pause();
                if pause_time.is_none() {
                    pause_time = Some(Instant::now());
                }
            } else {
                sink.play();
                if let Some(paused_at) = pause_time {
                    accumulated_pause_duration += paused_at.elapsed();
                    pause_time = None;
                }
            }

            // Update position only when not paused
            let elapsed = {
                let start_elapsed = start.elapsed();
                if let Some(paused_at) = pause_time {
                    // Currently paused: use time up to pause, guarding against underflow
                    let paused_elapsed = paused_at.elapsed();
                    let effective_elapsed =
                        start_elapsed.saturating_sub(accumulated_pause_duration + paused_elapsed);
                    effective_elapsed.as_millis() as u64
                } else {
                    // Not paused: use full elapsed time minus accumulated pause duration,
                    // guarding against underflow
                    let effective_elapsed =
                        start_elapsed.saturating_sub(accumulated_pause_duration);
                    effective_elapsed.as_millis() as u64
                }
            };

            if elapsed != handle.get_position() {
                handle.set_position(elapsed);
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
                tracing::info!("Track playback completed based on duration");
                handle.stop();
                break;
            }
        }

        sink.stop();
        Ok(())
    }

    /// Extract track ID from spotify: URI
    ///
    /// Handles formats like "spotify:track:3n3Ppam7vgaVa1iaRUc9Lp"
    /// Returns the base62 track ID if valid, None otherwise
    #[allow(dead_code)]
    fn extract_track_id(uri: &str) -> Option<String> {
        if uri.starts_with("spotify:track:") {
            uri.split(':').next_back().map(|s| s.to_string())
        } else if uri.contains('/') && uri.contains("track") {
            uri.split('/').next_back().map(|s| s.to_string())
        } else if !uri.contains(':') && !uri.contains('/') && uri.len() == 22 {
            // Likely a raw track ID
            Some(uri.to_string())
        } else {
            None
        }
    }

    /// Play a Spotify track via librespot using OAuth session authentication
    ///
    /// This implements full-track Spotify playback like spotify-player does:
    /// 1. Fetch track metadata from Web API via rspotify
    /// 2. Retrieve OAuth token from session
    /// 3. Use librespot to stream the actual audio
    async fn play_spotify_track(
        &self,
        track_id: &str,
        handle: &PlaybackHandle,
        providers: Arc<Mutex<ProviderRegistry>>,
        session: Option<Session>,
    ) -> Result<(), String> {
        // Extract clean track ID, stripping all spotify:track: prefixes
        let mut clean_id = track_id.trim_start_matches("spotify:track:").to_string();

        // Handle URL format
        if clean_id.contains("/track/") {
            clean_id = clean_id
                .split('/')
                .next_back()
                .unwrap_or(&clean_id)
                .to_string();
        }

        tracing::info!(
            "Starting Spotify track playback via librespot: {}",
            clean_id
        );

        let handle_clone = handle.clone();
        let providers_clone = providers.clone();
        let track_id_for_fetch = clean_id.clone();
        let audio_player_clone = self.clone();

        tokio::spawn(async move {
            #[allow(clippy::redundant_closure_call)]
            let result = (|| async {
                // Lock the providers registry to fetch track info and get session
                let providers_locked = providers_clone.lock().await;

                // Fetch track info from Spotify using rspotify to get metadata
                let track = providers_locked
                    .get_spotify_track(&track_id_for_fetch)
                    .await
                    .map_err(|e| format!("Failed to fetch track info: {}", e))?;

                tracing::info!("Track fetched: {} by {}", track.title, track.artist);

                // Set duration from track metadata
                if track.duration_ms > 0 {
                    handle_clone.set_duration(track.duration_ms);
                    tracing::info!("Track duration: {}ms", track.duration_ms);
                }

                // If a session was provided (manager created it), use it; otherwise obtain token and create one
                if let Some(sess) = session {
                    tracing::info!("Using existing librespot Session from manager");
                    Self::play_spotify_with_librespot(
                        &audio_player_clone,
                        &track_id_for_fetch,
                        &handle_clone,
                        sess,
                    )
                    .await?;
                } else {
                    // Get the OAuth access token from the providers
                    let access_token = providers_locked
                        .get_spotify_access_token()
                        .await
                        .ok_or("No Spotify access token available".to_string())?;

                    drop(providers_locked); // Release lock before async operations

                    tracing::info!("Retrieved OAuth token for Spotify playback");

                    // Create and connect librespot session
                    match Self::create_librespot_session(&access_token).await {
                        Ok(session) => {
                            tracing::info!("Librespot session created successfully");
                            // Use real streaming via librespot Player
                            Self::play_spotify_with_librespot(
                                &audio_player_clone,
                                &track_id_for_fetch,
                                &handle_clone,
                                session,
                            )
                            .await?;
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to create librespot session: {}, using simulation",
                                e
                            );
                            // Fallback to simulation playback
                            Self::play_audio_stream(
                                &format!("spotify:track:{}", track_id_for_fetch),
                                &handle_clone,
                                None,
                            )
                            .await?;
                        }
                    }
                }

                Ok::<(), String>(())
            })();

            match result.await {
                Ok(()) => {
                    tracing::info!("Spotify track playback completed");
                }
                Err(e) => {
                    tracing::error!("Spotify playback error: {}", e);
                }
            }
        });

        Ok(())
    }

    /// Create a librespot Session with OAuth credentials
    async fn create_librespot_session(access_token: &str) -> Result<Session, String> {
        tracing::info!("Creating librespot Session with OAuth token...");

        // Create a session configuration and credentials that use the OAuth token
        let session_config = SessionConfig::default();

        // Use the OAuth access token as credentials; librespot provides a dedicated constructor for this
        let credentials = Credentials::with_access_token(access_token.to_string());

        // Create a simple cache (no paths) - this is optional but the Session API expects an Option<Cache>
        let cache = Cache::new::<&std::path::Path>(None, None, None, None)
            .map_err(|e| format!("Failed to create librespot cache: {}", e))?;

        let session = Session::new(session_config, Some(cache));

        // Connect the Session (async)
        match Session::connect(&session, credentials, false).await {
            Ok(()) => {
                tracing::info!("Librespot Session connected");
                Ok(session)
            }
            Err(e) => Err(format!("Failed to connect librespot Session: {:?}", e)),
        }
    }

    /// Play a Spotify track using librespot's real streaming
    async fn play_spotify_with_librespot(
        audio_player: &AudioPlayer,
        track_id: &str,
        handle: &PlaybackHandle,
        _session: Session,
    ) -> Result<(), String> {
        tracing::info!(
            "Starting real Spotify playback with librespot for: {}",
            track_id
        );

        // Parse the Spotify track ID
        let _spotify_id = SpotifyId::from_base62(track_id)
            .map_err(|e| format!("Invalid Spotify track ID: {:?}", e))?;

        tracing::info!("Track ID parsed successfully");

        // Build a player and play the requested track using the provided session.
        // We'll use a small rodio-based sink implementation for audio output.

        // Build player config and a no-op volume getter
        let config = PlayerConfig::default();
        let volume_getter = Box::new(NoOpVolume {});

        // Create a shared sink handle that both the player and handle can access
        // We create it here and share it
        let shared_sink = Arc::new(Mutex::new(None::<Arc<Mutex<Sink>>>));
        let shared_sink_for_builder = shared_sink.clone();

        // Sink builder: create a new RodioSink and store its handle
        let sink_builder = move || -> Box<dyn LibrespotSink> {
            let rodio_sink = RodioSink::new().expect("Failed to create RodioSink");
            let sink_handle = rodio_sink.get_sink_handle();

            // Store the sink handle so we can access it later
            if let Ok(mut shared) = shared_sink_for_builder.try_lock() {
                *shared = Some(sink_handle);
            }

            Box::new(rodio_sink)
        };

        // Create the player (this will call sink_builder once)
        let player = LibrespotPlayer::new(config, _session.clone(), volume_getter, sink_builder);

        // Load and play the track
        let spotify_id = SpotifyId::from_base62(track_id)
            .map_err(|e| format!("Invalid Spotify track ID on load: {:?}", e))?;

        player.load(
            librespot_core::SpotifyUri::Track { id: spotify_id },
            true,
            0,
        );

        // Wait a moment for the sink_builder to be called during load
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Now retrieve the sink handle and set it in the playback handle
        if let Ok(shared) = shared_sink.try_lock() {
            if let Some(sink_handle) = shared.as_ref() {
                // Set the sink handle in the playback handle for direct control
                handle.set_sink(sink_handle.clone()).await;
                tracing::info!(
                    "Sink handle connected to PlaybackHandle for direct pause/play control"
                );
            } else {
                tracing::warn!("Sink handle not available after load - sink_builder may not have been called yet");
            }
        }

        // Store the player to keep it alive during playback
        {
            let mut active = audio_player.active_player.lock().await;
            *active = Some(player);
        }

        // Spawn a task to monitor playback and track progress
        let active_player = audio_player.active_player.clone();
        let handle_clone = handle.clone();
        tokio::spawn(async move {
            let start_time = Instant::now();
            let mut pause_time: Option<Instant> = None;
            let mut accumulated_pause_duration = Duration::from_secs(0);
            let mut last_paused_state = false;

            // Wait for the player to finish or be stopped
            loop {
                if handle_clone.should_stop() {
                    tracing::info!("Playback stopped by user");
                    break;
                }

                // Check for pause state changes
                let is_paused = handle_clone.is_paused();
                if is_paused != last_paused_state {
                    tracing::warn!(
                        "Pause state changed: was={}, now={}",
                        last_paused_state,
                        is_paused
                    );

                    if is_paused {
                        // Entering pause state
                        tracing::info!("Playback paused - stopping position updates");
                        pause_time = Some(Instant::now());
                        // Note: Actual audio pause is handled by PlaybackHandle::pause()
                        // which directly controls the rodio sink
                    } else {
                        // Exiting pause state (resuming)
                        tracing::info!("Playback resumed - restarting position updates");
                        if let Some(paused_at) = pause_time {
                            accumulated_pause_duration += paused_at.elapsed();
                            pause_time = None;
                        }
                        // Note: Actual audio resume is handled by PlaybackHandle::resume()
                        // which directly controls the rodio sink
                    }
                    last_paused_state = is_paused;
                }

                // Calculate current position accounting for pauses
                let elapsed = if let Some(paused_at) = pause_time {
                    // Currently paused: use time up to pause
                    start_time.elapsed() - accumulated_pause_duration - paused_at.elapsed()
                } else {
                    // Not paused: use full elapsed time minus accumulated pause duration
                    start_time.elapsed() - accumulated_pause_duration
                };

                let position_ms = elapsed.as_millis() as u64;
                handle_clone.set_position(position_ms);

                // Check if we've reached the end based on duration
                let duration_ms = handle_clone.get_duration();
                if duration_ms > 0 && position_ms >= duration_ms {
                    tracing::info!("Track playback completed based on duration");
                    handle_clone.stop();
                    break;
                }

                // Also check if player is still active
                {
                    let active_lock = active_player.lock().await;
                    if active_lock.is_none() {
                        tracing::warn!("Player no longer active (error or stopped externally)");
                        // Set stop flag so monitoring task can detect completion
                        handle_clone.stop();
                        break;
                    }
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            // Clean up the player when done
            {
                let mut active = active_player.lock().await;
                *active = None;
                tracing::info!("Librespot player cleaned up");
            }
        });

        Ok(())
    }

    /// Helper method to stream audio from a URL or Spotify URI
    async fn play_audio_stream(
        url: &str,
        handle: &PlaybackHandle,
        auth_headers: Option<Vec<(String, String)>>,
    ) -> Result<(), String> {
        // For Spotify URIs, we provide full-track duration simulation
        // In a full implementation with real librespot, this would stream actual audio
        if url.starts_with("spotify:track:") {
            Self::simulate_playback(handle).await
        } else {
            // For HTTP URLs (previews), use actual playback
            let url_copy = url.to_string();
            let handle_clone = handle.clone();

            tokio::task::spawn_blocking(move || {
                Self::play_http_audio(&url_copy, &handle_clone, auth_headers)
            })
            .await
            .map_err(|e| format!("Playback task failed: {}", e))?
        }
    }

    /// Simulate realistic playback timing based on track duration
    async fn simulate_playback(handle: &PlaybackHandle) -> Result<(), String> {
        let start_time = Instant::now();
        let mut last_position = 0u64;
        let duration = handle.get_duration();

        tracing::info!(
            "Simulating Spotify track playback (duration: {}ms)",
            duration
        );

        loop {
            if handle.should_stop() {
                tracing::info!("Playback stopped by user");
                break;
            }

            // Update position based on elapsed time
            let elapsed = start_time.elapsed();
            let elapsed_ms = elapsed.as_millis() as u64;

            if elapsed_ms != last_position {
                handle.set_position(elapsed_ms);
                last_position = elapsed_ms;
            }

            // Check if we've reached the end of track duration
            if duration > 0 && elapsed_ms >= duration {
                tracing::info!("Track playback completed");
                handle.stop();
                break;
            }

            // Sleep briefly to avoid busy waiting
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

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
    spotify_session: Arc<SpotifySessionManager>,
    providers: Arc<Mutex<ProviderRegistry>>,
    track_complete_tx: mpsc::UnboundedSender<()>,
    track_complete_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<()>>>>,
    monitoring_task_abort: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
}

impl PlaybackManager {
    pub fn new(providers: Arc<Mutex<ProviderRegistry>>) -> Self {
        // Create a channel for track completion events
        let (track_complete_tx, track_complete_rx) = mpsc::unbounded_channel::<()>();

        Self {
            queue: Arc::new(Mutex::new(PlaybackQueue::new())),
            info: Arc::new(Mutex::new(PlaybackInfo::default())),
            audio_player: Arc::new(AudioPlayer::new()),
            spotify_session: Arc::new(SpotifySessionManager::new(SPOTIFY_CLIENT_ID.to_string())),
            providers,
            track_complete_tx,
            track_complete_rx: Arc::new(Mutex::new(Some(track_complete_rx))),
            monitoring_task_abort: Arc::new(Mutex::new(None)),
        }
    }

    /// Take the track completion receiver.
    ///
    /// This *must* be called exactly once during application setup to start
    /// receiving track completion events. Calling it more than once will
    /// return `None` and is considered a programming error.
    ///
    /// # Usage
    /// Call this method once during application initialization to get the receiver,
    /// then use it to listen for track completion events and trigger auto-advance.
    pub async fn take_completion_receiver(&self) -> Option<mpsc::UnboundedReceiver<()>> {
        let mut rx_opt = self.track_complete_rx.lock().await;
        rx_opt.take()
    }

    /// Set current track and start playing
    pub async fn play_track(&self, track: Track) {
        tracing::info!("play_track called for: {} ({})", track.title, track.id);

        // Abort any existing monitoring task before starting a new one
        {
            let mut abort_handle = self.monitoring_task_abort.lock().await;
            if let Some(handle) = abort_handle.take() {
                handle.abort();
                tracing::debug!(
                    "Aborted previous monitoring task for new track: {}",
                    track.title
                );
            }
        }

        // Update queue's current_index if this track is in the queue
        {
            let mut queue = self.queue.lock().await;
            // Find the track in the queue and set current_index
            if let Some(index) = queue.tracks.iter().position(|t| t.id == track.id) {
                queue.current_index = index;
                tracing::debug!(
                    "Set queue current_index to {} for track: {}",
                    index,
                    track.title
                );
            }
        }

        let mut info = self.info.lock().await;
        info.current_track = Some(track.clone());
        info.state = PlaybackState::Playing;
        info.position_ms = 0;
        drop(info); // Release the lock

        // Attempt to play the audio
        if let Some(url) = &track.url {
            // Check if this is a Spotify URI requiring premium playback
            if url.starts_with("spotify:track:") {
                // Verify session is initialized before attempting playback
                if !self.spotify_session.is_initialized().await {
                    tracing::error!(
                        "Cannot play Spotify track: session not initialized. URL: {}",
                        url
                    );
                    let mut info = self.info.lock().await;
                    info.state = PlaybackState::Stopped;
                    return;
                }

                // Premium user with session initialized - use librespot
                tracing::info!("Playing Spotify track via librespot: {}", url);
                let track_complete_tx = self.track_complete_tx.clone();
                let monitoring_abort = self.monitoring_task_abort.clone();
                match self.play_spotify_track(url).await {
                    Ok(handle) => {
                        // Spawn a task to update playback position from the audio player
                        let info_arc = self.info.clone();

                        let task = tokio::spawn(async move {
                            tracing::debug!("Spotify monitoring task started");
                            loop {
                                let position = handle.get_position();
                                let duration = handle.get_duration();
                                let should_stop = handle.should_stop();
                                let is_paused = handle.is_paused();

                                {
                                    let mut info = info_arc.lock().await;
                                    info.position_ms = position;
                                    if duration > 0 && info.current_track.is_some() {
                                        info.current_track.as_mut().unwrap().duration_ms = duration;
                                    }

                                    // Update playback state based on pause status
                                    if is_paused {
                                        info.state = PlaybackState::Paused;
                                    } else if !should_stop {
                                        info.state = PlaybackState::Playing;
                                    }
                                }

                                // When track completes, send event to advance to next track
                                if should_stop {
                                    tracing::debug!(
                                        "Spotify monitoring task detected should_stop=true"
                                    );
                                    {
                                        let mut info = info_arc.lock().await;
                                        info.state = PlaybackState::Stopped;
                                    }

                                    tracing::info!(
                                        "Spotify track completed, sending auto-advance event"
                                    );
                                    let _ = track_complete_tx.send(());
                                    break;
                                }

                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                        });

                        // Store the abort handle immediately to prevent race conditions
                        // This ensures the task can be aborted before completion
                        {
                            let mut abort_handle = monitoring_abort.lock().await;
                            *abort_handle = Some(task.abort_handle());
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to play Spotify track: {}", e);
                        let mut info = self.info.lock().await;
                        info.state = PlaybackState::Stopped;
                    }
                }
            } else {
                // HTTP URL - play as normal
                let track_complete_tx = self.track_complete_tx.clone();
                let monitoring_abort = self.monitoring_task_abort.clone();
                match self
                    .audio_player
                    .play_url(url, track.auth_headers.clone())
                    .await
                {
                    Ok(handle) => {
                        // Spawn a task to update playback position from the audio player
                        let info_arc = self.info.clone();

                        let task = tokio::spawn(async move {
                            tracing::debug!("HTTP monitoring task started");
                            loop {
                                let position = handle.get_position();
                                let duration = handle.get_duration();
                                let should_stop = handle.should_stop();
                                let is_paused = handle.is_paused();

                                // Debug: Log every 10 seconds to confirm task is running
                                if position % 10000 < 100 {
                                    tracing::debug!(
                                        "HTTP monitor check: pos={}, dur={}, should_stop={}",
                                        position,
                                        duration,
                                        should_stop
                                    );
                                }

                                {
                                    let mut info = info_arc.lock().await;
                                    info.position_ms = position;
                                    if duration > 0 && info.current_track.is_some() {
                                        info.current_track.as_mut().unwrap().duration_ms = duration;
                                    }

                                    // Update playback state based on pause status
                                    if is_paused {
                                        info.state = PlaybackState::Paused;
                                    } else if !should_stop {
                                        info.state = PlaybackState::Playing;
                                    }
                                }

                                // When track completes, send event to advance to next track
                                if should_stop {
                                    tracing::debug!(
                                        "HTTP monitoring task detected should_stop=true"
                                    );
                                    {
                                        let mut info = info_arc.lock().await;
                                        info.state = PlaybackState::Stopped;
                                    }

                                    tracing::info!(
                                        "HTTP track completed, sending auto-advance event"
                                    );
                                    let _ = track_complete_tx.send(());
                                    break;
                                }

                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                        });

                        // Store the abort handle immediately to prevent race conditions
                        // This ensures the task can be aborted before completion
                        {
                            let mut abort_handle = monitoring_abort.lock().await;
                            *abort_handle = Some(task.abort_handle());
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to play audio: {}", e);
                        let mut info = self.info.lock().await;
                        info.state = PlaybackState::Stopped;
                    }
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
        // Get shuffle state from info
        let shuffle_enabled = {
            let info = self.info.lock().await;
            info.shuffle
        };

        let mut queue = self.queue.lock().await;
        let track_opt = queue.next_track_shuffled(shuffle_enabled);

        if let Some(track) = track_opt {
            let track_clone = track.clone();
            drop(queue); // Release the queue lock before calling play_track
            self.play_track(track_clone.clone()).await;
            Some(track_clone)
        } else {
            None
        }
    }

    /// Play previous track
    pub async fn previous_track(&self) -> Option<Track> {
        // Get shuffle state from info
        let shuffle_enabled = {
            let info = self.info.lock().await;
            info.shuffle
        };

        let mut queue = self.queue.lock().await;
        let track_opt = queue.previous_shuffled(shuffle_enabled);

        if let Some(track) = track_opt {
            let track_clone = track.clone();
            drop(queue); // Release the queue lock before calling play_track
            self.play_track(track_clone.clone()).await;
            Some(track_clone)
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
        let shuffle_enabled = info.shuffle;
        drop(info);

        // Generate or clear shuffle order based on new state
        let mut queue = self.queue.lock().await;
        if shuffle_enabled {
            // When enabling shuffle, generate a new shuffle order
            queue.generate_shuffle_order();
            // Reset to beginning of shuffled playlist
            queue.current_index = 0;
            tracing::info!("Shuffle enabled - generated new shuffle order");
        } else {
            // When disabling shuffle, clear the shuffle order
            queue.clear_shuffle_order();
            // Try to maintain the same track by finding it in the original order
            // This is best-effort - if we can't find it, just reset to 0
            queue.current_index = 0;
            tracing::info!("Shuffle disabled - cleared shuffle order");
        }
    }

    /// Set repeat mode
    pub async fn set_repeat_mode(&self, mode: RepeatMode) {
        let mut info = self.info.lock().await;
        info.repeat_mode = mode;
    }

    /// Get current playback info
    pub async fn get_info(&self) -> PlaybackInfo {
        let mut info = self.info.lock().await.clone();
        let queue = self.queue.lock().await;
        info.queue = queue.tracks.clone();
        info.current_index = queue.current_index;
        info.shuffle_order = queue.shuffle_order.clone();
        drop(queue);
        info
    }

    /// Get current queue length
    pub async fn queue_length(&self) -> usize {
        self.queue.lock().await.len()
    }

    /// Get the queue Arc for direct access (used internally)
    pub fn get_queue_arc(&self) -> Arc<Mutex<PlaybackQueue>> {
        Arc::clone(&self.queue)
    }

    /// Get current track
    pub async fn current_track(&self) -> Option<Track> {
        self.queue.lock().await.current_track().cloned()
    }

    /// Initialize the Spotify session with an OAuth access token
    ///
    /// This should be called after successful Spotify authentication
    /// to enable full track streaming for premium users.
    pub async fn initialize_spotify_session(&self, access_token: &str) -> Result<(), String> {
        self.spotify_session
            .initialize_with_oauth_token(access_token)
            .await
    }

    /// Check if Spotify session is initialized
    pub async fn is_spotify_session_ready(&self) -> bool {
        self.spotify_session.is_initialized().await
    }

    /// Play a Spotify track via librespot
    ///
    /// This method handles playback of full Spotify tracks for premium users.
    /// Returns a PlaybackHandle to control playback.
    pub async fn play_spotify_track(&self, track_id: &str) -> Result<PlaybackHandle, String> {
        // Check if session is initialized
        if !self.spotify_session.is_initialized().await {
            return Err(
                "Spotify session not initialized. Run initialize_spotify_session first."
                    .to_string(),
            );
        }

        let handle = PlaybackHandle::new();
        let track_id = track_id.to_string();

        // Store the handle
        {
            let mut current = self.audio_player.current_handle.lock().await;
            if let Some(old_handle) = current.take() {
                old_handle.stop();
            }
            *current = Some(handle.clone());
        }

        // Spawn task to play spotify track
        let handle_for_spawn = handle.clone();
        let audio_player = self.audio_player.clone();
        let providers = self.providers.clone();
        // Grab the session from the session manager and pass it into the audio player
        let spotify_session_clone = self.spotify_session.clone();
        tokio::spawn(async move {
            let session = spotify_session_clone.get_session().await;

            let result = audio_player
                .play_spotify_track(&track_id, &handle_for_spawn, providers, session)
                .await;

            match result {
                Ok(()) => {
                    tracing::info!("Spotify track playback completed");
                }
                Err(e) => {
                    tracing::error!("Spotify playback error: {}", e);
                }
            }
        });

        Ok(handle)
    }

    /// Close the Spotify session
    pub async fn close_spotify_session(&self) -> Result<(), String> {
        self.spotify_session.close_session().await
    }
}
