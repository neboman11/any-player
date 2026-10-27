//! The playback engine (Android `DefaultPlaybackEngine`, design §8.1): one tokio actor owning the
//! queue and player state, plus a level-triggered reconciler that drives the backends.
//!
//! Every mutation applies the new queue/player state synchronously inside the actor and requests
//! a reconcile. The reconciler compares the desired state (run, play intent, seek request, volume,
//! repeat ONE, normalization) with what the active backend has applied and issues one backend
//! command at a time. Backend commands are never cancelled; while one is in flight the actor keeps
//! applying commands and events, and the reconciler re-evaluates when it completes. Every load
//! carries a fresh `load_id`; events of other loads are dropped.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ap_model::planning::DEFAULT_MAX_RUN_ITEMS;
use ap_model::{
    BackendKind, PendingBreak, PlanItem, PlaybackStateType, Previous, QueueOps, QueueState,
    RepeatMode, Run, SourceType, Track, backend_for, break_run, plan_run, volume_normalization,
};
use rand::SeedableRng;
use rand::rngs::StdRng;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::{Instant, sleep_until};

use crate::backend::{Backend, BackendEvent, BackendRun, ItemKey, RunItem};
use crate::ports::{ConnectionGate, PlaybackPersistence, PlayerSettings};
use crate::state::{
    ActiveBreak, ChangeOrigin, EngineChange, EngineEvent, PlayerState, PositionAnchor,
};

/// Queue content is written this long after the last content change.
pub const CONTENT_WRITE_DEBOUNCE: Duration = Duration::from_millis(500);
/// Flags and position are checkpointed this often while play is requested.
pub const POSITION_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(15);
/// A Spotify device is pre-resolved this long before a LOCAL→SPOTIFY boundary.
pub const PREWARM_LEAD_MS: i64 = 20_000;
/// Capacity of `changes`; a slower subscriber lags (and observes `RecvError::Lagged`).
pub const CHANGES_CAPACITY: usize = 1_024;
/// Capacity of `events`.
pub const EVENTS_CAPACITY: usize = 256;

/// The gate text exposed in [`PlayerState::disabled_reason`].
#[must_use]
pub fn disabled_reason(name: &str) -> String {
    format!(
        "Playback disabled: {name} is not configured/authenticated for this app. Reconnect it in Settings. Next/Previous still works."
    )
}

/// What the engine is wired to.
pub struct EngineDeps {
    /// At most one backend per [`BackendKind`]; a missing kind surfaces as an error when needed.
    pub backends: Vec<Arc<dyn Backend>>,
    pub persistence: Arc<dyn PlaybackPersistence>,
    pub gate: Arc<dyn ConnectionGate>,
}

enum Command {
    SetQueue {
        tracks: Vec<Track>,
        start: isize,
        origin: ChangeOrigin,
        autoplay: bool,
    },
    PlayNext(Vec<Track>),
    JumpToUid(u64),
    Play(ChangeOrigin),
    Pause(ChangeOrigin),
    Toggle,
    Next,
    Previous,
    Seek(i64, ChangeOrigin),
    SetVolume(i32, ChangeOrigin),
    SetShuffle(bool, ChangeOrigin),
    SetRepeat(RepeatMode, ChangeOrigin),
    SetNormalization {
        enabled: bool,
        strict: bool,
    },
    SetPendingBreak(Option<PendingBreak>),
    Flush(oneshot::Sender<()>),
}

/// Handle to the engine actor. Cheap to clone; every method may be called from any thread and
/// returns immediately (commands are applied in order once the persisted state is restored).
#[derive(Clone)]
pub struct Engine {
    commands: mpsc::UnboundedSender<Command>,
    queue: watch::Receiver<QueueState>,
    player: watch::Receiver<PlayerState>,
    changes: broadcast::Sender<EngineChange>,
    events: broadcast::Sender<EngineEvent>,
    restored: watch::Receiver<bool>,
}

