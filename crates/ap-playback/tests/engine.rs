mod support;
use ap_model::{BackendKind, PendingBreak, PlaybackStateType, QueueOps, RepeatMode, SourceType};
use ap_playback::{
    Backend, BackendEvent, BackendRun, ChangeOrigin, Engine, EngineDeps, EngineEvent, ItemKey,
};
use async_trait::async_trait;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI64, Ordering},
};
use support::{Gate, Memory, settle, track};
use tokio::sync::broadcast;
struct Fake {
    kind: BackendKind,
    tx: broadcast::Sender<BackendEvent>,
    calls: Arc<Mutex<Vec<String>>>,
    loads: Mutex<Vec<(u64, BackendRun, i64)>>,
    position: AtomicI64,
    upcoming: Mutex<Vec<BackendRun>>,
    block_load: Mutex<Option<Arc<tokio::sync::Notify>>>,
}
impl Fake {
    fn new(kind: BackendKind, calls: Arc<Mutex<Vec<String>>>) -> Arc<Self> {
        let (tx, _) = broadcast::channel(256);
        Arc::new(Self {
            kind,
            tx,
            calls,
            loads: Mutex::new(vec![]),
            position: AtomicI64::new(0),
            upcoming: Mutex::new(vec![]),
            block_load: Mutex::new(None),
        })
    }
    fn event(&self, e: BackendEvent) {
        self.tx.send(e).unwrap();
    }
    fn latest(&self) -> (u64, BackendRun, i64) {
        self.loads.lock().unwrap().last().unwrap().clone()
    }
}
#[async_trait]
impl Backend for Fake {
    fn kind(&self) -> BackendKind {
        self.kind
    }
    fn subscribe(&self) -> broadcast::Receiver<BackendEvent> {
        self.tx.subscribe()
    }
    async fn load(
        &self,
        id: u64,
        run: BackendRun,
        _: usize,
        pos: i64,
        play: bool,
        _: ChangeOrigin,
    ) {
        self.calls
            .lock()
            .unwrap()
            .push(format!("load:{:?}", self.kind));
        self.position.store(pos, Ordering::Relaxed);
        self.loads.lock().unwrap().push((id, run.clone(), pos));
        let barrier = self.block_load.lock().unwrap().take();
        if let Some(barrier) = barrier {
            barrier.notified().await;
        }
        self.event(BackendEvent::ItemStarted {
            load_id: id,
            key: run.items[0].key(),
        });
        self.event(BackendEvent::PlayingChanged {
            load_id: id,
            is_playing: play,
            buffering: false,
            by_player: false,
        });
    }
    async fn update_upcoming(&self, _: u64, run: BackendRun, _: ItemKey) {
        self.upcoming.lock().unwrap().push(run);
    }
    async fn seek_to_item(&self, _: ItemKey, pos: i64) {
        self.position.store(pos, Ordering::Relaxed);
    }
    async fn play(&self, _: ChangeOrigin) {
        self.calls.lock().unwrap().push("play".into());
    }
    async fn pause(&self) {
        self.calls.lock().unwrap().push("pause".into());
    }
    async fn seek_to(&self, pos: i64) {
        self.position.store(pos, Ordering::Relaxed);
    }
    async fn set_volume(&self, _: i32) {}
    async fn set_repeat_one(&self, _: bool) {}
    async fn stop(&self) {
        self.calls
            .lock()
            .unwrap()
            .push(format!("stop:{:?}", self.kind));
    }
    fn position_ms(&self) -> i64 {
        self.position.load(Ordering::Relaxed)
    }
    fn duration_ms(&self) -> i64 {
        60_000
    }
}
struct Rig {
    engine: Engine,
    local: Arc<Fake>,
    spotify: Arc<Fake>,
    memory: Arc<Memory>,
    gate: Arc<Gate>,
    calls: Arc<Mutex<Vec<String>>>,
}
impl Rig {
    async fn new(memory: Arc<Memory>) -> Self {
        let calls = Arc::default();
        let local = Fake::new(BackendKind::Local, Arc::clone(&calls));
        let spotify = Fake::new(BackendKind::Spotify, Arc::clone(&calls));
        let gate = Gate::new();
        let engine = Engine::spawn(EngineDeps {
            backends: vec![local.clone(), spotify.clone()],
            persistence: memory.clone(),
            gate: gate.clone(),
        });
        engine.await_restored().await;
        settle().await;
        Self {
            engine,
            local,
            spotify,
            memory,
            gate,
            calls,
        }
    }
    async fn queue(&self) {
        self.engine.set_queue(
            vec![
                track("a", SourceType::Jellyfin),
                track("b", SourceType::Plex),
            ],
            0,
            ChangeOrigin::User,
            true,
        );
        settle().await;
    }
}
#[tokio::test(start_paused = true)]
async fn paused_restore_first_play_keeps_position() {
    let memory = Arc::new(Memory::default());
    *memory.queue.lock().unwrap() = QueueOps::replace(
        &ap_model::QueueState::default(),
        vec![track("a", SourceType::Plex)],
        0,
        &mut rand::rng(),
        false,
    );
    memory.settings.lock().unwrap().position_ms = 12_345;
    let r = Rig::new(memory).await;
    assert!(r.local.loads.lock().unwrap().is_empty());
    assert_eq!(r.engine.player().borrow().status, PlaybackStateType::Paused);
    r.engine.play(ChangeOrigin::User);
    settle().await;
    assert_eq!(r.local.latest().2, 12_345);
}
#[tokio::test(start_paused = true)]
async fn stale_events_cannot_advance_new_queue() {
    let r = Rig::new(Arc::default()).await;
    r.queue().await;
    let old = r.local.latest().0;
    r.engine.next();
    settle().await;
    let uid = r.engine.player().borrow().current_uid;
    r.local.event(BackendEvent::RunEnded { load_id: old });
    settle().await;
    assert_eq!(r.engine.player().borrow().current_uid, uid);
}
#[tokio::test(start_paused = true)]
async fn stops_old_backend_before_new_load() {
    let r = Rig::new(Arc::default()).await;
    r.engine.set_queue(
        vec![
            track("a", SourceType::Plex),
            track("0123456789012345678901", SourceType::Spotify),
        ],
        0,
        ChangeOrigin::User,
        true,
    );
    settle().await;
    r.engine.next();
    settle().await;
    let calls = r.calls.lock().unwrap();
    let stop = calls.iter().position(|c| c == "stop:Local").unwrap();
    let load = calls.iter().position(|c| c == "load:Spotify").unwrap();
    assert!(stop < load);
    assert_eq!(r.spotify.loads.lock().unwrap().len(), 1);
}
#[tokio::test(start_paused = true)]
async fn gate_blocks_every_play_origin_and_allows_next() {
    let r = Rig::new(Arc::default()).await;
    *r.gate.disabled.lock().unwrap() = Some("Plex".into());
    r.queue().await;
    for origin in [
        ChangeOrigin::User,
        ChangeOrigin::Sync,
        ChangeOrigin::Restore,
    ] {
        r.engine.play(origin);
        r.engine.toggle();
        settle().await;
        assert!(!r.engine.player().borrow().play_requested);
    }
    assert!(r.local.loads.lock().unwrap().is_empty());
    assert_eq!(
        r.engine.player().borrow().disabled_reason.as_deref(),
        Some(
            "Playback disabled: Plex is not configured/authenticated for this app. Reconnect it in Settings. Next/Previous still works."
        )
    );
    let uid = r.engine.player().borrow().current_uid;
    r.engine.next();
    settle().await;
    assert_ne!(r.engine.player().borrow().current_uid, uid);
}
#[tokio::test(start_paused = true)]
async fn player_pause_becomes_intent() {
    let r = Rig::new(Arc::default()).await;
    r.queue().await;
    r.local.event(BackendEvent::PlayingChanged {
        load_id: r.local.latest().0,
        is_playing: false,
        buffering: false,
        by_player: true,
    });
    settle().await;
    assert!(!r.engine.player().borrow().play_requested);
    assert!(!r.calls.lock().unwrap().iter().any(|c| c == "play"));
}
#[tokio::test(start_paused = true)]
async fn internal_advance_and_repeat_all() {
    let r = Rig::new(Arc::default()).await;
    r.queue().await;
    let first = r.engine.player().borrow().current_uid;
    r.engine.set_repeat(RepeatMode::All, ChangeOrigin::User);
    r.engine.next();
    settle().await;
    r.local.event(BackendEvent::RunEnded {
        load_id: r.local.latest().0,
    });
    settle().await;
    assert_eq!(r.engine.player().borrow().current_uid, first);
    assert!(r.engine.player().borrow().play_requested);
}
#[tokio::test(start_paused = true)]
async fn previous_after_three_seconds_seeks_without_moving() {
    let r = Rig::new(Arc::default()).await;
    r.queue().await;
    r.engine.next();
    settle().await;
    let uid = r.engine.player().borrow().current_uid;
    r.engine.seek(3_001, ChangeOrigin::User);
    r.engine.previous();
    settle().await;
    assert_eq!(r.engine.player().borrow().current_uid, uid);
    assert_eq!(r.local.position_ms(), 0);
    r.engine.previous();
    settle().await;
    assert_ne!(r.engine.player().borrow().current_uid, uid);
}
#[tokio::test(start_paused = true)]
async fn content_debounce_cursor_versions_and_checkpoint() {
    let r = Rig::new(Arc::default()).await;
    r.queue().await;
    tokio::time::advance(std::time::Duration::from_millis(499)).await;
    settle().await;
    assert!(r.memory.writes.lock().unwrap().is_empty());
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    settle().await;
    assert_eq!(r.memory.writes.lock().unwrap().len(), 1);
    r.engine.next();
    settle().await;
    let version = r.engine.queue().borrow().content_version;
    assert!(
        r.memory
            .cursors
            .lock()
            .unwrap()
            .iter()
            .any(|(_, v)| *v == version)
    );
    tokio::time::advance(std::time::Duration::from_secs(15)).await;
    settle().await;
    assert!(r.memory.settings.lock().unwrap().position_ms >= 15_000);
}
#[tokio::test(start_paused = true)]
async fn jump_drops_break_without_playing_it() {
    let r = Rig::new(Arc::default()).await;
    r.queue().await;
    let target = r.engine.queue().borrow().entries[1].uid;
    let mut events = r.engine.events();
    r.engine.set_pending_break(Some(PendingBreak {
        before_uid: target,
        break_id: "break".into(),
        path: "/tmp/break.wav".into(),
    }));
    r.engine.jump_to_uid(target);
    settle().await;
    let received = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(received.contains(&EngineEvent::BreakDropped {
        break_id: "break".into()
    }));
    assert!(
        !received
            .iter()
            .any(|e| matches!(e, EngineEvent::BreakStarted { .. }))
    );
}
#[tokio::test(start_paused = true)]
async fn break_cursor_stays_previous_and_previous_restarts_song() {
    let r = Rig::new(Arc::default()).await;
    r.queue().await;
    let previous = r.engine.player().borrow().current_uid;
    let target = r.engine.queue().borrow().entries[1].uid;
    let mut events = r.engine.events();
    r.engine.set_pending_break(Some(PendingBreak {
        before_uid: target,
        break_id: "break".into(),
        path: "/tmp/break.wav".into(),
    }));
    r.engine.next();
    settle().await;
    assert_eq!(r.engine.player().borrow().current_uid, previous);
    assert!(r.engine.player().borrow().active_break.is_some());
    r.engine.previous();
    settle().await;
    assert_eq!(r.engine.player().borrow().current_uid, previous);
    assert!(r.engine.player().borrow().active_break.is_none());
    let received = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(received.contains(&EngineEvent::BreakEnded {
        break_id: "break".into()
    }));
    assert!(!received.contains(&EngineEvent::BreakDropped {
        break_id: "break".into()
    }));
}
#[tokio::test(start_paused = true)]
async fn local_errors_skip_and_terminal_error_stops() {
    let r = Rig::new(Arc::default()).await;
    r.queue().await;
    let uid = r.engine.player().borrow().current_uid;
    r.local.event(BackendEvent::Error {
        load_id: r.local.latest().0,
        key: uid.map(ItemKey::Song),
        message: "bad audio".into(),
    });
    settle().await;
    assert_ne!(r.engine.player().borrow().current_uid, uid);
    r.local.event(BackendEvent::Error {
        load_id: r.local.latest().0,
        key: None,
        message: "bad audio".into(),
    });
    settle().await;
    assert_eq!(r.engine.player().borrow().status, PlaybackStateType::Error);
}

