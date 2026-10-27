//! Serialized Spotify Connect control. No audio decoding or Spotify queue endpoint.
use crate::backend::{BACKEND_EVENT_CAPACITY, Backend, BackendEvent, BackendRun, ItemKey, RunItem};
use crate::state::ChangeOrigin;
use ap_model::{BackendKind, normalize_spotify_track_id};
use async_trait::async_trait;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio::time::{Instant, sleep_until};

pub const ADOPT_TOLERANCE_MS: i64 = 3_000;
pub const SETTLE_WINDOW_MS: u64 = 3_000;
pub const POLL_PLAYING_MS: u64 = 2_000;
pub const POLL_NEAR_END_MS: u64 = 1_000;
pub const POLL_PAUSED_MS: u64 = 5_000;
pub const END_LEAD_MS: i64 = 300;
pub const END_TOLERANCE_MS: i64 = 700;
pub const DRIFT_TOLERANCE_MS: i64 = 750;
pub const STALL_MS: u64 = 10_000;
pub const STOPPED_AT_END_POLLS: usize = 2;
pub const MAX_RECOVERY_ATTEMPTS: usize = 3;
pub const DEVICE_CACHE_TTL: Duration = Duration::from_secs(30);
pub const DEVICE_WAIT_INTERVAL: Duration = Duration::from_millis(1_500);
pub const DEVICE_WAIT_ATTEMPTS: usize = 5;
pub const RELOAD_DEBOUNCE: Duration = Duration::from_millis(300);
pub const VOLUME_DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum SpotifyError {
    #[error(
        "No active Spotify Connect device found. Open Spotify on this computer (or another device on the same account) and try again."
    )]
    NoDevice,
    #[error("Spotify isn't installed. Install it, open it, and try again.")]
    NotInstalled,
    #[error("Premium required for full Spotify playback via Connect")]
    PremiumRequired,
    #[error("Spotify authorization expired. Reconnect it in Settings.")]
    Unauthorized,
    #[error("Spotify HTTP {status}: {message}")]
    Http { status: u16, message: String },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotifyDevice {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub is_active: bool,
    pub is_restricted: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreferredDevice {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotifyPlayback {
    pub id: String,
    pub linked_from: Option<String>,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub is_playing: bool,
    pub shuffle: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayRequest {
    pub uris: Vec<String>,
    pub offset: usize,
    pub position_ms: i64,
    pub device_id: String,
}
#[async_trait]
pub trait SpotifyPlayerApi: Send + Sync + 'static {
    async fn devices(&self) -> Result<Vec<SpotifyDevice>, SpotifyError>;
    async fn current_playback(&self) -> Result<Option<SpotifyPlayback>, SpotifyError>;
    async fn play(&self, request: PlayRequest) -> Result<(), SpotifyError>;
    async fn pause(&self, device_id: &str) -> Result<(), SpotifyError>;
    async fn seek(&self, position_ms: i64, device_id: &str) -> Result<(), SpotifyError>;
    async fn volume(&self, percent: i32, device_id: &str) -> Result<(), SpotifyError>;
    async fn shuffle(&self, enabled: bool, device_id: &str) -> Result<(), SpotifyError>;
    async fn repeat(&self, one: bool, device_id: &str) -> Result<(), SpotifyError>;
}
#[async_trait]
pub trait SpotifyLauncher: Send + Sync + 'static {
    fn hostname(&self) -> String;
    async fn launch(&self) -> Result<(), SpotifyError>;
}
struct DevicesState {
    preferred: Option<PreferredDevice>,
    cached: Option<(String, Instant)>,
}
/// Single-flight discovery, including client launch and five bounded readiness polls.
pub struct SpotifyDeviceResolver {
    api: Arc<dyn SpotifyPlayerApi>,
    launcher: Arc<dyn SpotifyLauncher>,
    state: Mutex<DevicesState>,
}
impl SpotifyDeviceResolver {
    pub fn new(api: Arc<dyn SpotifyPlayerApi>, launcher: Arc<dyn SpotifyLauncher>) -> Self {
        Self {
            api,
            launcher,
            state: Mutex::new(DevicesState {
                preferred: None,
                cached: None,
            }),
        }
    }
    pub async fn available(&self) -> Result<Vec<SpotifyDevice>, SpotifyError> {
        Ok(self
            .api
            .devices()
            .await?
            .into_iter()
            .filter(|d| !d.is_restricted && !d.id.is_empty())
            .collect())
    }
    pub async fn set_preferred(&self, preferred: Option<PreferredDevice>) {
        let mut state = self.state.lock().await;
        state.preferred = preferred;
        state.cached = None;
    }
    pub async fn invalidate(&self) {
        self.state.lock().await.cached = None;
    }
    fn pick(
        &self,
        devices: &[SpotifyDevice],
        preferred: Option<&PreferredDevice>,
    ) -> Option<String> {
        preferred
            .and_then(|p| {
                devices
                    .iter()
                    .find(|d| d.id == p.id)
                    .or_else(|| devices.iter().find(|d| d.name == p.name))
            })
            .or_else(|| devices.iter().find(|d| d.is_active))
            .or_else(|| {
                devices
                    .iter()
                    .find(|d| d.name.eq_ignore_ascii_case(&self.launcher.hostname()))
            })
            .or_else(|| {
                devices
                    .iter()
                    .find(|d| d.device_type.eq_ignore_ascii_case("Computer"))
            })
            .map(|d| d.id.clone())
    }
    pub async fn resolve(&self, origin: ChangeOrigin) -> Result<String, SpotifyError> {
        let mut state = self.state.lock().await;
        if let Some((id, at)) = &state.cached
            && at.elapsed() <= DEVICE_CACHE_TTL
        {
            return Ok(id.clone());
        }
        let mut devices = self.available().await?;
        let mut picked = self.pick(&devices, state.preferred.as_ref());
        if picked.is_none() && origin == ChangeOrigin::User {
            match self.launcher.launch().await {
                Ok(()) => {
                    for _ in 0..DEVICE_WAIT_ATTEMPTS {
                        tokio::time::sleep(DEVICE_WAIT_INTERVAL).await;
                        devices = self.available().await?;
                        picked = self.pick(&devices, state.preferred.as_ref());
                        if picked.is_some() {
                            break;
                        }
                    }
                }
                Err(e) if devices.is_empty() => return Err(e),
                Err(_) => {}
            }
        }
        let id = picked
            .or_else(|| devices.first().map(|d| d.id.clone()))
            .ok_or(SpotifyError::NoDevice)?;
        state.cached = Some((id.clone(), Instant::now()));
        Ok(id)
    }
}
#[derive(Clone, Copy)]
struct Anchor {
    position: i64,
    duration: i64,
    at: Instant,
    playing: bool,
}
impl Anchor {
    fn position(self) -> i64 {
        let elapsed = if self.playing {
            i64::try_from(self.at.elapsed().as_millis()).unwrap_or(i64::MAX)
        } else {
            0
        };
        let value = self.position.saturating_add(elapsed).max(0);
        if self.duration > 0 {
            value.min(self.duration)
        } else {
            value
        }
    }
}
enum Command {
    Load(u64, BackendRun, usize, i64, bool, ChangeOrigin),
    Upcoming(u64, BackendRun, ItemKey),
    Item(ItemKey, i64),
    Play(ChangeOrigin),
    Pause,
    Seek(i64),
    Volume(i32),
    Repeat(bool),
    Stop,
}
struct Envelope {
    command: Command,
    done: oneshot::Sender<()>,
}
/// All requests and polls run on one actor; dropped command futures never cancel requests.
pub struct SpotifyBackend {
    commands: mpsc::UnboundedSender<Envelope>,
    events: broadcast::Sender<BackendEvent>,
    anchor: Arc<RwLock<Anchor>>,
    devices: Arc<SpotifyDeviceResolver>,
}
impl SpotifyBackend {
    pub fn new(api: Arc<dyn SpotifyPlayerApi>, devices: Arc<SpotifyDeviceResolver>) -> Self {
        let (commands, rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(BACKEND_EVENT_CAPACITY);
        let anchor = Arc::new(RwLock::new(Anchor {
            position: 0,
            duration: 0,
            at: Instant::now(),
            playing: false,
        }));
        tokio::spawn(Actor::new(api, devices.clone(), events.clone(), anchor.clone()).run(rx));
        Self {
            commands,
            events,
            anchor,
            devices,
        }
    }
    async fn command(&self, command: Command) {
        let (done, wait) = oneshot::channel();
        if self.commands.send(Envelope { command, done }).is_ok() {
            let _ = wait.await;
        }
    }
}
struct Actor {
    api: Arc<dyn SpotifyPlayerApi>,
    devices: Arc<SpotifyDeviceResolver>,
    events: broadcast::Sender<BackendEvent>,
    anchor: Arc<RwLock<Anchor>>,
    items: Vec<RunItem>,
    index: usize,
    stop: usize,
    load: u64,
    device: String,
    holding: bool,
    started: bool,
    intended: bool,
    repeat: bool,
    volume: i32,
    playing_seen: bool,
    unknown: Option<String>,
    unknown_count: usize,
    stopped_count: usize,
    null_count: usize,
    settle: Instant,
    raw_position: i64,
    progress_at: Instant,
    recoveries: usize,
    recovery_at: Option<Instant>,
    poll_at: Option<Instant>,
    reload: Option<(Instant, Vec<RunItem>, ItemKey)>,
    volume_at: Option<Instant>,
    manual_pause_until: Option<Instant>,
    start_at: Instant,
    repeat_dirty: bool,
    volume_dirty: bool,
}
impl Actor {
    fn new(
        api: Arc<dyn SpotifyPlayerApi>,
        devices: Arc<SpotifyDeviceResolver>,
        events: broadcast::Sender<BackendEvent>,
        anchor: Arc<RwLock<Anchor>>,
    ) -> Self {
        Self {
            api,
            devices,
            events,
            anchor,
            items: vec![],
            index: 0,
            stop: 0,
            load: 0,
            device: String::new(),
            holding: false,
            started: false,
            intended: false,
            repeat: false,
            volume: 50,
            playing_seen: false,
            unknown: None,
            unknown_count: 0,
            stopped_count: 0,
            null_count: 0,
            settle: Instant::now(),
            raw_position: -1,
            progress_at: Instant::now(),
            recoveries: 0,
            recovery_at: None,
            poll_at: None,
            reload: None,
            volume_at: None,
            manual_pause_until: None,
            start_at: Instant::now(),
            repeat_dirty: true,
            volume_dirty: true,
        }
    }
    fn emit(&self, event: BackendEvent) {
        let _ = self.events.send(event);
    }
    fn a(&self) -> Anchor {
        *self.anchor.read().expect("anchor poisoned")
    }
    fn anchor(&self, position: i64, duration: i64, playing: bool) {
        *self.anchor.write().expect("anchor poisoned") = Anchor {
            position: position.max(0),
            duration,
            playing,
            at: Instant::now(),
        };
    }
    fn report(&self, playing: bool, by_player: bool) {
        self.emit(BackendEvent::PlayingChanged {
            load_id: self.load,
            is_playing: playing,
            buffering: false,
            by_player,
        });
    }
    fn position_event(&self) {
        let a = self.a();
        self.emit(BackendEvent::PositionDiscontinuity {
            load_id: self.load,
            position_ms: a.position(),
            duration_ms: a.duration,
        });
    }
    fn key(&self) -> Option<ItemKey> {
        self.items.get(self.index).map(RunItem::key)
    }
    fn duration(&self) -> i64 {
        match self.items.get(self.index) {
            Some(RunItem::Song { track, .. }) => track.duration_ms.unwrap_or(0),
            _ => 0,
        }
    }
    fn id(item: &RunItem) -> &str {
        match item {
            RunItem::Song { track, .. } => normalize_spotify_track_id(&track.id),
            RunItem::Break { .. } => "",
        }
    }
    fn matches(item: &RunItem, s: &SpotifyPlayback) -> bool {
        let id = Self::id(item);
        id == s.id || s.linked_from.as_deref() == Some(id)
    }
    fn schedule_poll(&mut self) {
        self.poll_at = (self.holding && self.started).then(|| {
            let a = self.a();
            let wait = if !self.intended {
                POLL_PAUSED_MS
            } else if a.duration > 0 && a.duration - a.position() < 5_000 {
                POLL_NEAR_END_MS
            } else {
                POLL_PLAYING_MS
            };
            Instant::now() + Duration::from_millis(wait)
        });
    }
    fn boundary_at(&self) -> Option<Instant> {
        let a = self.a();
        (self.holding
            && self.started
            && self.playing_seen
            && self.intended
            && !self.repeat
            && self.index == self.stop
            && a.playing
            && a.duration > 0)
            .then(|| {
                Instant::now()
                    + Duration::from_millis(
                        u64::try_from((a.duration - a.position() - END_LEAD_MS).max(0))
                            .unwrap_or(0),
                    )
            })
    }
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Envelope>) {
        loop {
            let boundary = self.boundary_at();
            let deadline = [
                self.poll_at,
                boundary,
                self.reload.as_ref().map(|r| r.0),
                self.volume_at,
            ]
            .into_iter()
            .flatten()
            .min();
            tokio::select! {
                envelope = rx.recv() => {
                    let Some(envelope) = envelope else { break };
                    if let Err(e) = self.handle(envelope.command).await { self.fail(&e).await; }
                    let _ = envelope.done.send(());
                }
                () = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                    if let Err(e) = self.timers(boundary).await { self.fail(&e).await; }
                }
            }
        }
        if self.started && self.intended {
            let _ = self.api.pause(&self.device).await;
        }
    }
    async fn fail(&mut self, error: &SpotifyError) {
        tracing::warn!(%error, load_id = self.load, "Spotify Connect command failed");
        if self.started
            && self.intended
            && let Err(pause_error) = self.api.pause(&self.device).await
        {
            tracing::warn!(%pause_error, "Spotify could not stop after a playback failure");
        }
        self.emit(BackendEvent::Error {
            load_id: self.load,
            key: self.key(),
            message: error.to_string(),
        });
        self.intended = false;
        self.started = false;
        self.poll_at = None;
        self.reload = None;
        let a = self.a();
        self.anchor(a.position(), a.duration, false);
    }
    /// Activation can transiently reject volume/repeat despite accepting play. Keep the
    /// successful run and retry unsent settings on its next poll (Android parity).
    async fn apply_settings(&mut self) {
        if !self.started {
            return;
        }
        if self.repeat_dirty {
            self.repeat_dirty = self.api.repeat(self.repeat, &self.device).await.is_err();
        }
        if self.volume_dirty && self.volume_at.is_none() {
            self.volume_dirty = self.api.volume(self.volume, &self.device).await.is_err();
        }
    }
    async fn start(
        &mut self,
        position: i64,
        origin: ChangeOrigin,
        adopt: bool,
    ) -> Result<(), SpotifyError> {
        self.device = self.devices.resolve(origin).await?;
        let snapshot = self.api.current_playback().await?;
        if snapshot.as_ref().is_some_and(|s| s.shuffle) {
            self.api.shuffle(false, &self.device).await?;
        }
        let adopted = adopt
            && snapshot.as_ref().is_some_and(|s| {
                s.is_playing
                    && Self::matches(&self.items[self.index], s)
                    && (s.position_ms - position).abs() <= ADOPT_TOLERANCE_MS
            });
        if !adopted {
            let request = PlayRequest {
                uris: self
                    .items
                    .iter()
                    .map(|i| format!("spotify:track:{}", Self::id(i)))
                    .collect(),
                offset: self.index,
                position_ms: position.max(0),
                device_id: self.device.clone(),
            };
            if let Err(error) = self.api.play(request.clone()).await {
                self.devices.invalidate().await;
                let fresh = self.devices.resolve(origin).await?;
                if fresh == self.device {
                    return Err(error);
                }
                self.device = fresh;
                self.api
                    .play(PlayRequest {
                        device_id: self.device.clone(),
                        ..request
                    })
                    .await?;
            }
        }
        self.started = true;
        self.intended = true;
        self.playing_seen = adopted;
        self.repeat_dirty = true;
        self.volume_dirty = true;
        self.volume_at = None;
        self.apply_settings().await;
        self.start_at = Instant::now();
        self.manual_pause_until = None;
        self.settle = Instant::now() + Duration::from_millis(SETTLE_WINDOW_MS);
        self.raw_position = -1;
        self.progress_at = Instant::now();
        self.stopped_count = 0;
        self.unknown_count = 0;
        let (position, duration) = if adopted {
            let s = snapshot.expect("adopt checked");
            (s.position_ms, s.duration_ms)
        } else {
            (position, self.duration())
        };
        self.anchor(position, duration, adopted);
        if let Some(key) = self.key() {
            self.emit(BackendEvent::ItemStarted {
                load_id: self.load,
                key,
            });
        }
        self.report(adopted, false);
        self.position_event();
        self.schedule_poll();
        Ok(())
    }
    async fn handle(&mut self, command: Command) -> Result<(), SpotifyError> {
        match command {
            Command::Load(id, run, index, position, play, origin) => {
                self.load = id;
                self.items = run.items;
                self.index = index;
                self.stop = self.items.len().saturating_sub(1);
                self.holding = !self.items.is_empty() && index < self.items.len();
                self.started = false;
                self.intended = play;
                self.poll_at = None;
                self.reload = None;
                self.recoveries = 0;
                self.recovery_at = None;
                self.anchor(position, self.duration(), false);
                if self.holding && play {
                    self.start(position, origin, true).await?;
                }
            }
            Command::Upcoming(id, run, current) if id == self.load => {
                let Some(index) = self.items.iter().position(|i| i.key() == current) else {
                    return Ok(());
                };
                let Some(next) = run.items.iter().position(|i| i.key() == current) else {
                    return Ok(());
                };
                let old = &self.items[index..];
                let new = &run.items[next..];
                if new.len() <= old.len() && new == &old[..new.len()] {
                    self.stop = index + new.len() - 1;
                    self.reload = None;
                } else {
                    self.reload = Some((Instant::now() + RELOAD_DEBOUNCE, run.items, current));
                }
            }
            Command::Item(key, position) => {
                if let Some(index) = self.items.iter().position(|i| i.key() == key) {
                    self.index = index;
                    if self.intended {
                        self.start(position, ChangeOrigin::User, false).await?;
                    } else {
                        self.started = false;
                        self.anchor(position, self.duration(), false);
                    }
                }
            }
            Command::Play(origin) if self.holding => {
                self.recoveries = 0;
                self.start(self.a().position(), origin, false).await?;
            }
            Command::Upcoming(..) | Command::Play(_) => {}
            Command::Pause => {
                if self.started
                    && let Err(e) = self.api.pause(&self.device).await
                    && self
                        .api
                        .current_playback()
                        .await?
                        .is_some_and(|s| s.is_playing)
                {
                    return Err(e);
                }
                let a = self.a();
                self.anchor(a.position(), a.duration, false);
                self.intended = false;
                self.manual_pause_until = Some(Instant::now() + Duration::from_millis(5_000));
                self.report(false, false);
                self.schedule_poll();
            }
            Command::Seek(position) => {
                if self.started {
                    self.api.seek(position.max(0), &self.device).await?;
                }
                let a = self.a();
                self.anchor(position, a.duration, a.playing);
                self.position_event();
                self.settle = Instant::now() + Duration::from_millis(SETTLE_WINDOW_MS);
                self.raw_position = -1;
            }
            Command::Volume(value) => {
                self.volume = value.clamp(0, 100);
                self.volume_dirty = true;
                self.volume_at = Some(Instant::now() + VOLUME_DEBOUNCE);
            }
            Command::Repeat(one) => {
                self.repeat = one;
                self.repeat_dirty = true;
                self.apply_settings().await;
            }
            Command::Stop => {
                if self.started && self.intended {
                    self.api.pause(&self.device).await?;
                }
                self.holding = false;
                self.started = false;
                self.intended = false;
                self.poll_at = None;
                self.reload = None;
                let a = self.a();
                self.anchor(a.position(), a.duration, false);
            }
        }
        Ok(())
    }
    async fn timers(&mut self, boundary: Option<Instant>) -> Result<(), SpotifyError> {
        let now = Instant::now();
        if self.volume_at.is_some_and(|t| t <= now) {
            self.volume_at = None;
            self.apply_settings().await;
        }
        if self.reload.as_ref().is_some_and(|r| r.0 <= now) {
            let (_, items, key) = self.reload.take().expect("due");
            let position = self.a().position();
            self.index = items.iter().position(|i| i.key() == key).unwrap_or(0);
            self.stop = items.len().saturating_sub(1);
            self.items = items;
            if self.intended {
                self.start(position, ChangeOrigin::Sync, false).await?;
            } else {
                self.started = false;
                self.poll_at = None;
            }
        }
        if boundary.is_some_and(|t| t <= now) {
            let snapshot = self.api.current_playback().await?;
            if let Some(s) = &snapshot
                && Self::matches(&self.items[self.index], s)
                && s.is_playing
                && s.duration_ms - s.position_ms > END_LEAD_MS + END_TOLERANCE_MS
            {
                self.accept(snapshot).await?;
                return Ok(());
            }
            self.finish_boundary().await?;
        } else if self.poll_at.is_some_and(|t| t <= now) {
            let snapshot = self.api.current_playback().await?;
            self.accept(snapshot).await?;
            self.schedule_poll();
        }
        Ok(())
    }
    fn end(&mut self) {
        if !self.playing_seen || !self.holding {
            return;
        }
        self.started = false;
        self.holding = false;
        self.poll_at = None;
        self.reload = None;
        let a = self.a();
        self.anchor(a.position(), a.duration, false);
        self.emit(BackendEvent::RunEnded { load_id: self.load });
    }
    async fn finish_boundary(&mut self) -> Result<(), SpotifyError> {
        if !self.playing_seen {
            return Ok(());
        }
        if let Err(error) = self.api.pause(&self.device).await {
            if !matches!(error, SpotifyError::Http { .. }) {
                return Err(error);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            if self.api.current_playback().await?.is_some_and(|s| {
                s.is_playing
                    && (!Self::matches(&self.items[self.index], &s)
                        || s.duration_ms - s.position_ms > END_LEAD_MS)
            }) {
                self.api.pause(&self.device).await?;
            }
        }
        self.end();
        Ok(())
    }
    async fn recover(&mut self, reason: &str) -> Result<(), SpotifyError> {
        if !self.intended {
            return Ok(());
        }
        let cooldown = Duration::from_millis(if self.recoveries == 0 { 2_500 } else { 4_000 });
        if self.recovery_at.is_some_and(|at| at.elapsed() < cooldown) {
            return Ok(());
        }
        if self.recoveries >= MAX_RECOVERY_ATTEMPTS {
            return Err(SpotifyError::Http {
                status: 0,
                message: reason.into(),
            });
        }
        self.recoveries += 1;
        self.recovery_at = Some(Instant::now());
        self.start(self.raw_position.max(0), ChangeOrigin::Sync, false)
            .await
    }
    async fn accept(&mut self, snapshot: Option<SpotifyPlayback>) -> Result<(), SpotifyError> {
        if self
            .manual_pause_until
            .is_some_and(|at| Instant::now() < at)
        {
            return Ok(());
        }
        let Some(s) = snapshot else {
            self.null_count += 1;
            if self.null_count >= 3 && Instant::now() >= self.settle {
                self.recover("Spotify snapshot unavailable after interruption")
                    .await?;
            }
            return Ok(());
        };
        self.null_count = 0;
        let mapped = self
            .items
            .iter()
            .enumerate()
            .skip(self.index)
            .find(|(_, i)| Self::matches(i, &s))
            .map(|(i, _)| i)
            .or_else(|| self.items.iter().position(|i| Self::matches(i, &s)));
        let settling = Instant::now() < self.settle;
        if settling
            && (mapped != Some(self.index)
                || (s.position_ms - self.a().position()).abs() > ADOPT_TOLERANCE_MS)
        {
            return Ok(());
        }
        if mapped == Some(self.index) {
            self.settle = Instant::now();
        }
        if self.intended
            && !self.playing_seen
            && self.start_at.elapsed() >= Duration::from_millis(STALL_MS)
        {
            return self
                .recover("Spotify playback stalled while reporting playing")
                .await;
        }
        let Some(index) = mapped else {
            if self.unknown.as_ref() == Some(&s.id) {
                self.unknown_count += 1;
            } else {
                self.unknown = Some(s.id);
                self.unknown_count = 1;
            }
            if self.unknown_count >= 2 {
                self.end();
            }
            return Ok(());
        };
        self.unknown = None;
        self.unknown_count = 0;
        if index < self.index && !settling {
            if !s.is_playing && self.index >= self.stop {
                self.end();
            }
            return Ok(());
        }
        if index > self.stop {
            return self.finish_boundary().await;
        }
        let changed = index != self.index;
        if changed {
            self.index = index;
            self.playing_seen = false;
            self.raw_position = -1;
            self.stopped_count = 0;
            self.emit(BackendEvent::ItemStarted {
                load_id: self.load,
                key: self.items[index].key(),
            });
        }
        let duration = if s.duration_ms > 0 {
            s.duration_ms
        } else {
            self.duration()
        };
        if s.is_playing {
            self.playing_seen = true;
        }
        if s.shuffle
            && let Err(error) = self.api.shuffle(false, &self.device).await
        {
            tracing::debug!(%error, "Spotify shuffle correction will retry at the next poll");
        }
        self.apply_settings().await;
        if !s.is_playing
            && self.playing_seen
            && !self.repeat
            && duration > 0
            && s.position_ms >= duration - END_TOLERANCE_MS
        {
            self.stopped_count += 1;
            if self.stopped_count >= STOPPED_AT_END_POLLS {
                if index >= self.stop {
                    self.end();
                } else {
                    self.index += 1;
                    self.start(0, ChangeOrigin::Sync, false).await?;
                }
                return Ok(());
            }
        } else {
            self.stopped_count = 0;
        }
        let a = self.a();
        let state_changed = a.playing != s.is_playing;
        if !s.is_playing
            && self.playing_seen
            && self.intended
            && (duration == 0 || s.position_ms < duration - 1_500)
        {
            self.intended = false;
            self.report(false, true);
        } else if s.is_playing && state_changed {
            self.report(true, !self.intended);
            self.intended = true;
        }
        if s.is_playing {
            if s.position_ms == self.raw_position {
                if self.progress_at.elapsed() >= Duration::from_millis(STALL_MS) {
                    self.recover("Spotify playback stalled while reporting playing")
                        .await?;
                    return Ok(());
                }
            } else {
                self.raw_position = s.position_ms;
                self.progress_at = Instant::now();
                if self.recoveries > 0 && s.position_ms > a.position {
                    self.recoveries = 0;
                }
            }
        } else {
            self.progress_at = Instant::now();
        }
        if changed
            || state_changed
            || (a.position() - s.position_ms).abs() > DRIFT_TOLERANCE_MS
            || a.duration != duration
        {
            self.anchor(s.position_ms, duration, s.is_playing);
            self.position_event();
        }
        Ok(())
    }
}
#[async_trait]
impl Backend for SpotifyBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Spotify
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
        origin: ChangeOrigin,
    ) {
        self.command(Command::Load(id, run, index, position, play, origin))
            .await;
    }
    async fn update_upcoming(&self, id: u64, run: BackendRun, current: ItemKey) {
        self.command(Command::Upcoming(id, run, current)).await;
    }
    async fn seek_to_item(&self, key: ItemKey, position: i64) {
        self.command(Command::Item(key, position)).await;
    }
    async fn play(&self, origin: ChangeOrigin) {
        self.command(Command::Play(origin)).await;
    }
    async fn pause(&self) {
        self.command(Command::Pause).await;
    }
    async fn seek_to(&self, position: i64) {
        self.command(Command::Seek(position)).await;
    }
    async fn set_volume(&self, volume: i32) {
        self.command(Command::Volume(volume)).await;
    }
    async fn set_repeat_one(&self, one: bool) {
        self.command(Command::Repeat(one)).await;
    }
    async fn stop(&self) {
        self.command(Command::Stop).await;
    }
    async fn prepare(&self, origin: ChangeOrigin) -> bool {
        self.devices.resolve(origin).await.is_ok()
    }
    fn position_ms(&self) -> i64 {
        self.anchor.read().expect("anchor poisoned").position()
    }
    fn duration_ms(&self) -> i64 {
        self.anchor.read().expect("anchor poisoned").duration
    }
}