impl Engine {
    /// Spawns the engine actor and its persistence writer on the current tokio runtime and starts
    /// restoring the persisted queue (paused; no backend is loaded until the first Play).
    #[must_use]
    pub fn spawn(deps: EngineDeps) -> Self {
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (queue_tx, queue) = watch::channel(QueueState::default());
        let (player_tx, player) = watch::channel(PlayerState::default());
        let (changes, _) = broadcast::channel(CHANGES_CAPACITY);
        let (events, _) = broadcast::channel(EVENTS_CAPACITY);
        let (restored_tx, restored) = watch::channel(false);
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        for (index, backend) in deps.backends.iter().enumerate() {
            let mut rx = backend.subscribe();
            let tx = event_tx.clone();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            if tx.send((index, event)).is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(backend = index, skipped = n, "backend events lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }
        let (persist_tx, persist_rx) = mpsc::unbounded_channel();
        tokio::spawn(persistence_writer(
            deps.persistence.clone(),
            persist_rx,
            queue.clone(),
        ));
        let gate_rx = deps.gate.subscribe();
        let core = Core {
            kinds: deps.backends.iter().map(|b| b.kind()).collect(),
            backends: deps.backends,
            persistence: deps.persistence,
            gate: deps.gate,
            persist: persist_tx,
            queue_tx,
            player_tx,
            changes_tx: changes.clone(),
            events_tx: events.clone(),
            q: QueueState::default(),
            p: PlayerState::default(),
            pending_changes: Vec::new(),
            pending_events: Vec::new(),
            rng: StdRng::from_os_rng(),
            generation: 0,
            active: None,
            pending: None,
            break_target: None,
            break_item: None,
            seek_serial: 0,
            seek_position: 0,
            strict: false,
            last_cursor: None,
            uid_positions: HashMap::new(),
            uid_positions_version: None,
            periodic_at: None,
            prewarm: None,
            warmed_uid: None,
            play_origin: ChangeOrigin::User,
            reconcile_pending: false,
        };
        tokio::spawn(run(core, command_rx, event_rx, gate_rx, restored_tx));
        Self {
            commands,
            queue,
            player,
            changes,
            events,
            restored,
        }
    }

    #[must_use]
    pub fn queue(&self) -> watch::Receiver<QueueState> {
        self.queue.clone()
    }

    #[must_use]
    pub fn player(&self) -> watch::Receiver<PlayerState> {
        self.player.clone()
    }

    #[must_use]
    pub fn changes(&self) -> broadcast::Receiver<EngineChange> {
        self.changes.subscribe()
    }

    #[must_use]
    pub fn events(&self) -> broadcast::Receiver<EngineEvent> {
        self.events.subscribe()
    }

    /// Resolves once the persisted queue was restored (or there was nothing to restore).
    pub async fn await_restored(&self) {
        let mut restored = self.restored.clone();
        let _ = restored.wait_for(|done| *done).await;
    }

    /// The current position extrapolated from the published anchor.
    #[must_use]
    pub fn position_ms(&self) -> i64 {
        self.player.borrow().position_at(Instant::now())
    }

    fn send(&self, command: Command) {
        // The actor only stops when every handle is gone.
        let _ = self.commands.send(command);
    }

    /// Replaces the queue; `start` is the play-order start (random with shuffle when `start == 0`,
    /// `autoplay` and `origin == User`).
    pub fn set_queue(
        &self,
        tracks: Vec<Track>,
        start: isize,
        origin: ChangeOrigin,
        autoplay: bool,
    ) {
        self.send(Command::SetQueue {
            tracks,
            start,
            origin,
            autoplay,
        });
    }

    /// Inserts `tracks` after the current entry (stacking in call order).
    pub fn play_next(&self, tracks: Vec<Track>) {
        self.send(Command::PlayNext(tracks));
    }

    /// Makes the entry `uid` current and plays it (a pending DJ break before it is dropped).
    pub fn jump_to_uid(&self, uid: u64) {
        self.send(Command::JumpToUid(uid));
    }

    pub fn play(&self, origin: ChangeOrigin) {
        self.send(Command::Play(origin));
    }

    pub fn pause(&self, origin: ChangeOrigin) {
        self.send(Command::Pause(origin));
    }

    pub fn toggle(&self) {
        self.send(Command::Toggle);
    }

    pub fn next(&self) {
        self.send(Command::Next);
    }

    pub fn previous(&self) {
        self.send(Command::Previous);
    }

    pub fn seek(&self, position_ms: i64, origin: ChangeOrigin) {
        self.send(Command::Seek(position_ms, origin));
    }

    pub fn set_volume(&self, percent: i32, origin: ChangeOrigin) {
        self.send(Command::SetVolume(percent, origin));
    }

    pub fn set_shuffle(&self, on: bool, origin: ChangeOrigin) {
        self.send(Command::SetShuffle(on, origin));
    }

    pub fn set_repeat(&self, mode: RepeatMode, origin: ChangeOrigin) {
        self.send(Command::SetRepeat(mode, origin));
    }

    pub fn set_normalization(&self, enabled: bool, strict: bool) {
        self.send(Command::SetNormalization { enabled, strict });
    }

    /// DJ host: the break plays right before `before_uid` when the cursor advances into it.
    pub fn set_pending_break(&self, pending: Option<PendingBreak>) {
        self.send(Command::SetPendingBreak(pending));
    }

    /// Writes flags, position and any debounced queue content now (call on quit).
    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        self.send(Command::Flush(tx));
        let _ = rx.await;
    }
}

type Op = Pin<Box<dyn Future<Output = ()> + Send>>;

/// What the active backend has applied.
struct Applied {
    id: u64,
    backend: usize,
    run: Run,
    key: ItemKey,
    playing: bool,
    seek: u64,
    volume: i32,
    repeat: bool,
    normalization: bool,
    /// The run finished or failed terminally; the next Play reloads from 0.
    ended: bool,
    /// Stopped for a backend switch; the replacement load follows.
    stopped: bool,
}

enum PersistMsg {
    Content,
    Cursor { uid: Option<u64>, version: u64 },
    Settings(PlayerSettings),
    Flush(oneshot::Sender<()>),
}

struct Core {
    backends: Vec<Arc<dyn Backend>>,
    kinds: Vec<BackendKind>,
    persistence: Arc<dyn PlaybackPersistence>,
    gate: Arc<dyn ConnectionGate>,
    persist: mpsc::UnboundedSender<PersistMsg>,
    queue_tx: watch::Sender<QueueState>,
    player_tx: watch::Sender<PlayerState>,
    changes_tx: broadcast::Sender<EngineChange>,
    events_tx: broadcast::Sender<EngineEvent>,
    q: QueueState,
    p: PlayerState,
    pending_changes: Vec<EngineChange>,
    pending_events: Vec<EngineEvent>,
    rng: StdRng,
    generation: u64,
    active: Option<Applied>,
    pending: Option<PendingBreak>,
    break_target: Option<u64>,
    break_item: Option<PendingBreak>,
    seek_serial: u64,
    seek_position: i64,
    strict: bool,
    last_cursor: Option<(u64, Option<u64>)>,
    uid_positions: HashMap<u64, usize>,
    uid_positions_version: Option<u64>,
    periodic_at: Option<Instant>,
    prewarm: Option<(Instant, u64)>,
    warmed_uid: Option<u64>,
    /// Origin of the latest play intent; only `User` may launch a local Spotify client.
    play_origin: ChangeOrigin,
    reconcile_pending: bool,
}

async fn run(
    mut core: Core,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut events: mpsc::UnboundedReceiver<(usize, BackendEvent)>,
    mut gate: watch::Receiver<()>,
    restored: watch::Sender<bool>,
) {
    core.restore().await;
    core.publish();
    let _ = restored.send(true);
    let mut in_flight: Option<Op> = None;
    let mut gate_open = true;
    loop {
        if in_flight.is_none() {
            in_flight = core.drive();
        }
        core.publish();
        let periodic = core.periodic_at;
        let prewarm = core.prewarm.map(|(at, _)| at);
        tokio::select! {
            biased;
            () = async { in_flight.as_mut().expect("guarded").await }, if in_flight.is_some() => {
                in_flight = None;
            }
            Some((index, event)) = events.recv() => core.on_backend_event(index, event),
            command = commands.recv() => {
                if let Some(command) = command {
                    core.handle(command);
                } else {
                    if let Some(operation) = in_flight.take() { operation.await; }
                    for backend in &core.backends { backend.stop().await; }
                    break;
                }
            },
            changed = gate.changed(), if gate_open => {
                if changed.is_ok() {
                    core.enforce_gate();
                    core.request_reconcile();
                } else {
                    gate_open = false;
                }
            }
            () = sleep_until(periodic.unwrap_or_else(Instant::now)), if periodic.is_some() => {
                core.save_flags();
                core.periodic_at = Some(Instant::now() + POSITION_CHECKPOINT_INTERVAL);
            }
            () = sleep_until(prewarm.unwrap_or_else(Instant::now)), if prewarm.is_some() => core.fire_prewarm(),
        }
    }
}

async fn persistence_writer(
    persistence: Arc<dyn PlaybackPersistence>,
    mut rx: mpsc::UnboundedReceiver<PersistMsg>,
    queue: watch::Receiver<QueueState>,
) {
    let mut content_at: Option<Instant> = None;
    let mut cursor: Option<(Option<u64>, u64)> = None;
    let mut settings: Option<PlayerSettings> = None;
    let mut flushes: Vec<oneshot::Sender<()>> = Vec::new();
    loop {
        if !flushes.is_empty() && content_at.take().is_some() {
            write_content(&*persistence, &queue, &mut cursor).await;
        }
        if let Some(s) = settings.take()
            && let Err(e) = persistence.save_settings(&s).await
        {
            tracing::warn!("saving player settings failed: {e}");
        }
        if let Some((uid, version)) = cursor.take()
            && let Err(e) = persistence.save_cursor(uid, version).await
        {
            tracing::warn!("saving queue cursor failed: {e}");
        }
        for done in flushes.drain(..) {
            let _ = done.send(());
        }
        let msg = tokio::select! {
            msg = rx.recv() => msg,
            () = sleep_until(content_at.unwrap_or_else(Instant::now)), if content_at.is_some() => {
                content_at = None;
                write_content(&*persistence, &queue, &mut cursor).await;
                continue;
            }
        };
        let Some(first) = msg else { break };
        let mut next = Some(first);
        // Conflate everything already queued.
        while let Some(msg) = next.take().or_else(|| rx.try_recv().ok()) {
            match msg {
                PersistMsg::Content => content_at = Some(Instant::now() + CONTENT_WRITE_DEBOUNCE),
                PersistMsg::Cursor { uid, version } => cursor = Some((uid, version)),
                PersistMsg::Settings(s) => settings = Some(s),
                PersistMsg::Flush(done) => flushes.push(done),
            }
        }
    }
    if content_at.is_some() {
        write_content(&*persistence, &queue, &mut cursor).await;
    }
}

async fn write_content(
    persistence: &dyn PlaybackPersistence,
    queue: &watch::Receiver<QueueState>,
    cursor: &mut Option<(Option<u64>, u64)>,
) {
    let snapshot = queue.borrow().clone();
    if let Err(e) = persistence.save_queue_content(&snapshot).await {
        tracing::warn!("saving queue content failed: {e}");
    }
    *cursor = Some((snapshot.current().map(|e| e.uid), snapshot.content_version));
}

fn first_song_uid(run: &Run) -> Option<u64> {
    run.items.iter().find_map(|item| match item {
        PlanItem::Song { uid, .. } => Some(*uid),
        PlanItem::Break { .. } => None,
    })
}

fn plan_from(q: &QueueState, from: usize, pending: Option<&PendingBreak>) -> Option<Run> {
    plan_run(q, from, pending, DEFAULT_MAX_RUN_ITEMS)
}

/// The run after advancing to `next`, wrapping for repeat ALL.
fn plan_after_advance(next: &QueueState) -> Option<Run> {
    next.cursor_pos()
        .and_then(|c| plan_from(next, c, None))
        .or_else(|| {
            (next.repeat == RepeatMode::All)
                .then(|| plan_from(next, 0, None))
                .flatten()
        })
}

impl Core {
    // ------------------------------------------------------------------ plumbing

