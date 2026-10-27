use super::*;
use ap_library::{
    CatalogRepository,
    transfer::{ConnectionStore, StoredConnection},
};
use ap_model::{BackendKind, QueueState, SourceType};
use ap_playback::{
    Backend, BackendEvent, BackendRun, EngineDeps, ItemKey,
    ports::{BoxError, ConnectionGate, PlaybackPersistence, PlayerSettings},
};
use async_trait::async_trait;
use axum::{
    Router,
    body::Bytes,
    extract::{
        Path, State,
        ws::{Message, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, put},
};
use indexmap::IndexMap;
use std::sync::{Mutex, atomic::AtomicI64};
use tokio::sync::{Semaphore, broadcast};

#[derive(Default)]
struct Connections(Mutex<IndexMap<SourceType, StoredConnection>>);
#[async_trait]
impl ConnectionStore for Connections {
    async fn snapshot(&self) -> ap_library::Result<IndexMap<SourceType, StoredConnection>> {
        Ok(self.0.lock().unwrap().clone())
    }
    async fn put(&self, connection: StoredConnection) -> ap_library::Result<()> {
        self.0.lock().unwrap().insert(connection.source, connection);
        Ok(())
    }
}
#[derive(Default)]
struct Memory {
    queue: Mutex<QueueState>,
    settings: Mutex<PlayerSettings>,
}
#[async_trait]
impl PlaybackPersistence for Memory {
    async fn load_queue(&self) -> std::result::Result<QueueState, BoxError> {
        Ok(self.queue.lock().unwrap().clone())
    }
    async fn save_queue_content(&self, q: &QueueState) -> std::result::Result<(), BoxError> {
        *self.queue.lock().unwrap() = q.clone();
        Ok(())
    }
    async fn save_cursor(&self, _: Option<u64>, _: u64) -> std::result::Result<(), BoxError> {
        Ok(())
    }
    async fn load_settings(&self) -> std::result::Result<PlayerSettings, BoxError> {
        Ok(self.settings.lock().unwrap().clone())
    }
    async fn save_settings(&self, s: &PlayerSettings) -> std::result::Result<(), BoxError> {
        *self.settings.lock().unwrap() = s.clone();
        Ok(())
    }
    async fn track_gain(&self, _: &str) -> Option<f32> {
        None
    }
    async fn put_track_gain(&self, _: &str, _: f32) {}
}
struct Gate(watch::Sender<()>);
impl ConnectionGate for Gate {
    fn disabled_name(&self, _: SourceType) -> Option<String> {
        None
    }
    fn subscribe(&self) -> watch::Receiver<()> {
        self.0.subscribe()
    }
}
struct FakeBackend {
    events: broadcast::Sender<BackendEvent>,
    position: AtomicI64,
}
#[async_trait]
impl Backend for FakeBackend {
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
        _: usize,
        pos: i64,
        play: bool,
        _: ChangeOrigin,
    ) {
        self.position.store(pos, Ordering::SeqCst);
        let _ = self.events.send(BackendEvent::ItemStarted {
            load_id: id,
            key: run.items[0].key(),
        });
        let _ = self.events.send(BackendEvent::PlayingChanged {
            load_id: id,
            is_playing: play,
            buffering: false,
            by_player: false,
        });
    }
    async fn update_upcoming(&self, _: u64, _: BackendRun, _: ItemKey) {}
    async fn seek_to_item(&self, _: ItemKey, pos: i64) {
        self.position.store(pos, Ordering::SeqCst);
    }
    async fn play(&self, _: ChangeOrigin) {}
    async fn pause(&self) {}
    async fn seek_to(&self, pos: i64) {
        self.position.store(pos, Ordering::SeqCst);
    }
    async fn set_volume(&self, _: i32) {}
    async fn set_repeat_one(&self, _: bool) {}
    async fn stop(&self) {}
    fn position_ms(&self) -> i64 {
        self.position.load(Ordering::SeqCst)
    }
    fn duration_ms(&self) -> i64 {
        60_000
    }
}
#[derive(Clone)]
struct Server(Arc<ServerState>);
struct ServerState {
    snapshot: Mutex<String>,
    app: Mutex<Option<String>>,
    requests: Mutex<Vec<(String, String, String)>>,
    failed_push: AtomicBool,
    not_modified: AtomicBool,
    block: AtomicBool,
    requested: Semaphore,
    release: Semaphore,
    updates: broadcast::Sender<String>,
    sockets: AtomicU64,
}
async fn snapshot(State(s): State<Server>, headers: HeaderMap, uri: Uri) -> Response {
    s.0.requests
        .lock()
        .unwrap()
        .push((uri.to_string(), authorization(&headers), String::new()));
    s.0.requested.add_permits(1);
    if s.0.block.load(Ordering::SeqCst) {
        s.0.release.acquire().await.unwrap().forget();
    }
    if s.0.not_modified.load(Ordering::SeqCst) {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    s.0.snapshot.lock().unwrap().clone().into_response()
}
async fn app(State(s): State<Server>) -> Response {
    s.0.app.lock().unwrap().clone().map_or_else(
        || StatusCode::NOT_FOUND.into_response(),
        IntoResponse::into_response,
    )
}
async fn push(
    State(s): State<Server>,
    Path(ns): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    s.0.requests.lock().unwrap().push((
        format!("/v1/state/{ns}"),
        authorization(&headers),
        String::from_utf8(body.to_vec()).unwrap(),
    ));
    if s.0.failed_push.load(Ordering::SeqCst) {
        StatusCode::SERVICE_UNAVAILABLE.into_response()
    } else {
        StatusCode::OK.into_response()
    }
}
async fn push_app(State(s): State<Server>, headers: HeaderMap, body: Bytes) -> Response {
    push(State(s), Path("app-state".to_owned()), headers, body).await
}
fn authorization(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .map_or("<none>", |v| v.to_str().unwrap())
        .into()
}
async fn ws(
    State(s): State<Server>,
    headers: HeaderMap,
    uri: Uri,
    upgrade: WebSocketUpgrade,
) -> Response {
    assert_eq!(headers["authorization"], "Bearer secret");
    assert!(uri.query().is_none());
    let mut events = s.0.updates.subscribe();
    upgrade.on_upgrade(move |mut socket| async move {
        s.0.sockets.fetch_add(1, Ordering::SeqCst);
        loop {
            tokio::select! {
                message=events.recv()=>{
                    let Ok(message)=message else{break};
                    if message=="CLOSE"{let _=socket.send(Message::Close(None)).await;break;}
                    if socket.send(Message::Text(message.into())).await.is_err(){break;}
                }
                message=socket.recv()=>{if message.is_none(){break;}}
            }
        }
    })
}
/// Records revalidated providers; `fail` makes the next revalidation report an error.
#[derive(Default)]
struct Revalidations {
    sources: Mutex<Vec<SourceType>>,
    fail: AtomicBool,
}
#[async_trait]
impl crate::ProviderRevalidation for Revalidations {
    async fn revalidate(&self, source: SourceType) -> std::result::Result<(), String> {
        self.sources.lock().unwrap().push(source);
        if self.fail.load(Ordering::SeqCst) {
            Err("unreachable".into())
        } else {
            Ok(())
        }
    }
}
struct Rig {
    actor: Actor,
    server: Server,
    target: SyncTarget,
    connections: Arc<Connections>,
    revalidated: Arc<Revalidations>,
    _dir: tempfile::TempDir,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Rig {
    async fn new(domains: SyncDomains) -> Self {
        let server = Server(Arc::new(ServerState {
            snapshot: Mutex::new("{\"version\":1,\"app_state\":{}}".into()),
            app: Mutex::new(None),
            requests: Mutex::new(vec![]),
            failed_push: AtomicBool::new(false),
            not_modified: AtomicBool::new(false),
            block: AtomicBool::new(false),
            requested: Semaphore::new(0),
            release: Semaphore::new(0),
            updates: broadcast::channel(32).0,
            sockets: AtomicU64::new(0),
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = SyncTarget::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            " Bearer secret ",
        );
        let router = Router::new()
            .route("/v1/snapshot", get(snapshot))
            .route("/v1/state/app-state", get(app).put(push_app))
            .route("/v1/state/{ns}", put(push))
            .route("/v1/ws", get(ws))
            .with_state(server.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("test.db")).await.unwrap();
        let settings = Arc::new(SettingsStore::open(db.clone()).await.unwrap());
        settings
            .update(|s| {
                s.sync.server_target.clone_from(&target.server_target);
                s.sync.domains = domains;
            })
            .await
            .unwrap();
        let secrets = Arc::new(Secrets::new(Arc::new(
            ap_store::secrets::MemoryBackend::new().with(SecretAccount::SyncToken, "secret"),
        )));
        secrets.load().await;
        let backend = Arc::new(FakeBackend {
            events: broadcast::channel(32).0,
            position: AtomicI64::new(0),
        });
        let engine = Engine::spawn(EngineDeps {
            backends: vec![backend],
            persistence: Arc::new(Memory::default()),
            gate: Arc::new(Gate(watch::channel(()).0)),
        });
        engine.await_restored().await;
        let catalog = CatalogRepository::new(db.clone(), vec![]).await.unwrap();
        let playlists = PlaylistRepository::new(db.clone(), catalog.clone())
            .await
            .unwrap();
        let revalidated = Arc::new(Revalidations::default());
        let connections = Arc::new(Connections::default());
        let transfer = TransferService::new(db.clone(), connections.clone());
        let deps = SyncDeps {
            engine,
            db,
            settings,
            secrets,
            transfer,
            playlists,
            catalog,
            providers: revalidated.clone(),
            network: watch::channel(false).1,
        };
        let actor = Actor::new(
            deps,
            watch::channel(SyncUiState::default()).0,
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicBool::new(false)),
        );
        Self {
            actor,
            server,
            target,
            connections,
            revalidated,
            _dir: dir,
            task,
        }
    }
    fn snapshot(&self, value: impl std::fmt::Display) {
        *self.server.0.snapshot.lock().unwrap() = value.to_string();
    }
    async fn start(&mut self) {
        self.actor.command(Command::Start).await.unwrap();
    }
    fn pushes(&self, namespace: &str) -> Vec<JsonElement> {
        self.server
            .0
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.0 == format!("/v1/state/{namespace}"))
            .map(|r| JsonElement::parse(&r.2).unwrap())
            .collect()
    }
    async fn queue(&mut self, id: &str) {
        self.actor
            .deps
            .engine
            .set_queue(vec![song(id)], 0, ChangeOrigin::User, false);
        self.actor.deps.engine.flush().await;
        self.actor.observed.store(true, Ordering::SeqCst);
    }
}
fn domains() -> SyncDomains {
    SyncDomains {
        app_state: true,
        playlists: false,
        settings: false,
        provider_configuration: false,
    }
}
fn song(id: &str) -> Track {
    Track {
        url: Some(format!("http://localhost/{id}")),
        duration_ms: Some(60_000),
        ..Track::new(id, id, "Artist", SourceType::Jellyfin)
    }
}
fn json(raw: &str) -> JsonElement {
    JsonElement::parse(raw).unwrap()
}
fn state(id: &str) -> JsonElement {
    object([
        (
            "current_track",
            JsonElement::parse(&serde_json::to_string(&song(id)).unwrap()).unwrap(),
        ),
        (
            "queue",
            JsonElement::Array(vec![
                JsonElement::parse(&serde_json::to_string(&song(id)).unwrap()).unwrap(),
            ]),
        ),
        ("state", JsonElement::String("paused".into())),
        ("position", JsonElement::Number("10000".into())),
        ("volume", JsonElement::Number("73".into())),
        ("repeat_mode", JsonElement::String("all".into())),
        ("shuffle", JsonElement::Bool(false)),
    ])
}

#[test]
fn merge_matrix() {
    let base = SyncSignatures {
        remote: "old".into(),
        local: "old".into(),
    };
    for (local, remote, realtime, expected) in [
        ("old", "old", false, PlaylistMerge::Unchanged),
        ("new", "old", false, PlaylistMerge::PushLocal),
        ("old", "new", false, PlaylistMerge::ApplyRemote),
        ("new", "new", false, PlaylistMerge::Confirm),
        ("new", "new", true, PlaylistMerge::PushLocal),
    ] {
        assert_eq!(
            playlist_merge(true, false, local, remote, Some(&base), realtime, false),
            expected
        );
    }
    assert_eq!(
        playlist_merge(true, true, "old", "empty", Some(&base), true, false),
        PlaylistMerge::Confirm
    );
    assert_eq!(
        playlist_merge(true, false, "same", "same", None, false, false),
        PlaylistMerge::Unchanged
    );
    assert_eq!(
        playlist_merge(true, false, "a", "b", None, false, false),
        PlaylistMerge::Confirm
    );
    assert_eq!(
        playlist_merge(false, false, "empty", "b", None, false, false),
        PlaylistMerge::ApplyRemote
    );
    assert_eq!(
        playlist_merge(true, false, "old", "old", Some(&base), true, true),
        PlaylistMerge::Confirm
    );
}
#[tokio::test]
async fn transport_auth_normalization_304_and_exact_numbers() {
    let rig = Rig::new(domains()).await;
    let client = SyncClient::default();
    *rig.server.0.snapshot.lock().unwrap() =
        "{\"version\":2,\"playlists\":[{\"playlist\":{\"id\":\"n\",\"n\":1e3}}]}".into();
    let response = client
        .snapshot(&rig.target, Some(17))
        .await
        .unwrap()
        .unwrap();
    assert!(response.raw.contains("1e3"));
    assert!(response.data.to_string().contains("1e3"));
    {
        let requests = rig.server.0.requests.lock().unwrap();
        assert_eq!(requests[0].0, "/v1/snapshot?since_version=17");
        assert_eq!(requests[0].1, "Bearer secret");
    }
    rig.server.0.not_modified.store(true, Ordering::SeqCst);
    assert!(client.snapshot(&rig.target, None).await.unwrap().is_none());
    *rig.server.0.app.lock().unwrap() = Some("{\"version\":3,\"data\":{\"volume\":1e1}}".into());
    let response = client.app_state(&rig.target).await.unwrap().unwrap();
    assert_eq!(
        field(field(&response.data, "app_state"), "volume"),
        &JsonElement::Number("1e1".into())
    );
}
#[tokio::test]
async fn signature_fixture_payloads_survive_transport() {
    let rig = Rig::new(domains()).await;
    let fixtures = json(include_str!("../../../../fixtures/sync_signatures.json"));
    for fixture in field(&fixtures, "playlists").as_array().unwrap() {
        let raw = text(field(fixture, "input_text")).unwrap_or("null");
        *rig.server.0.snapshot.lock().unwrap() = format!("{{\"version\":1,\"playlists\":{raw}}}");
        let response = SyncClient::default()
            .snapshot(&rig.target, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            signatures::playlists(Some(field(&response.data, "playlists"))),
            text(field(fixture, "expected")).unwrap()
        );
    }
}
#[tokio::test]
async fn connect_conflict_cancel_and_explicit_resolution() {
    let mut rig = Rig::new(domains()).await;
    rig.snapshot(object([
        ("version", JsonElement::Number("1".into())),
        ("app_state", state("remote")),
    ]));
    rig.actor.command(Command::Connect).await.unwrap();
    assert!(rig.actor.state.borrow().conflict_pending);
    assert_eq!(rig.actor.state.borrow().status, CONFLICT);
    rig.actor.command(Command::Dismiss).await.unwrap();
    assert!(!rig.actor.state.borrow().confirmed);
    assert_eq!(rig.actor.state.borrow().status, "Sync connect cancelled.");
    rig.queue("local").await;
    rig.actor.command(Command::Local).await.unwrap();
    rig.actor.timers().await.unwrap();
    assert!(rig.pushes("app-state").is_empty());
    rig.actor.command(Command::Connect).await.unwrap();
    rig.actor.command(Command::Resolve(false)).await.unwrap();
    assert_eq!(
        rig.actor
            .deps
            .engine
            .queue()
            .borrow()
            .current()
            .unwrap()
            .track
            .id,
        "remote"
    );
    assert!(!rig.actor.state.borrow().conflict_pending);
}
#[tokio::test]
async fn startup_applies_paused_state_and_echo_guard() {
    let mut rig = Rig::new(domains()).await;
    rig.snapshot(object([
        ("version", JsonElement::Number("1".into())),
        ("app_state", state("remote")),
    ]));
    rig.start().await;
    assert_eq!(
        rig.actor
            .deps
            .engine
            .queue()
            .borrow()
            .current()
            .unwrap()
            .track
            .id,
        "remote"
    );
    assert_eq!(rig.actor.deps.engine.player().borrow().volume, 73);
    assert_eq!(rig.actor.deps.engine.position_ms(), 10000);
    rig.actor.observed.store(true, Ordering::SeqCst);
    rig.actor.command(Command::Local).await.unwrap();
    assert!(rig.actor.app_due.is_none());
    assert!(rig.pushes("app-state").is_empty());
    rig.actor.suppress_until = Instant::now();
    rig.actor.command(Command::Local).await.unwrap();
    rig.actor.timers().await.unwrap();
    assert_eq!(rig.pushes("app-state").len(), 1);
}
#[tokio::test]
async fn startup_never_replaces_active_playback() {
    let mut rig = Rig::new(domains()).await;
    rig.queue("local").await;
    rig.actor.deps.engine.play(ChangeOrigin::User);
    rig.actor.deps.engine.flush().await;
    rig.snapshot(object([
        ("version", JsonElement::Number("1".into())),
        ("app_state", state("remote")),
    ]));
    rig.start().await;
    assert_eq!(
        rig.actor
            .deps
            .engine
            .queue()
            .borrow()
            .current()
            .unwrap()
            .track
            .id,
        "local"
    );
    assert_eq!(rig.pushes("app-state").len(), 1);
}
#[tokio::test]
async fn startup_fetch_does_not_overwrite_concurrent_local_change() {
    let mut rig = Rig::new(domains()).await;
    rig.snapshot(object([
        ("version", JsonElement::Number("1".into())),
        ("app_state", state("remote")),
    ]));
    rig.server.0.block.store(true, Ordering::SeqCst);
    let engine = rig.actor.deps.engine.clone();
    let server = rig.server.clone();
    let revisions = rig.actor.revisions.clone();
    let edit = tokio::spawn(async move {
        server.0.requested.acquire().await.unwrap().forget();
        engine.set_queue(vec![song("during")], 0, ChangeOrigin::User, false);
        engine.flush().await;
        revisions.fetch_add(1, Ordering::SeqCst);
        server.0.release.add_permits(1);
    });
    rig.start().await;
    edit.await.unwrap();
    assert_eq!(
        rig.actor
            .deps
            .engine
            .queue()
            .borrow()
            .current()
            .unwrap()
            .track
            .id,
        "during"
    );
}
#[tokio::test]
async fn empty_restore_does_not_push_but_later_clear_does() {
    let mut rig = Rig::new(domains()).await;
    rig.start().await;
    rig.actor.command(Command::Local).await.unwrap();
    assert!(rig.actor.app_due.is_none());
    rig.queue("local").await;
    rig.actor.command(Command::Local).await.unwrap();
    rig.actor.timers().await.unwrap();
    assert_eq!(rig.pushes("app-state").len(), 1);
    rig.actor
        .deps
        .engine
        .set_queue(vec![], 0, ChangeOrigin::User, false);
    rig.actor.deps.engine.flush().await;
    rig.actor.app_last = None;
    rig.actor.command(Command::Local).await.unwrap();
    rig.actor.timers().await.unwrap();
    assert_eq!(rig.pushes("app-state").len(), 2);
    assert_eq!(
        field(
            field(rig.pushes("app-state").last().unwrap(), "data"),
            "queue"
        ),
        &JsonElement::Null
    );
}
#[tokio::test]
async fn target_edits_do_not_redirect_confirmed_account() {
    let mut rig = Rig::new(domains()).await;
    rig.start().await;
    rig.actor
        .deps
        .settings
        .update(|s| s.sync.server_target = "http://127.0.0.1:1".into())
        .await
        .unwrap();
    rig.actor
        .deps
        .secrets
        .set(SecretAccount::SyncToken, Some("different".into()))
        .await
        .unwrap();
    rig.queue("local").await;
    rig.actor.command(Command::Local).await.unwrap();
    rig.actor.timers().await.unwrap();
    assert_eq!(rig.pushes("app-state").len(), 1);
    assert!(
        rig.server
            .0
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.1 == "Bearer secret")
    );
}
#[tokio::test]
async fn rollback_clears_all_bases_and_resets_cursor() {
    let mut rig = Rig::new(domains()).await;
    let target = rig.target.clone();
    rig.actor
        .record(SyncDomain::Playlists, &target, "r".into(), "l".into())
        .await
        .unwrap();
    let key = target.key();
    rig.actor
        .deps
        .db
        .call(move |c| sync_state::record_version(c, &key, 100))
        .await
        .unwrap();
    rig.actor.cursor = 100;
    rig.actor
        .apply_snapshot(&json("{\"version\":2}"), &target, Apply::default())
        .await
        .unwrap();
    assert_eq!(rig.actor.cursor, 2);
    assert!(
        rig.actor
            .base(SyncDomain::Playlists, &target)
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn provider_config_import_merges_url_without_secrets() {
    let mut d = domains();
    d.app_state = false;
    d.provider_configuration = true;
    let mut rig = Rig::new(d).await;
    let mut original = StoredConnection::new(SourceType::Jellyfin);
    original.token = Some("keep-secret".into());
    rig.connections.put(original).await.unwrap();
    rig.snapshot(json(
        "{\"version\":1,\"provider_configuration\":{\"jellyfin\":{\"base_url\":\"http://new\"}}}",
    ));
    rig.start().await;
    let connections = rig.connections.snapshot().await.unwrap();
    let imported = &connections[&SourceType::Jellyfin];
    assert_eq!(imported.server_url.as_deref(), Some("http://new"));
    assert_eq!(imported.token.as_deref(), Some("keep-secret"));
}
#[tokio::test]
async fn settings_three_way_conflict_and_camel_fallback() {
    let mut d = domains();
    d.app_state = false;
    d.settings = true;
    let mut rig = Rig::new(d).await;
    let target = rig.target.clone();
    rig.actor.apply_snapshot(&json("{\"version\":1,\"settings\":{\"audioNormalizationEnabled\":true,\"audioNormalizationStrictMode\":true}}"),&target,Apply::default()).await.unwrap();
    assert!(rig.actor.deps.settings.get().player.normalization_strict);
    rig.actor
        .deps
        .settings
        .update(|s| s.player.normalization_enabled = false)
        .await
        .unwrap();
    rig.actor.apply_snapshot(&json("{\"version\":2,\"settings\":{\"audio_normalization_enabled\":true,\"audio_normalization_strict_mode\":false}}"),&target,Apply{merge:true,..Apply::default()}).await.unwrap();
    assert!(rig.actor.state.borrow().conflict_pending);
    assert!(!rig.actor.deps.settings.get().player.normalization_enabled);
}
#[tokio::test]
async fn realtime_playlist_confirmation_pauses_and_cancel_keeps_pause() {
    let mut d = domains();
    d.app_state = false;
    d.playlists = true;
    let mut rig = Rig::new(d).await;
    let target = rig.target.clone();
    rig.actor
        .deps
        .playlists
        .create("Local", ap_model::PlaylistType::Standard)
        .await
        .unwrap();
    rig.actor
        .apply_snapshot(
            &json("{\"version\":1,\"playlists\":[]}"),
            &target,
            Apply {
                realtime: true,
                ..Apply::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(rig.actor.state.borrow().status, PAUSED);
    assert!(rig.actor.state.borrow().playlist_sync_paused);
    rig.actor.command(Command::Dismiss).await.unwrap();
    assert!(rig.actor.state.borrow().playlist_sync_paused);
    assert!(!rig.actor.state.borrow().playlist_confirmation_required);
    rig.actor.command(Command::Resolve(true)).await.unwrap();
    assert!(!rig.actor.state.borrow().playlist_sync_paused);
    assert_eq!(rig.pushes("playlists").len(), 1);
}
#[tokio::test]
async fn websocket_filters_messages_and_authenticates_header() {
    let rig = Rig::new(domains()).await;
    let mut socket = SyncClient::default().updates(&rig.target).await.unwrap();
    rig.server
        .0
        .updates
        .send("{\"event_type\":\"ping\",\"namespace\":\"settings\"}".into())
        .unwrap();
    rig.server
        .0
        .updates
        .send("{\"event_type\":\"state_updated\",\"namespace\":\"app_state\",\"version\":4}".into())
        .unwrap();
    assert_eq!(
        crate::client::next_update(&mut socket)
            .await
            .unwrap()
            .unwrap()
            .version,
        Some(4)
    );
}
#[tokio::test]
async fn realtime_app_fetch_falls_back_and_ignores_self_events() {
    let mut rig = Rig::new(domains()).await;
    rig.start().await;
    rig.snapshot(object([
        ("version", JsonElement::Number("2".into())),
        ("app_state", state("realtime")),
    ]));
    let event = SyncUpdate {
        namespace: "app_state".into(),
        version: Some(2),
        source_client_id: None,
    };
    rig.actor.receive(event.clone()).await.unwrap();
    assert_eq!(
        rig.actor
            .deps
            .engine
            .queue()
            .borrow()
            .current()
            .unwrap()
            .track
            .id,
        "realtime"
    );
    let count = rig.server.0.requests.lock().unwrap().len();
    rig.actor
        .receive(SyncUpdate {
            version: Some(3),
            source_client_id: Some(rig.actor.deps.settings.get().sync.client_id),
            ..event
        })
        .await
        .unwrap();
    assert_eq!(rig.server.0.requests.lock().unwrap().len(), count);
}
#[tokio::test]
async fn failed_domain_push_retries_after_snapshot_merge() {
    let mut d = domains();
    d.app_state = false;
    d.settings = true;
    let mut rig = Rig::new(d).await;
    rig.start().await;
    let target = rig.target.clone();
    rig.actor
        .deps
        .settings
        .update(|s| s.player.normalization_enabled = true)
        .await
        .unwrap();
    rig.server.0.failed_push.store(true, Ordering::SeqCst);
    assert!(!rig.actor.push_domain(1, &target, None).await.unwrap());
    assert!(rig.actor.domains[1].reconcile);
    assert!(rig.actor.domains[1].due.is_some());
    rig.server.0.failed_push.store(false, Ordering::SeqCst);
    rig.actor.domains[1].last = None;
    rig.actor.domains[1].due = Some(Instant::now());
    let (tx, rx) = watch::channel(true);
    rig.actor.deps.network = rx;
    rig.actor.socket_due = None;
    rig.actor.timers().await.unwrap();
    drop(tx);
    assert_eq!(rig.pushes("settings").len(), 2);
    assert!(!rig.actor.domains[1].reconcile);
    let requests = rig.server.0.requests.lock().unwrap();
    assert_eq!(requests[requests.len() - 2].0, "/v1/snapshot");
}
#[tokio::test(start_paused = true)]
async fn backoff_only_resets_after_stable_socket_and_is_network_gated() {
    let mut rig = Rig::new(domains()).await;
    for expected in [1, 2, 4, 8, 16, 32, 60, 60] {
        rig.actor.opened_at = Some(Instant::now());
        rig.actor.socket_failed();
        let delay = rig.actor.socket_due.unwrap() - Instant::now();
        assert!(delay >= Duration::from_millis(expected * 750));
        assert!(delay <= Duration::from_secs(expected));
    }
    rig.actor.opened_at = Some(Instant::now() - Duration::from_secs(31));
    rig.actor.socket_failed();
    assert_eq!(rig.actor.socket_backoff, Duration::from_secs(2));
    rig.actor.set_confirmed(Some(rig.target.clone()));
    rig.actor.socket_due = Some(Instant::now());
    rig.actor.timers().await.unwrap();
    assert_eq!(rig.server.0.sockets.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn app_push_throttles_exact_seek_and_pause_without_heartbeat() {
    let mut rig = Rig::new(domains()).await;
    rig.start().await;
    rig.queue("local").await;
    rig.actor.command(Command::Local).await.unwrap();
    rig.actor.timers().await.unwrap();
    assert_eq!(rig.pushes("app-state").len(), 1);
    rig.actor.deps.engine.seek(17_123, ChangeOrigin::User);
    rig.actor.deps.engine.pause(ChangeOrigin::User);
    rig.actor.deps.engine.flush().await;
    // Pause after real network/DB setup: otherwise idle HTTP timers can auto-advance while
    // the dedicated SQLite thread is completing a barrier.
    tokio::time::pause();
    rig.actor.command(Command::Local).await.unwrap();
    rig.actor.timers().await.unwrap();
    assert_eq!(rig.pushes("app-state").len(), 1);
    tokio::time::advance(MIN_PUSH).await;
    rig.actor.timers().await.unwrap();
    let pushes = rig.pushes("app-state");
    assert_eq!(pushes.len(), 2);
    assert_eq!(
        field(field(&pushes[1], "data"), "position").as_long_lenient(),
        Some(17_123)
    );
    assert_eq!(
        text(field(field(&pushes[1], "data"), "state")),
        Some("paused")
    );
    tokio::time::advance(Duration::from_secs(120)).await;
    rig.actor.timers().await.unwrap();
    assert_eq!(rig.pushes("app-state").len(), 2);
}

#[tokio::test]
async fn engine_observer_ignores_sync_restore_and_observes_remote_nonempty_queue() {
    let rig = Rig::new(domains()).await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let revisions = Arc::new(AtomicU64::new(0));
    let observed = Arc::new(AtomicBool::new(false));
    let task = observe(
        &rig.actor.deps,
        tx.downgrade(),
        revisions.clone(),
        observed.clone(),
    );
    rig.actor
        .deps
        .engine
        .set_queue(vec![song("remote")], 0, ChangeOrigin::Sync, false);
    rig.actor.deps.engine.set_volume(20, ChangeOrigin::Restore);
    rig.actor.deps.engine.set_volume(21, ChangeOrigin::User);
    rig.actor.deps.engine.flush().await;
    let command = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(command, Command::Local));
    assert_eq!(revisions.load(Ordering::SeqCst), 1);
    assert!(observed.load(Ordering::SeqCst));
    assert!(rx.try_recv().is_err());
    task.abort();
}

#[tokio::test]
async fn playlist_wire_base_and_local_base_make_repeat_pull_idempotent() {
    let mut d = domains();
    d.app_state = false;
    d.playlists = true;
    let mut rig = Rig::new(d).await;
    let target = rig.target.clone();
    let fixtures = json(include_str!("../../../../fixtures/sync_signatures.json"));
    let sample = &field(&fixtures, "playlists").as_array().unwrap()[3];
    let remote = json(text(field(sample, "input_text")).unwrap());
    let snapshot = object([
        ("version", JsonElement::Number("3".into())),
        ("playlists", remote.clone()),
    ]);
    rig.actor
        .apply_snapshot(&snapshot, &target, Apply::default())
        .await
        .unwrap();
    let rows = rig.actor.deps.playlists.list().await.unwrap();
    assert_eq!(rows.len(), 1);
    let base = rig
        .actor
        .base(SyncDomain::Playlists, &target)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(base.remote, signatures::playlists(Some(&remote)));
    assert_eq!(
        base.local,
        signatures::playlists(Some(
            &rig.actor
                .deps
                .transfer
                .export_playlists_json()
                .await
                .unwrap()
        ))
    );
    rig.actor
        .apply_snapshot(&snapshot, &target, Apply::default())
        .await
        .unwrap();
    assert!(!rig.actor.state.borrow().playlist_sync_paused);
    assert_eq!(rig.actor.deps.playlists.list().await.unwrap(), rows);
    assert!(rig.pushes("playlists").is_empty());
}

#[tokio::test]
async fn realtime_socket_open_fetches_snapshot_before_pending_domain_retry() {
    let mut d = domains();
    d.app_state = false;
    d.settings = true;
    let mut rig = Rig::new(d).await;
    rig.start().await;
    let target = rig.target.clone();
    rig.actor
        .deps
        .settings
        .update(|s| s.player.normalization_enabled = true)
        .await
        .unwrap();
    rig.server.0.failed_push.store(true, Ordering::SeqCst);
    assert!(!rig.actor.push_domain(1, &target, None).await.unwrap());
    rig.server.0.failed_push.store(false, Ordering::SeqCst);
    rig.actor.domains[1].last = None;
    let (online, rx) = watch::channel(true);
    rig.actor.deps.network = rx;
    rig.actor.socket_due = Some(Instant::now());
    rig.actor.timers().await.unwrap();
    assert!(rig.actor.state.borrow().realtime_connected);
    assert_eq!(rig.pushes("settings").len(), 2);
    rig.server.0.updates.send("CLOSE".into()).unwrap();
    assert!(
        socket_update(&mut rig.actor.socket)
            .await
            .unwrap()
            .is_none()
    );
    rig.actor.socket_failed();
    assert!(!rig.actor.state.borrow().realtime_connected);
    rig.actor.socket_due = Some(Instant::now());
    rig.actor.timers().await.unwrap();
    assert_eq!(rig.server.0.sockets.load(Ordering::SeqCst), 2);
    assert!(rig.actor.state.borrow().realtime_connected);
    drop(online);
}

#[test]
fn target_debug_redacts_credentials() {
    assert!(
        !format!("{:?}", SyncTarget::new("example.com", "sensitive-token"))
            .contains("sensitive-token")
    );
}

#[tokio::test]
async fn provider_url_change_revalidates_once_and_publishes_import_summary() {
    let mut d = domains();
    d.app_state = false;
    d.provider_configuration = true;
    let mut rig = Rig::new(d).await;
    let mut jellyfin = StoredConnection::new(SourceType::Jellyfin);
    jellyfin.server_url = Some("http://old".into());
    jellyfin.token = Some("keep-secret".into());
    rig.connections.put(jellyfin).await.unwrap();
    rig.revalidated.fail.store(true, Ordering::SeqCst);
    let snapshot = json(
        "{\"version\":1,\"provider_configuration\":{\"jellyfin\":{\"base_url\":\"http://new\"},\"plex\":{\"base_url\":\"http://plex\"}}}",
    );
    let target = rig.target.clone();
    assert!(
        rig.actor
            .apply_snapshot(&snapshot, &target, Apply::default())
            .await
            .unwrap()
    );
    assert_eq!(
        *rig.revalidated.sources.lock().unwrap(),
        [SourceType::Jellyfin, SourceType::Plex]
    );
    let status = rig.actor.state.borrow().import_status.clone().unwrap();
    assert!(status.starts_with("Sync import complete."), "{status}");
    assert!(status.contains("Provider refresh failed: JELLYFIN: unreachable"));
    assert!(status.contains("Provider refresh failed: PLEX: unreachable"));
    // Sync status stays about transport; the import summary has its own line.
    assert_eq!(rig.actor.state.borrow().status, "Sync idle");
    rig.actor
        .apply_snapshot(&snapshot, &target, Apply::default())
        .await
        .unwrap();
    assert_eq!(rig.revalidated.sources.lock().unwrap().len(), 2);
    let connections = rig.connections.snapshot().await.unwrap();
    assert_eq!(
        connections[&SourceType::Jellyfin].token.as_deref(),
        Some("keep-secret")
    );
}

#[tokio::test]
async fn import_failure_reports_import_status_and_still_records_version() {
    let mut d = domains();
    d.app_state = false;
    d.playlists = true;
    let mut rig = Rig::new(d).await;
    rig.snapshot("{\"version\":7,\"playlists\":[{\"playlist\":{\"id\":\"broken\"}}]}");
    rig.actor.command(Command::Pull(false)).await.unwrap();
    let status = rig.actor.state.borrow().import_status.clone().unwrap();
    assert!(status.starts_with("Sync import failed: "), "{status}");
    // Not applied: no "Sync pull complete." over the failed import.
    assert_eq!(rig.actor.state.borrow().status, "Sync idle");
    assert!(rig.actor.deps.playlists.list().await.unwrap().is_empty());
    assert_eq!(rig.actor.cursor, 7);
    let key = rig.target.key();
    assert_eq!(
        rig.actor
            .deps
            .db
            .call(move |c| sync_state::version(c, &key))
            .await
            .unwrap(),
        Some(7)
    );
}

#[tokio::test]
async fn client_matches_android_transport_edge_cases() {
    let rig = Rig::new(domains()).await;
    let client = SyncClient::default();
    let anonymous = SyncTarget::new(&rig.target.server_target, "  ");
    client.snapshot(&anonymous, None).await.unwrap();
    assert_eq!(rig.server.0.requests.lock().unwrap()[0].1, "<none>");
    *rig.server.0.app.lock().unwrap() = Some("{\"version\":3,\"data\":[]}".into());
    assert!(client.app_state(&rig.target).await.unwrap().is_none());
    *rig.server.0.app.lock().unwrap() = Some("{\"data\":{\"volume\":5}}".into());
    let wrapped = client.app_state(&rig.target).await.unwrap().unwrap();
    assert_eq!(wrapped.data.to_string(), "{\"app_state\":{\"volume\":5}}");
    rig.snapshot("[]");
    assert!(matches!(
        client.snapshot(&rig.target, None).await,
        Err(SyncError::InvalidResponse)
    ));
    assert!(matches!(
        client.snapshot(&SyncTarget::new(" ", "x"), None).await,
        Err(SyncError::TargetNotSet)
    ));
    assert_eq!(
        SyncUpdate::parse(
            "{\"event_type\":\"state_updated\",\"namespace\":\"settings\",\"version\":\"5\",\"source_client_id\":9}"
        ),
        Some(SyncUpdate {
            namespace: "settings".into(),
            version: Some(5),
            source_client_id: Some("9".into()),
        })
    );
    assert_eq!(
        SyncUpdate::parse("{\"event_type\":\"state_updated\",\"namespace\":\"x\",\"version\":6.0}")
            .unwrap()
            .version,
        Some(6)
    );
    assert!(SyncUpdate::parse("{\"event_type\":\"state_updated\"}").is_none());
    assert!(SyncUpdate::parse("[1]").is_none());
}

#[tokio::test]
async fn failed_connect_keeps_previous_account_realtime() {
    let mut rig = Rig::new(domains()).await;
    rig.start().await;
    let confirmed = rig.actor.confirmed.clone();
    assert!(confirmed.is_some());
    rig.actor
        .deps
        .settings
        .update(|s| s.sync.server_target = "http://127.0.0.1:1".into())
        .await
        .unwrap();
    rig.actor.socket_due = None;
    assert!(rig.actor.command(Command::Connect).await.is_err());
    assert_eq!(rig.actor.confirmed, confirmed);
    assert!(rig.actor.socket_due.is_some());
}

#[tokio::test]
async fn realtime_fetch_and_push_failures_do_not_report_transport_status() {
    let mut rig = Rig::new(domains()).await;
    rig.start().await;
    assert_eq!(rig.actor.state.borrow().status, "Sync pull complete.");
    rig.snapshot("not json");
    rig.actor
        .receive(SyncUpdate {
            namespace: "app_state".into(),
            version: Some(9),
            source_client_id: None,
        })
        .await
        .unwrap();
    rig.server.0.failed_push.store(true, Ordering::SeqCst);
    rig.queue("local").await;
    rig.actor.command(Command::Local).await.unwrap();
    rig.actor.timers().await.unwrap();
    assert_eq!(rig.pushes("app-state").len(), 1);
    assert!(rig.actor.app_last.is_some());
    assert_eq!(rig.actor.state.borrow().status, "Sync pull complete.");
}
