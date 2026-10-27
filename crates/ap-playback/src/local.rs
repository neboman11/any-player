//! Local decoder → rate converter → half-second SPSC ring → allocation-free audio callback.
//!
//! Device creation and decoding stay off tokio. Sink streams stay on their owning OS
//! thread (CPAL streams need not be Send). Boundary messages share the sample ring, so events
//! describe consumed audio, never decoder look-ahead. CaptureSink exposes manual pumping for
//! deterministic device-free integration tests.
#[cfg(any(windows, test))]
#[path = "local_cpal_windows.rs"]
mod cpal_windows;
#[path = "local_http.rs"]
pub mod http;
#[path = "local_null.rs"]
mod null;
use crate::backend::{BACKEND_EVENT_CAPACITY, Backend, BackendEvent, BackendRun, ItemKey, RunItem};
use crate::ports::{PlaybackPersistence, StreamRequest, StreamResolver, track_key};
use crate::state::ChangeOrigin;
use ap_model::{BackendKind, volume_normalization::track_gain};
use async_trait::async_trait;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(windows)]
pub use cpal_windows::CpalSink;
use http::{AudioCache, HttpSource};
pub use null::NullSink;
use rtrb::{Consumer, Producer, RingBuffer};
use rubato::{FftFixedIn, Resampler};
use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, Ordering},
};
use std::time::Duration;
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{Decoder, DecoderOptions},
    errors::Error as DecodeError,
    formats::{FormatOptions, FormatReader},
    io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions,
    probe::Hint,
};
use tokio::sync::{broadcast, oneshot};