    fn publish(&mut self) {
        let q = &self.q;
        self.queue_tx.send_if_modified(|v| {
            if v == q {
                false
            } else {
                v.clone_from(q);
                true
            }
        });
        let p = &self.p;
        self.player_tx.send_if_modified(|v| {
            if v == p {
                false
            } else {
                v.clone_from(p);
                true
            }
        });
        for change in self.pending_changes.drain(..) {
            let _ = self.changes_tx.send(change);
        }
        for event in self.pending_events.drain(..) {
            let _ = self.events_tx.send(event);
        }
    }

    fn emit(&mut self, event: EngineEvent) {
        self.pending_events.push(event);
    }

    fn request_reconcile(&mut self) {
        self.reconcile_pending = true;
    }

    fn invalidate_load(&mut self) {
        self.generation += 1;
        self.seek_serial += 1;
        self.request_reconcile();
    }

    fn backend_index(&self, kind: BackendKind) -> Option<usize> {
        self.kinds.iter().position(|k| *k == kind)
    }

    fn order_position(&mut self, uid: u64) -> Option<usize> {
        if self.uid_positions_version != Some(self.q.content_version) {
            self.uid_positions = self
                .q
                .order
                .iter()
                .enumerate()
                .map(|(pos, &index)| (self.q.entries[index].uid, pos))
                .collect();
            self.uid_positions_version = Some(self.q.content_version);
        }
        self.uid_positions.get(&uid).copied()
    }