#[tokio::test(start_paused = true)]
async fn in_flight_load_completes_and_latest_intent_wins() {
    let r = Rig::new(Arc::default()).await;
    let barrier = Arc::new(tokio::sync::Notify::new());
    *r.local.block_load.lock().unwrap() = Some(barrier.clone());
    r.queue().await;
    let stale = r.local.latest().0;
    r.engine.set_queue(
        vec![track("replacement", SourceType::Plex)],
        0,
        ChangeOrigin::Sync,
        true,
    );
    r.engine.pause(ChangeOrigin::User);
    settle().await;
    assert_eq!(r.local.loads.lock().unwrap().len(), 1);
    barrier.notify_one();
    settle().await;
    assert_eq!(r.local.loads.lock().unwrap().len(), 2);
    assert!(r.local.latest().0 > stale);
    assert!(!r.engine.player().borrow().play_requested);
    assert_eq!(
        r.engine.queue().borrow().current().unwrap().track.id,
        "replacement"
    );
}

#[tokio::test(start_paused = true)]
async fn local_run_slides_and_retains_at_most_three_history_items() {
    let r = Rig::new(Arc::default()).await;
    r.engine.set_queue(
        (0..70)
            .map(|i| track(&format!("song{i}"), SourceType::Plex))
            .collect(),
        0,
        ChangeOrigin::User,
        true,
    );
    settle().await;
    let id = r.local.latest().0;
    let entries = r.engine.queue().borrow().entries.clone();
    for entry in entries.iter().take(10).skip(1) {
        r.local.event(BackendEvent::ItemStarted {
            load_id: id,
            key: ItemKey::Song(entry.uid),
        });
        settle().await;
    }
    let updates = r.local.upcoming.lock().unwrap();
    let latest = updates.last().unwrap();
    assert_eq!(latest.items.len(), 50);
    assert_eq!(latest.items[3].key(), ItemKey::Song(entries[9].uid));
    assert_eq!(r.local.loads.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn embedded_break_is_delivered_once_and_keeps_gapless_run() {
    let r = Rig::new(Arc::default()).await;
    r.queue().await;
    let target = r.engine.queue().borrow().entries[1].uid;
    let previous = r.engine.player().borrow().current_uid;
    let mut events = r.engine.events();
    r.engine.set_pending_break(Some(PendingBreak {
        before_uid: target,
        break_id: "embedded".into(),
        path: "/tmp/embedded.wav".into(),
    }));
    settle().await;
    let id = r.local.latest().0;
    for _ in 0..2 {
        r.local.event(BackendEvent::ItemStarted {
            load_id: id,
            key: ItemKey::Break("embedded".into()),
        });
        settle().await;
    }
    assert_eq!(r.engine.player().borrow().current_uid, previous);
    assert_eq!(r.local.loads.lock().unwrap().len(), 1);
    r.local.event(BackendEvent::ItemStarted {
        load_id: id,
        key: ItemKey::Song(target),
    });
    settle().await;
    let events = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, EngineEvent::BreakStarted { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, EngineEvent::BreakEnded { .. }))
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, EngineEvent::BreakDropped { .. }))
    );
    assert_eq!(r.engine.player().borrow().current_uid, Some(target));
}

#[tokio::test(start_paused = true)]
async fn gate_cannot_be_bypassed_by_external_resume_after_auto_pause() {
    let r = Rig::new(Arc::default()).await;
    r.queue().await;
    *r.gate.disabled.lock().unwrap() = Some("Plex".into());
    r.gate.tx.send(()).unwrap();
    settle().await;
    assert!(!r.engine.player().borrow().play_requested);
    let pauses = r
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| *call == "pause")
        .count();
    for _ in 0..3 {
        r.gate.tx.send(()).unwrap();
        settle().await;
    }
    assert_eq!(
        r.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| *call == "pause")
            .count(),
        pauses
    );
    r.local.event(BackendEvent::PlayingChanged {
        load_id: r.local.latest().0,
        is_playing: true,
        buffering: false,
        by_player: true,
    });
    settle().await;
    assert!(!r.engine.player().borrow().play_requested);
    assert_eq!(r.engine.player().borrow().status, PlaybackStateType::Paused);
    assert_eq!(
        r.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| *call == "pause")
            .count(),
        pauses + 1
    );
}
