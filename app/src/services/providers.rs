//! Authenticated provider sessions, secret persistence and playback resolution.
use super::error;
use crate::vm::{
    Device, ProviderForm, ProvidersCommand, ProvidersPort, ProvidersSnapshot, Result, ServiceError,
};
use ap_model::{ProviderConnectionProfile, SourceType, Track};
use ap_providers::{
    ProviderClient,
    spotify::{SpotifyAccounts, SpotifyToken, SpotifyTokenProvider, TokenSink},
};
use ap_store::{
    Db,
    secrets::{SecretAccount, Secrets},
    settings::SettingsStore,
};
use async_trait::async_trait;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::{Mutex, watch};

pub struct Connections {
    pub secrets: Arc<Secrets>,
}
fn accounts(source: SourceType) -> Option<(SecretAccount, SecretAccount)> {
    match source {
        SourceType::Jellyfin => Some((SecretAccount::JellyfinUrl, SecretAccount::JellyfinApiKey)),
        SourceType::Plex => Some((SecretAccount::PlexUrl, SecretAccount::PlexToken)),
        _ => None,
    }
}
#[async_trait]
impl ap_library::transfer::ConnectionStore for Connections {
    async fn snapshot(
        &self,
    ) -> ap_library::Result<indexmap::IndexMap<SourceType, ap_library::transfer::StoredConnection>>
    {
        let s = self.secrets.subscribe().borrow().clone();
        let mut out = indexmap::IndexMap::new();
        for source in [SourceType::Jellyfin, SourceType::Plex] {
            let (url, token) = accounts(source).expect("server provider");
            if let Some(url) = s.get(url) {
                let mut c = ap_library::transfer::StoredConnection::new(source);
                c.server_url = Some(url.into());
                c.token = s.get(token).map(str::to_owned);
                out.insert(source, c);
            }
        }
        Ok(out)
    }
    async fn put(&self, c: ap_library::transfer::StoredConnection) -> ap_library::Result<()> {
        if let Some((url, token)) = accounts(c.source) {
            if let Some(v) = c.server_url {
                self.secrets
                    .set(url, Some(v))
                    .await
                    .map_err(|e| ap_library::LibraryError::Invalid(e.to_string()))?;
            }
            if let Some(v) = c.token {
                self.secrets
                    .set(token, Some(v))
                    .await
                    .map_err(|e| ap_library::LibraryError::Invalid(e.to_string()))?;
            }
        }
        Ok(())
    }
}
struct SecretTokenSink {
    secrets: Arc<Secrets>,
}
#[async_trait]
impl TokenSink for SecretTokenSink {
    async fn token_updated(&self, token: &SpotifyToken) {
        if let Err(e) = self
            .secrets
            .set(SecretAccount::SpotifyToken, Some(token.to_json()))
            .await
        {
            tracing::warn!(error=%e,"Spotify token keyring save failed");
        }
    }
    async fn refresh_rejected(&self) {
        if let Err(e) = self.secrets.set(SecretAccount::SpotifyToken, None).await {
            tracing::warn!(error=%e,"Spotify token removal failed");
        }
    }
}
struct State {
    clients: HashMap<SourceType, Arc<dyn ProviderClient>>,
    profiles: HashMap<SourceType, ProviderConnectionProfile>,
    self_signed: HashMap<SourceType, bool>,
}
pub struct ProvidersAdapter {
    pub tokens: Arc<SpotifyTokenProvider>,
    pub devices: Arc<ap_playback::spotify::SpotifyDeviceResolver>,
    state: RwLock<State>,
    changed: watch::Sender<()>,
    ui: watch::Sender<ProvidersSnapshot>,
    secrets: Arc<Secrets>,
    settings: Arc<SettingsStore>,
    db: Db,
    catalog: ap_library::CatalogRepository,
    login: Mutex<Option<tokio_util::sync::CancellationToken>>,
}
impl ProvidersAdapter {
    pub fn new(
        secrets: Arc<Secrets>,
        settings: Arc<SettingsStore>,
        db: Db,
        catalog: ap_library::CatalogRepository,
    ) -> Arc<Self> {
        let tokens = Arc::new(SpotifyTokenProvider::new(
            SpotifyAccounts::new(reqwest::Client::new()),
            None,
            Arc::new(SecretTokenSink {
                secrets: secrets.clone(),
            }),
        ));
        let api = Arc::new(super::spotify::SpotifyApi::new(tokens.clone()));
        let devices = Arc::new(ap_playback::spotify::SpotifyDeviceResolver::new(
            api,
            Arc::new(ap_platform::spotify_launcher::DesktopSpotifyLauncher),
        ));
        let profiles = [SourceType::Spotify, SourceType::Jellyfin, SourceType::Plex]
            .into_iter()
            .map(|source| {
                (
                    source,
                    ProviderConnectionProfile {
                        source,
                        connected: false,
                        server_url: None,
                        username: None,
                        has_token: false,
                        is_premium: None,
                        playback_ready: Some(false),
                        last_error: None,
                    },
                )
            })
            .collect();
        Arc::new(Self {
            tokens,
            devices,
            state: RwLock::new(State {
                clients: HashMap::new(),
                profiles,
                self_signed: HashMap::new(),
            }),
            changed: watch::Sender::new(()),
            ui: watch::Sender::new(ProvidersSnapshot::default()),
            secrets,
            settings,
            db,
            catalog,
            login: Mutex::new(None),
        })
    }
    pub async fn restore(self: &Arc<Self>) -> Vec<String> {
        let secrets = tokio::time::timeout(Duration::from_secs(10), self.secrets.load()).await;
        let mut warnings = match secrets {
            Ok(s) => s.errors.values().cloned().collect::<Vec<_>>(),
            Err(_) => vec!["Keyring restore timed out".into()],
        };
        let (a, b, c) = tokio::join!(
            self.restore_step(SourceType::Spotify),
            self.restore_step(SourceType::Jellyfin),
            self.restore_step(SourceType::Plex)
        );
        warnings.extend(
            [a, b, c]
                .into_iter()
                .filter_map(std::result::Result::err)
                .map(|e| e.to_string()),
        );
        self.publish().await;
        warnings
    }
    pub async fn restore_step(&self, source: SourceType) -> Result {
        self.revalidate(source)
            .await
            .map_err(|e| ServiceError(format!("{}: {e}", source.name())))
    }
    /// Rebuilds one provider session from stored credentials (10 s bound). Errors are unprefixed
    /// and also recorded on the provider profile.
    async fn revalidate(&self, source: SourceType) -> Result {
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            let s = self.secrets.snapshot();
            if source == SourceType::Spotify {
                if let Some(raw) = s.get(SecretAccount::SpotifyToken) {
                    self.tokens
                        .install(SpotifyToken::from_json(raw).map_err(error)?)
                        .await;
                    self.validate_spotify().await?;
                }
            } else if let Some((url, token)) = accounts(source)
                && let (Some(url), Some(secret)) = (s.get(url), s.get(token))
            {
                let prefs = self.settings.get();
                let page_size = if source == SourceType::Jellyfin {
                    prefs.providers.jellyfin_page_size
                } else {
                    prefs.providers.plex_page_size
                };
                let key = format!("providers.{}.self_signed", source.name().to_lowercase());
                let self_signed = self
                    .db
                    .read(move |c| ap_store::settings::get::<bool>(c, &key))
                    .await
                    .map_err(error)?
                    .unwrap_or(false);
                self.connect_server(
                    ProviderForm {
                        source,
                        url: url.into(),
                        secret: secret.into(),
                        page_size: u16::try_from(page_size).unwrap_or(300),
                        self_signed,
                    },
                    false,
                )
                .await?;
            }
            Ok::<_, ServiceError>(())
        })
        .await
        .unwrap_or_else(|_| Err(ServiceError("restore timed out after 10 s".into())));
        if let Err(e) = &result {
            if let Some(profile) = self
                .state
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .profiles
                .get_mut(&source)
            {
                profile.last_error = Some(e.to_string());
            }
            self.publish().await;
        }
        result
    }
    pub async fn retry_keyring(&self) -> Result {
        let state = tokio::time::timeout(Duration::from_secs(10), self.secrets.load())
            .await
            .map_err(|_| ServiceError("Keyring restore timed out".into()))?;
        if state.errors.is_empty() {
            Ok(())
        } else {
            Err(ServiceError(
                state
                    .errors
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; "),
            ))
        }
    }
    pub fn image_request(&self, url: &str) -> Option<ap_providers::provider::MediaRequest> {
        let state = self.state.read().ok()?;
        state
            .clients
            .values()
            .filter_map(|client| client.image_request(url))
            .find(|request| !request.headers.is_empty())
    }
    async fn publish(&self) {
        let profiles = {
            let state = self
                .state
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            [SourceType::Spotify, SourceType::Jellyfin, SourceType::Plex]
                .iter()
                .filter_map(|s| state.profiles.get(s).cloned())
                .collect()
        };
        let clients = self
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clients
            .values()
            .cloned()
            .collect();
        self.catalog.set_clients(clients).await;
        self.ui.send_modify(|s| {
            s.profiles = profiles;
            s.preferred = self
                .settings
                .get()
                .spotify
                .preferred_device
                .map_or_else(String::new, |d| d.id);
        });
        self.changed.send_replace(());
    }
    async fn validate_spotify(&self) -> Result {
        let client = Arc::new(ap_providers::spotify::SpotifyClient::new(
            reqwest::Client::new(),
            self.tokens.clone(),
        ));
        let validation = client.validate().await.map_err(error)?;
        {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.clients.insert(SourceType::Spotify, client);
            if let Some(p) = state.profiles.get_mut(&SourceType::Spotify) {
                p.connected = true;
                p.has_token = true;
                p.username = validation.username;
                p.is_premium = validation.premium;
                p.playback_ready = validation.premium;
            }
        }
        if let Some(d) = self.settings.get().spotify.preferred_device {
            self.devices
                .set_preferred(Some(ap_playback::spotify::PreferredDevice {
                    id: d.id,
                    name: d.name,
                }))
                .await;
        }
        let devices = self.devices.available().await.map_err(error)?;
        self.ui.send_modify(|s| {
            s.devices = devices
                .into_iter()
                .map(|d| Device {
                    id: d.id,
                    name: d.name,
                })
                .collect();
        });
        self.publish().await;
        Ok(())
    }
    async fn connect_server(&self, form: ProviderForm, persist: bool) -> Result {
        let prefs = self.settings.get();
        let client: Arc<dyn ProviderClient> = match form.source {
            SourceType::Jellyfin => Arc::new(
                ap_providers::jellyfin::JellyfinClient::new(
                    reqwest::Client::builder()
                        .danger_accept_invalid_certs(form.self_signed)
                        .timeout(Duration::from_secs(10))
                        .build()
                        .map_err(error)?,
                    ap_providers::jellyfin::JellyfinConfig {
                        base_url: form.url.clone(),
                        api_key: form.secret.clone(),
                        user_id: prefs.providers.jellyfin_user_id,
                        page_size: u32::from(form.page_size),
                        device_id: prefs.sync.client_id,
                        app_version: env!("CARGO_PKG_VERSION").into(),
                    },
                )
                .map_err(error)?,
            ),
            SourceType::Plex => Arc::new(
                ap_providers::plex::PlexClient::new(&ap_providers::plex::PlexConfig {
                    base_url: form.url.clone(),
                    token: form.secret.clone(),
                    page_size: u32::from(form.page_size),
                    allow_self_signed: form.self_signed,
                    client_identifier: prefs.sync.client_id,
                })
                .map_err(error)?,
            ),
            _ => return Err(ServiceError("Unsupported server provider".into())),
        };
        let validation = client.validate().await.map_err(error)?;
        if persist {
            let (url, token) = accounts(form.source).expect("server provider");
            self.secrets
                .set_many(vec![
                    (url, Some(form.url.clone())),
                    (token, Some(form.secret)),
                ])
                .await
                .map_err(error)?;
        }
        self.settings
            .update(|s| {
                if form.source == SourceType::Jellyfin {
                    s.providers.jellyfin_page_size = u32::from(form.page_size);
                    s.providers.jellyfin_user_id = validation.jellyfin_user_id;
                } else {
                    s.providers.plex_page_size = u32::from(form.page_size);
                }
            })
            .await
            .map_err(error)?;
        let key = format!(
            "providers.{}.self_signed",
            form.source.name().to_lowercase()
        );
        self.db
            .call(move |c| ap_store::settings::put(c, &key, &form.self_signed))
            .await
            .map_err(error)?;
        {
            let mut state = self
                .state
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.clients.insert(form.source, client);
            state.self_signed.insert(form.source, form.self_signed);
            state.profiles.insert(
                form.source,
                ProviderConnectionProfile {
                    source: form.source,
                    connected: true,
                    server_url: Some(form.url),
                    username: validation.username,
                    has_token: true,
                    is_premium: None,
                    playback_ready: Some(true),
                    last_error: None,
                },
            );
        }
        self.publish().await;
        Ok(())
    }
}
#[async_trait]
impl ProvidersPort for ProvidersAdapter {
    fn subscribe(&self) -> watch::Receiver<ProvidersSnapshot> {
        self.ui.subscribe()
    }
    async fn providers(&self, command: ProvidersCommand) -> Result {
        match command {
            ProvidersCommand::Connect(form) if form.source == SourceType::Spotify => {
                let session = ap_providers::spotify::PkceSession::generate();
                let listener = ap_platform::oauth_loopback::SignInFlow::new(&session.state)
                    .bind()
                    .await
                    .map_err(error)?;
                let cancel = tokio_util::sync::CancellationToken::new();
                if let Some(old) = self.login.lock().await.replace(cancel.clone()) {
                    old.cancel();
                }
                let url = session.authorize_url();
                self.ui.send_modify(|s| s.login_url.clone_from(&url));
                if let Err(e) = open::that_detached(&url) {
                    tracing::warn!(error=%e,"Open Spotify login URL manually");
                }
                let result = listener.wait(cancel).await.map_err(error);
                self.ui.send_modify(|s| s.login_url.clear());
                let code = result?;
                let token = self
                    .tokens
                    .accounts()
                    .exchange(&code, &session.verifier)
                    .await
                    .map_err(error)?;
                self.tokens.install(token).await;
                self.validate_spotify().await?;
            }
            ProvidersCommand::Connect(form) => self.connect_server(form, true).await?,
            ProvidersCommand::Disconnect(source) => {
                if source == SourceType::Spotify {
                    self.tokens.clear().await;
                    self.secrets
                        .set(SecretAccount::SpotifyToken, None)
                        .await
                        .map_err(error)?;
                } else if let Some((url, token)) = accounts(source) {
                    self.secrets
                        .set_many(vec![(url, None), (token, None)])
                        .await
                        .map_err(error)?;
                }
                {
                    let mut s = self
                        .state
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    s.clients.remove(&source);
                    if let Some(p) = s.profiles.get_mut(&source) {
                        p.connected = false;
                        p.has_token = false;
                        p.playback_ready = Some(false);
                    }
                }
                self.publish().await;
            }
            ProvidersCommand::Preferred(id) => {
                let name = self
                    .ui
                    .borrow()
                    .devices
                    .iter()
                    .find(|d| d.id == id)
                    .map_or_else(String::new, |d| d.name.clone());
                let preferred = (!id.is_empty()).then_some(ap_store::settings::PreferredDevice {
                    id: id.clone(),
                    name: name.clone(),
                });
                self.settings
                    .update(|s| s.spotify.preferred_device = preferred)
                    .await
                    .map_err(error)?;
                self.devices
                    .set_preferred(
                        (!id.is_empty())
                            .then_some(ap_playback::spotify::PreferredDevice { id, name }),
                    )
                    .await;
                self.publish().await;
            }
            ProvidersCommand::CancelLogin => {
                if let Some(cancel) = self.login.lock().await.take() {
                    cancel.cancel();
                }
            }
        }
        Ok(())
    }
}
impl ap_playback::ports::ConnectionGate for ProvidersAdapter {
    fn disabled_name(&self, source: SourceType) -> Option<String> {
        if !matches!(
            source,
            SourceType::Spotify | SourceType::Jellyfin | SourceType::Plex
        ) {
            return None;
        }
        let state = self.state.read().ok()?;
        let profile = state.profiles.get(&source)?;
        (!profile.connected || profile.playback_ready == Some(false))
            .then(|| crate::presenter::source_label(source).to_owned())
    }
    fn subscribe(&self) -> watch::Receiver<()> {
        self.changed.subscribe()
    }
}
#[async_trait]
impl ap_sync::ProviderRevalidation for ProvidersAdapter {
    async fn revalidate(&self, source: SourceType) -> std::result::Result<(), String> {
        Self::revalidate(self, source).await.map_err(|e| e.0)
    }
}
#[async_trait]
impl ap_playback::ports::StreamResolver for ProvidersAdapter {
    async fn resolve(
        &self,
        track: &Track,
    ) -> std::result::Result<ap_playback::ports::StreamRequest, ap_playback::ports::BoxError> {
        let (client, allow_invalid_certs) = {
            let state = self
                .state
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                state.clients.get(&track.source).cloned(),
                state
                    .self_signed
                    .get(&track.source)
                    .copied()
                    .unwrap_or(false),
            )
        };
        if let Some(client) = client {
            let r = client.stream_request(track).await?;
            Ok(ap_playback::ports::StreamRequest {
                url: r.url,
                headers: r.headers,
                allow_invalid_certs,
            })
        } else {
            Ok(ap_playback::ports::StreamRequest {
                url: track.url.as_deref().ok_or("No stream URL")?.parse()?,
                headers: reqwest::header::HeaderMap::new(),
                allow_invalid_certs: false,
            })
        }
    }
}