    fn position_ms(&self) -> i64 {
        self.p.position_at(Instant::now())
    }

    fn anchor(&mut self, position: i64, duration: i64) {
        self.p.anchor = PositionAnchor {
            position_ms: position.max(0),
            at: Instant::now(),
        };
        self.p.duration_ms = duration.max(0);
    }

    fn anchor_now(&mut self) {
        let position = self.position_ms();
        self.anchor(position, self.p.duration_ms);
    }

    fn changed(&mut self, origin: ChangeOrigin, content: bool) {
        self.pending_changes.push(EngineChange {
            origin,
            content_changed: content,
        });
        if origin != ChangeOrigin::Restore {
            self.persist_state(content);
        }
        self.periodic_at = self
            .p
            .play_requested
            .then(|| Instant::now() + POSITION_CHECKPOINT_INTERVAL);
        self.schedule_prewarm();
    }

    fn settings_snapshot(&self) -> PlayerSettings {
        PlayerSettings {
            volume: self.p.volume,
            shuffle: self.q.shuffle,
            repeat: self.q.repeat,
            position_ms: self.position_ms(),
            normalization_enabled: self.p.normalization,
            normalization_strict: self.strict,
        }
    }

    fn save_flags(&mut self) {
        let _ = self
            .persist
            .send(PersistMsg::Settings(self.settings_snapshot()));
    }

    fn persist_state(&mut self, content: bool) {
        self.save_flags();
        if content {
            let _ = self.persist.send(PersistMsg::Content);
        } else {
            let uid = self.q.current().map(|e| e.uid);
            let key = (self.q.content_version, uid);
            if self.last_cursor != Some(key) {
                self.last_cursor = Some(key);
                let _ = self.persist.send(PersistMsg::Cursor {
                    uid,
                    version: self.q.content_version,
                });
            }
        }
    }

    fn schedule_prewarm(&mut self) {
        self.prewarm = None;
        if !self.p.play_requested {
            return;
        }
        let Some(active) = &self.active else { return };
        if self.kinds[active.backend] != BackendKind::Local
            || self.backend_index(BackendKind::Spotify).is_none()
        {
            return;
        }
        let Some(cursor) = self.q.cursor_pos() else {
            return;
        };
        let Some(run) = plan_from(&self.q, cursor, self.pending.as_ref()) else {
            return;
        };
        let songs: Vec<(u64, usize)> = run
            .items
            .iter()
            .filter_map(|item| match item {
                PlanItem::Song { uid, order_pos } => Some((*uid, *order_pos)),
                PlanItem::Break { .. } => None,
            })
            .collect();
        let Some(&(_, last_pos)) = songs.last() else {
            return;
        };
        let Some(next) = plan_from(&self.q, last_pos + 1, None) else {
            return;
        };
        if next.kind != BackendKind::Spotify {
            return;
        }
        let Some(PlanItem::Song { uid: target, .. }) = next.items.first() else {
            return;
        };
        if Some(*target) == self.warmed_uid {
            return;
        }
        let rest: i64 = songs
            .iter()
            .skip(1)
            .map(|&(_, pos)| self.q.entry_at(pos).track.duration_ms.unwrap_or(0))
            .sum();
        let until_boundary = (self.p.duration_ms - self.position_ms()).max(0) + rest;
        let wait = u64::try_from((until_boundary - PREWARM_LEAD_MS).max(0)).unwrap_or(0);
        self.prewarm = Some((Instant::now() + Duration::from_millis(wait), *target));
    }

    fn fire_prewarm(&mut self) {
        let Some((_, uid)) = self.prewarm.take() else {
            return;
        };
        self.warmed_uid = Some(uid);
        if let Some(index) = self.backend_index(BackendKind::Spotify) {
            let backend = self.backends[index].clone();
            let origin = self.play_origin;
            tokio::spawn(async move {
                backend.prepare(origin).await;
            });
        }
    }

    // ------------------------------------------------------------------ restore

    async fn restore(&mut self) {
        let loaded = async {
            let saved = self.persistence.load_queue().await?;
            let flags = self.persistence.load_settings().await?;
            Ok::<_, crate::ports::BoxError>((saved, flags))
        }
        .await;
        match loaded {
            Ok((saved, flags)) => {
                self.q = QueueState {
                    shuffle: flags.shuffle,
                    repeat: flags.repeat,
                    ..saved
                };
                self.uid_positions_version = None;
                let current = self.q.current();
                self.p = PlayerState {
                    status: if current.is_none() {
                        PlaybackStateType::Idle
                    } else {
                        PlaybackStateType::Paused
                    },
                    current_uid: current.map(|e| e.uid),
                    duration_ms: current.and_then(|e| e.track.duration_ms).unwrap_or(0),
                    anchor: PositionAnchor {
                        position_ms: flags.position_ms.max(0),
                        at: Instant::now(),
                    },
                    volume: flags.volume.clamp(0, 100),
                    normalization: flags.normalization_enabled,
                    ..PlayerState::default()
                };
                self.strict = flags.normalization_strict;
                self.seek_position = flags.position_ms.max(0);
            }
            Err(e) => {
                self.p.error = Some(format!("Playback restore failed: {e}"));
            }
        }
        self.changed(ChangeOrigin::Restore, false);
        self.enforce_gate();
        self.request_reconcile();
    }