#[derive(Debug, thiserror::Error)]
pub enum LocalError {
    #[error("Audio I/O: {0}")]
    Io(#[from] io::Error),
    #[error("Audio decoder: {0}")]
    Decode(String),
    #[error("Audio output: {0}")]
    Output(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputFormat {
    pub sample_rate: u32,
    pub channels: u16,
}
pub type Render = Box<dyn FnMut(&mut [f32]) + Send>;
/// Lifetime guard for a running output stream; drop stops output.
pub trait SinkStream {}
/// Output adapter. `start` is called only on the dedicated local worker thread.
pub trait AudioSink: Send + Sync + 'static {
    fn devices(&self) -> Result<Vec<String>, LocalError>;
    fn format(&self, device: Option<&str>) -> Result<OutputFormat, LocalError>;
    fn start(
        &self,
        device: Option<&str>,
        render: Render,
        lost: Arc<AtomicBool>,
    ) -> Result<Box<dyn SinkStream>, LocalError>;
    fn default_device_name(&self) -> Option<String> {
        None
    }
}
/// CPAL output, default device when selection is None; device loss requests a rebuild.
#[cfg(not(windows))]
pub use NativeCpalSink as CpalSink;
#[derive(Default)]
pub struct NativeCpalSink;
struct CpalStream {
    _stream: cpal::Stream,
}
impl SinkStream for CpalStream {}
impl NativeCpalSink {
    fn device(name: Option<&str>) -> Result<cpal::Device, LocalError> {
        let host = cpal::default_host();
        if let Some(name) = name {
            host.output_devices()
                .map_err(|e| LocalError::Output(e.to_string()))?
                .find(|d| d.name().is_ok_and(|n| n == name))
                .ok_or_else(|| LocalError::Output(format!("Output device not found: {name}")))
        } else {
            host.default_output_device()
                .ok_or_else(|| LocalError::Output("No default output device".into()))
        }
    }
}
impl AudioSink for NativeCpalSink {
    fn default_device_name(&self) -> Option<String> {
        cpal::default_host()
            .default_output_device()
            .and_then(|d| d.name().ok())
    }
    fn devices(&self) -> Result<Vec<String>, LocalError> {
        cpal::default_host()
            .output_devices()
            .map_err(|e| LocalError::Output(e.to_string()))?
            .map(|d| d.name().map_err(|e| LocalError::Output(e.to_string())))
            .collect()
    }
    fn format(&self, name: Option<&str>) -> Result<OutputFormat, LocalError> {
        let config = Self::device(name)?
            .default_output_config()
            .map_err(|e| LocalError::Output(e.to_string()))?;
        Ok(OutputFormat {
            sample_rate: config.sample_rate().0,
            channels: config.channels(),
        })
    }
    fn start(
        &self,
        name: Option<&str>,
        mut render: Render,
        lost: Arc<AtomicBool>,
    ) -> Result<Box<dyn SinkStream>, LocalError> {
        let device = Self::device(name)?;
        let supported = device
            .default_output_config()
            .map_err(|e| LocalError::Output(e.to_string()))?;
        let config: cpal::StreamConfig = supported.clone().into();
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &config,
                move |out: &mut [f32], _| render(out),
                move |_| {
                    lost.store(true, Ordering::Release);
                },
                None,
            ),
            cpal::SampleFormat::I16 => {
                let mut scratch = vec![0.0; usize::from(config.channels) * 16_384];
                device.build_output_stream(
                    &config,
                    move |out: &mut [i16], _| {
                        for chunk in out.chunks_mut(scratch.len()) {
                            render(&mut scratch[..chunk.len()]);
                            for (o, s) in chunk.iter_mut().zip(&scratch) {
                                *o = cpal::Sample::from_sample(*s);
                            }
                        }
                    },
                    move |_| {
                        lost.store(true, Ordering::Release);
                    },
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let mut scratch = vec![0.0; usize::from(config.channels) * 16_384];
                device.build_output_stream(
                    &config,
                    move |out: &mut [u16], _| {
                        for chunk in out.chunks_mut(scratch.len()) {
                            render(&mut scratch[..chunk.len()]);
                            for (o, s) in chunk.iter_mut().zip(&scratch) {
                                *o = cpal::Sample::from_sample(*s);
                            }
                        }
                    },
                    move |_| {
                        lost.store(true, Ordering::Release);
                    },
                    None,
                )
            }
            other => {
                return Err(LocalError::Output(format!(
                    "Unsupported output sample format {other:?}"
                )));
            }
        }
        .map_err(|e| LocalError::Output(e.to_string()))?;
        stream
            .play()
            .map_err(|e| LocalError::Output(e.to_string()))?;
        Ok(Box::new(CpalStream { _stream: stream }))
    }
}
struct CaptureState {
    render: Option<Render>,
    samples: Vec<f32>,
    generation: u64,
}
/// A manually clocked null/capture output. `pump` consumes precisely the requested sample count.
#[derive(Clone)]
pub struct CaptureSink {
    format: OutputFormat,
    state: Arc<Mutex<CaptureState>>,
}
impl CaptureSink {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            format: OutputFormat {
                sample_rate,
                channels,
            },
            state: Arc::new(Mutex::new(CaptureState {
                render: None,
                samples: vec![],
                generation: 0,
            })),
        }
    }
    pub fn pump(&self, frames: usize) -> Vec<f32> {
        let mut state = self.state.lock().expect("capture poisoned");
        let mut samples = vec![0.0; frames * usize::from(self.format.channels)];
        if let Some(render) = &mut state.render {
            render(&mut samples);
        }
        state.samples.extend_from_slice(&samples);
        samples
    }
    pub fn samples(&self) -> Vec<f32> {
        self.state.lock().expect("capture poisoned").samples.clone()
    }
    pub fn is_started(&self) -> bool {
        self.state
            .lock()
            .expect("capture poisoned")
            .render
            .is_some()
    }
}
struct CaptureStream {
    state: Arc<Mutex<CaptureState>>,
    generation: u64,
}
impl SinkStream for CaptureStream {}
impl Drop for CaptureStream {
    fn drop(&mut self) {
        let mut s = self.state.lock().expect("capture poisoned");
        if s.generation == self.generation {
            s.render = None;
        }
    }
}
impl AudioSink for CaptureSink {
    fn devices(&self) -> Result<Vec<String>, LocalError> {
        Ok(vec!["Capture".into()])
    }
    fn format(&self, _: Option<&str>) -> Result<OutputFormat, LocalError> {
        Ok(self.format)
    }
    fn start(
        &self,
        _: Option<&str>,
        render: Render,
        _: Arc<AtomicBool>,
    ) -> Result<Box<dyn SinkStream>, LocalError> {
        let mut state = self.state.lock().expect("capture poisoned");
        state.generation += 1;
        state.render = Some(render);
        Ok(Box::new(CaptureStream {
            state: self.state.clone(),
            generation: state.generation,
        }))
    }
}
struct Shared {
    playing: AtomicBool,
    normalization: AtomicBool,
    volume: AtomicI32,
    repeat: AtomicBool,
    position: AtomicU64,
    duration: AtomicI64,
    sample_rate: AtomicU64,
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            playing: AtomicBool::new(false),
            normalization: AtomicBool::new(false),
            volume: AtomicI32::new(100),
            repeat: AtomicBool::new(false),
            position: AtomicU64::new(0),
            duration: AtomicI64::new(0),
            sample_rate: AtomicU64::new(48_000),
        }
    }
}
enum Message {
    Sample(f32),
    Boundary {
        event: BackendEvent,
        position: u64,
        duration: i64,
        gain: f32,
    },
    End(BackendEvent),
    Position(BackendEvent),
}
enum Command {
    Load(u64, BackendRun, usize, i64, bool),
    Upcoming(u64, BackendRun, ItemKey),
    Item(ItemKey, i64),
    Seek(i64),
    Transport(bool),
    Stop,
    Device(Option<String>),
}
struct Envelope {
    command: Command,
    done: oneshot::Sender<()>,
}
pub struct LocalBackend {
    commands: std::sync::mpsc::Sender<Envelope>,
    events: broadcast::Sender<BackendEvent>,
    shared: Arc<Shared>,
    sink: Arc<dyn AudioSink>,
}
impl LocalBackend {
    pub fn new(
        resolver: Arc<dyn StreamResolver>,
        persistence: Arc<dyn PlaybackPersistence>,
        cache: AudioCache,
        sink: Arc<dyn AudioSink>,
    ) -> Result<Self, LocalError> {
        let (commands, rx) = std::sync::mpsc::channel();
        let (events, _) = broadcast::channel(BACKEND_EVENT_CAPACITY);
        let shared = Arc::new(Shared::default());
        let runtime = tokio::runtime::Handle::current();
        let worker_sink = sink.clone();
        let worker_shared = shared.clone();
        let worker_events = events.clone();
        std::thread::Builder::new()
            .name("ap-audio-decoder".into())
            .spawn(move || {
                let mut worker = Worker {
                    resolver,
                    persistence,
                    cache,
                    sink: worker_sink,
                    runtime,
                    shared: worker_shared,
                    events: worker_events,
                    load: 0,
                    run: vec![],
                    index: 0,
                    decoder: None,
                    producer: None,
                    stream: None,
                    messages: VecDeque::new(),
                    device: None,
                    lost: Arc::default(),
                    format: OutputFormat {
                        sample_rate: 48_000,
                        channels: 2,
                    },
                    ended: false,
                    prefetch: None,
                    output_events: None,
                    next_default_check: std::time::Instant::now(),
                    default_name: None,
                    audible: None,
                };
                worker.run(&rx);
            })?;
        Ok(Self {
            commands,
            events,
            shared,
            sink,
        })
    }
    async fn command(&self, command: Command) {
        let (done, wait) = oneshot::channel();
        if self.commands.send(Envelope { command, done }).is_ok() {
            let _ = wait.await;
        }
    }
    pub fn output_devices(&self) -> Result<Vec<String>, LocalError> {
        self.sink.devices()
    }
    pub async fn select_output_device(&self, name: Option<String>) {
        self.command(Command::Device(name)).await;
    }
}
#[async_trait]
impl Backend for LocalBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Local
    }
    fn subscribe(&self) -> broadcast::Receiver<BackendEvent> {
        self.events.subscribe()
    }
    async fn load(
        &self,
        id: u64,
        run: BackendRun,
        index: usize,
        position: i64,
        play: bool,
        _: ChangeOrigin,
    ) {
        self.command(Command::Load(id, run, index, position, play))
            .await;
    }
    async fn update_upcoming(&self, id: u64, run: BackendRun, current: ItemKey) {
        self.command(Command::Upcoming(id, run, current)).await;
    }
    async fn seek_to_item(&self, key: ItemKey, position: i64) {
        self.command(Command::Item(key, position)).await;
    }
    async fn play(&self, _: ChangeOrigin) {
        self.command(Command::Transport(true)).await;
    }
    async fn pause(&self) {
        self.command(Command::Transport(false)).await;
    }
    async fn seek_to(&self, position: i64) {
        self.command(Command::Seek(position)).await;
    }
    async fn set_volume(&self, volume: i32) {
        self.shared
            .volume
            .store(volume.clamp(0, 100), Ordering::Release);
    }
    async fn set_normalization(&self, enabled: bool) {
        self.shared.normalization.store(enabled, Ordering::Release);
    }
    async fn set_repeat_one(&self, one: bool) {
        self.shared.repeat.store(one, Ordering::Release);
    }
    async fn stop(&self) {
        self.command(Command::Stop).await;
    }
    fn position_ms(&self) -> i64 {
        i64::try_from(
            self.shared
                .position
                .load(Ordering::Acquire)
                .saturating_mul(1000)
                / self.shared.sample_rate.load(Ordering::Acquire).max(1),
        )
        .unwrap_or(i64::MAX)
    }
    fn duration_ms(&self) -> i64 {
        self.shared.duration.load(Ordering::Acquire)
    }
}
fn render(
    mut consumer: Consumer<Message>,
    shared: Arc<Shared>,
    events: std::sync::mpsc::SyncSender<BackendEvent>,
    format: OutputFormat,
) -> Render {
    let mut gain = 1.0_f32;
    let mut applied = 0.0_f32;
    let mut target = 0.0_f32;
    let mut ramp = 0_u32;
    let mut channel = 0_u16;
    let mut pending = None;
    Box::new(move |output| {
        output.fill(0.0);
        if !shared.playing.load(Ordering::Acquire) {
            return;
        }
        for sample in output.iter_mut() {
            loop {
                let message = pending.take().or_else(|| consumer.pop().ok());
                match message {
                    Some(Message::Sample(value)) => {
                        #[allow(clippy::cast_precision_loss)]
                        let next = shared.volume.load(Ordering::Relaxed) as f32 / 100.0
                            * if shared.normalization.load(Ordering::Relaxed) {
                                gain
                            } else {
                                1.0
                            };
                        if next.to_bits() != target.to_bits() {
                            target = next;
                            ramp = (format.sample_rate / 100).max(1) * u32::from(format.channels);
                        }
                        if ramp > 0 {
                            #[allow(clippy::cast_precision_loss)]
                            {
                                applied += (target - applied) / ramp as f32;
                            }
                            ramp -= 1;
                        }
                        *sample = value * applied;
                        channel += 1;
                        if channel == format.channels {
                            channel = 0;
                            shared.position.fetch_add(1, Ordering::Release);
                        }
                        break;
                    }
                    Some(Message::Boundary {
                        event,
                        position,
                        duration,
                        gain: next_gain,
                    }) => match events.try_send(event) {
                        Ok(()) => {
                            gain = next_gain;
                            channel = 0;
                            shared.position.store(position, Ordering::Release);
                            shared.duration.store(duration, Ordering::Release);
                        }
                        Err(std::sync::mpsc::TrySendError::Full(event)) => {
                            pending = Some(Message::Boundary {
                                event,
                                position,
                                duration,
                                gain: next_gain,
                            });
                            return;
                        }
                        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => return,
                    },
                    Some(Message::Position(event)) => {
                        if let BackendEvent::PositionDiscontinuity { duration_ms, .. } = &event {
                            shared.duration.store(*duration_ms, Ordering::Release);
                        }
                        match events.try_send(event) {
                            Ok(()) => {}
                            Err(std::sync::mpsc::TrySendError::Full(event)) => {
                                pending = Some(Message::Position(event));
                                return;
                            }
                            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => return,
                        }
                    }
                    Some(Message::End(event)) => {
                        match events.try_send(event) {
                            Ok(()) => {
                                shared.playing.store(false, Ordering::Release);
                            }
                            Err(std::sync::mpsc::TrySendError::Full(event)) => {
                                pending = Some(Message::End(event));
                            }
                            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {}
                        }
                        return;
                    }
                    None => return,
                }
            }
        }
    })
}
struct Decoded {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    rate: u32,
    channels: usize,
    samples: VecDeque<f32>,
    eof: bool,
    resampler: Option<FftFixedIn<f32>>,
    output: OutputFormat,
    gain: f32,
    duration: i64,
    skip_frames: u64,
    input_frames: u64,
    start_position: u64,
    output_skip: usize,
    output_frames: u64,
    output_limit: Option<u64>,
}
impl Decoded {
    fn open(
        source: Box<dyn MediaSource>,
        output: OutputFormat,
        position: i64,
    ) -> Result<Self, LocalError> {
        let stream = MediaSourceStream::new(
            source,
            symphonia::core::io::MediaSourceStreamOptions::default(),
        );
        let probed = symphonia::default::get_probe()
            .format(
                &Hint::new(),
                stream,
                &FormatOptions {
                    enable_gapless: true,
                    ..Default::default()
                },
                &MetadataOptions::default(),
            )
            .map_err(|e| LocalError::Decode(format!("Probe: {e}")))?;
        let format = probed.format;
        let track = format
            .default_track()
            .ok_or_else(|| LocalError::Decode("No audio track".into()))?;
        let params = &track.codec_params;
        let rate = params
            .sample_rate
            .ok_or_else(|| LocalError::Decode("Unknown sample rate".into()))?;
        let channels = params
            .channels
            .ok_or_else(|| LocalError::Decode("Unknown channels".into()))?
            .count();
        let duration = params.n_frames.map_or(0, |n| {
            i64::try_from(n.saturating_mul(1000) / u64::from(rate)).unwrap_or(i64::MAX)
        });
        let decoder = symphonia::default::get_codecs()
            .make(params, &DecoderOptions::default())
            .map_err(|e| LocalError::Decode(e.to_string()))?;
        let track_id = track.id;
        let resampler = if rate == output.sample_rate {
            None
        } else {
            Some(
                FftFixedIn::new(
                    rate as usize,
                    output.sample_rate as usize,
                    1024,
                    2,
                    channels,
                )
                .map_err(|e| LocalError::Decode(e.to_string()))?,
            )
        };
        let skip_frames = u64::try_from(position.max(0))
            .unwrap_or(0)
            .saturating_mul(u64::from(rate))
            / 1000;
        let output_skip = resampler.as_ref().map_or(0, Resampler::output_delay);
        let output_limit = params.n_frames.map(|n| {
            n.saturating_sub(skip_frames) * u64::from(output.sample_rate) / u64::from(rate)
        });
        Ok(Self {
            format,
            decoder,
            track_id,
            rate,
            channels,
            samples: VecDeque::new(),
            eof: false,
            resampler,
            output,
            gain: 1.0,
            duration,
            skip_frames,
            input_frames: 0,
            start_position: u64::try_from(position.max(0)).unwrap_or(0)
                * u64::from(output.sample_rate)
                / 1000,
            output_skip,
            output_frames: 0,
            output_limit,
        })
    }
    fn packet(&mut self) -> Result<(), LocalError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(DecodeError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    self.eof = true;
                    return Ok(());
                }
                Err(e) => return Err(LocalError::Decode(e.to_string())),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let audio = self
                .decoder
                .decode(&packet)
                .map_err(|e| LocalError::Decode(format!("Frame: {e}")))?;
            let mut samples = SampleBuffer::<f32>::new(audio.capacity() as u64, *audio.spec());
            samples.copy_interleaved_ref(audio);
            let samples = samples.samples();
            self.input_frames += (samples.len() / self.channels) as u64;
            let skip = self.skip_frames.min((samples.len() / self.channels) as u64);
            self.skip_frames -= skip;
            self.samples.extend(
                samples
                    .iter()
                    .skip(
                        usize::try_from(skip).expect("skip bounded by packet size") * self.channels,
                    )
                    .copied(),
            );
            return Ok(());
        }
    }
    fn analyze(&mut self) -> Result<f32, LocalError> {
        let target = self.rate as usize * self.channels * 6;
        while self.samples.len() < target && !self.eof {
            self.packet()?;
        }
        let slice = self.samples.make_contiguous();
        self.gain = track_gain(slice[..slice.len().min(target)].iter().copied());
        Ok(self.gain)
    }
    /// Apply an initial seek after analyzing the beginning, retaining already-decoded pre-roll.
    fn skip_to(&mut self, position: i64) {
        let frames = u64::try_from(position.max(0)).unwrap_or(0) * u64::from(self.rate) / 1000;
        let buffered = (self.samples.len() / self.channels) as u64;
        let discard =
            usize::try_from(frames.min(buffered)).expect("bounded by buffer") * self.channels;
        self.samples.drain(..discard);
        self.skip_frames = frames.saturating_sub(buffered);
        self.start_position =
            u64::try_from(position.max(0)).unwrap_or(0) * u64::from(self.output.sample_rate) / 1000;
        self.output_limit = self
            .output_limit
            .map(|n| n.saturating_sub(self.start_position));
    }
    fn next(&mut self) -> Result<Option<Vec<f32>>, LocalError> {
        loop {
            if self.eof && self.output_limit.is_some_and(|n| self.output_frames >= n) {
                return Ok(None);
            }
            let frames = self
                .resampler
                .as_ref()
                .map_or(1024, Resampler::input_frames_next);
            let needed = frames * self.channels;
            while self.samples.len() < needed && !self.eof {
                self.packet()?;
            }
            if self.eof {
                let total =
                    self.input_frames * u64::from(self.output.sample_rate) / u64::from(self.rate);
                let actual = total.saturating_sub(self.start_position);
                self.output_limit =
                    Some(self.output_limit.map_or(actual, |limit| limit.min(actual)));
                if self.duration == 0 {
                    self.duration = i64::try_from(self.input_frames * 1000 / u64::from(self.rate))
                        .unwrap_or(i64::MAX);
                }
            }
            if self.samples.is_empty() && self.resampler.is_none() {
                return Ok(None);
            }
            let frames_in = self.samples.len().min(needed) / self.channels;
            let mut planar = vec![vec![0.0; frames_in]; self.channels];
            for frame in 0..frames_in {
                for channel in &mut planar {
                    channel[frame] = self.samples.pop_front().expect("count checked");
                }
            }
            let planar = if let Some(resampler) = &mut self.resampler {
                resampler
                    .process_partial(Some(&planar), None)
                    .map_err(|e| LocalError::Decode(e.to_string()))?
            } else {
                planar
            };
            let skip = self.output_skip.min(planar[0].len());
            self.output_skip -= skip;
            let available = planar[0].len() - skip;
            let remaining = self
                .output_limit
                .map_or(u64::MAX, |n| n.saturating_sub(self.output_frames));
            let frames_out = available.min(usize::try_from(remaining).unwrap_or(usize::MAX));
            if frames_out == 0 {
                if self.eof && remaining == 0 {
                    return Ok(None);
                }
                continue;
            }
            let mut output = Vec::with_capacity(frames_out * usize::from(self.output.channels));
            for frame in skip..skip + frames_out {
                for channel in 0..usize::from(self.output.channels) {
                    let sample = if self.output.channels == 1 && self.channels > 1 {
                        #[allow(clippy::cast_precision_loss)]
                        {
                            planar.iter().map(|c| c[frame]).sum::<f32>() / self.channels as f32
                        }
                    } else {
                        planar[channel.min(self.channels - 1)][frame]
                    };
                    output.push(sample);
                }
            }
            self.output_frames += frames_out as u64;
            return Ok(Some(output));
        }
    }
}
struct Worker {
    resolver: Arc<dyn StreamResolver>,
    persistence: Arc<dyn PlaybackPersistence>,
    cache: AudioCache,
    sink: Arc<dyn AudioSink>,
    runtime: tokio::runtime::Handle,
    shared: Arc<Shared>,
    events: broadcast::Sender<BackendEvent>,
    load: u64,
    run: Vec<RunItem>,
    index: usize,
    decoder: Option<Decoded>,
    producer: Option<Producer<Message>>,
    stream: Option<Box<dyn SinkStream>>,
    messages: VecDeque<Message>,
    device: Option<String>,
    lost: Arc<AtomicBool>,
    format: OutputFormat,
    ended: bool,
    prefetch: Option<ItemKey>,
    output_events: Option<std::sync::mpsc::Receiver<BackendEvent>>,
    next_default_check: std::time::Instant,
    default_name: Option<String>,
    audible: Option<ItemKey>,
}
impl Worker {
    fn emit(&self, event: BackendEvent) {
        let _ = self.events.send(event);
    }
    fn stop(&mut self) {
        self.shared.playing.store(false, Ordering::Release);
        self.stream = None;
        self.producer = None;
        self.decoder = None;
        self.messages.clear();
        self.output_events = None;
    }
    fn start_stream(&mut self) -> Result<(), LocalError> {
        self.stream = None;
        self.format = self.sink.format(self.device.as_deref())?;
        self.shared
            .sample_rate
            .store(u64::from(self.format.sample_rate), Ordering::Release);
        let (producer, consumer) = RingBuffer::new(
            self.format.sample_rate as usize * usize::from(self.format.channels) / 2,
        );
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        self.output_events = Some(rx);
        self.lost.store(false, Ordering::Release);
        self.stream = Some(self.sink.start(
            self.device.as_deref(),
            render(consumer, self.shared.clone(), tx, self.format),
            self.lost.clone(),
        )?);
        self.producer = Some(producer);
        self.default_name = self.sink.default_device_name();
        Ok(())
    }
    fn open(&mut self, position: i64) -> Result<(), LocalError> {
        let item = self
            .run
            .get(self.index)
            .ok_or_else(|| LocalError::Decode("Run item not found".into()))?;
        let (source, key): (Box<dyn MediaSource>, Option<String>) = match item {
            RunItem::Song { track, .. } => {
                let request = self
                    .runtime
                    .block_on(self.resolver.resolve(track))
                    .map_err(|e| LocalError::Decode(e.to_string()))?;
                let key = track_key(track);
                (self.source(request, &key)?, Some(key))
            }
            RunItem::Break { path, .. } => {
                let path = reqwest::Url::parse(path)
                    .ok()
                    .filter(|u| u.scheme() == "file")
                    .and_then(|u| u.to_file_path().ok())
                    .unwrap_or_else(|| PathBuf::from(path));
                (Box::new(File::open(path)?), None)
            }
        };
        let cached_gain = key
            .as_ref()
            .and_then(|key| self.runtime.block_on(self.persistence.track_gain(key)));
        let needs_analysis = key.is_some() && cached_gain.is_none();
        let mut decoded = Decoded::open(
            source,
            self.format,
            if needs_analysis { 0 } else { position },
        )?;
        if let Some(gain) = cached_gain {
            decoded.gain = gain;
        } else if let Some(key) = key {
            let gain = decoded.analyze()?;
            self.runtime
                .block_on(self.persistence.put_track_gain(&key, gain));
            decoded.skip_to(position);
        }
        let event = BackendEvent::ItemStarted {
            load_id: self.load,
            key: item.key(),
        };
        self.messages.push_back(Message::Boundary {
            event,
            position: decoded.start_position,
            duration: decoded.duration,
            gain: decoded.gain,
        });
        self.decoder = Some(decoded);
        self.ended = false;
        Ok(())
    }
    fn source(
        &self,
        request: StreamRequest,
        key: &str,
    ) -> Result<Box<dyn MediaSource>, LocalError> {
        if request.url.scheme() == "file" {
            Ok(Box::new(File::open(
                request
                    .url
                    .to_file_path()
                    .map_err(|()| io::Error::other("Invalid file URL"))?,
            )?))
        } else {
            Ok(Box::new(HttpSource::new(
                request,
                key.into(),
                self.cache.clone(),
            )?))
        }
    }
    fn restart(&mut self, position: i64) -> Result<(), LocalError> {
        let playing = self.shared.playing.swap(false, Ordering::AcqRel);
        self.messages.clear();
        self.start_stream()?;
        self.open(position)?;
        self.shared.playing.store(playing, Ordering::Release);
        self.emit(BackendEvent::PositionDiscontinuity {
            load_id: self.load,
            position_ms: position,
            duration_ms: self.decoder.as_ref().map_or(0, |d| d.duration),
        });
        Ok(())
    }
    fn handle(&mut self, command: Command) -> Result<(), LocalError> {
        match command {
            Command::Transport(playing) => {
                self.shared.playing.store(playing, Ordering::Release);
                self.emit(BackendEvent::PlayingChanged {
                    load_id: self.load,
                    is_playing: playing,
                    buffering: false,
                    by_player: false,
                });
            }
            Command::Load(id, run, index, position, play) => {
                self.stop();
                self.load = id;
                self.run = run.items;
                self.index = index;
                self.prefetch = None;
                self.audible = self.run.get(index).map(RunItem::key);
                self.start_stream()?;
                self.open(position)?;
                self.shared.playing.store(play, Ordering::Release);
                self.emit(BackendEvent::PlayingChanged {
                    load_id: id,
                    is_playing: play,
                    buffering: false,
                    by_player: false,
                });
            }
            Command::Upcoming(id, run, current) if id == self.load => {
                // Already-decoded look-ahead can contain removed items. Rebuild from consumed
                // position when the future changes; the usual sliding-only edit retains ring.
                let old_index = self.run.iter().position(|i| i.key() == current);
                let new_index = run.items.iter().position(|i| i.key() == current);
                if let (Some(old), Some(new)) = (old_index, new_index) {
                    let old_tail = &self.run[old..];
                    let new_tail = &run.items[new..];
                    // Appending a sliding window must not restart the audible item or ring.
                    let changed =
                        new_tail.len() < old_tail.len() || new_tail[..old_tail.len()] != *old_tail;
                    self.run = run.items;
                    if changed {
                        self.index = new;
                        let position = self.position();
                        self.restart(position)?;
                    } else {
                        self.index = new + self.index.saturating_sub(old);
                    }
                }
            }
            Command::Upcoming(..) => {}
            Command::Item(key, position) => {
                if let Some(index) = self.run.iter().position(|i| i.key() == key) {
                    self.index = index;
                    self.restart(position)?;
                }
            }
            Command::Seek(position) => {
                // Resolve current audible item, not the decoder's preloaded next item.
                if let Some(index) = self.run.iter().position(|i| Some(i.key()) == self.audible) {
                    self.index = index;
                }
                self.restart(position.max(0))?;
            }
            Command::Stop => {
                self.stop();
                self.run.clear();
            }
            Command::Device(name) => {
                self.device = name;
                if !self.run.is_empty() {
                    self.restart(self.position())?;
                }
            }
        }
        Ok(())
    }
    fn position(&self) -> i64 {
        i64::try_from(
            self.shared.position.load(Ordering::Acquire) * 1000
                / u64::from(self.format.sample_rate),
        )
        .unwrap_or(i64::MAX)
    }
    fn error(&mut self, error: &LocalError) {
        let key = self.run.get(self.index).map(RunItem::key);
        self.stop();
        if matches!(error, LocalError::Output(_)) {
            self.emit(BackendEvent::PlayingChanged {
                load_id: self.load,
                is_playing: false,
                buffering: false,
                by_player: true,
            });
        }
        self.emit(BackendEvent::Error {
            load_id: self.load,
            key,
            message: error.to_string(),
        });
    }
    fn fill(&mut self) -> Result<(), LocalError> {
        if self.producer.as_ref().is_none_or(Producer::is_full) {
            return Ok(());
        }
        if self.messages.is_empty() && !self.ended {
            if let Some(decoder) = &mut self.decoder {
                if let Some(samples) = decoder.next()? {
                    self.messages
                        .extend(samples.into_iter().map(Message::Sample));
                } else {
                    let duration = if decoder.duration > 0 {
                        decoder.duration
                    } else {
                        i64::try_from(decoder.input_frames * 1000 / u64::from(decoder.rate))
                            .unwrap_or(i64::MAX)
                    };
                    self.messages.push_back(Message::Position(
                        BackendEvent::PositionDiscontinuity {
                            load_id: self.load,
                            position_ms: i64::try_from(
                                (decoder.start_position + decoder.output_frames) * 1000
                                    / u64::from(self.format.sample_rate),
                            )
                            .unwrap_or(i64::MAX),
                            duration_ms: duration,
                        },
                    ));
                    let repeat = self.shared.repeat.load(Ordering::Acquire)
                        && matches!(self.run.get(self.index), Some(RunItem::Song { .. }));
                    if !repeat {
                        self.index += 1;
                    }
                    if self.index < self.run.len() {
                        self.open(0)?;
                    } else {
                        self.ended = true;
                        self.messages
                            .push_back(Message::End(BackendEvent::RunEnded { load_id: self.load }));
                    }
                }
            }
        }
        if let Some(producer) = &mut self.producer {
            while !producer.is_full() {
                let Some(message) = self.messages.pop_front() else {
                    break;
                };
                if let Err(rtrb::PushError::Full(message)) = producer.push(message) {
                    self.messages.push_front(message);
                    break;
                }
            }
        }
        Ok(())
    }
    fn prefetch(&mut self) {
        let duration = self.shared.duration.load(Ordering::Acquire);
        if duration <= 0 || duration - self.position() > 30_000 {
            return;
        }
        let Some(item @ RunItem::Song { track, .. }) = self.run.get(self.index + 1) else {
            return;
        };
        if self.prefetch.as_ref() == Some(&item.key()) {
            return;
        }
        self.prefetch = Some(item.key());
        let track = track.clone();
        let resolver = self.resolver.clone();
        let cache = self.cache.clone();
        let persistence = self.persistence.clone();
        let runtime = self.runtime.clone();
        let format = self.format;
        self.runtime.spawn_blocking(move || {
            let result = (|| -> Result<(), LocalError> {
                let request = runtime
                    .block_on(resolver.resolve(&track))
                    .map_err(|e| LocalError::Decode(e.to_string()))?;
                if request.url.scheme() == "file" {
                    return Ok(());
                }
                let key = track_key(&track);
                let mut source = HttpSource::new(request.clone(), key.clone(), cache.clone())?;
                source.prefetch()?;
                if runtime.block_on(persistence.track_gain(&key)).is_none() {
                    let mut decoder = Decoded::open(
                        Box::new(HttpSource::new(request, key.clone(), cache)?),
                        format,
                        0,
                    )?;
                    let gain = decoder.analyze()?;
                    runtime.block_on(persistence.put_track_gain(&key, gain));
                }
                Ok(())
            })();
            if let Err(error) = result {
                tracing::debug!(%error, "Audio prefetch failed");
            }
        });
    }
    fn run(&mut self, rx: &std::sync::mpsc::Receiver<Envelope>) {
        loop {
            match rx.recv_timeout(Duration::from_millis(2)) {
                Ok(envelope) => {
                    if let Err(error) = self.handle(envelope.command) {
                        self.error(&error);
                    }
                    let _ = envelope.done.send(());
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
            while let Some(event) = self
                .output_events
                .as_ref()
                .and_then(|rx| rx.try_recv().ok())
            {
                if let BackendEvent::ItemStarted { key, .. } = &event {
                    self.audible = Some(key.clone());
                }
                self.emit(event);
            }
            if self.stream.is_some() {
                if self.lost.swap(false, Ordering::AcqRel) {
                    self.shared.playing.store(false, Ordering::Release);
                    self.emit(BackendEvent::PlayingChanged {
                        load_id: self.load,
                        is_playing: false,
                        buffering: false,
                        by_player: true,
                    });
                    if let Some(index) = self.run.iter().position(|i| Some(i.key()) == self.audible)
                    {
                        self.index = index;
                    }
                    if let Err(error) = self.restart(self.position()) {
                        self.error(&error);
                    }
                }
                if self.device.is_none() && self.next_default_check <= std::time::Instant::now() {
                    self.next_default_check = std::time::Instant::now() + Duration::from_secs(1);
                    let name = self.sink.default_device_name();
                    if name != self.default_name {
                        if let Some(index) =
                            self.run.iter().position(|i| Some(i.key()) == self.audible)
                        {
                            self.index = index;
                        }
                        if let Err(error) = self.restart(self.position()) {
                            self.error(&error);
                        }
                    }
                }
                self.prefetch();
                if let Err(error) = self.fill() {
                    self.decoder = None;
                    self.ended = true;
                    self.messages.push_back(Message::End(BackendEvent::Error {
                        load_id: self.load,
                        key: self.run.get(self.index).map(RunItem::key),
                        message: error.to_string(),
                    }));
                }
            }
        }
        self.stop();
    }
}
