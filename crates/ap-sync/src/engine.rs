//! Serialized sync operations. User revisions are observed independently of network requests.
use crate::{
    ProviderRevalidation, Result, SyncClient, SyncError, SyncSocket, SyncTarget, SyncUpdate,
    boolean, field, object, text,
};
use ap_library::{
    CatalogRepository, PlaylistRepository,
    transfer::{ImportSummary, MergePolicy, TransferService},
};
use ap_model::{PlaybackStateType, RepeatMode, SourceType, Track, json::JsonElement, signatures};
use ap_playback::{Engine, state::ChangeOrigin};
use ap_store::{
    Db,
    secrets::{SecretAccount, Secrets},
    settings::{SettingsStore, SyncDomains},
    sync_state::{self, SyncDomain, SyncSignatures},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};
const MIN_PUSH: Duration = Duration::from_secs(2);
const ECHO: Duration = Duration::from_millis(3500);
const CONFLICT: &str = "Server already has synced data - choose which side to keep.";
const PAUSED: &str = "Playlist sync paused: confirm in Settings";

pub struct SyncDeps {
    pub engine: Engine,
    pub db: Db,
    pub settings: Arc<SettingsStore>,
    pub secrets: Arc<Secrets>,
    pub transfer: TransferService,
    pub playlists: PlaylistRepository,
    pub catalog: CatalogRepository,
    pub providers: Arc<dyn ProviderRevalidation>,
    pub network: watch::Receiver<bool>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncUiState {
    pub status: String,
    pub conflict_pending: bool,
    pub playlist_confirmation_required: bool,
    pub playlist_sync_paused: bool,
    pub confirmed: bool,
    pub in_progress: bool,
    pub realtime_connected: bool,
    /// Android `SyncImportStatus`: last sync import summary ("Sync import complete. …") or
    /// failure, shared with Settings' state-transfer status line rather than the sync status.
    pub import_status: Option<String>,
}
impl Default for SyncUiState {
    fn default() -> Self {
        Self {
            status: "Sync idle".to_owned(),
            conflict_pending: false,
            playlist_confirmation_required: false,
            playlist_sync_paused: false,
            confirmed: false,
            in_progress: false,
            realtime_connected: false,
            import_status: None,
        }
    }
}
#[derive(Clone)]
pub struct SyncEngine {
    commands: mpsc::UnboundedSender<Command>,
    state: watch::Receiver<SyncUiState>,
    _tasks: Arc<ServiceTasks>,
}
struct ServiceTasks {
    actor: tokio::task::JoinHandle<()>,
    observer: tokio::task::JoinHandle<()>,
}
impl Drop for ServiceTasks {
    fn drop(&mut self) {
        self.actor.abort();
        self.observer.abort();
    }
}
enum Command {
    Start,
    Connect,
    Pull(bool),
    Resolve(bool),
    Dismiss,
    Local,
    Domains,
    Providers,
}
impl SyncEngine {
    pub fn spawn(deps: SyncDeps) -> Self {
        let (commands, rx) = mpsc::unbounded_channel();
        let (state_tx, state) = watch::channel(SyncUiState::default());
        let revisions = Arc::new(AtomicU64::new(0));
        let observed = Arc::new(AtomicBool::new(false));
        let observer = observe(
            &deps,
            commands.downgrade(),
            revisions.clone(),
            observed.clone(),
        );
        let actor = tokio::spawn(Actor::new(deps, state_tx, revisions, observed).run(rx));
        Self {
            commands,
            state,
            _tasks: Arc::new(ServiceTasks { actor, observer }),
        }
    }
    pub fn state(&self) -> watch::Receiver<SyncUiState> {
        self.state.clone()
    }
    pub fn start(&self) {
        let _ = self.commands.send(Command::Start);
    }
    pub fn connect(&self) {
        let _ = self.commands.send(Command::Connect);
    }
    pub fn pull(&self, confirm_playlist_overwrite: bool) {
        let _ = self
            .commands
            .send(Command::Pull(confirm_playlist_overwrite));
    }
    pub fn resolve_conflict(&self, use_local: bool) {
        let _ = self.commands.send(Command::Resolve(use_local));
    }
    pub fn dismiss_conflict(&self) {
        let _ = self.commands.send(Command::Dismiss);
    }
}
fn observe(
    deps: &SyncDeps,
    commands: mpsc::WeakUnboundedSender<Command>,
    revisions: Arc<AtomicU64>,
    observed: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    let mut changes = deps.engine.changes();
    let engine = deps.engine.clone();
    let mut queue = engine.queue();
    let mut settings = deps.settings.subscribe();
    let mut secrets = deps.secrets.subscribe();
    let mut playlists = deps.playlists.subscribe();
    tokio::spawn(async move {
        engine.await_restored().await;
        observed.store(
            !engine.queue().borrow().entries.is_empty(),
            Ordering::SeqCst,
        );
        loop {
            let command = tokio::select! {
                result = queue.changed() => {
                    if result.is_err() { break; }
                    if !queue.borrow().entries.is_empty() { observed.store(true, Ordering::SeqCst); }
                    continue;
                },
                change = changes.recv() => match change {
                    Ok(change) if change.origin == ChangeOrigin::User => { revisions.fetch_add(1, Ordering::SeqCst); if !engine.queue().borrow().entries.is_empty() { observed.store(true, Ordering::SeqCst); } Command::Local },
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                result = settings.changed() => { if result.is_err() { break; } Command::Domains },
                result = secrets.changed() => { if result.is_err() { break; } Command::Providers },
                result = playlists.recv() => { if matches!(result, Err(tokio::sync::broadcast::error::RecvError::Closed)) { break; } Command::Domains },
            };
            let Some(tx) = commands.upgrade() else { break };
            if tx.send(command).is_err() {
                break;
            }
        }
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaylistMerge {
    ApplyRemote,
    PushLocal,
    Unchanged,
    Confirm,
}
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "independent inputs of the Android merge decision table"
)]
pub(crate) fn playlist_merge(
    local_nonempty: bool,
    remote_empty: bool,
    local: &str,
    remote: &str,
    base: Option<&SyncSignatures>,
    realtime: bool,
    awaiting: bool,
) -> PlaylistMerge {
    if local_nonempty && remote_empty && base.is_none_or(|b| remote != b.remote) {
        return PlaylistMerge::Confirm;
    }
    let Some(base) = base else {
        return if local == remote {
            PlaylistMerge::Unchanged
        } else if local_nonempty {
            PlaylistMerge::Confirm
        } else {
            PlaylistMerge::ApplyRemote
        };
    };
    if awaiting {
        return PlaylistMerge::Confirm;
    }
    match (local != base.local, remote != base.remote) {
        (true, false) => PlaylistMerge::PushLocal,
        (false, false) => PlaylistMerge::Unchanged,
        (false, true) => PlaylistMerge::ApplyRemote,
        (true, true) if realtime => PlaylistMerge::PushLocal,
        (true, true) => PlaylistMerge::Confirm,
    }
}
struct DomainPush {
    domain: SyncDomain,
    due: Option<Instant>,
    last: Option<Instant>,
    retry: Duration,
    reconcile: bool,
    pending_target: Option<String>,
}
impl DomainPush {
    fn new(domain: SyncDomain) -> Self {
        Self {
            domain,
            due: None,
            last: None,
            retry: MIN_PUSH,
            reconcile: false,
            pending_target: None,
        }
    }
    fn request(&mut self) {
        if self.due.is_none() {
            self.due = Some(Instant::now() + MIN_PUSH);
        }
    }
}
struct Actor {
    deps: SyncDeps,
    client: SyncClient,
    state: watch::Sender<SyncUiState>,
    revisions: Arc<AtomicU64>,
    observed: Arc<AtomicBool>,
    started: bool,
    confirmed: Option<SyncTarget>,
    pending: Option<SyncTarget>,
    conflict: Option<SyncTarget>,
    playlist_target: Option<SyncTarget>,
    cursor: i64,
    version_target: Option<String>,
    suppress_until: Instant,
    app_last: Option<Instant>,
    app_due: Option<Instant>,
    domains: [DomainPush; 3],
    startup_due: Option<Instant>,
    startup_backoff: Duration,
    socket: Option<SyncSocket>,
    socket_due: Option<Instant>,
    opened_at: Option<Instant>,
    socket_backoff: Duration,
}
impl Actor {
    fn new(
        deps: SyncDeps,
        state: watch::Sender<SyncUiState>,
        revisions: Arc<AtomicU64>,
        observed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            deps,
            client: SyncClient::default(),
            state,
            revisions,
            observed,
            started: false,
            confirmed: None,
            pending: None,
            conflict: None,
            playlist_target: None,
            cursor: 0,
            version_target: None,
            suppress_until: Instant::now(),
            app_last: None,
            app_due: None,
            domains: [
                DomainPush::new(SyncDomain::Playlists),
                DomainPush::new(SyncDomain::Settings),
                DomainPush::new(SyncDomain::ProviderConfiguration),
            ],
            startup_due: None,
            startup_backoff: MIN_PUSH,
            socket: None,
            socket_due: None,
            opened_at: None,
            socket_backoff: Duration::from_secs(1),
        }
    }
    fn status(&self, status: &str) {
        self.state.send_modify(|s| status.clone_into(&mut s.status));
    }
    fn set_confirmed(&mut self, target: Option<SyncTarget>) {
        self.confirmed = target;
        self.state
            .send_modify(|s| s.confirmed = self.confirmed.is_some());
    }
    fn selected(&self) -> SyncDomains {
        self.deps.settings.get().sync.domains
    }
    fn target(&self) -> SyncTarget {
        SyncTarget::new(
            &self.deps.settings.get().sync.server_target,
            &self
                .deps
                .secrets
                .get(SecretAccount::SyncToken)
                .unwrap_or_default(),
        )
    }
    fn can_push(&self) -> bool {
        self.confirmed.is_some() && !self.state.borrow().conflict_pending
    }
    fn can_app(&self) -> bool {
        self.can_push()
            && self.observed.load(Ordering::SeqCst)
            && Instant::now() >= self.suppress_until
    }
    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        self.deps.engine.await_restored().await;
        self.deps.secrets.await_loaded().await;
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            let operation = tokio::select! {
                command = commands.recv() => { let Some(command) = command else { break }; Some(command) },
                _ = tick.tick() => None,
                result = self.deps.network.changed() => {
                    if result.is_err() { break; }
                    if *self.deps.network.borrow() {
                        if self.pending.is_some() { self.startup_due = Some(Instant::now()); }
                        if self.confirmed.is_some() { self.socket_due = Some(Instant::now()); }
                    } else {
                        self.close_socket();
                    }
                    None
                },
                result = socket_update(&mut self.socket) => {
                    // Android: a failed event application ends the socket and reconnects without a status.
                    match result { Ok(Some(event)) => { if self.receive(event).await.is_err() { self.socket_failed(); } }, Ok(None) | Err(_) => self.socket_failed() }
                    continue;
                }
            };
            if let Some(command) = operation {
                self.state.send_modify(|s| s.in_progress = true);
                let startup = matches!(command, Command::Start);
                if let Err(error) = self.command(command).await {
                    self.failure(&error, startup);
                }
                self.state.send_modify(|s| s.in_progress = false);
            }
            if let Err(error) = self.timers().await {
                self.failure(&error, self.pending.is_some());
            }
        }
    }
    fn failure(&self, _error: &SyncError, startup: bool) {
        self.status(if startup {
            "Sync server unavailable. Continued with local state."
        } else {
            "Sync server unavailable."
        });
    }
    async fn command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Start => {
                if self.started {
                    return Ok(());
                }
                self.started = true;
                let target = self.target();
                if target.server_target.is_empty() {
                    self.status("Sync server target is not set.");
                    return Ok(());
                }
                self.pending = Some(target.clone());
                self.startup_due = Some(Instant::now() + MIN_PUSH);
                self.pull_now(&target, false, true).await?;
            }
            Command::Connect => {
                self.started = true;
                self.pending = None;
                self.startup_due = None;
                self.close_socket();
                let result = self.connect_now().await;
                // Like Android, a failed Connect resumes realtime for the previously confirmed account.
                if self.confirmed.is_some() {
                    self.socket_due = Some(Instant::now());
                }
                result?;
            }
            Command::Pull(confirm) => {
                self.state
                    .send_modify(|s| s.playlist_confirmation_required = false);
                let target = self.target();
                if target.server_target.is_empty() {
                    self.status("Sync server target is not set.");
                } else {
                    self.pull_now(&target, confirm, false).await?;
                }
            }
            Command::Resolve(local) => {
                let playlist_only = !self.state.borrow().conflict_pending
                    && self.state.borrow().playlist_sync_paused;
                let selected = if playlist_only {
                    self.playlist_target.clone()
                } else {
                    self.conflict.clone()
                };
                let Some(target) = selected else {
                    return Ok(());
                };
                if target != self.target() {
                    self.dismiss();
                    return Ok(());
                }
                if !playlist_only {
                    self.conflict = None;
                    self.state.send_modify(|s| s.conflict_pending = false);
                }
                if local {
                    let success = if playlist_only {
                        matches!(self.push_domain(0, &target, None).await, Ok(true))
                    } else {
                        self.push_local(&target).await
                    };
                    self.status(if success {
                        "Pushed local data to server."
                    } else {
                        "Some local data failed to push to server."
                    });
                } else {
                    self.pull_now(&target, true, false).await?;
                }
            }
            Command::Dismiss => self.dismiss(),
            Command::Local => {
                if self.can_app() && self.selected().app_state {
                    self.app_due = Some(
                        self.app_last
                            .map_or(Instant::now(), |last| (last + MIN_PUSH).max(Instant::now())),
                    );
                }
            }
            Command::Domains => {
                if self.can_push() {
                    self.domains[0].request();
                    self.domains[1].request();
                }
            }
            Command::Providers => {
                if self.can_push() {
                    self.domains[2].request();
                }
            }
        }
        Ok(())
    }
    async fn connect_now(&mut self) -> Result<()> {
        let target = self.target();
        if target.server_target.is_empty() {
            self.status("Sync server target is not set.");
            return Ok(());
        }
        self.deps
            .settings
            .update(|s| s.sync.server_target.clone_from(&target.server_target))
            .await?;
        self.deps
            .secrets
            .set(SecretAccount::SyncToken, Some(target.auth_token.clone()))
            .await
            .map_err(|e| SyncError::Credentials(e.to_string()))?;
        self.reset_cursor(&target);
        let snapshot = self
            .client
            .snapshot(&target, None)
            .await?
            .ok_or(SyncError::Unavailable)?
            .data;
        self.set_confirmed(Some(target.clone()));
        if has_server_data(&snapshot, self.selected()) {
            self.conflict = Some(target);
            self.state.send_modify(|s| s.conflict_pending = true);
            self.status(CONFLICT);
        } else {
            self.conflict = None;
            self.state.send_modify(|s| s.conflict_pending = false);
            let success = self.push_local(&target).await;
            self.status(if success {
                "Connected. Pushed local data to server."
            } else {
                "Connected, but some local data failed to push."
            });
        }
        Ok(())
    }
    fn dismiss(&mut self) {
        if !self.state.borrow().conflict_pending && self.state.borrow().playlist_sync_paused {
            self.state
                .send_modify(|s| s.playlist_confirmation_required = false);
            self.status(PAUSED);
            return;
        }
        self.pending = None;
        self.startup_due = None;
        self.set_confirmed(None);
        self.conflict = None;
        self.close_socket();
        self.socket_due = None;
        self.app_due = None;
        self.state.send_modify(|s| s.conflict_pending = false);
        self.status("Sync connect cancelled.");
    }
    fn reset_cursor(&mut self, target: &SyncTarget) {
        let key = target.key();
        if self.version_target.as_ref() != Some(&key) {
            self.version_target = Some(key);
            self.cursor = 0;
        }
    }
    async fn pull_now(&mut self, target: &SyncTarget, confirm: bool, startup: bool) -> Result<()> {
        self.reset_cursor(target);
        let before = self.deps.engine.queue().borrow().clone();
        let revision = self.revisions.load(Ordering::SeqCst);
        let snapshot = self
            .client
            .snapshot(target, None)
            .await?
            .ok_or(SyncError::Unavailable)?
            .data;
        if startup {
            self.set_confirmed(Some(target.clone()));
            self.pending = None;
            self.startup_due = None;
            self.socket_due = Some(Instant::now());
        }
        let after = self.deps.engine.queue().borrow().clone();
        let unchanged = revision == self.revisions.load(Ordering::SeqCst)
            && before.content_version == after.content_version
            && before.current().map(|e| e.uid) == after.current().map(|e| e.uid)
            && before.shuffle == after.shuffle;
        let applied = self
            .apply_snapshot(
                &snapshot,
                target,
                Apply {
                    confirm,
                    startup,
                    merge: startup,
                    playback: !startup || unchanged,
                    ..Apply::default()
                },
            )
            .await?;
        if applied {
            let d = self.selected();
            self.status(
                if d.app_state || d.playlists || d.settings || d.provider_configuration {
                    "Sync pull complete."
                } else {
                    "Sync completed with no selected domains."
                },
            );
        }
        Ok(())
    }
    async fn base(
        &self,
        domain: SyncDomain,
        target: &SyncTarget,
    ) -> Result<Option<SyncSignatures>> {
        let key = target.key();
        Ok(self
            .deps
            .db
            .call(move |c| sync_state::base(c, domain, &key))
            .await?)
    }
    async fn record(
        &mut self,
        domain: SyncDomain,
        target: &SyncTarget,
        remote: String,
        local: String,
    ) -> Result<()> {
        let key = target.key();
        self.deps
            .db
            .call(move |c| sync_state::set_base(c, domain, &key, &remote, &local))
            .await?;
        // Android DomainPush.record: an agreed base supersedes a pending pre-push reconcile.
        if let Some(push) = self.domains.iter_mut().find(|p| p.domain == domain) {
            push.reconcile = false;
        }
        if domain == SyncDomain::Playlists {
            self.playlist_target = None;
            self.state.send_modify(|s| {
                s.playlist_confirmation_required = false;
                s.playlist_sync_paused = false;
            });
        }
        Ok(())
    }
    fn import_status(&self, message: String) {
        self.state.send_modify(|s| s.import_status = Some(message));
    }
    /// Stored Jellyfin/Plex base URLs, read through the same connection store the importer writes.
    async fn provider_urls(&self) -> Result<[Option<String>; 2]> {
        let config = self.deps.transfer.export_provider_config_json().await?;
        Ok(["jellyfin", "plex"]
            .map(|source| text(field(field(&config, source), "base_url")).map(str::to_owned)))
    }
    /// Android `SyncImports.apply` after the import transaction committed: write imported URLs,
    /// re-validate only providers whose stored URL actually changed, refresh their playlists,
    /// then publish the import summary.
    async fn finish_import(&mut self, mut summary: ImportSummary) -> Result<()> {
        let before = self.provider_urls().await?;
        self.deps
            .transfer
            .apply_connection_updates(std::mem::take(&mut summary.connection_updates))
            .await?;
        self.deps.playlists.reload().await?;
        let after = self.provider_urls().await?;
        let mut changed = false;
        for (source, (before, after)) in [SourceType::Jellyfin, SourceType::Plex]
            .into_iter()
            .zip(before.into_iter().zip(after))
        {
            if after.is_none() || before == after {
                continue;
            }
            changed = true;
            if let Err(message) = self.deps.providers.revalidate(source).await {
                summary
                    .warnings
                    .push(format!("Provider refresh failed: {source}: {message}"));
            }
        }
        if changed {
            match self.deps.catalog.refresh_playlists().await {
                Ok(failures) => summary.warnings.extend(
                    failures
                        .into_iter()
                        .map(|f| format!("Provider refresh failed: {}: {}", f.source, f.message)),
                ),
                Err(error) => summary
                    .warnings
                    .push(format!("Provider refresh failed: {error}")),
            }
        }
        self.import_status(summary.format("Sync import"));
        Ok(())
    }
    async fn apply_snapshot(
        &mut self,
        snapshot: &JsonElement,
        target: &SyncTarget,
        apply: Apply,
    ) -> Result<bool> {
        self.reset_cursor(target);
        let version = field(snapshot, "version").as_long_lenient();
        if let Some(version) = version {
            let key = target.key();
            let recorded = self
                .deps
                .db
                .call(move |c| sync_state::version(c, &key))
                .await?;
            if recorded.is_some_and(|v| version < v) {
                let key = target.key();
                self.deps
                    .db
                    .call(move |c| sync_state::clear_target(c, &key))
                    .await?;
                self.cursor = 0;
            }
        }
        let selected = apply.domains.unwrap_or_else(|| self.selected());
        let empty_object = object([]);
        let empty_array = JsonElement::Array(vec![]);
        if selected.settings {
            let remote = if field(snapshot, "settings").as_object().is_some() {
                field(snapshot, "settings")
            } else {
                &empty_object
            };
            let local_settings = self.deps.settings.get().player;
            let local = signatures::settings_json(
                local_settings.normalization_enabled,
                local_settings.normalization_strict,
            );
            let rs = signatures::settings(remote);
            let ls = signatures::settings(&local);
            let base = self.base(SyncDomain::Settings, target).await?;
            let lc = base.as_ref().is_some_and(|b| b.local != ls);
            let rc = base.as_ref().is_some_and(|b| b.remote != rs);
            if apply.merge && lc && rc && ls != rs {
                self.conflict = Some(target.clone());
                self.state.send_modify(|s| s.conflict_pending = true);
                self.status(CONFLICT);
            } else if apply.merge && lc && !rc {
                self.push_domain(1, target, Some(local)).await?;
            } else {
                let enabled = boolean(field(remote, "audio_normalization_enabled"))
                    .or_else(|| boolean(field(remote, "audioNormalizationEnabled")))
                    .unwrap_or(local_settings.normalization_enabled);
                let strict = boolean(field(remote, "audio_normalization_strict_mode"))
                    .or_else(|| boolean(field(remote, "audioNormalizationStrictMode")))
                    .unwrap_or(local_settings.normalization_strict);
                self.deps
                    .settings
                    .update(|s| {
                        s.player.normalization_enabled = enabled;
                        s.player.normalization_strict = strict;
                    })
                    .await?;
                self.suppress_until = Instant::now() + ECHO;
                self.deps.engine.set_normalization(enabled, strict);
                self.record(
                    SyncDomain::Settings,
                    target,
                    rs,
                    signatures::settings(&signatures::settings_json(enabled, strict)),
                )
                .await?;
            }
        }
        if selected.app_state
            && apply.playback
            && field(snapshot, "app_state").as_object().is_some()
        {
            let remote = field(snapshot, "app_state");
            let base = self.base(SyncDomain::AppState, target).await?;
            let ls = self.local_signature();
            let rs = remote_signature(remote);
            let local_newer = base
                .as_ref()
                .is_some_and(|b| b.remote == rs && b.local != ls);
            if apply.merge
                && ((apply.startup && self.deps.engine.player().borrow().play_requested)
                    || local_newer)
            {
                if base.as_ref().is_none_or(|b| b.local != ls) {
                    self.push_app(target).await?;
                }
            } else if self.apply_app(remote, apply.startup).await {
                self.record(SyncDomain::AppState, target, rs, self.local_signature())
                    .await?;
            }
        }
        let mut applied = true;
        if selected.playlists || selected.provider_configuration {
            let remote = field(snapshot, "playlists").as_array().map_or_else(
                || empty_array.clone(),
                |_| field(snapshot, "playlists").clone(),
            );
            let providers = field(snapshot, "provider_configuration")
                .as_object()
                .map_or_else(
                    || empty_object.clone(),
                    |_| field(snapshot, "provider_configuration").clone(),
                );
            let key = target.key();
            let awaiting = self.state.borrow().playlist_sync_paused;
            // Local export, decision, REPLACE_ALL and resulting base must see one writer transaction.
            let outcome = self
                .deps
                .db
                .call(move |conn| {
                    ap_store::in_tx(conn, |conn| {
                        use ap_library::transfer::{export_playlists_json_conn, import_json_conn};
                        let library_error = |error: ap_library::LibraryError| {
                            ap_store::StoreError::Invalid(error.to_string())
                        };
                        let local = export_playlists_json_conn(conn).map_err(library_error)?;
                        let local_sig = signatures::playlists(Some(&local));
                        let remote_sig = signatures::playlists(Some(&remote));
                        let base = sync_state::base(conn, SyncDomain::Playlists, &key)?;
                        let decision = if !selected.playlists {
                            PlaylistMerge::Unchanged
                        } else if apply.confirm {
                            PlaylistMerge::ApplyRemote
                        } else {
                            playlist_merge(
                                !local.as_array().unwrap_or_default().is_empty(),
                                remote.as_array().unwrap_or_default().is_empty(),
                                &local_sig,
                                &remote_sig,
                                base.as_ref(),
                                apply.realtime,
                                awaiting,
                            )
                        };
                        let replace = selected.playlists && decision == PlaylistMerge::ApplyRemote;
                        let mut summary = None;
                        let mut recorded = false;
                        if decision != PlaylistMerge::Confirm {
                            if replace || selected.provider_configuration {
                                let file = object([
                                    ("export_version", JsonElement::Number("1".into())),
                                    (
                                        "provider_configs",
                                        if selected.provider_configuration {
                                            providers
                                        } else {
                                            object([])
                                        },
                                    ),
                                    (
                                        "custom_playlists",
                                        if replace {
                                            remote
                                        } else {
                                            JsonElement::Array(vec![])
                                        },
                                    ),
                                ]);
                                summary = Some(
                                    import_json_conn(
                                        conn,
                                        &file,
                                        if replace {
                                            MergePolicy::ReplaceAll
                                        } else {
                                            MergePolicy::MergeKeepLocal
                                        },
                                        false,
                                    )
                                    .map_err(library_error)?,
                                );
                            }
                            if replace
                                || (selected.playlists
                                    && decision == PlaylistMerge::Unchanged
                                    && base.is_none())
                            {
                                let imported =
                                    export_playlists_json_conn(conn).map_err(library_error)?;
                                sync_state::set_base(
                                    conn,
                                    SyncDomain::Playlists,
                                    &key,
                                    &remote_sig,
                                    &signatures::playlists(Some(&imported)),
                                )?;
                                recorded = true;
                            }
                        }
                        Ok((decision, summary, recorded))
                    })
                })
                .await;
            // Android SyncImports: an import failure is published on the import status line, leaves
            // the pull unapplied, and still records the snapshot version below.
            let (decision, summary, recorded) = outcome.unwrap_or_else(|error| {
                self.import_status(format!("Sync import failed: {error}"));
                applied = false;
                (PlaylistMerge::Unchanged, None, false)
            });
            if decision == PlaylistMerge::Confirm {
                self.playlist_target = Some(target.clone());
                self.state.send_modify(|s| {
                    s.playlist_confirmation_required = true;
                    s.playlist_sync_paused = true;
                });
                self.status(if apply.realtime {
                    PAUSED
                } else {
                    "Sync cancelled: playlist overwrite requires confirmation."
                });
                applied = apply.realtime;
            }
            if let Some(summary) = summary {
                match self.finish_import(summary).await {
                    Ok(()) if selected.provider_configuration => {
                        let imported = self.deps.transfer.export_provider_config_json().await?;
                        self.record(
                            SyncDomain::ProviderConfiguration,
                            target,
                            signatures::settings(field(snapshot, "provider_configuration")),
                            signatures::settings(&imported),
                        )
                        .await?;
                    }
                    Ok(()) => {}
                    Err(error) => {
                        self.import_status(format!("Sync import failed: {error}"));
                        applied = false;
                    }
                }
            }
            if recorded {
                self.playlist_target = None;
                self.state.send_modify(|s| {
                    s.playlist_confirmation_required = false;
                    s.playlist_sync_paused = false;
                });
            }
            if decision == PlaylistMerge::PushLocal {
                self.push_domain(0, target, None).await?;
            }
        }
        if let Some(version) = version {
            self.cursor = self.cursor.max(version);
            let key = target.key();
            self.deps
                .db
                .call(move |c| sync_state::record_version(c, &key, version))
                .await?;
        }
        Ok(applied)
    }
    fn local_signature(&self) -> String {
        let q = self.deps.engine.queue().borrow().clone();
        signatures::app_state(
            q.current().map(|e| &e.track),
            &q.entries
                .iter()
                .map(|e| e.track.clone())
                .collect::<Vec<_>>(),
            Some(q.shuffle),
            repeat_name(q.repeat),
        )
    }
    async fn apply_app(&mut self, data: &JsonElement, preserve_empty: bool) -> bool {
        let current = decode_track(field(data, "current_track"));
        let mut tracks = decode_queue(data);
        if preserve_empty && current.is_none() && tracks.is_empty() {
            return false;
        }
        if let Some(current) = &current {
            if !tracks.iter().any(|t| t.same_track(current)) {
                tracks.insert(0, current.clone());
            }
        }
        self.suppress_until = Instant::now() + ECHO;
        let engine = &self.deps.engine;
        if let Some(volume) = field(data, "volume")
            .as_long_lenient()
            .and_then(|v| i32::try_from(v).ok())
        {
            engine.set_volume(volume, ChangeOrigin::Sync);
        }
        // Android re-shuffles only when the remote flag differs from the local one.
        if let Some(shuffle) =
            boolean(field(data, "shuffle")).filter(|s| *s != engine.queue().borrow().shuffle)
        {
            engine.set_shuffle(shuffle, ChangeOrigin::Sync);
        }
        engine.set_repeat(remote_repeat(data), ChangeOrigin::Sync);
        let local = engine.queue().borrow().clone();
        if let Some(current) = &current {
            if tracks
                != local
                    .entries
                    .iter()
                    .map(|e| e.track.clone())
                    .collect::<Vec<_>>()
                || local.current().map(|e| &e.track) != Some(current)
            {
                let start = tracks
                    .iter()
                    .position(|t| t.same_track(current))
                    .and_then(|i| isize::try_from(i).ok())
                    .unwrap_or(0);
                engine.set_queue(tracks, start, ChangeOrigin::Sync, false);
            }
        } else if tracks.is_empty() && !local.entries.is_empty() {
            engine.set_queue(vec![], 0, ChangeOrigin::Sync, false);
        }
        if let Some(position) = field(data, "position").as_long_lenient() {
            if engine.position_ms().abs_diff(position) > 2500 {
                engine.seek(position, ChangeOrigin::Sync);
            }
        }
        match text(field(data, "state")) {
            Some("playing") => engine.play(ChangeOrigin::Sync),
            Some("paused" | "stopped") => engine.pause(ChangeOrigin::Sync),
            _ => {}
        }
        // Flush is an actor barrier, ensuring bases describe the applied state rather than queued commands.
        engine.flush().await;
        self.suppress_until = Instant::now() + ECHO;
        true
    }
    /// `Ok(false)` is a failed upload (already throttled); `Err` only for local failures.
    async fn push_app(&mut self, target: &SyncTarget) -> Result<bool> {
        if !self.can_app() {
            return Ok(true);
        }
        if let Some(last) = self.app_last {
            tokio::time::sleep_until(last + MIN_PUSH).await;
        }
        if !self.can_app() {
            return Ok(true);
        }
        let q = self.deps.engine.queue().borrow().clone();
        let p = self.deps.engine.player().borrow().clone();
        let mut payload = object([
            (
                "state",
                JsonElement::String(
                    match p.status {
                        PlaybackStateType::Playing => "playing",
                        PlaybackStateType::Paused => "paused",
                        _ => "stopped",
                    }
                    .into(),
                ),
            ),
            ("shuffle", JsonElement::Bool(q.shuffle)),
            (
                "repeat_mode",
                JsonElement::String(repeat_name(q.repeat).into()),
            ),
            ("volume", JsonElement::Number(p.volume.to_string())),
            (
                "position",
                JsonElement::Number(self.deps.engine.position_ms().to_string()),
            ),
            ("duration", JsonElement::Number(p.duration_ms.to_string())),
        ]);
        if let JsonElement::Object(map) = &mut payload {
            if let Some(current) = q.current() {
                map.insert(
                    "current_track".into(),
                    JsonElement::parse(&serde_json::to_string(&current.track)?)?,
                );
            }
            if !q.entries.is_empty() {
                map.insert(
                    "queue".into(),
                    JsonElement::parse(&serde_json::to_string(
                        &q.entries.iter().map(|e| &e.track).collect::<Vec<_>>(),
                    )?)?,
                );
            }
        }
        let signature = remote_signature(&payload);
        let result = self
            .client
            .push(
                target,
                "app-state",
                &self.deps.settings.get().sync.client_id,
                payload,
            )
            .await;
        // Even failed attempts are throttled: never burst on a failing server.
        self.app_last = Some(Instant::now());
        if result.is_err() {
            return Ok(false);
        }
        self.record(SyncDomain::AppState, target, signature.clone(), signature)
            .await?;
        Ok(true)
    }
    async fn export(&self, domain: SyncDomain) -> Result<JsonElement> {
        match domain {
            SyncDomain::Playlists => Ok(self.deps.transfer.export_playlists_json().await?),
            SyncDomain::Settings => {
                let s = self.deps.settings.get().player;
                Ok(signatures::settings_json(
                    s.normalization_enabled,
                    s.normalization_strict,
                ))
            }
            SyncDomain::ProviderConfiguration => {
                Ok(self.deps.transfer.export_provider_config_json().await?)
            }
            SyncDomain::AppState => unreachable!("app state has a separate push path"),
        }
    }
    /// `Ok(false)` is a failed upload, scheduled for reconcile + retry; `Err` only for local failures.
    async fn push_domain(
        &mut self,
        index: usize,
        target: &SyncTarget,
        payload: Option<JsonElement>,
    ) -> Result<bool> {
        if let Some(last) = self.domains[index].last {
            tokio::time::sleep_until(last + MIN_PUSH).await;
        }
        let domain = self.domains[index].domain;
        let payload = match payload {
            Some(payload) => payload,
            None => self.export(domain).await?,
        };
        let signature = domain_signature(domain, &payload);
        self.domains[index].pending_target = Some(target.key());
        let namespace = match domain {
            SyncDomain::Playlists => "playlists",
            SyncDomain::Settings => "settings",
            SyncDomain::ProviderConfiguration => "provider-configuration",
            SyncDomain::AppState => unreachable!("app state has a separate push path"),
        };
        let result = self
            .client
            .push(
                target,
                namespace,
                &self.deps.settings.get().sync.client_id,
                payload,
            )
            .await;
        self.domains[index].last = Some(Instant::now());
        if result.is_ok() {
            self.record(domain, target, signature.clone(), signature)
                .await?;
            let push = &mut self.domains[index];
            push.retry = MIN_PUSH;
            push.reconcile = false;
            push.pending_target = None;
            push.due = None;
        } else {
            let push = &mut self.domains[index];
            // Provider configuration has no merge; its retry simply re-pushes the latest export.
            push.reconcile = domain != SyncDomain::ProviderConfiguration;
            push.due = Some(Instant::now() + push.retry);
            push.retry = (push.retry * 2).min(Duration::from_secs(60));
        }
        Ok(result.is_ok())
    }
    async fn push_local(&mut self, target: &SyncTarget) -> bool {
        let selected = self.selected();
        let mut success = true;
        if selected.app_state {
            success &= matches!(self.push_app(target).await, Ok(true));
        }
        for (index, enabled) in [
            selected.playlists,
            selected.settings,
            selected.provider_configuration,
        ]
        .into_iter()
        .enumerate()
        {
            if enabled {
                success &= matches!(self.push_domain(index, target, None).await, Ok(true));
            }
        }
        success
    }
    async fn timers(&mut self) -> Result<()> {
        let now = Instant::now();
        let online = *self.deps.network.borrow();
        if online && self.startup_due.is_some_and(|due| due <= now) {
            if let Some(target) = self.pending.clone() {
                self.startup_due = Some(now + jitter(self.startup_backoff));
                self.startup_backoff = (self.startup_backoff * 2).min(Duration::from_secs(60));
                self.pull_now(&target, false, true).await?;
            }
        }
        let Some(target) = self.confirmed.clone() else {
            return Ok(());
        };
        if online && self.socket.is_none() && self.socket_due.is_some_and(|due| due <= now) {
            self.socket_due = None;
            match self.client.updates(&target).await {
                Ok(socket) => {
                    self.socket = Some(socket);
                    self.opened_at = Some(Instant::now());
                    let result = if self.state.borrow().conflict_pending {
                        Ok(true)
                    } else {
                        async {
                            let snapshot = self
                                .client
                                .snapshot(&target, None)
                                .await?
                                .ok_or(SyncError::Unavailable)?
                                .data;
                            self.apply_snapshot(
                                &snapshot,
                                &target,
                                Apply {
                                    merge: true,
                                    ..Apply::default()
                                },
                            )
                            .await
                        }
                        .await
                    };
                    // Android: a failed open snapshot only ends this socket; reconnect backs off silently.
                    if result.is_err() {
                        self.socket_failed();
                    } else {
                        self.state.send_modify(|s| s.realtime_connected = true);
                        for push in &mut self.domains {
                            push.request();
                        }
                    }
                }
                Err(_) => self.socket_failed(),
            }
        }
        if self.app_due.is_some_and(|due| due <= now) {
            self.app_due = None;
            if self.can_app() && self.selected().app_state {
                self.push_app(&target).await?;
            }
        }
        if !online || !self.can_push() {
            return Ok(());
        }
        for index in 0..self.domains.len() {
            if self.domains[index].due.is_none_or(|due| due > now) {
                continue;
            }
            self.domains[index].due = None;
            let domain = self.domains[index].domain;
            let selected = self.selected();
            let enabled = match domain {
                SyncDomain::Playlists => {
                    selected.playlists && !self.state.borrow().playlist_sync_paused
                }
                SyncDomain::Settings => selected.settings,
                SyncDomain::ProviderConfiguration => selected.provider_configuration,
                SyncDomain::AppState => false,
            };
            if !enabled {
                continue;
            }
            let base = self.base(domain, &target).await?;
            if domain == SyncDomain::Playlists
                && base.is_none()
                && self.domains[index].pending_target.as_deref() != Some(&target.key())
            {
                continue;
            }
            if self.domains[index].reconcile {
                let snapshot = if let Ok(Some(response)) = self.client.snapshot(&target, None).await
                {
                    response.data
                } else {
                    let push = &mut self.domains[index];
                    push.due = Some(Instant::now() + push.retry);
                    push.retry = (push.retry * 2).min(Duration::from_secs(60));
                    continue;
                };
                self.domains[index].reconcile = false;
                let domains = SyncDomains {
                    app_state: false,
                    playlists: domain == SyncDomain::Playlists,
                    settings: domain == SyncDomain::Settings,
                    provider_configuration: domain == SyncDomain::ProviderConfiguration,
                };
                self.apply_snapshot(
                    &snapshot,
                    &target,
                    Apply {
                        merge: true,
                        playback: false,
                        domains: Some(domains),
                        ..Apply::default()
                    },
                )
                .await?;
                // Android: the merge itself pushes when only the local side changed since the base.
            } else {
                let data = self.export(domain).await?;
                if base
                    .as_ref()
                    .is_none_or(|b| b.local != domain_signature(domain, &data))
                {
                    self.push_domain(index, &target, Some(data)).await?;
                }
            }
        }
        Ok(())
    }
    fn close_socket(&mut self) {
        self.socket = None;
        self.state.send_modify(|s| s.realtime_connected = false);
    }
    fn socket_failed(&mut self) {
        self.close_socket();
        if self
            .opened_at
            .take()
            .is_some_and(|at| at.elapsed() >= Duration::from_secs(30))
        {
            self.socket_backoff = Duration::from_secs(1);
        }
        self.socket_due = Some(Instant::now() + jitter(self.socket_backoff));
        self.socket_backoff = (self.socket_backoff * 2).min(Duration::from_secs(60));
    }
    async fn receive(&mut self, event: SyncUpdate) -> Result<()> {
        if !self.can_push() {
            return Ok(());
        }
        let Some(target) = self.confirmed.clone() else {
            return Ok(());
        };
        if event.version.is_some_and(|v| v < self.cursor) {
            // Android SyncClient swallows transport failures: a missed event waits for the next one.
            if let Ok(Some(response)) = self.client.snapshot(&target, None).await {
                self.apply_snapshot(
                    &response.data,
                    &target,
                    Apply {
                        merge: true,
                        ..Apply::default()
                    },
                )
                .await?;
            }
            return Ok(());
        }
        if event.source_client_id.as_deref() == Some(&self.deps.settings.get().sync.client_id)
            || event.version.is_some_and(|v| v <= self.cursor)
        {
            return Ok(());
        }
        let selected = self.selected();
        let domains = SyncDomains {
            app_state: event.namespace == "app_state" && selected.app_state,
            playlists: event.namespace == "playlists" && selected.playlists,
            settings: event.namespace == "settings" && selected.settings,
            provider_configuration: event.namespace == "provider_configuration"
                && selected.provider_configuration,
        };
        if !(domains.app_state
            || domains.playlists
            || domains.settings
            || domains.provider_configuration)
        {
            return Ok(());
        }
        let snapshot = if domains.app_state {
            match self.client.app_state(&target).await {
                Ok(Some(response)) => Some(response),
                _ => self
                    .client
                    .snapshot(&target, Some(self.cursor))
                    .await
                    .ok()
                    .flatten(),
            }
        } else {
            self.client
                .snapshot(&target, Some(self.cursor))
                .await
                .ok()
                .flatten()
        };
        if let Some(snapshot) = snapshot {
            self.apply_snapshot(
                &snapshot.data,
                &target,
                Apply {
                    merge: domains.app_state,
                    realtime: !domains.app_state,
                    domains: Some(domains),
                    ..Apply::default()
                },
            )
            .await?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
struct Apply {
    confirm: bool,
    startup: bool,
    merge: bool,
    playback: bool,
    realtime: bool,
    domains: Option<SyncDomains>,
}
impl Default for Apply {
    fn default() -> Self {
        Self {
            confirm: false,
            startup: false,
            merge: false,
            playback: true,
            realtime: false,
            domains: None,
        }
    }
}
async fn socket_update(socket: &mut Option<SyncSocket>) -> Result<Option<SyncUpdate>> {
    match socket {
        Some(socket) => crate::client::next_update(socket).await,
        None => std::future::pending().await,
    }
}
fn jitter(base: Duration) -> Duration {
    let millis = u64::try_from(base.as_millis()).unwrap_or(u64::MAX);
    Duration::from_millis(rand::random_range((millis * 3 / 4).max(1)..=millis))
}
fn decode_track(value: &JsonElement) -> Option<Track> {
    serde_json::from_str(&value.to_string()).ok()
}
fn decode_queue(value: &JsonElement) -> Vec<Track> {
    field(value, "queue")
        .as_array()
        .unwrap_or_default()
        .iter()
        .filter_map(decode_track)
        .collect()
}
fn remote_repeat(data: &JsonElement) -> RepeatMode {
    match text(field(data, "repeat_mode")) {
        Some("one") => RepeatMode::One,
        Some("all") => RepeatMode::All,
        _ => RepeatMode::Off,
    }
}
fn repeat_name(repeat: RepeatMode) -> &'static str {
    match repeat {
        RepeatMode::Off => "off",
        RepeatMode::One => "one",
        RepeatMode::All => "all",
    }
}
fn remote_signature(data: &JsonElement) -> String {
    signatures::app_state(
        decode_track(field(data, "current_track")).as_ref(),
        &decode_queue(data),
        boolean(field(data, "shuffle")),
        repeat_name(remote_repeat(data)),
    )
}
fn domain_signature(domain: SyncDomain, data: &JsonElement) -> String {
    if domain == SyncDomain::Playlists {
        signatures::playlists(Some(data))
    } else {
        signatures::settings(data)
    }
}
fn has_server_data(snapshot: &JsonElement, selected: SyncDomains) -> bool {
    (selected.app_state
        && (decode_track(field(field(snapshot, "app_state"), "current_track")).is_some()
            || !decode_queue(field(snapshot, "app_state")).is_empty()))
        || (selected.playlists
            && field(snapshot, "playlists")
                .as_array()
                .is_some_and(|a| !a.is_empty()))
        || (selected.settings
            && field(snapshot, "settings")
                .as_object()
                .is_some_and(|o| !o.is_empty()))
        || (selected.provider_configuration
            && field(snapshot, "provider_configuration")
                .as_object()
                .is_some_and(|o| !o.is_empty()))
}

#[cfg(test)]
mod tests;