    // ------------------------------------------------------------------ commands

    fn handle(&mut self, command: Command) {
        match command {
            Command::SetQueue {
                tracks,
                start,
                origin,
                autoplay,
            } => {
                self.drop_break();
                self.q = QueueOps::replace(
                    &self.q,
                    tracks,
                    start,
                    &mut self.rng,
                    autoplay && origin == ChangeOrigin::User,
                );
                self.emit(EngineEvent::QueueReplaced);
                self.select_current(0);
                self.invalidate_load();
                self.transport(autoplay, origin);
                self.changed(origin, true);
            }
            Command::PlayNext(tracks) => {
                if tracks.is_empty() {
                    return;
                }
                for track in tracks {
                    self.q = QueueOps::play_next(&self.q, track);
                }
                if self.p.current_uid.is_none() {
                    self.select_current(0);
                }
                self.validate_break();
                self.request_reconcile();
                self.changed(ChangeOrigin::User, true);
            }
            Command::JumpToUid(uid) => {
                if let Some(pos) = self.order_position(uid) {
                    self.finish_break();
                    self.q =
                        QueueOps::move_cursor(&self.q, isize::try_from(pos).unwrap_or(isize::MAX));
                    self.validate_break();
                    self.select_current(0);
                    self.invalidate_load();
                    self.transport(true, ChangeOrigin::User);
                }
            }
            Command::Play(origin) => self.transport(true, origin),
            Command::Pause(origin) => self.transport(false, origin),
            Command::Toggle => self.transport(!self.p.play_requested, ChangeOrigin::User),
            Command::Next => self.advance(true),
            Command::Previous => self.previous(),
            Command::Seek(ms, origin) => self.seek(ms, origin),
            Command::SetVolume(percent, origin) => {
                self.p.volume = percent.clamp(0, 100);
                self.request_reconcile();
                self.changed(origin, false);
            }
            Command::SetShuffle(on, origin) => {
                let next = QueueOps::set_shuffle(&self.q, on, &mut self.rng);
                if next != self.q {
                    let content_changed = next.content_version != self.q.content_version;
                    self.q = next;
                    self.validate_break();
                    self.request_reconcile();
                    self.changed(origin, content_changed);
                }
            }
            Command::SetRepeat(mode, origin) => {
                self.q = QueueOps::set_repeat(&self.q, mode);
                self.request_reconcile();
                self.changed(origin, false);
            }
            Command::SetNormalization { enabled, strict } => {
                self.strict = strict;
                self.p.normalization = enabled;
                self.request_reconcile();
                self.changed(ChangeOrigin::User, false);
            }
            Command::SetPendingBreak(pending) => {
                if let Some(old) = &self.pending
                    && pending.as_ref().is_none_or(|p| p.break_id != old.break_id)
                {
                    let break_id = old.break_id.clone();
                    self.emit(EngineEvent::BreakDropped { break_id });
                }
                self.pending = pending;
                self.validate_break();
                self.request_reconcile();
                self.changed(ChangeOrigin::User, false);
            }
            Command::Flush(done) => {
                self.save_flags();
                let _ = self.persist.send(PersistMsg::Flush(done));
            }
        }
    }

    fn previous(&mut self) {
        if self.break_item.is_some() {
            self.drop_break();
            self.select_current(0);
            self.invalidate_load();
            self.transport(true, ChangeOrigin::User);
            return;
        }
        match QueueOps::previous(&self.q, self.position_ms()) {
            Previous::SeekToStart => self.seek(0, ChangeOrigin::User),
            Previous::MoveTo(pos) => {
                self.q = QueueOps::move_cursor(&self.q, isize::try_from(pos).unwrap_or(isize::MAX));
                self.validate_break();
                self.select_current(0);
                self.invalidate_load();
                self.transport(true, ChangeOrigin::User);
            }
        }
    }

    fn seek(&mut self, ms: i64, origin: ChangeOrigin) {
        let mut position = ms.max(0);
        if self.p.duration_ms > 0 {
            position = position.min(self.p.duration_ms);
        }
        self.seek_position = position;
        self.anchor(position, self.p.duration_ms);
        self.seek_serial += 1;
        self.request_reconcile();
        self.changed(origin, false);
    }

    fn transport(&mut self, play: bool, origin: ChangeOrigin) {
        let allowed = self.enforce_gate();
        let start = play && allowed;
        tracing::debug!(play, allowed, ?origin, current = ?self.p.current_uid, "transport");
        if start && self.active.as_ref().is_some_and(|a| a.ended) {
            self.seek_position = 0;
            self.anchor(0, self.p.duration_ms);
            self.invalidate_load();
        }
        self.anchor_now();
        if self.p.status == PlaybackStateType::Error {
            self.p.error = None;
        }
        self.p.play_requested = start;
        self.p.status = if start {
            PlaybackStateType::Buffering
        } else if self.q.current().is_none() {
            PlaybackStateType::Idle
        } else {
            PlaybackStateType::Paused
        };
        if start {
            self.play_origin = origin;
        }
        self.request_reconcile();
        self.changed(origin, false);
    }

