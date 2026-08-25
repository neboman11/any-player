/// Playback management
use crate::models::{PlaybackInfo, PlaybackState, RepeatMode, Track};
use crate::providers::{spotify::SPOTIFY_CLIENT_ID, ProviderRegistry};
use any_player_core::audio_normalization::{
    effective_output_volume, AdaptiveNormalizationState, AudioNormalizationSettings,
    AudioNormalizationSource, INTERNAL_NORMALIZATION_TARGET,
};
use rodio::{Decoder, OutputStream, Sink, Source};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};

pub mod spotify_connect;
pub mod spotify_session;
pub use spotify_connect::SpotifyConnectBridge;
pub use spotify_session::SpotifySessionManager;

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
    /// Last per-track normalization gain applied (NaN when unknown)
    normalization_gain_bits: Arc<std::sync::atomic::AtomicU32>,
}

impl PlaybackHandle {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            position_ms: Arc::new(AtomicU64::new(0)),
            duration_ms: Arc::new(AtomicU64::new(0)),
            is_paused: Arc::new(AtomicBool::new(false)),
            sink: Arc::new(Mutex::new(None)),
            normalization_gain_bits: Arc::new(std::sync::atomic::AtomicU32::new(
                f32::NAN.to_bits(),
            )),
        }
    }

    pub fn set_normalization_gain(&self, gain: f32) {
        self.normalization_gain_bits
            .store(gain.to_bits(), Ordering::SeqCst);
    }

    pub fn get_normalization_gain(&self) -> Option<f32> {
        let gain = f32::from_bits(self.normalization_gain_bits.load(Ordering::SeqCst));
        gain.is_finite().then_some(gain)
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
        // This works for both HTTP and Spotify playback since both use rodio for audio output
        let sink_arc = self.sink.clone();
        tokio::spawn(async move {
            let sink_opt = sink_arc.lock().await;
            if let Some(sink_handle) = sink_opt.as_ref() {
                if let Ok(s) = sink_handle.try_lock() {
                    tracing::info!("PlaybackHandle::pause() - pausing rodio sink");
                    s.pause();
                }
            }
        });
    }

    pub fn resume(&self) {
        self.is_paused.store(false, Ordering::SeqCst);
        // Directly resume the rodio sink for immediate effect
        // This works for both HTTP and Spotify playback since both use rodio for audio output
        let sink_arc = self.sink.clone();
        tokio::spawn(async move {
            let sink_opt = sink_arc.lock().await;
            if let Some(sink_handle) = sink_opt.as_ref() {
                if let Ok(s) = sink_handle.try_lock() {
                    tracing::info!("PlaybackHandle::resume() - resuming rodio sink");
                    s.play();
                }
            }
        });
    }

    pub fn set_volume(&self, volume: u32) {
        // Set volume on the rodio sink (0-100 scale converted to 0.0-1.0)
        let volume_f32 = (volume.min(100) as f32) / 100.0;
        let sink_arc = self.sink.clone();
        tokio::spawn(async move {
            let sink_opt = sink_arc.lock().await;
            if let Some(sink_handle) = sink_opt.as_ref() {
                if let Ok(s) = sink_handle.try_lock() {
                    tracing::info!(
                        "PlaybackHandle::set_volume() - setting volume to {}",
                        volume_f32
                    );
                    s.set_volume(volume_f32);
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

    /// Generate a shuffle order that keeps the first track at position 0
    /// This is used when playing from a specific track - we want that track first,
    /// then shuffle the rest
    pub fn generate_shuffle_order_keep_first(&mut self) {
        use rand::seq::SliceRandom;
        use rand::thread_rng;

        let track_count = self.tracks.len();
        if track_count == 0 {
            self.shuffle_order.clear();
            return;
        }

        if track_count == 1 {
            self.shuffle_order = vec![0];
            return;
        }

        // Create a vector starting with 0, then shuffle the rest
        let mut indices: Vec<usize> = vec![0];
        let mut rest: Vec<usize> = (1..track_count).collect();

        // Shuffle the remaining indices
        let mut rng = thread_rng();
        rest.shuffle(&mut rng);

        indices.extend(rest);
        self.shuffle_order = indices;
        tracing::info!(
            "Generated shuffle order (keeping first): {:?}",
            self.shuffle_order
        );
    }

    /// Clear the shuffle order (used when shuffle is disabled)
    pub fn clear_shuffle_order(&mut self) {
        self.shuffle_order.clear();
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

    /// Skip to a specific index in the queue
    /// The index represents the position in the displayed queue (respecting shuffle order)
    pub fn skip_to_queue_index(
        &mut self,
        queue_index: usize,
        shuffle_enabled: bool,
    ) -> Option<&Track> {
        if shuffle_enabled && !self.shuffle_order.is_empty() {
            // In shuffle mode, queue_index refers to position in shuffle_order
            // We need to add 1 because the queue displayed to the user starts after current track
            let target_shuffle_index = self.current_index + 1 + queue_index;

            if target_shuffle_index < self.shuffle_order.len() {
                self.current_index = target_shuffle_index;
                let actual_index = self.shuffle_order[self.current_index];

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
            // Normal mode - queue_index refers to position after current track
            let target_index = self.current_index + 1 + queue_index;

            if target_index < self.tracks.len() {
                self.current_index = target_index;
                return self.current_track();
            }
            None
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
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn play_url(
        &self,
        url: &str,
        auth_headers: Option<Vec<(String, String)>>,
        volume: u32,
        normalize_enabled: bool,
        normalize_target: u32,
        normalize_strict_mode: bool,
        source: crate::models::Source,
        preloaded_audio_bytes: Option<Vec<u8>>,
        precomputed_track_gain: Option<f32>,
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
                move || {
                    Self::play_audio_blocking(
                        &url,
                        &handle,
                        auth_headers,
                        volume,
                        normalize_enabled,
                        normalize_target,
                        normalize_strict_mode,
                        source,
                        preloaded_audio_bytes,
                        precomputed_track_gain,
                    )
                }
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

    #[allow(clippy::too_many_arguments)]
    fn play_audio_blocking(
        url: &str,
        handle: &PlaybackHandle,
        auth_headers: Option<Vec<(String, String)>>,
        volume: u32,
        normalize_enabled: bool,
        normalize_target: u32,
        normalize_strict_mode: bool,
        source: crate::models::Source,
        preloaded_audio_bytes: Option<Vec<u8>>,
        precomputed_track_gain: Option<f32>,
    ) -> Result<(), String> {
        // Spotify URIs are handled by the dedicated shared-engine Spotify path.
        if url.starts_with("spotify:track:") {
            return Err("Spotify URI is not supported in HTTP playback path".to_string());
        }

        // Check if URL is valid (should be HTTP(S))
        if !url.starts_with("http") {
            return Err(format!(
                "Invalid playback URL format. Expected HTTP URL or spotify: URI, got: {}",
                url
            ));
        }

        Self::play_http_audio(
            url,
            handle,
            auth_headers,
            volume,
            normalize_enabled,
            normalize_target,
            normalize_strict_mode,
            source,
            preloaded_audio_bytes,
            precomputed_track_gain,
        )
    }

    fn target_rms_from_percent(target: u32) -> f32 {
        let normalized = (target.min(100) as f32) / 100.0;
        0.04 + (normalized * 0.18)
    }

    fn compute_track_normalization_gain(bytes: &[u8], target: u32) -> f32 {
        const MIN_SAMPLES: usize = 4_096;
        const MAX_SAMPLES: usize = 44_100 * 2 * 6;

        let decoder = match Decoder::new(Cursor::new(bytes.to_vec())) {
            Ok(decoder) => decoder,
            Err(error) => {
                tracing::warn!("Failed to decode track for loudness analysis: {}", error);
                return 1.0;
            }
        };

        let mut sum_sq = 0.0_f64;
        let mut peak = 0.0_f32;
        let mut count = 0usize;

        for sample in decoder.convert_samples::<f32>().take(MAX_SAMPLES) {
            let amplitude = sample.abs();
            peak = peak.max(amplitude);
            let amplitude_f64 = f64::from(amplitude);
            sum_sq += amplitude_f64 * amplitude_f64;
            count += 1;
        }

        if count < MIN_SAMPLES {
            return 1.0;
        }

        let rms = (sum_sq / (count as f64)).sqrt() as f32;
        if rms <= 0.0005 {
            return 1.0;
        }

        let target_rms = Self::target_rms_from_percent(target);
        let mut gain = (target_rms / rms).clamp(0.4, 3.0);

        if peak > 0.0 {
            gain = gain.min(0.98 / peak);
        }

        let clamped_gain = gain.clamp(0.25, 3.0);
        tracing::debug!(
            "Track normalization analysis: samples={}, rms={:.5}, peak={:.5}, target_rms={:.5}, gain={:.3}",
            count,
            rms,
            peak,
            target_rms,
            clamped_gain
        );
        clamped_gain
    }

    #[allow(clippy::too_many_arguments)]
    fn play_http_audio(
        url: &str,
        handle: &PlaybackHandle,
        auth_headers: Option<Vec<(String, String)>>,
        volume: u32,
        normalize_enabled: bool,
        normalize_target: u32,
        normalize_strict_mode: bool,
        source: crate::models::Source,
        preloaded_audio_bytes: Option<Vec<u8>>,
        precomputed_track_gain: Option<f32>,
    ) -> Result<(), String> {
        // Get audio output stream
        let (_stream, stream_handle) = OutputStream::try_default()
            .map_err(|e| format!("Failed to get audio output: {}", e))?;

        let bytes: Vec<u8> = if let Some(preloaded) = preloaded_audio_bytes {
            tracing::debug!("Using preloaded audio bytes for immediate playback start");
            preloaded
        } else {
            let client = reqwest::blocking::Client::new();
            let mut request = client
                .get(url)
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)");

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

            response
                .bytes()
                .map_err(|e| format!("Failed to read response body: {}", e))?
                .to_vec()
        };

        let track_gain = if normalize_enabled {
            if let Some(precomputed) = precomputed_track_gain {
                precomputed
            } else {
                let skip_heavy_analysis =
                    source == crate::models::Source::Plex && !normalize_strict_mode;
                if skip_heavy_analysis {
                    1.0
                } else {
                    Self::compute_track_normalization_gain(bytes.as_ref(), normalize_target)
                }
            }
        } else {
            1.0
        };

        handle.set_normalization_gain(track_gain);

        tracing::debug!(
            "HTTP track normalization settings: enabled={}, runtime_target={}, track_gain={:.3}",
            normalize_enabled,
            normalize_target,
            track_gain
        );

        // Decode audio data
        let cursor = Cursor::new(bytes);
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

        // Wrap sink in Arc<Mutex<>> and store in handle for volume control
        let sink_handle = Arc::new(Mutex::new(sink));

        // Store the sink in the PlaybackHandle for direct control (pause/play/volume)
        tokio::task::block_in_place(|| {
            let runtime = tokio::runtime::Handle::current();
            runtime.block_on(handle.set_sink(sink_handle.clone()));
        });

        // Apply initial volume (0-100 scale converted to 0.0-1.0)
        let volume_f32 = (volume.min(100) as f32) / 100.0;
        if let Ok(s) = sink_handle.try_lock() {
            s.set_volume(volume_f32);
            tracing::info!("Set initial volume to {} ({}%)", volume_f32, volume);
        }

        // Check if we should start paused (for restore scenarios)
        let should_start_paused = handle.is_paused();
        if should_start_paused {
            if let Ok(s) = sink_handle.try_lock() {
                s.pause();
            }
        } else {
            // Start playback if not paused
            if let Ok(s) = sink_handle.try_lock() {
                s.play();
            }
        }

        // Convert to f32 samples
        let source = source.convert_samples::<f32>();

        // Check if we need to seek to a specific position (for restore)
        let initial_position = handle.get_position();
        if initial_position > 0 {
            tracing::info!("Seeking to restored position: {}ms", initial_position);
            // Use skip_duration to skip ahead
            let source = source
                .skip_duration(Duration::from_millis(initial_position))
                .amplify(track_gain);
            if let Ok(s) = sink_handle.try_lock() {
                s.append(source);
            }
        } else if let Ok(s) = sink_handle.try_lock() {
            s.append(source.amplify(track_gain));
        }

        // Track playback progress - initialize from handle position for restore support
        let start = Instant::now();
        let initial_offset = Duration::from_millis(initial_position);
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
                if let Ok(s) = sink_handle.try_lock() {
                    s.pause();
                }
                if pause_time.is_none() {
                    pause_time = Some(Instant::now());
                }
            } else {
                if let Ok(s) = sink_handle.try_lock() {
                    s.play();
                }
                if let Some(paused_at) = pause_time {
                    accumulated_pause_duration += paused_at.elapsed();
                    pause_time = None;
                }
            }

            // Update position only when not paused - add initial offset for restore support
            let elapsed = {
                let start_elapsed = start.elapsed();
                let raw_elapsed = if let Some(paused_at) = pause_time {
                    // Currently paused: use time up to pause, guarding against underflow
                    let paused_elapsed = paused_at.elapsed();
                    start_elapsed.saturating_sub(accumulated_pause_duration + paused_elapsed)
                } else {
                    // Not paused: use full elapsed time minus accumulated pause duration
                    start_elapsed.saturating_sub(accumulated_pause_duration)
                };
                // Add the initial offset to account for restored position
                (raw_elapsed + initial_offset).as_millis() as u64
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

        if let Ok(s) = sink_handle.try_lock() {
            s.stop();
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

    pub async fn seek(&self, position_ms: u64) -> Result<(), String> {
        if let Some(handle) = &*self.current_handle.lock().await {
            handle.set_position(position_ms);
            Ok(())
        } else {
            Err("No playback in progress".to_string())
        }
    }

    pub async fn set_volume(&self, volume: u32) -> Result<(), String> {
        if let Some(handle) = &*self.current_handle.lock().await {
            handle.set_volume(volume);
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

#[derive(Clone)]
struct PreloadedHttpTrack {
    bytes: Vec<u8>,
    track_gain: Option<f32>,
}

/// Playback manager - handles playback state and queue
/// Poll interval for Spotify Connect playback state - matches the Android
/// app's `SpotifyConnectBridge`. A poll only samples periodically, unlike the
/// old in-webview SDK, which reported state changes as they happened.
const SPOTIFY_CONNECT_POLL_INTERVAL_MS: u64 = 2_000;

/// Periodically refreshes `info` from the Spotify Connect API while the
/// current track is a Spotify track, so `get_info()`/the `playback-status`
/// broadcast keep reflecting playback that may be happening on any Connect
/// device (including changes made from the official Spotify app/other
/// devices), the same way the Android app's poll loop does.
fn spawn_spotify_connect_poller(
    info: Arc<Mutex<PlaybackInfo>>,
    spotify_connect: Arc<SpotifyConnectBridge>,
    track_complete_tx: mpsc::UnboundedSender<()>,
) {
    tauri::async_runtime::spawn(async move {
        // Track whether the previous poll saw active playback so that the
        // end-of-track signal fires at most once per track (on the first poll
        // that observes playback stopped at the end, not on every subsequent
        // poll while the Connect device is between tracks).
        let mut was_playing = false;
        loop {
            tokio::time::sleep(Duration::from_millis(SPOTIFY_CONNECT_POLL_INTERVAL_MS)).await;

            let is_spotify_track = {
                let info = info.lock().await;
                info.current_track.as_ref().map(|track| track.source)
                    == Some(crate::models::Source::Spotify)
            };
            if !is_spotify_track {
                was_playing = false;
                continue;
            }

            let Some(context) = spotify_connect.playback_state().await else {
                continue;
            };

            let mut info = info.lock().await;
            // Discard stale polls that raced past a track change/stop.
            if info.current_track.as_ref().map(|track| track.source)
                != Some(crate::models::Source::Spotify)
            {
                was_playing = false;
                continue;
            }

            info.position_ms = context
                .progress
                .map(|position| position.num_milliseconds().max(0) as u64)
                .unwrap_or(info.position_ms);
            info.state = if context.is_playing {
                PlaybackState::Playing
            } else {
                PlaybackState::Paused
            };
            if let Some(rspotify::model::PlayableItem::Track(track)) = &context.item {
                let duration_ms = track.duration.num_milliseconds().max(0) as u64;
                if duration_ms > 0 {
                    if let Some(current_track) = info.current_track.as_mut() {
                        current_track.duration_ms = duration_ms;
                    }
                }

                // Detect end-of-track: playback transitioned from playing to
                // stopped while progress was at or near the end of the track,
                // meaning the Connect device finished the track rather than
                // the user pausing mid-way. Require the track to be
                // meaningfully longer than one poll interval - otherwise a
                // pause anywhere in a very short track (duration <= poll
                // interval) would always fall within the "near the end"
                // window and be misread as completion. The `was_playing`
                // guard ensures the signal fires at most once per track
                // completion so the queue advances by exactly one step.
                let progress_ms = info.position_ms;
                let at_end = duration_ms > SPOTIFY_CONNECT_POLL_INTERVAL_MS
                    && progress_ms
                        >= duration_ms.saturating_sub(SPOTIFY_CONNECT_POLL_INTERVAL_MS);
                if was_playing && !context.is_playing && at_end {
                    let _ = track_complete_tx.send(());
                }
            }
            was_playing = context.is_playing;
        }
    });
}

pub struct PlaybackManager {
    queue: Arc<Mutex<PlaybackQueue>>,
    info: Arc<Mutex<PlaybackInfo>>,
    audio_player: Arc<AudioPlayer>,
    spotify_session: Arc<SpotifySessionManager>,
    spotify_connect: Arc<SpotifyConnectBridge>,
    providers: Arc<Mutex<ProviderRegistry>>,
    track_complete_tx: mpsc::UnboundedSender<()>,
    track_complete_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<()>>>>,
    monitoring_task_abort: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
    audio_normalization: Arc<Mutex<AudioNormalizationSettings>>,
    adaptive_normalization: Arc<Mutex<AdaptiveNormalizationState>>,
    preloaded_http_tracks: Arc<Mutex<HashMap<String, PreloadedHttpTrack>>>,
    state_save_tx: mpsc::UnboundedSender<()>,
    state_save_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<()>>>>,
}

impl PlaybackManager {
    pub fn new(providers: Arc<Mutex<ProviderRegistry>>) -> Self {
        // Create a channel for track completion events
        let (track_complete_tx, track_complete_rx) = mpsc::unbounded_channel::<()>();

        // Create a channel for state save requests
        let (state_save_tx, state_save_rx) = mpsc::unbounded_channel::<()>();

        let info = Arc::new(Mutex::new(PlaybackInfo::default()));
        let spotify_connect = Arc::new(SpotifyConnectBridge::new(providers.clone()));
        spawn_spotify_connect_poller(info.clone(), spotify_connect.clone(), track_complete_tx.clone());

        Self {
            queue: Arc::new(Mutex::new(PlaybackQueue::new())),
            info,
            audio_player: Arc::new(AudioPlayer::new()),
            spotify_session: Arc::new(SpotifySessionManager::new(SPOTIFY_CLIENT_ID.to_string())),
            spotify_connect,
            providers,
            track_complete_tx,
            track_complete_rx: Arc::new(Mutex::new(Some(track_complete_rx))),
            monitoring_task_abort: Arc::new(Mutex::new(None)),
            audio_normalization: Arc::new(Mutex::new(AudioNormalizationSettings::default())),
            adaptive_normalization: Arc::new(Mutex::new(AdaptiveNormalizationState::default())),
            preloaded_http_tracks: Arc::new(Mutex::new(HashMap::new())),
            state_save_tx,
            state_save_rx: Arc::new(Mutex::new(Some(state_save_rx))),
        }
    }

    /// Start the state saver task - must be called from a Tokio runtime context
    pub async fn start_state_saver(&self) {
        if let Some(state_save_rx) = self.state_save_rx.lock().await.take() {
            let info_clone = self.info.clone();
            let queue_clone = self.queue.clone();
            let normalization_clone = self.audio_normalization.clone();
            tokio::spawn(Self::state_saver_task(
                info_clone,
                queue_clone,
                normalization_clone,
                state_save_rx,
            ));
        }
    }

    /// Centralized state saver task with debouncing
    /// Saves state at most once every 5 seconds, even if requested more frequently
    async fn state_saver_task(
        info: Arc<Mutex<PlaybackInfo>>,
        queue: Arc<Mutex<PlaybackQueue>>,
        audio_normalization: Arc<Mutex<AudioNormalizationSettings>>,
        mut save_rx: mpsc::UnboundedReceiver<()>,
    ) {
        let mut last_save = Instant::now();
        const SAVE_INTERVAL: Duration = Duration::from_secs(5);

        loop {
            // Wait for save request or timeout
            match tokio::time::timeout(SAVE_INTERVAL, save_rx.recv()).await {
                Ok(Some(())) => {
                    // Request received, check if enough time has passed
                    if last_save.elapsed() >= SAVE_INTERVAL {
                        Self::perform_state_save(&info, &queue, &audio_normalization).await;
                        last_save = Instant::now();
                    }
                    // If not enough time has passed, the request is dropped (debounced)
                }
                Ok(None) => {
                    // Channel closed, exit the task
                    tracing::info!("State saver task exiting (channel closed)");
                    break;
                }
                Err(_) => {
                    // Timeout, perform periodic save if state might have changed
                    // This ensures we save even during continuous playback
                    if last_save.elapsed() >= SAVE_INTERVAL {
                        Self::perform_state_save(&info, &queue, &audio_normalization).await;
                        last_save = Instant::now();
                    }
                }
            }
        }
    }

    /// Perform the actual state save operation
    async fn perform_state_save(
        info: &Arc<Mutex<PlaybackInfo>>,
        queue: &Arc<Mutex<PlaybackQueue>>,
        audio_normalization: &Arc<Mutex<AudioNormalizationSettings>>,
    ) {
        use crate::state::PersistentPlaybackState;

        let info_locked = info.lock().await;
        let queue_locked = queue.lock().await;
        let normalization_settings = audio_normalization.lock().await.clone();

        let state = PersistentPlaybackState {
            current_track: info_locked.current_track.clone(),
            queue: queue_locked.tracks.clone(),
            current_index: queue_locked.current_index,
            position_ms: info_locked.position_ms,
            shuffle: info_locked.shuffle,
            repeat_mode: info_locked.repeat_mode,
            volume: info_locked.volume,
            audio_normalization_enabled: normalization_settings.enabled,
            audio_normalization_target: normalization_settings.target,
            audio_normalization_strict_mode: normalization_settings.strict_mode,
            shuffle_order: queue_locked.shuffle_order.clone(),
            state: info_locked.state,
        };

        drop(info_locked);
        drop(queue_locked);

        if let Err(e) = state.save().await {
            tracing::error!("Failed to save state: {}", e);
        }
    }

    /// Request a state save (debounced)
    pub fn request_state_save(&self) {
        // Ignore send errors - if the channel is closed, we're shutting down anyway
        let _ = self.state_save_tx.send(());
    }

    fn source_key(source: crate::models::Source) -> String {
        source.to_string()
    }

    fn normalization_source(source: Option<crate::models::Source>) -> AudioNormalizationSource {
        match source {
            Some(crate::models::Source::Spotify) => AudioNormalizationSource::Spotify,
            _ => AudioNormalizationSource::Other,
        }
    }

    fn preload_track_key(track: &Track) -> String {
        format!("{}:{}", Self::source_key(track.source), track.id)
    }

    async fn take_preloaded_http_track(&self, track: &Track) -> Option<PreloadedHttpTrack> {
        let key = Self::preload_track_key(track);
        self.preloaded_http_tracks.lock().await.remove(&key)
    }

    async fn prune_preloaded_http_tracks(&self, keep_tracks: &[Track]) {
        let keep_keys: HashSet<String> = keep_tracks.iter().map(Self::preload_track_key).collect();
        self.preloaded_http_tracks
            .lock()
            .await
            .retain(|key, _| keep_keys.contains(key));
    }

    async fn strict_source_compensation_gain(
        &self,
        source: Option<crate::models::Source>,
        strict_mode: bool,
    ) -> f32 {
        if !strict_mode {
            return 1.0;
        }

        let Some(source) = source else {
            return 1.0;
        };

        let source_key = Self::source_key(source);
        let adaptive = self.adaptive_normalization.lock().await;
        adaptive.strict_compensation_gain(&source_key)
    }

    async fn effective_output_volume(
        &self,
        base_volume: u32,
        source: Option<crate::models::Source>,
    ) -> u32 {
        let normalization = self.audio_normalization.lock().await.clone();
        let strict_gain = self
            .strict_source_compensation_gain(source, normalization.strict_mode)
            .await;

        effective_output_volume(
            base_volume,
            Self::normalization_source(source),
            &normalization,
            strict_gain,
        )
    }

    /// Effective output volume (post-normalization) for Spotify Connect,
    /// clamped to the 0-100 range the Connect API accepts.
    async fn spotify_playback_volume(&self) -> u8 {
        let base_volume = {
            let info = self.info.lock().await;
            info.volume
        };
        self.effective_output_volume(base_volume, Some(crate::models::Source::Spotify))
            .await
            .min(100) as u8
    }

    pub async fn get_audio_normalization_settings(&self) -> AudioNormalizationSettings {
        self.audio_normalization.lock().await.clone()
    }

    pub async fn set_audio_normalization_settings(&self, enabled: bool, strict_mode: bool) {
        {
            let mut normalization = self.audio_normalization.lock().await;
            normalization.enabled = enabled;
            normalization.target = INTERNAL_NORMALIZATION_TARGET;
            normalization.strict_mode = strict_mode;
        }

        let base_volume = {
            let info = self.info.lock().await;
            info.volume
        };
        let current_source = {
            let info = self.info.lock().await;
            info.current_track.as_ref().map(|track| track.source)
        };
        let effective_volume = self
            .effective_output_volume(base_volume, current_source)
            .await;

        if let Err(error) = self.audio_player.set_volume(effective_volume).await {
            tracing::warn!(
                "Failed to apply audio normalization settings to active playback: {}",
                error
            );
        }

        let _ = self.save_state().await;
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
        // BUT: If shuffle is enabled, we need to find the track's position in the shuffle_order,
        // not its position in the tracks array
        {
            let info = self.info.lock().await;
            let shuffle_enabled = info.shuffle;
            drop(info);

            let mut queue = self.queue.lock().await;

            if shuffle_enabled && !queue.shuffle_order.is_empty() {
                // Find the shuffle position (index in shuffle_order) that points to this track
                if let Some(actual_index) = queue.tracks.iter().position(|t| t.id == track.id) {
                    if let Some(shuffle_pos) = queue
                        .shuffle_order
                        .iter()
                        .position(|&idx| idx == actual_index)
                    {
                        queue.current_index = shuffle_pos;
                        tracing::debug!(
                            "Set queue current_index to shuffle position {} (actual index {}) for track: {}",
                            shuffle_pos,
                            actual_index,
                            track.title
                        );
                    }
                }
            } else {
                // Normal mode: find the track in the queue and set current_index
                if let Some(index) = queue.tracks.iter().position(|t| t.id == track.id) {
                    queue.current_index = index;
                    tracing::debug!(
                        "Set queue current_index to {} for track: {}",
                        index,
                        track.title
                    );
                }
            }
        }

        let mut info = self.info.lock().await;
        info.current_track = Some(track.clone());
        info.state = PlaybackState::Playing;
        info.position_ms = 0;
        drop(info); // Release the lock

        // Save state AFTER track info is updated
        let _ = self.save_state().await;

        // Attempt to play the audio
        if let Some(url) = &track.url {
            // Spotify tracks are driven via the Web API's Connect endpoints
            // (`/v1/me/player/*`), targeting whichever Connect device is active
            // on the account - auto-launching the local Spotify desktop app to
            // register one if none is - rather than an in-process decoder or
            // in-webview SDK. `info.current_track`/`state`/`position_ms` were
            // already set above; the poll loop started in `new()` keeps them in
            // sync with the Connect device going forward.
            if let Some(track_id) = url.strip_prefix("spotify:track:") {
                let track_id = track_id.to_string();
                let spotify_connect = self.spotify_connect.clone();
                let volume = self.spotify_playback_volume().await;
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = spotify_connect.play_uri(&track_id, None, Some(volume)).await {
                        tracing::warn!("Failed to start Spotify Connect playback: {}", error);
                    }
                });
            } else {
                // HTTP URL - play as normal
                let track_complete_tx = self.track_complete_tx.clone();
                let monitoring_abort = self.monitoring_task_abort.clone();
                let state_save_tx = self.state_save_tx.clone();
                let adaptive_normalization = self.adaptive_normalization.clone();
                let track_source = track.source;
                let preloaded = self.take_preloaded_http_track(&track).await;
                if preloaded.is_some() {
                    tracing::debug!("Using preloaded audio payload for track {}", track.id);
                }

                // Fetch auth headers dynamically from provider if needed (e.g., for Jellyfin)
                let auth_headers = if track.source == crate::models::Source::Jellyfin {
                    let providers = self.providers.lock().await;
                    providers.get_auth_headers(track.source).await
                } else {
                    track.auth_headers.clone()
                };

                // Get current volume and apply normalization settings
                let base_volume = {
                    let info = self.info.lock().await;
                    info.volume
                };
                let volume = self
                    .effective_output_volume(base_volume, Some(track.source))
                    .await;
                let normalization = self.audio_normalization.lock().await.clone();

                let (preloaded_bytes, preloaded_gain) = preloaded
                    .map(|entry| (Some(entry.bytes), entry.track_gain))
                    .unwrap_or((None, None));

                match self
                    .audio_player
                    .play_url(
                        url,
                        auth_headers,
                        volume,
                        normalization.enabled,
                        normalization.target,
                        normalization.strict_mode,
                        track.source,
                        preloaded_bytes,
                        preloaded_gain,
                    )
                    .await
                {
                    Ok(handle) => {
                        // Spawn a task to update playback position from the audio player
                        let info_arc = self.info.clone();
                        let _queue_arc = self.queue.clone();

                        let task = tokio::spawn(async move {
                            tracing::debug!("HTTP monitoring task started");
                            let mut last_state_save = std::time::Instant::now();
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

                                // Request state save periodically (every 5 seconds)
                                if last_state_save.elapsed().as_secs() >= 5 {
                                    let _ = state_save_tx.send(());
                                    last_state_save = std::time::Instant::now();
                                }

                                // When track completes, send event to advance to next track
                                if should_stop {
                                    tracing::debug!(
                                        "HTTP monitoring task detected should_stop=true"
                                    );
                                    if let Some(gain) = handle.get_normalization_gain() {
                                        let source_key = PlaybackManager::source_key(track_source);
                                        let mut adaptive = adaptive_normalization.lock().await;
                                        adaptive.push_gain(&source_key, gain);
                                    }
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
        drop(queue);

        // Save state when queue changes
        let _ = self.save_state().await;
    }

    /// Clear the playback queue
    pub async fn clear_queue(&self) {
        let mut queue = self.queue.lock().await;
        queue.clear();
        self.preloaded_http_tracks.lock().await.clear();
        let mut info = self.info.lock().await;
        info.state = PlaybackState::Stopped;
        info.current_track = None;
        drop(info);
        drop(queue);

        // Don't save state immediately - let the caller decide when to save
        // This prevents saving an empty state when clearing before loading a new track
    }

    /// Play a track (start playback)
    pub async fn play(&self) {
        let source = {
            let info = self.info.lock().await;
            info.current_track.as_ref().map(|track| track.source)
        };

        if source == Some(crate::models::Source::Spotify) {
            match self.spotify_connect.resume().await {
                Ok(()) => {
                    let mut info = self.info.lock().await;
                    info.state = PlaybackState::Playing;
                }
                Err(error) => {
                    // Nothing to resume (e.g. no active Connect device yet - the
                    // common case right after a restore, which only restores
                    // `current_track`/`position_ms` without starting Connect
                    // playback). Start the track fresh at the saved position -
                    // passed as part of the same start request rather than a
                    // separate seek call, since the device isn't ready to accept
                    // commands until playback has actually started.
                    tracing::warn!(
                        "Failed to resume Spotify Connect playback, starting track fresh: {}",
                        error
                    );
                    let (current_track, position_ms) = {
                        let info = self.info.lock().await;
                        (info.current_track.clone(), info.position_ms)
                    };

                    let Some(track) = current_track else {
                        tracing::warn!("No active Spotify playback and no current track to play");
                        return;
                    };
                    let Some(track_id) = track
                        .url
                        .as_deref()
                        .and_then(|url| url.strip_prefix("spotify:track:"))
                    else {
                        tracing::warn!("Current Spotify track has no spotify:track: URI to restart");
                        return;
                    };

                    let volume = self.spotify_playback_volume().await;
                    let position = (position_ms > 0).then_some(position_ms as i64);
                    match self
                        .spotify_connect
                        .play_uri(track_id, position, Some(volume))
                        .await
                    {
                        Ok(()) => {
                            let mut info = self.info.lock().await;
                            info.state = PlaybackState::Playing;
                        }
                        Err(error) => {
                            tracing::warn!("Failed to start Spotify Connect playback: {}", error);
                            let mut info = self.info.lock().await;
                            info.state = PlaybackState::Stopped;
                        }
                    }
                }
            }
            return;
        }

        // Try to resume existing playback first
        match self.audio_player.resume().await {
            Ok(_) => {
                // Successfully resumed
                let mut info = self.info.lock().await;
                info.state = PlaybackState::Playing;
            }
            Err(_) => {
                // No active playback - try to load and play the current track
                let current_track = {
                    let info = self.info.lock().await;
                    info.current_track.clone()
                };

                if let Some(track) = current_track {
                    tracing::info!(
                        "No active playback, loading track: {} - {}",
                        track.artist,
                        track.title
                    );
                    self.play_track(track).await;
                } else {
                    tracing::warn!("No active playback and no current track to play");
                }
            }
        }
    }

    /// Pause playback
    pub async fn pause(&self) {
        let source = {
            let info = self.info.lock().await;
            info.current_track.as_ref().map(|track| track.source)
        };

        let mut info = self.info.lock().await;
        info.state = PlaybackState::Paused;
        drop(info);

        if source == Some(crate::models::Source::Spotify) {
            if let Err(error) = self.spotify_connect.pause().await {
                tracing::warn!("Failed to pause Spotify Connect playback: {}", error);
            }
            return;
        }

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
        let (new_state, source) = {
            let info = info_arc.lock().await;
            let new_state = match info.state {
                PlaybackState::Playing => PlaybackState::Paused,
                PlaybackState::Paused | PlaybackState::Stopped => PlaybackState::Playing,
            };
            (new_state, info.current_track.as_ref().map(|track| track.source))
        };

        if source == Some(crate::models::Source::Spotify) {
            // `new_state` above is only ever Playing or Paused (Stopped always
            // maps to Playing), so delegate to `play`/`pause`, which already
            // handle the Spotify resume-fallback and state bookkeeping.
            match new_state {
                PlaybackState::Playing => self.play().await,
                PlaybackState::Paused | PlaybackState::Stopped => self.pause().await,
            }
            return;
        }

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

    /// Skip to a specific track in the queue by its index
    /// The index represents the position in the displayed queue (0-based, after current track)
    pub async fn skip_to_queue_index(&self, queue_index: usize) -> Option<Track> {
        // Get shuffle state from info
        let shuffle_enabled = {
            let info = self.info.lock().await;
            info.shuffle
        };

        let mut queue = self.queue.lock().await;
        let track_opt = queue.skip_to_queue_index(queue_index, shuffle_enabled);

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
        let source = info.current_track.as_ref().map(|track| track.source);
        drop(info);

        if source == Some(crate::models::Source::Spotify) {
            if let Err(error) = self.spotify_connect.seek(position_ms as i64).await {
                tracing::warn!("Failed to seek Spotify Connect playback: {}", error);
            }
            return;
        }

        if let Err(error) = self.audio_player.seek(position_ms).await {
            tracing::warn!("Failed to seek active playback: {}", error);
        }
    }

    /// Set volume (0-100)
    pub async fn set_volume(&self, volume: u32) {
        let mut info = self.info.lock().await;
        info.volume = volume.min(100);
        let source = info.current_track.as_ref().map(|track| track.source);
        drop(info);

        if source == Some(crate::models::Source::Spotify) {
            if let Err(error) = self.spotify_connect.set_volume(volume.min(100) as u8).await {
                tracing::warn!("Failed to set Spotify Connect volume: {}", error);
            }
            self.request_state_save();
            return;
        }

        let effective_volume = self.effective_output_volume(volume, source).await;

        if let Err(error) = self.audio_player.set_volume(effective_volume).await {
            tracing::warn!("Failed to set active playback volume: {}", error);
        }

        self.request_state_save();
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
        drop(queue);

        // Save state when shuffle changes
        let _ = self.save_state().await;
    }

    /// Set repeat mode
    pub async fn set_repeat_mode(&self, mode: RepeatMode) {
        let mut info = self.info.lock().await;
        info.repeat_mode = mode;
        drop(info);

        // Save state when repeat mode changes
        let _ = self.save_state().await;
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

    /// Warm up Spotify session state with an OAuth access token.
    ///
    /// This is kept for compatibility with existing command flows and startup restore.
    /// Active playback now obtains token from the provider and uses the shared engine path.
    pub async fn initialize_spotify_session(&self, access_token: &str) -> Result<(), String> {
        self.spotify_session
            .initialize_with_oauth_token(access_token)
            .await
    }

    /// Check whether warm-up Spotify session state is initialized.
    pub async fn is_spotify_session_ready(&self) -> bool {
        self.spotify_session.is_initialized().await
    }

    /// Close warm-up Spotify session state.
    pub async fn close_spotify_session(&self) -> Result<(), String> {
        self.spotify_session.close_session().await
    }

    /// Build persistent state from current playback info and queue
    async fn build_persistent_state(&self) -> crate::state::PersistentPlaybackState {
        use crate::state::PersistentPlaybackState;

        let info = self.info.lock().await;
        let queue = self.queue.lock().await;
        let normalization = self.audio_normalization.lock().await;

        PersistentPlaybackState {
            current_track: info.current_track.clone(),
            queue: queue.tracks.clone(),
            current_index: queue.current_index,
            position_ms: info.position_ms,
            shuffle: info.shuffle,
            repeat_mode: info.repeat_mode,
            volume: info.volume,
            audio_normalization_enabled: normalization.enabled,
            audio_normalization_target: normalization.target,
            audio_normalization_strict_mode: normalization.strict_mode,
            shuffle_order: queue.shuffle_order.clone(),
            state: info.state,
        }
    }

    /// Save current playback state to disk
    pub async fn save_state(&self) -> Result<(), String> {
        let state = self.build_persistent_state().await;
        state.save().await
    }

    /// Restore playback state from disk
    pub async fn restore_state(&self) -> Result<(), String> {
        use crate::state::PersistentPlaybackState;

        let saved_state = match PersistentPlaybackState::load().await? {
            Some(state) => state,
            None => {
                tracing::info!("No saved state to restore");
                return Ok(());
            }
        };

        // Only restore if there's actually a current track to restore
        if saved_state.current_track.is_none() {
            tracing::info!("No current track in saved state, skipping restore");
            return Ok(());
        }

        // Abort any existing monitoring task before restoring
        {
            let mut abort_handle = self.monitoring_task_abort.lock().await;
            if let Some(handle) = abort_handle.take() {
                handle.abort();
                tracing::debug!("Aborted existing monitoring task for restore");
            }
        }

        // Stop current playback before restoring
        {
            let current_handle = self.audio_player.current_handle.lock().await;
            if let Some(handle) = current_handle.as_ref() {
                handle.stop();
                tracing::debug!("Stopped current playback for restore");
            }
        }

        let queue_len = saved_state.queue.len();
        let current_idx = saved_state.current_index;
        let shuffle_enabled = saved_state.shuffle;
        let position = saved_state.position_ms;

        // Restore queue
        {
            let mut queue = self.queue.lock().await;
            queue.tracks = saved_state.queue;
            queue.current_index = saved_state.current_index;
            queue.shuffle_order = saved_state.shuffle_order;
        }

        // Restore info
        {
            let mut info = self.info.lock().await;
            info.current_track = saved_state.current_track.clone();
            info.position_ms = saved_state.position_ms;
            info.shuffle = saved_state.shuffle;
            info.repeat_mode = saved_state.repeat_mode;
            info.volume = saved_state.volume;
            // Always start paused on restore
            info.state = PlaybackState::Paused;
        }

        {
            let mut normalization = self.audio_normalization.lock().await;
            normalization.enabled = saved_state.audio_normalization_enabled;
            normalization.target = INTERNAL_NORMALIZATION_TARGET;
            normalization.strict_mode = saved_state.audio_normalization_strict_mode;
        }

        tracing::info!(
            "Restored playback state: {} tracks in queue, current_index: {}, shuffle: {}, position: {}ms",
            queue_len,
            current_idx,
            shuffle_enabled,
            position
        );

        // If there was a current track, load it (but don't start playing)
        if let Some(track) = saved_state.current_track {
            tracing::info!(
                "Restoring track: {} - {} at position {}ms",
                track.artist,
                track.title,
                position
            );

            // Set the info state to match what we're about to do (load but paused)
            {
                let mut info = self.info.lock().await;
                info.state = PlaybackState::Paused;
            }

            // Actually load the track into the player so it's ready to play
            if let Some(url) = &track.url {
                // Spotify tracks aren't resumed automatically on restore (matching
                // HTTP tracks, which are only pre-loaded, not played) - position/state
                // were already restored into `info` above, and playback will resume
                // via the Connect API on the next explicit play/resume command.
                if url.starts_with("spotify:track:") {
                    tracing::info!(
                        "Spotify track restored at position {}ms; will resume via Spotify Connect on next play",
                        position
                    );
                } else {
                    // HTTP URL - pre-load it with pre-configured handle
                    tracing::info!(
                        "Pre-loading HTTP track for restored session at position {}ms",
                        position
                    );

                    // Create a new PlaybackHandle and pre-configure it with the restored position/pause state
                    let handle = PlaybackHandle::new();
                    if position > 0 {
                        handle.set_position(position);
                    }
                    handle.pause(); // Start paused for restore

                    // Store the handle before spawning playback
                    {
                        let mut current = self.audio_player.current_handle.lock().await;
                        if let Some(old_handle) = current.take() {
                            old_handle.stop();
                        }
                        *current = Some(handle.clone());
                    }

                    // Now spawn HTTP playback - play_audio_blocking will check is_paused and get_position
                    let url_clone = url.to_string();
                    let handle_clone = handle.clone();

                    // Fetch auth headers dynamically from provider if needed (e.g., for Jellyfin)
                    let auth_headers = if track.source == crate::models::Source::Jellyfin {
                        let providers = self.providers.lock().await;
                        providers.get_auth_headers(track.source).await
                    } else {
                        track.auth_headers.clone()
                    };

                    // Get current volume for restore
                    let volume = self
                        .effective_output_volume(saved_state.volume, Some(track.source))
                        .await;
                    let normalization = self.audio_normalization.lock().await.clone();

                    tokio::spawn(async move {
                        tracing::info!(
                            "Starting HTTP audio playback from URL (restore): {}",
                            url_clone
                        );

                        let result = tokio::task::spawn_blocking({
                            let url = url_clone.clone();
                            let handle = handle_clone.clone();
                            move || {
                                AudioPlayer::play_audio_blocking(
                                    &url,
                                    &handle,
                                    auth_headers,
                                    volume,
                                    normalization.enabled,
                                    normalization.target,
                                    normalization.strict_mode,
                                    track.source,
                                    None,
                                    None,
                                )
                            }
                        })
                        .await;

                        match result {
                            Ok(Ok(())) => {
                                tracing::info!(
                                    "HTTP audio playback (restore) completed successfully"
                                );
                            }
                            Ok(Err(e)) => {
                                tracing::error!("HTTP audio playback (restore) error: {}", e);
                            }
                            Err(e) => {
                                tracing::error!("HTTP restore task join error: {}", e);
                            }
                        }
                    });

                    // Update the info position
                    {
                        let mut info = self.info.lock().await;
                        info.position_ms = position;
                        info.state = PlaybackState::Paused;
                    }

                    // Spawn monitoring task for HTTP restore path
                    let info_arc = self.info.clone();
                    let _queue_arc = self.queue.clone();
                    let track_complete_tx = self.track_complete_tx.clone();
                    let monitoring_abort = self.monitoring_task_abort.clone();
                    let state_save_tx = self.state_save_tx.clone();
                    let adaptive_normalization = self.adaptive_normalization.clone();
                    let track_source = track.source;

                    let task = tokio::spawn(async move {
                        tracing::debug!("HTTP restore monitoring task started");
                        let mut last_state_save = std::time::Instant::now();
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

                            // Request state save periodically (every 5 seconds)
                            if last_state_save.elapsed().as_secs() >= 5 {
                                let _ = state_save_tx.send(());
                                last_state_save = std::time::Instant::now();
                            }

                            // When track completes, send event to advance to next track
                            if should_stop {
                                tracing::debug!(
                                    "HTTP restore monitoring task detected should_stop=true"
                                );
                                if let Some(gain) = handle.get_normalization_gain() {
                                    let source_key = PlaybackManager::source_key(track_source);
                                    let mut adaptive = adaptive_normalization.lock().await;
                                    adaptive.push_gain(&source_key, gain);
                                }
                                {
                                    let mut info = info_arc.lock().await;
                                    info.state = PlaybackState::Stopped;
                                }
                                tracing::info!("HTTP track completed, sending auto-advance event");
                                let _ = track_complete_tx.send(());
                                break;
                            }

                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    });

                    // Store the abort handle
                    {
                        let mut abort_handle = monitoring_abort.lock().await;
                        *abort_handle = Some(task.abort_handle());
                    }

                    tracing::info!("HTTP track pre-loaded and ready at position {}ms", position);
                }
            }

            // Start look-ahead preloading for the next track(s) in queue
            self.start_lookahead_preload().await;
        }

        Ok(())
    }

    /// Start look-ahead preloading for upcoming tracks in the queue
    async fn start_lookahead_preload(&self) {
        let queue = self.queue.lock().await;
        let info = self.info.lock().await;
        let shuffle_enabled = info.shuffle;
        let current_index = queue.current_index;

        // Preload next 10 tracks in the queue
        let tracks_to_preload: Vec<Track> = if shuffle_enabled && !queue.shuffle_order.is_empty() {
            // Use shuffle order
            (1..=10)
                .filter_map(|offset| {
                    let next_pos = current_index + offset;
                    if next_pos < queue.shuffle_order.len() {
                        let actual_index = queue.shuffle_order[next_pos];
                        queue.tracks.get(actual_index).cloned()
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            // Use normal order
            (1..=10)
                .filter_map(|offset| {
                    let next_index = current_index + offset;
                    queue.tracks.get(next_index).cloned()
                })
                .collect()
        };

        drop(queue);
        drop(info);

        self.prune_preloaded_http_tracks(&tracks_to_preload).await;

        if !tracks_to_preload.is_empty() {
            tracing::info!(
                "Starting look-ahead preload for {} tracks",
                tracks_to_preload.len()
            );

            let providers = self.providers.clone();
            let normalization = self.audio_normalization.lock().await.clone();
            let preload_cache = self.preloaded_http_tracks.clone();

            // Spawn async task for preloading
            tokio::spawn(async move {
                let client = reqwest::Client::new();

                for track in tracks_to_preload {
                    if let Some(url) = &track.url {
                        if url.starts_with("spotify:track:") {
                            continue;
                        }

                        let cache_key =
                            format!("{}:{}", PlaybackManager::source_key(track.source), track.id);

                        if preload_cache.lock().await.contains_key(&cache_key) {
                            continue;
                        }

                        let auth_headers = if track.source == crate::models::Source::Jellyfin {
                            let providers_guard = providers.lock().await;
                            providers_guard.get_auth_headers(track.source).await
                        } else {
                            track.auth_headers.clone()
                        };

                        let mut request = client
                            .get(url)
                            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)");

                        if let Some(headers) = auth_headers {
                            for (key, value) in headers {
                                request = request.header(key, value);
                            }
                        }

                        match request.send().await {
                            Ok(response) if response.status().is_success() => {
                                // Skip preload if Content-Length exceeds 50 MB to avoid OOM
                                const MAX_PRELOAD_BYTES: u64 = 50 * 1024 * 1024;
                                if response
                                    .content_length()
                                    .is_some_and(|len| len > MAX_PRELOAD_BYTES)
                                {
                                    tracing::debug!(
                                        "Skipping preload for {} - Content-Length exceeds {} bytes",
                                        track.id,
                                        MAX_PRELOAD_BYTES
                                    );
                                    continue;
                                }
                                match response.bytes().await {
                                    Ok(bytes) => {
                                        let bytes_vec = bytes.to_vec();
                                        let track_gain = if normalization.enabled {
                                            let skip_heavy_analysis = track.source
                                                == crate::models::Source::Plex
                                                && !normalization.strict_mode;
                                            if skip_heavy_analysis {
                                                Some(1.0)
                                            } else {
                                                Some(AudioPlayer::compute_track_normalization_gain(
                                                    bytes_vec.as_ref(),
                                                    normalization.target,
                                                ))
                                            }
                                        } else {
                                            None
                                        };

                                        preload_cache.lock().await.insert(
                                            cache_key,
                                            PreloadedHttpTrack {
                                                bytes: bytes_vec,
                                                track_gain,
                                            },
                                        );
                                        tracing::debug!(
                                            "Preloaded full track payload: {} - {}",
                                            track.artist,
                                            track.title
                                        );
                                    }
                                    Err(error) => {
                                        tracing::debug!(
                                            "Lookahead preload failed reading body for {}: {}",
                                            track.id,
                                            error
                                        );
                                    }
                                }
                            }
                            Ok(response) => {
                                tracing::debug!(
                                    "Lookahead preload HTTP error for {}: {}",
                                    track.id,
                                    response.status()
                                );
                            }
                            Err(error) => {
                                tracing::debug!(
                                    "Lookahead preload request failed for {}: {}",
                                    track.id,
                                    error
                                );
                            }
                        }
                    }
                }
            });
        }
    }
}
