//! Remote-only Android DJ controller/coordinator (design §14).
//!
//! One actor serializes schedule transitions and file ownership. Queue target changes never
//! cancel server inference: completed audio can be reused when the target flips back. Disable,
//! account changes, queue replacement and shutdown cancel work. The engine owns started files
//! until its BreakEnded event, including across disable/replace.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ap_model::break_schedule::{self, BreakSchedule, ScheduleEvent};
use ap_model::{PendingBreak, QueueState, Track};
use ap_playback::{Engine, EngineEvent};
use rand::{SeedableRng, rngs::StdRng};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::audio_files::{AudioKey, DjAudioFiles, OwnedAudio, sweep};
use crate::{
    DjAccount, DjDiagnostics, PassageClient, PassageStateStore, Passages, RemoteBreakGenerator,
    StoreFuture, VoiceCatalog, VoiceCatalogClient,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DjSettings {
    pub enabled: bool,
    pub show_entries_in_queue: bool,
    pub selected_voice_id: Option<String>,
    pub voice_level: f32,
}
impl Default for DjSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            show_entries_in_queue: false,
            selected_voice_id: None,
            voice_level: 1.0,
        }
    }
}
/// Non-secret settings persistence; passage state uses the separate PassageStateStore port.
pub trait DjSettingsStore: Send + Sync {
    fn load(&self) -> StoreFuture<'_, DjSettings>;
    fn save(&self, settings: DjSettings) -> StoreFuture<'_, ()>;
}
#[derive(Default)]
pub struct MemoryDjSettingsStore(Mutex<DjSettings>);
impl MemoryDjSettingsStore {
    pub fn snapshot(&self) -> DjSettings {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}
impl DjSettingsStore for MemoryDjSettingsStore {
    fn load(&self) -> StoreFuture<'_, DjSettings> {
        Box::pin(async { Ok(self.snapshot()) })
    }
    fn save(&self, settings: DjSettings) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            *self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = settings;
            Ok(())
        })
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PreparationStatus {
    #[default]
    NotStarted,
    Processing,
    Ready,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DjStatus {
    pub settings: DjSettings,
    pub songs_away: Option<i32>,
    pub preparation: PreparationStatus,
    pub target_uid: Option<u64>,
    pub pending_break: Option<PendingBreak>,
    pub message: Option<String>,
    pub voice_catalog: VoiceCatalog,
    pub catalog_loading: bool,
}

pub struct DjCoordinatorDeps {
    pub engine: Engine,
    /// App data directory's dj_filler child. Startup sweeps only owned WAV/part artifacts.
    pub root: PathBuf,
    pub settings: Arc<dyn DjSettingsStore>,
    pub passage_state: Arc<dyn PassageStateStore>,
    /// Only the committed sync account, never the settings editor's draft.
    pub account: watch::Receiver<DjAccount>,
    pub diagnostics: DjDiagnostics,
}
#[derive(Debug, thiserror::Error)]
pub enum CoordinatorError {
    #[error("DJ audio directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("DJ HTTP client: {0}")]
    Http(#[from] reqwest::Error),
    #[error("DJ settings could not be loaded: {0}")]
    Settings(String),
}

/// Cloneable UI facade. Dropping the last handle closes the actor once active audio ends.
#[derive(Clone)]
pub struct DjController {
    commands: mpsc::UnboundedSender<Command>,
    state: watch::Receiver<DjStatus>,
    diagnostics: DjDiagnostics,
}
impl DjController {
    pub fn subscribe(&self) -> watch::Receiver<DjStatus> {
        self.state.clone()
    }
    pub fn diagnostics(&self) -> DjDiagnostics {
        self.diagnostics.clone()
    }
    pub fn set_enabled(&self, enabled: bool) {
        self.send(Command::Enable(enabled));
    }
    pub fn set_show_entries_in_queue(&self, show: bool) {
        self.send(Command::Show(show));
    }
    pub fn select_voice(&self, id: String) {
        self.send(Command::Voice(id));
    }
    pub fn set_voice_level(&self, level: f32) {
        if level.is_finite() {
            self.send(Command::Level(level.clamp(0.5, 2.0)));
        }
    }
    pub fn refresh_catalogs(&self) {
        self.send(Command::Refresh);
    }
    pub fn clear_logs(&self) {
        self.diagnostics.clear();
    }
    pub fn set_debug_logging(&self, enabled: bool) {
        self.diagnostics.set_debug_logging(enabled);
    }
    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}
enum Command {
    Enable(bool),
    Show(bool),
    Voice(String),
    Level(f32),
    Refresh,
}

pub struct DjCoordinator;
impl DjCoordinator {
    pub async fn spawn(deps: DjCoordinatorDeps) -> Result<DjController, CoordinatorError> {
        sweep(&deps.root).await?;
        let mut settings = deps
            .settings
            .load()
            .await
            .map_err(|e| CoordinatorError::Settings(e.to_string()))?;
        settings.voice_level = if settings.voice_level.is_finite() {
            settings.voice_level.clamp(0.5, 2.0)
        } else {
            1.0
        };
        let passages = PassageClient::new(deps.passage_state, deps.diagnostics.clone())?;
        let remote = RemoteBreakGenerator::new(deps.diagnostics.clone())?;
        let catalog = VoiceCatalogClient::new(deps.diagnostics.clone())?;
        let events = deps.engine.events();
        deps.engine.await_restored().await;
        let queue = deps.engine.queue();
        let rng = StdRng::from_os_rng();
        let (commands, rx) = mpsc::unbounded_channel();
        let (state, state_rx) = watch::channel(DjStatus {
            settings,
            ..DjStatus::default()
        });
        let actor = Actor {
            engine: deps.engine,
            queue,
            events,
            account: deps.account,
            root: deps.root,
            store: deps.settings,
            diagnostics: deps.diagnostics.clone(),
            passages,
            remote,
            catalog,
            state,
            schedule: BreakSchedule::default(),
            rng,
            files: DjAudioFiles::default(),
            deliveries: HashMap::new(),
            work: None,
            catalog_work: None,
        };
        tokio::spawn(actor.run(rx));
        Ok(DjController {
            commands,
            state: state_rx,
            diagnostics: deps.diagnostics,
        })
    }
}
struct Delivery {
    account: DjAccount,
    passages: Passages,
}
struct Work {
    uid: u64,
    track: Track,
    account: DjAccount,
    task: JoinHandle<WorkResult>,
}
enum WorkResult {
    Lookup(Option<Passages>),
    Generated(Passages, Option<OwnedAudio>),
}
struct Actor {
    engine: Engine,
    queue: watch::Receiver<QueueState>,
    events: broadcast::Receiver<EngineEvent>,
    account: watch::Receiver<DjAccount>,
    root: PathBuf,
    store: Arc<dyn DjSettingsStore>,
    diagnostics: DjDiagnostics,
    passages: PassageClient,
    remote: RemoteBreakGenerator,
    catalog: VoiceCatalogClient,
    state: watch::Sender<DjStatus>,
    schedule: BreakSchedule,
    rng: StdRng,
    files: DjAudioFiles,
    deliveries: HashMap<String, Delivery>,
    work: Option<Work>,
    catalog_work: Option<JoinHandle<Result<VoiceCatalog, crate::CatalogError>>>,
}
impl Actor {
    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        let enabled = self.state.borrow().settings.enabled;
        self.transition(ScheduleEvent::Enabled(enabled));
        self.prepare();
        let mut account_open = true;
        let mut commands_open = true;
        loop {
            tokio::select! {
                biased;
                event = self.events.recv() => match event {
                    Ok(event) => self.event(event),
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // Reconcile lifecycle against authoritative player state after lost events.
                        let active = self.engine.player().borrow().active_break.clone();
                        let ids: Vec<_> = self.deliveries.keys().cloned().collect();
                        for id in ids {
                            if active.as_ref().is_some_and(|a| a.break_id == id) {
                                self.event(EngineEvent::BreakStarted { break_id: id });
                            } else {
                                self.files.ended(&id); self.deliveries.remove(&id);
                            }
                        }
                        self.cancel(); self.engine.set_pending_break(None);
                        self.state.send_modify(|s| s.pending_break = None);
                        self.transition(ScheduleEvent::QueueReplaced);
                        self.prepare();
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                command = commands.recv(), if commands_open => {
                    if let Some(command) = command { self.command(command).await; }
                    else {
                        commands_open = false;
                        self.cancel(); self.files.clear_finished();
                        self.engine.set_pending_break(None);
                        self.schedule.enabled = false;
                    }
                },
                result = async { (&mut self.work.as_mut().expect("guarded work").task).await }, if self.work.is_some() => {
                    let work = self.work.take().expect("completed work");
                    match result {
                        Ok(result) => self.completed(work.uid, work.track, work.account, result).await,
                        Err(_) => self.failed(work.uid),
                    }
                    self.prepare();
                },
                result = async { self.catalog_work.as_mut().expect("guarded catalog").await }, if self.catalog_work.is_some() => {
                    self.catalog_work = None;
                    self.state.send_modify(|s| {
                        s.catalog_loading = false;
                        match result {
                            Ok(Ok(catalog)) => { s.voice_catalog = catalog; s.message = None; }
                            Ok(Err(error)) => s.message = Some(error.to_string()),
                            Err(_) => s.message = Some("Voice catalog task failed.".into()),
                        }
                    });
                },
                changed = self.account.changed(), if account_open => {
                    if changed.is_err() { account_open = false; }
                    else {
                        self.cancel(); self.files.clear_finished();
                        self.engine.set_pending_break(None);
                        self.state.send_modify(|s| { s.pending_break = None; s.voice_catalog = VoiceCatalog::default(); });
                        if let Some(task) = self.catalog_work.take() { task.abort(); }
                        self.state.send_modify(|s| s.catalog_loading = false);
                        self.transition(ScheduleEvent::QueueReplaced);
                        self.prepare();
                    }
                },
                changed = self.queue.changed() => {
                    if changed.is_err() { break; }
                    self.transition(ScheduleEvent::CursorMoved);
                    self.prepare();
                },
            }
            if !commands_open && self.deliveries.is_empty() {
                break;
            }
        }
        self.cancel();
        if let Some(task) = self.catalog_work.take() {
            task.abort();
        }
    }
    fn transition(&mut self, event: ScheduleEvent) {
        let queue = self.queue.borrow();
        self.schedule = break_schedule::reduce(&self.schedule, event, &queue, &mut self.rng);
        self.state.send_modify(|s| {
            s.songs_away = self.schedule.songs_away(&queue);
            s.target_uid = self.schedule.target_uid;
        });
    }
    fn cancel(&mut self) {
        if let Some(work) = self.work.take() {
            work.task.abort();
        }
        self.state
            .send_modify(|s| s.preparation = PreparationStatus::NotStarted);
    }
    async fn command(&mut self, command: Command) {
        if matches!(command, Command::Refresh) {
            if self.catalog_work.is_none() {
                let client = self.catalog.clone();
                let account = self.account.borrow().clone();
                self.state.send_modify(|s| s.catalog_loading = true);
                self.catalog_work = Some(tokio::spawn(async move { client.fetch(&account).await }));
            }
            return;
        }
        let mut settings = self.state.borrow().settings.clone();
        match command {
            Command::Enable(on) => settings.enabled = on,
            Command::Show(show) => settings.show_entries_in_queue = show,
            Command::Level(level) => settings.voice_level = level,
            Command::Voice(id) => {
                if !self
                    .state
                    .borrow()
                    .voice_catalog
                    .voices
                    .iter()
                    .any(|v| v.id == id)
                {
                    return;
                }
                settings.selected_voice_id = Some(id);
            }
            Command::Refresh => unreachable!("handled above"),
        }
        if let Err(error) = self.store.save(settings.clone()).await {
            self.diagnostics.log(
                "DjController",
                "DJ settings could not be saved",
                Some("Store"),
            );
            // UI gets the error without exposing arbitrary store error text (which may contain secrets).
            drop(error);
            self.state
                .send_modify(|s| s.message = Some("DJ settings could not be saved.".into()));
            return;
        }
        let enabled_changed = settings.enabled != self.schedule.enabled;
        self.state.send_modify(|s| s.settings = settings);
        if enabled_changed {
            let enabled = self.state.borrow().settings.enabled;
            self.transition(ScheduleEvent::Enabled(enabled));
            if !enabled {
                self.cancel();
                self.files.clear_finished();
                self.engine.set_pending_break(None);
                self.state.send_modify(|s| s.pending_break = None);
            }
        }
        self.prepare();
    }
    fn event(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::QueueReplaced => {
                self.cancel();
                self.files.clear_finished();
                self.engine.set_pending_break(None);
                self.state.send_modify(|s| s.pending_break = None);
                self.transition(ScheduleEvent::QueueReplaced);
            }
            EngineEvent::BreakStarted { break_id } => {
                if self.files.started(&break_id) {
                    if let Some(delivery) = self.deliveries.get(&break_id) {
                        let client = self.passages.clone();
                        let account = delivery.account.clone();
                        let passages = delivery.passages.clone();
                        tokio::spawn(async move {
                            client.record_played(&account, &passages).await;
                        });
                    }
                    self.state.send_modify(|s| {
                        if s.pending_break
                            .as_ref()
                            .is_some_and(|p| p.break_id == break_id)
                        {
                            s.pending_break = None;
                            s.preparation = PreparationStatus::NotStarted;
                        }
                    });
                    self.transition(ScheduleEvent::BreakStarted);
                }
            }
            EngineEvent::BreakEnded { break_id } => {
                self.files.ended(&break_id);
                self.deliveries.remove(&break_id);
            }
            EngineEvent::BreakDropped { break_id } => {
                if !self.files.dropped(&break_id) {
                    return;
                }
                self.deliveries.remove(&break_id);
                let was_pending = self
                    .state
                    .borrow()
                    .pending_break
                    .as_ref()
                    .is_some_and(|p| p.break_id == break_id);
                if was_pending {
                    self.state.send_modify(|s| {
                        s.pending_break = None;
                        s.preparation = PreparationStatus::NotStarted;
                    });
                    if self.schedule.songs_until_break == 0 {
                        self.transition(ScheduleEvent::GenerationFailed);
                    }
                }
            }
            EngineEvent::ItemStarted { .. } | EngineEvent::Error { .. } => return,
        }
        self.prepare();
    }
    fn prepare(&mut self) {
        if !self.schedule.enabled {
            return;
        }
        let target = {
            let queue = self.queue.borrow();
            self.schedule
                .target_uid
                .and_then(|uid| queue.entries.iter().find(|e| e.uid == uid))
                .cloned()
        };
        let pending_uid = self
            .state
            .borrow()
            .pending_break
            .as_ref()
            .map(|p| p.before_uid);
        if pending_uid.is_some() && pending_uid != target.as_ref().map(|t| t.uid) {
            self.engine.set_pending_break(None);
            self.state.send_modify(|s| {
                s.pending_break = None;
                s.preparation = PreparationStatus::NotStarted;
            });
        }
        let account = self.account.borrow().clone();
        if !account.is_configured() {
            self.state.send_modify(|s| {
                s.preparation = PreparationStatus::NotStarted;
                s.message = Some("Sync server target is not set.".into());
            });
            return;
        }
        self.state.send_modify(|s| {
            if s.message.as_deref() == Some("Sync server target is not set.") {
                s.message = None;
            }
        });
        let Some(target) = target else {
            self.state
                .send_modify(|s| s.preparation = PreparationStatus::NotStarted);
            return;
        };
        if self.schedule.failed_on_current || pending_uid == Some(target.uid) {
            return;
        }
        if let Some(work) = &self.work {
            self.state.send_modify(|s| {
                s.preparation = if work.uid == target.uid {
                    PreparationStatus::Processing
                } else {
                    PreparationStatus::NotStarted
                }
            });
            return;
        }
        self.state
            .send_modify(|s| s.preparation = PreparationStatus::Processing);
        let passages = self.passages.clone();
        let track = target.track;
        let lookup_track = track.clone();
        let lookup_account = account.clone();
        self.work = Some(Work {
            uid: target.uid,
            track,
            account,
            task: tokio::spawn(async move {
                WorkResult::Lookup(passages.find_passages(&lookup_account, &lookup_track).await)
            }),
        });
    }
    fn failed(&mut self, uid: u64) {
        if self.schedule.target_uid == Some(uid) {
            self.transition(ScheduleEvent::GenerationFailed);
            self.state
                .send_modify(|s| s.preparation = PreparationStatus::NotStarted);
        }
    }
    async fn completed(&mut self, uid: u64, track: Track, account: DjAccount, result: WorkResult) {
        let (passages, generated) = match result {
            WorkResult::Lookup(None) => {
                self.files.forget_track(&track.id);
                self.failed(uid);
                return;
            }
            WorkResult::Lookup(Some(passages)) => (passages, None),
            WorkResult::Generated(_, None) => {
                self.failed(uid);
                return;
            }
            WorkResult::Generated(passages, Some(audio)) => (passages, Some(audio)),
        };
        let key = AudioKey {
            track_id: track.id.clone(),
            chunk_ids: passages.chunk_ids(),
        };
        if let Some(audio) = generated {
            self.files.remember(key.clone(), audio);
        }
        let Some(cached) = self.files.cached(&key) else {
            let remote = self.remote.clone();
            let generation_account = account.clone();
            let generation_track = track.clone();
            let settings = self.state.borrow().settings.clone();
            let output = OwnedAudio(
                self.root
                    .join("cache")
                    .join(format!("{}.wav", Uuid::new_v4())),
            );
            self.work = Some(Work {
                uid,
                track,
                account,
                task: tokio::spawn(async move {
                    let success = remote
                        .generate(
                            &generation_account,
                            &generation_track,
                            &passages,
                            settings.selected_voice_id.as_deref(),
                            settings.voice_level,
                            &output.0,
                        )
                        .await;
                    WorkResult::Generated(passages, success.then_some(output))
                }),
            });
            return;
        };
        if !self.schedule.enabled || self.schedule.target_uid != Some(uid) {
            return;
        }
        let id = Uuid::new_v4().to_string();
        let file = OwnedAudio(self.root.join(format!("{id}.wav")));
        if tokio::fs::copy(cached, &file.0).await.is_err() {
            self.diagnostics.log(
                "DjCoordinator",
                "Break delivery could not be copied",
                Some("Io"),
            );
            self.failed(uid);
            return;
        }
        let pending = PendingBreak {
            before_uid: uid,
            break_id: id.clone(),
            path: file.0.to_string_lossy().into_owned(),
        };
        self.files.deliver(id.clone(), key, file);
        self.deliveries.insert(id, Delivery { account, passages });
        self.engine.set_pending_break(Some(pending.clone()));
        self.state.send_modify(|s| {
            s.pending_break = Some(pending);
            s.preparation = PreparationStatus::Ready;
        });
    }
}