    fn advance(&mut self, manual: bool) {
        let intent = manual || self.p.play_requested;
        let target = if self.break_item.is_some() {
            self.break_target
                .map(|uid| QueueOps::jump_to_uid(&self.q, uid))
        } else {
            QueueOps::advance(&self.q, true).and_then(|next| {
                let uid = plan_after_advance(&next)
                    .as_ref()
                    .and_then(first_song_uid)?;
                Some(QueueOps::jump_to_uid(&next, uid))
            })
        };
        if self.break_item.is_some() {
            self.finish_break();
        } else if let Some(t) = &target
            && let Some(pending) = &self.pending
            && t.current().map(|e| e.uid) == Some(pending.before_uid)
            && t.cursor > self.q.cursor
        {
            let item = self.pending.take().expect("checked");
            self.break_target = Some(item.before_uid);
            self.break_item = Some(item);
            self.anchor(0, 0);
            self.seek_position = 0;
            self.invalidate_load();
            self.transport(intent, ChangeOrigin::User);
            return;
        }
        let Some(target) = target else {
            if manual {
                return;
            }
            let position = match &self.active {
                Some(a) => self.backends[a.backend].position_ms(),
                None => self.position_ms(),
            };
            self.anchor(position, self.p.duration_ms);
            self.p.play_requested = false;
            self.p.status = PlaybackStateType::Paused;
            if let Some(a) = &mut self.active {
                a.ended = true;
            }
            self.request_reconcile();
            self.changed(ChangeOrigin::User, false);
            return;
        };
        self.q = target;
        self.validate_break();
        self.select_current(0);
        self.invalidate_load();
        self.transport(intent, ChangeOrigin::User);
    }

    /// Moves the cursor to the first playable entry of the run at the cursor and resets the
    /// position; reports a skipped or missing playable entry.
    fn select_current(&mut self, position: i64) {
        let before = self.q.current().map(|e| (e.uid, e.track.title.clone()));
        let first = self
            .q
            .cursor_pos()
            .and_then(|c| plan_from(&self.q, c, None))
            .as_ref()
            .and_then(first_song_uid);
        if let Some(uid) = first
            && Some(uid) != before.as_ref().map(|b| b.0)
        {
            self.q = QueueOps::jump_to_uid(&self.q, uid);
        }
        let current = self.q.current();
        let current_uid = current.map(|e| e.uid);
        let duration = current.and_then(|e| e.track.duration_ms).unwrap_or(0);
        self.seek_position = position;
        self.p.current_uid = current_uid;
        self.p.active_break = None;
        self.anchor(position, duration);
        self.p.error = match (&before, first) {
            (Some(_), None) => Some("Nothing playable in the queue".to_owned()),
            (Some((uid, title)), Some(first)) if *uid != first => {
                Some(format!("Can't play \"{title}\""))
            }
            _ => None,
        };
    }

    // ------------------------------------------------------------------ gate & breaks

    fn gate_reason(&self) -> Option<String> {
        let source = self.q.current()?.track.source;
        self.gate
            .disabled_name(source)
            .map(|name| disabled_reason(&name))
    }

    /// Applies the playback gate. Clearing intent makes repeated auth ticks idempotent;
    /// a subsequent player-originated resume is a new intent and must be blocked too.
    fn enforce_gate(&mut self) -> bool {
        let reason = self.gate_reason();
        let pause = reason.is_some() && self.p.play_requested;
        if pause {
            self.anchor_now();
        }
        let allowed = reason.is_none();
        if self.p.disabled_reason != reason || pause {
            self.p.disabled_reason = reason;
            if pause {
                self.p.play_requested = false;
                self.p.status = PlaybackStateType::Paused;
            }
            self.changed(ChangeOrigin::User, false);
        }
        allowed
    }

    fn validate_break(&mut self) {
        let Some(before_uid) = self.pending.as_ref().map(|b| b.before_uid) else {
            return;
        };
        let valid = self.order_position(before_uid).is_some_and(|pos| {
            isize::try_from(pos).is_ok_and(|p| p > self.q.cursor)
                && backend_for(&self.q.entry_at(pos).track) != BackendKind::Unplayable
        });
        if !valid {
            let dropped = self.pending.take().expect("checked");
            self.emit(EngineEvent::BreakDropped {
                break_id: dropped.break_id,
            });
        }
    }

    fn finish_break(&mut self) {
        if let Some(item) = self.break_item.take() {
            self.emit(
                if self
                    .p
                    .active_break
                    .as_ref()
                    .is_some_and(|b| b.break_id == item.break_id)
                {
                    EngineEvent::BreakEnded {
                        break_id: item.break_id,
                    }
                } else {
                    EngineEvent::BreakDropped {
                        break_id: item.break_id,
                    }
                },
            );
        }
        self.break_target = None;
        self.p.active_break = None;
    }

    fn drop_break(&mut self) {
        if let Some(pending) = self.pending.take() {
            self.emit(EngineEvent::BreakDropped {
                break_id: pending.break_id,
            });
        }
        self.finish_break();
    }

    // ------------------------------------------------------------------ reconcile

    fn desired_run(&self) -> Option<Run> {
        if let Some(item) = &self.break_item {
            // A break already embedded in a LOCAL timeline remains gapless into its target.
            if let Some(active) = &self.active
                && active.id == self.generation
                && active.run.items.iter().any(|planned| matches!(planned, PlanItem::Break { break_id, .. } if break_id == &item.break_id))
            {
                return Some(active.run.clone());
            }
            return Some(break_run(item));
        }
        let mut run = self
            .q
            .cursor_pos()
            .and_then(|c| plan_from(&self.q, c, self.pending.as_ref()))?;
        if run.kind == BackendKind::Local
            && let Some(active) = &self.active
            && active.id == self.generation
            && let Some(uid) = self.q.current().map(|entry| entry.uid)
            && let Some(current) = active.run.items.iter().position(
                |item| matches!(item, PlanItem::Song { uid: item_uid, .. } if *item_uid == uid),
            )
        {
            let mut history: Vec<_> = active.run.items[..current]
                .iter()
                .rev()
                .filter_map(|item| {
                    if let PlanItem::Song { uid, .. } = item {
                        self.q
                            .order_pos_of_uid(*uid)
                            .map(|order_pos| PlanItem::Song {
                                uid: *uid,
                                order_pos,
                            })
                    } else {
                        None
                    }
                })
                .take(3)
                .collect();
            history.reverse();
            history.extend(run.items);
            history.truncate(DEFAULT_MAX_RUN_ITEMS);
            run.items = history;
        }
        Some(run)
    }

