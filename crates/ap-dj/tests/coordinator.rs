#[path = "../../ap-playback/tests/support/mod.rs"]
mod support;

use ap_dj::{
    DjAccount, DjController, DjCoordinator, DjCoordinatorDeps, DjDiagnostics, DjStatus,
    MemoryDjSettingsStore, MemoryPassageStateStore, PreparationStatus,
};
use ap_model::{BackendKind, PendingBreak, SourceType};
use ap_playback::{Backend, BackendEvent, BackendRun, ChangeOrigin, Engine, EngineDeps, ItemKey};
use async_trait::async_trait;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct Fake {
    events: broadcast::Sender<BackendEvent>,
    run: Mutex<Option<(u64, BackendRun)>>,
}
impl Fake {
    fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        Arc::new(Self {
            events,
            run: Mutex::new(None),
        })
    }
    fn started(&self, key: ItemKey) {
        let id = self.run.lock().unwrap().as_ref().unwrap().0;
        self.events
            .send(BackendEvent::ItemStarted { load_id: id, key })
            .unwrap();
    }
}
#[async_trait]
impl Backend for Fake {
    fn kind(&self) -> BackendKind {
        BackendKind::Local
    }
    fn subscribe(&self) -> broadcast::Receiver<BackendEvent> {
        self.events.subscribe()
    }
    async fn load(&self, id: u64, run: BackendRun, start: usize, _: i64, _: bool, _: ChangeOrigin) {
        let key = run.items[start].key();
        *self.run.lock().unwrap() = Some((id, run));
        self.started(key);
    }
    async fn update_upcoming(&self, id: u64, run: BackendRun, _: ItemKey) {
        *self.run.lock().unwrap() = Some((id, run));
    }
    async fn seek_to_item(&self, key: ItemKey, _: i64) {
        self.started(key);
    }
    async fn play(&self, _: ChangeOrigin) {}
    async fn pause(&self) {}
    async fn seek_to(&self, _: i64) {}
    async fn set_volume(&self, _: i32) {}
    async fn set_repeat_one(&self, _: bool) {}
    async fn stop(&self) {}
    fn position_ms(&self) -> i64 {
        0
    }
    fn duration_ms(&self) -> i64 {
        60_000
    }
}
struct Rig {
    engine: Engine,
    controller: DjController,
    backend: Arc<Fake>,
    root: tempfile::TempDir,
    settings: Arc<MemoryDjSettingsStore>,
    _account: watch::Sender<DjAccount>,
}
impl Rig {
    async fn new(server: &str) -> Self {
        let backend = Fake::new();
        let engine = Engine::spawn(EngineDeps {
            backends: vec![backend.clone()],
            persistence: Arc::new(support::Memory::default()),
            gate: support::Gate::new(),
        });
        engine.await_restored().await;
        engine.set_queue(
            (0..24)
                .map(|i| support::track(&i.to_string(), SourceType::Jellyfin))
                .collect(),
            0,
            ChangeOrigin::User,
            true,
        );
        wait_queue(&engine, 0).await;
        let root = tempfile::tempdir().unwrap();
        let settings = Arc::new(MemoryDjSettingsStore::default());
        let (account_tx, account) = watch::channel(DjAccount::new(server, "token"));
        let controller = DjCoordinator::spawn(DjCoordinatorDeps {
            engine: engine.clone(),
            root: root.path().join("dj_filler"),
            settings: settings.clone(),
            passage_state: Arc::new(MemoryPassageStateStore::default()),
            account,
            diagnostics: DjDiagnostics::new(),
        })
        .await
        .unwrap();
        // Drain the original queue-replaced event before enabling generation.
        support::settle().await;
        Self {
            engine,
            controller,
            backend,
            root,
            settings,
            _account: account_tx,
        }
    }
    async fn ready(&self) -> PendingBreak {
        wait_status(&self.controller, |s| {
            s.preparation == PreparationStatus::Ready
        })
        .await
        .pending_break
        .unwrap()
    }
    async fn enable_ready(&self) -> PendingBreak {
        self.controller.set_enabled(true);
        self.ready().await
    }
    async fn before(&self, pending: &PendingBreak) {
        let q = self.engine.queue().borrow().clone();
        let pos = q.order_pos_of_uid(pending.before_uid).unwrap();
        self.engine.jump_to_uid(q.entry_at(pos - 1).uid);
        wait_queue(&self.engine, isize::try_from(pos - 1).unwrap()).await;
        wait_status(&self.controller, |s| {
            s.songs_away == Some(0) && s.pending_break.is_some()
        })
        .await;
    }
    async fn start(&self, pending: &PendingBreak) {
        self.before(pending).await;
        self.engine.next();
        let mut player = self.engine.player();
        tokio::time::timeout(
            Duration::from_secs(5),
            player.wait_for(|p| {
                p.active_break
                    .as_ref()
                    .is_some_and(|b| b.break_id == pending.break_id)
            }),
        )
        .await
        .unwrap()
        .unwrap();
        wait_status(&self.controller, |s| {
            s.pending_break
                .as_ref()
                .is_none_or(|p| p.break_id != pending.break_id)
        })
        .await;
    }
}
async fn wait_status(controller: &DjController, predicate: impl Fn(&DjStatus) -> bool) -> DjStatus {
    let mut rx = controller.subscribe();
    let result = tokio::time::timeout(Duration::from_secs(5), rx.wait_for(|s| predicate(s))).await;
    assert!(
        matches!(result, Ok(Ok(_))),
        "DJ status timeout: {:?}; logs: {:?}",
        controller.subscribe().borrow(),
        controller.diagnostics().entries()
    );
    drop(result);
    rx.borrow().clone()
}
async fn wait_queue(engine: &Engine, cursor: isize) {
    let mut rx = engine.queue();
    tokio::time::timeout(
        Duration::from_secs(5),
        rx.wait_for(|q| q.cursor == cursor && !q.entries.is_empty()),
    )
    .await
    .unwrap()
    .unwrap();
}
async fn wait_deleted(file: &Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while file.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn wave() -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(
            &mut bytes,
            hound::WavSpec {
                channels: 1,
                sample_rate: 24_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for i in 0..2400 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
            let sample = ((i as f32 * 0.1).sin() * 8000.0) as i16;
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
    }
    bytes.into_inner()
}
async fn server(status: u16, delay: Duration) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/dj-passages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":"ok","passages":[{"chunk_id":1,"text":"A musical fact.","source":"book"}]}))).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1/dj-breaks"))
        .respond_with(
            ResponseTemplate::new(status)
                .set_body_bytes(wave())
                .set_delay(delay),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/dj-passages/played"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}
async fn generation_count(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/v1/dj-breaks")
        .count()
}

#[tokio::test]
async fn no_account_is_idle_and_controller_persists_settings() {
    let rig = Rig::new("").await;
    rig.controller.set_enabled(true);
    rig.controller.set_show_entries_in_queue(true);
    rig.controller.set_voice_level(8.0);
    let snapshot = wait_status(&rig.controller, |s| {
        (s.settings.voice_level - 2.0).abs() < f32::EPSILON
    })
    .await;
    assert!(snapshot.settings.enabled);
    assert!(snapshot.settings.show_entries_in_queue);
    assert_eq!(snapshot.preparation, PreparationStatus::NotStarted);
    assert_eq!(
        snapshot.message.as_deref(),
        Some("Sync server target is not set.")
    );
    assert_eq!(rig.settings.snapshot(), snapshot.settings);
}
#[tokio::test]
async fn advance_plays_break_previous_restarts_previous_and_receipt_is_sent_on_start() {
    let server = server(200, Duration::ZERO).await;
    let rig = Rig::new(&server.uri()).await;
    let pending = rig.enable_ready().await;
    let file = Path::new(&pending.path);
    assert!(file.exists());
    assert!(
        !server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().ends_with("/played"))
    );
    rig.start(&pending).await;
    let previous = rig.engine.queue().borrow().current().unwrap().uid;
    assert_ne!(previous, pending.before_uid);
    assert!(file.exists());
    tokio::time::timeout(Duration::from_secs(5), async {
        while !server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().ends_with("/played"))
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    rig.engine.previous();
    wait_deleted(file).await;
    assert_eq!(rig.engine.queue().borrow().current().unwrap().uid, previous);
    assert!(rig.engine.player().borrow().active_break.is_none());
}
#[tokio::test]
async fn next_during_break_skips_to_target_and_ends_file_ownership() {
    let server = server(200, Duration::ZERO).await;
    let rig = Rig::new(&server.uri()).await;
    let pending = rig.enable_ready().await;
    rig.start(&pending).await;
    rig.engine.next();
    let pos = rig
        .engine
        .queue()
        .borrow()
        .order_pos_of_uid(pending.before_uid)
        .unwrap();
    wait_queue(&rig.engine, isize::try_from(pos).unwrap()).await;
    wait_deleted(Path::new(&pending.path)).await;
    assert!(rig.engine.player().borrow().active_break.is_none());
}
#[tokio::test]
async fn jump_drops_unstarted_delivery_without_playing_break() {
    let server = server(200, Duration::ZERO).await;
    let rig = Rig::new(&server.uri()).await;
    let pending = rig.enable_ready().await;
    let q = rig.engine.queue().borrow().clone();
    let target_pos = q.order_pos_of_uid(pending.before_uid).unwrap();
    rig.engine.jump_to_uid(pending.before_uid);
    wait_queue(&rig.engine, isize::try_from(target_pos).unwrap()).await;
    wait_deleted(Path::new(&pending.path)).await;
    assert!(rig.engine.player().borrow().active_break.is_none());
}
#[tokio::test]
async fn target_change_does_not_cancel_inference_and_flip_back_reuses_cache() {
    let server = server(200, Duration::from_millis(100)).await;
    let rig = Rig::new(&server.uri()).await;
    rig.controller.set_enabled(true);
    let original = wait_status(&rig.controller, |s| {
        s.preparation == PreparationStatus::Processing && s.target_uid.is_some()
    })
    .await
    .target_uid
    .unwrap();
    let original_track = rig
        .engine
        .queue()
        .borrow()
        .entries
        .iter()
        .find(|e| e.uid == original)
        .unwrap()
        .track
        .clone();
    tokio::time::timeout(Duration::from_secs(5), async {
        while generation_count(&server).await == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    rig.engine.play_next(vec![
        support::track("other-target", SourceType::Jellyfin);
        6
    ]);
    wait_status(&rig.controller, |s| {
        s.target_uid.is_some_and(|id| id != original)
    })
    .await;
    let changed = rig.ready().await;
    assert_ne!(changed.before_uid, original);
    // Both A and B must finish: cancelling A would leave only B in the finished cache.
    assert_eq!(
        std::fs::read_dir(rig.root.path().join("dj_filler/cache"))
            .unwrap()
            .count(),
        2
    );
    assert_eq!(generation_count(&server).await, 2);
    let current = rig.engine.queue().borrow().current().unwrap().uid;
    rig.engine.jump_to_uid(current); // Resets play-next stacking without moving the cursor.
    rig.engine.play_next(vec![original_track.clone(); 6]);
    wait_status(&rig.controller, |s| {
        s.target_uid.is_some_and(|id| id != changed.before_uid)
    })
    .await;
    let reused = rig.ready().await;
    let target_id = rig
        .engine
        .queue()
        .borrow()
        .entries
        .iter()
        .find(|e| e.uid == reused.before_uid)
        .unwrap()
        .track
        .id
        .clone();
    assert_eq!(target_id, original_track.id);
    assert_ne!(reused.break_id, changed.break_id);
    assert_eq!(generation_count(&server).await, 2);
    assert!(Path::new(&reused.path).exists());
    wait_deleted(Path::new(&changed.path)).await;
}
#[tokio::test]
async fn disable_and_replace_cancel_generation_and_remove_never_started_files() {
    let server = server(200, Duration::from_millis(200)).await;
    let rig = Rig::new(&server.uri()).await;
    rig.controller.set_enabled(true);
    wait_status(&rig.controller, |s| {
        s.preparation == PreparationStatus::Processing
    })
    .await;
    rig.controller.set_enabled(false);
    wait_status(&rig.controller, |s| !s.settings.enabled).await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(rig.controller.subscribe().borrow().pending_break.is_none());
    let pending = rig.enable_ready().await;
    rig.engine.set_queue(
        vec![support::track("replacement", SourceType::Jellyfin)],
        0,
        ChangeOrigin::User,
        false,
    );
    wait_deleted(Path::new(&pending.path)).await;
    wait_status(&rig.controller, |s| {
        s.target_uid.is_none() && s.preparation == PreparationStatus::NotStarted
    })
    .await;
    assert_eq!(
        std::fs::read_dir(rig.root.path().join("dj_filler/cache"))
            .unwrap()
            .count(),
        0
    );
}
#[tokio::test]
async fn rejected_script_waits_until_next_song_then_retries_interval_three() {
    let server = server(204, Duration::ZERO).await;
    let rig = Rig::new(&server.uri()).await;
    rig.controller.set_enabled(true);
    wait_status(&rig.controller, |s| {
        s.preparation == PreparationStatus::Processing
    })
    .await;
    let failed = wait_status(&rig.controller, |s| {
        s.preparation == PreparationStatus::NotStarted
    })
    .await;
    assert_eq!(failed.songs_away, Some(3));
    assert_eq!(generation_count(&server).await, 1);
    rig.engine.next();
    wait_queue(&rig.engine, 1).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while generation_count(&server).await < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn startup_sweeps_owned_orphans_only() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("cache")).unwrap();
    for name in [
        "orphan.wav",
        "partial.part",
        "ready-track-id",
        "ready-passages.json",
        "cache/old.wav",
        "keep.txt",
    ] {
        std::fs::write(root.path().join(name), b"old").unwrap();
    }
    let engine = Engine::spawn(EngineDeps {
        backends: vec![],
        persistence: Arc::new(support::Memory::default()),
        gate: support::Gate::new(),
    });
    let (_account, account) = watch::channel(DjAccount::new("", ""));
    let _controller = DjCoordinator::spawn(DjCoordinatorDeps {
        engine,
        root: root.path().into(),
        settings: Arc::new(MemoryDjSettingsStore::default()),
        passage_state: Arc::new(MemoryPassageStateStore::default()),
        account,
        diagnostics: DjDiagnostics::new(),
    })
    .await
    .unwrap();
    assert!(root.path().join("keep.txt").exists());
    assert!(!root.path().join("orphan.wav").exists());
    assert!(!root.path().join("cache/old.wav").exists());
    assert!(!root.path().join("partial.part").exists());
    assert!(!root.path().join("ready-track-id").exists());
}

#[tokio::test]
async fn disable_keeps_active_file_until_natural_break_end() {
    let server = server(200, Duration::ZERO).await;
    let rig = Rig::new(&server.uri()).await;
    let pending = rig.enable_ready().await;
    rig.start(&pending).await;
    rig.controller.set_enabled(false);
    wait_status(&rig.controller, |s| !s.settings.enabled).await;
    support::settle().await;
    assert!(Path::new(&pending.path).exists());
    assert!(rig.engine.player().borrow().active_break.is_some());
    rig.backend.started(ItemKey::Song(pending.before_uid));
    wait_deleted(Path::new(&pending.path)).await;
    let pos = rig
        .engine
        .queue()
        .borrow()
        .order_pos_of_uid(pending.before_uid)
        .unwrap();
    wait_queue(&rig.engine, isize::try_from(pos).unwrap()).await;
    assert!(rig.engine.player().borrow().active_break.is_none());
}

#[tokio::test]
async fn queue_replacement_cancels_inflight_generation_without_late_delivery() {
    let server = server(200, Duration::from_millis(100)).await;
    let rig = Rig::new(&server.uri()).await;
    rig.controller.set_enabled(true);
    tokio::time::timeout(Duration::from_secs(5), async {
        while generation_count(&server).await == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    rig.engine.set_queue(
        vec![support::track("replacement", SourceType::Jellyfin)],
        0,
        ChangeOrigin::User,
        false,
    );
    wait_status(&rig.controller, |s| {
        s.settings.enabled
            && s.target_uid.is_none()
            && s.preparation == PreparationStatus::NotStarted
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(rig.controller.subscribe().borrow().pending_break.is_none());
    assert_eq!(
        std::fs::read_dir(rig.root.path().join("dj_filler/cache"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn facade_refreshes_voice_catalog_validates_selection_and_controls_logs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/dj-voice-models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "default_id":"voice", "voices":[{"id":"voice","name":"Voice","version":"1",
                "size_bytes":1,"sha256":"a".repeat(64)}]
        })))
        .mount(&server)
        .await;
    let rig = Rig::new(&server.uri()).await;
    rig.controller.refresh_catalogs();
    wait_status(&rig.controller, |s| !s.voice_catalog.voices.is_empty()).await;
    rig.controller.select_voice("unknown".into());
    rig.controller.select_voice("voice".into());
    rig.controller.set_voice_level(f32::NAN);
    let selected = wait_status(&rig.controller, |s| {
        s.settings.selected_voice_id.as_deref() == Some("voice")
    })
    .await;
    assert!((selected.settings.voice_level - 1.0).abs() < f32::EPSILON);
    assert_eq!(rig.settings.snapshot(), selected.settings);
    rig.controller.set_debug_logging(true);
    let diagnostics = rig.controller.diagnostics();
    diagnostics.debug("Test", "visible");
    assert_eq!(diagnostics.entries().last().unwrap().message, "visible");
    rig.controller.clear_logs();
    assert!(diagnostics.entries().is_empty());
}