    fn desired_key(&self) -> Option<ItemKey> {
        match &self.break_item {
            Some(item) => Some(ItemKey::Break(item.break_id.clone())),
            None => self.q.current().map(|e| ItemKey::Song(e.uid)),
        }
    }

    fn effective_volume(&self) -> i32 {
        let source = if self.break_item.is_some() {
            SourceType::All
        } else {
            self.q.current().map_or(SourceType::All, |e| e.track.source)
        };
        volume_normalization::effective(self.p.volume, source, self.p.normalization)
    }

    fn desired_repeat(&self) -> bool {
        self.q.repeat == RepeatMode::One && self.break_item.is_none()
    }

    fn backend_run(&self, run: &Run) -> BackendRun {
        BackendRun {
            kind: run.kind,
            items: run
                .items
                .iter()
                .map(|item| match item {
                    PlanItem::Song { uid, order_pos } => RunItem::Song {
                        uid: *uid,
                        track: self.q.entry_at(*order_pos).track.clone(),
                    },
                    PlanItem::Break { break_id, path } => RunItem::Break {
                        break_id: break_id.clone(),
                        path: path.clone(),
                    },
                })
                .collect(),
        }
    }

    /// Runs reconcile steps until one issues a backend command (returned) or nothing differs.
    fn drive(&mut self) -> Option<Op> {
        while self.reconcile_pending {
            if let Some(op) = self.step() {
                return Some(op);
            }
            self.reconcile_pending = false;
        }
        None
    }

    /// One level-triggered comparison of desired vs applied state; returns the next command.
    fn step(&mut self) -> Option<Op> {
        let run = self.desired_run();
        let key = self.desired_key();
        let backend = run.as_ref().and_then(|r| self.backend_index(r.kind));
        let (Some(run), Some(key), Some(index)) = (run.clone(), key, backend) else {
            if let Some(old) = self.active.take()
                && !old.stopped
            {
                let b = self.backends[old.backend].clone();
                return Some(Box::pin(async move { b.stop().await }));
            }
            let has_current = self.q.current().is_some();
            self.p.play_requested = false;
            self.p.status = if has_current {
                PlaybackStateType::Error
            } else {
                PlaybackStateType::Idle
            };
            self.p.error = if !has_current {
                None
            } else if run.is_some_and(|r| r.kind == BackendKind::Spotify) {
                Some("Spotify playback is not available in this build".to_owned())
            } else {
                Some("Nothing playable in the queue".to_owned())
            };
            self.changed(ChangeOrigin::User, false);
            return None;
        };
        self.enforce_gate();
        let requested = self.p.play_requested;
        let seek = self.seek_serial;
        let volume = self.effective_volume();
        let repeat = self.desired_repeat();
        let normalization = self.p.normalization;
        let needs_load = self
            .active
            .as_ref()
            .is_none_or(|a| a.backend != index || a.id != self.generation || a.key != key);
        if needs_load {
            if let Some(a) = &mut self.active
                && a.backend != index
                && !a.stopped
            {
                a.stopped = true;
                let b = self.backends[a.backend].clone();
                return Some(Box::pin(async move { b.stop().await }));
            }
            // Restore is local-only: nothing loads (not even a paused Spotify run) until Play.
            if !requested && self.active.is_none() {
                return None;
            }
            self.generation += 1;
            let id = self.generation;
            let backend_run = self.backend_run(&run);
            tracing::info!(load_id = id, backend = ?self.kinds[index], items = run.items.len(), playing = requested, "loading run");
            self.active = Some(Applied {
                id,
                backend: index,
                run,
                key,
                playing: requested,
                seek,
                volume,
                repeat,
                normalization,
                ended: false,
                stopped: false,
            });
            let b = self.backends[index].clone();
            let position = self.seek_position;
            let origin = self.play_origin;
            return Some(Box::pin(async move {
                b.set_volume(volume).await;
                b.set_repeat_one(repeat).await;
                b.set_normalization(normalization).await;
                b.load(id, backend_run, 0, position, requested, origin)
                    .await;
            }));
        }
        let b = self.backends[index].clone();
        let backend_run = {
            let a = self.active.as_ref().expect("loaded");
            (a.run != run).then(|| self.backend_run(&run))
        };
        let a = self.active.as_mut().expect("loaded");
        if let Some(backend_run) = backend_run {
            a.run = run;
            let id = a.id;
            return Some(Box::pin(async move {
                b.update_upcoming(id, backend_run, key).await;
            }));
        }
        if a.seek != seek {
            a.seek = seek;
            let position = self.seek_position;
            return Some(Box::pin(async move { b.seek_to(position).await }));
        }
        if a.playing != requested {
            a.playing = requested;
            let origin = self.play_origin;
            return Some(if requested {
                Box::pin(async move { b.play(origin).await })
            } else {
                Box::pin(async move { b.pause().await })
            });
        }
        if a.volume != volume {
            a.volume = volume;
            return Some(Box::pin(async move { b.set_volume(volume).await }));
        }
        if a.repeat != repeat {
            a.repeat = repeat;
            return Some(Box::pin(async move { b.set_repeat_one(repeat).await }));
        }
        if a.normalization != normalization {
            a.normalization = normalization;
            return Some(Box::pin(
                async move { b.set_normalization(normalization).await },
            ));
        }
        None
    }

    // ------------------------------------------------------------------ backend events

    fn on_backend_event(&mut self, index: usize, event: BackendEvent) {
        tracing::debug!(backend = ?self.kinds[index], ?event, "backend event");
        let Some(applied) = &self.active else { return };
        let load_id = event.load_id();
        if load_id != self.generation || load_id != applied.id || index != applied.backend {
            tracing::debug!(
                load_id,
                current = self.generation,
                "dropped stale backend event"
            );
            return;
        }
        let backend = self.backends[index].clone();
        match event {
            BackendEvent::ItemStarted {
                key: ItemKey::Song(uid),
                ..
            } => {
                let Some(pos) = self.order_position(uid) else {
                    return;
                };
                let same_item = self.p.current_uid == Some(uid);
                let skipped = usize::try_from(self.q.cursor + 1)
                    .ok()
                    .and_then(|from| {
                        (from..pos).find(|&i| {
                            backend_for(&self.q.entry_at(i).track) == BackendKind::Unplayable
                        })
                    })
                    .map(|i| format!("Can't play \"{}\"", self.q.entry_at(i).track.title));
                if self.break_item.is_some() {
                    self.finish_break();
                }
                if self.q.cursor_pos() != Some(pos) {
                    self.q =
                        QueueOps::move_cursor(&self.q, isize::try_from(pos).unwrap_or(isize::MAX));
                }
                if let Some(a) = &mut self.active {
                    a.key = ItemKey::Song(uid);
                }
                let playing = self.p.status == PlaybackStateType::Playing;
                self.p.current_uid = Some(uid);
                self.p.active_break = None;
                self.p.error = if playing {
                    None
                } else if same_item {
                    skipped.or_else(|| self.p.error.take())
                } else {
                    skipped
                };
                let duration = match backend.duration_ms() {
                    d if d > 0 => d,
                    _ => self
                        .q
                        .current()
                        .and_then(|e| e.track.duration_ms)
                        .unwrap_or(0),
                };
                self.anchor(backend.position_ms(), duration);
                self.validate_break();
                self.emit(EngineEvent::ItemStarted {
                    uid,
                    order_pos: pos,
                });
                self.request_reconcile();
            }
            BackendEvent::ItemStarted {
                key: ItemKey::Break(break_id),
                ..
            } => {
                if self.break_item.is_none()
                    && self
                        .pending
                        .as_ref()
                        .is_some_and(|p| p.break_id == break_id)
                {
                    let item = self.pending.take().expect("checked");
                    self.break_target = Some(item.before_uid);
                    self.break_item = Some(item);
                }
                if self
                    .break_item
                    .as_ref()
                    .is_none_or(|b| b.break_id != break_id)
                {
                    return;
                }
                let first = self.p.active_break.is_none();
                if let Some(a) = &mut self.active {
                    a.key = ItemKey::Break(break_id.clone());
                }
                self.p.active_break = Some(ActiveBreak {
                    break_id: break_id.clone(),
                    duration_ms: backend.duration_ms(),
                });
                self.anchor(backend.position_ms(), backend.duration_ms());
                if first {
                    self.emit(EngineEvent::BreakStarted { break_id });
                }
                self.request_reconcile();
            }
            BackendEvent::PlayingChanged {
                is_playing,
                buffering,
                by_player,
                ..
            } => {
                self.anchor(backend.position_ms(), backend.duration_ms());
                let intent = if by_player {
                    is_playing || buffering
                } else {
                    self.p.play_requested
                };
                self.p.status =
                    if self.p.status == PlaybackStateType::Error && !is_playing && !buffering {
                        PlaybackStateType::Error
                    } else if is_playing {
                        PlaybackStateType::Playing
                    } else if buffering || intent {
                        PlaybackStateType::Buffering
                    } else {
                        PlaybackStateType::Paused
                    };
                self.p.play_requested = intent;
                if is_playing {
                    self.p.error = None;
                }
                if let Some(a) = &mut self.active {
                    a.playing = intent;
                }
                self.enforce_gate();
                self.request_reconcile();
            }
            BackendEvent::PositionDiscontinuity {
                position_ms,
                duration_ms,
                ..
            } => {
                self.anchor(position_ms, duration_ms);
            }
            BackendEvent::RunEnded { .. } => {
                if !applied.ended {
                    self.advance(false);
                }
                return;
            }
            BackendEvent::Error { key, message, .. } => {
                tracing::warn!(backend = ?self.kinds[index], ?key, %message, "playback backend error");
                let uid = match key {
                    Some(ItemKey::Song(uid)) => Some(uid),
                    _ => None,
                };
                if self.kinds[index] == BackendKind::Local
                    && let Some(uid) = uid
                    && let Some(position) = self.order_position(uid)
                {
                    // Decoder look-ahead can fail before the target's ItemStarted marker.
                    // Skip that failed occurrence, not the preceding audible song.
                    self.q = QueueOps::move_cursor(
                        &self.q,
                        isize::try_from(position).unwrap_or(isize::MAX),
                    );
                    self.p.current_uid = Some(uid);
                }
                let has_next = self.break_target.is_some()
                    || QueueOps::advance(&self.q, true)
                        .is_some_and(|next| plan_after_advance(&next).is_some());
                self.emit(EngineEvent::Error {
                    uid,
                    message: message.clone(),
                });
                if self.kinds[index] == BackendKind::Local && has_next && self.p.play_requested {
                    self.advance(false);
                } else {
                    if let Some(a) = &mut self.active {
                        a.ended = true;
                    }
                    self.p.status = PlaybackStateType::Error;
                    self.p.play_requested = false;
                    self.p.error = Some(message);
                    self.request_reconcile();
                }
            }
        }
        self.changed(ChangeOrigin::User, false);
    }
}
