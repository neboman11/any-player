use super::{MusicProvider, ProviderAuthRequest, ProviderAuthResponse, ProviderError};
use crate::models::{Playlist, Source, Track};
use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use futures::stream::StreamExt;
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use rspotify::{prelude::*, scopes, AuthCodePkceSpotify, Credentials, OAuth, Token};
use std::path::PathBuf;

/// Retry policy applied to every Spotify Web API request (rspotify's HTTP
/// client). Handles 429/5xx responses automatically, honoring the
/// `Retry-After` header Spotify sends when rate limiting, so we don't need
/// bespoke retry logic scattered across every provider method.
fn spotify_retry_middleware() -> RetryTransientMiddleware<ExponentialBackoff> {
    let retry_policy = ExponentialBackoff::builder().build_with_max_retries(5);
    RetryTransientMiddleware::new_with_policy(retry_policy)
}

/// Spotify Client ID used across the application.
///
/// This is our own self-registered dev-portal app, currently in Spotify's
/// restricted "default" quota mode (not the `spotify-player`-CLI-borrowed ID
/// this previously pointed at, which has extended quota mode). Default quota
/// has much tighter rate limits and may 429 under normal use, and its spclient
/// private streaming endpoints (e.g. extended-metadata) may return
/// `403 RBAC: access denied` regardless of OAuth scopes granted, since only
/// client IDs Spotify has separately granted "extended quota mode" can use
/// them. Swap back to `65b708073fc0480ea92a077233ca87bd` if this proves
/// unworkable.
pub const SPOTIFY_CLIENT_ID: &str = "243bb6667db04143b6586d8598aed48b";

/// Default OAuth redirect URI. Must exactly match the redirect URI
/// registered for `SPOTIFY_CLIENT_ID` above in the Spotify developer
/// dashboard for that app — Spotify's authorization server rejects any
/// non-matching redirect_uri.
const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8989/login";

/// Maximum consecutive errors allowed when streaming playlist items
/// before giving up. Allows for transient network issues.
const MAX_CONSECUTIVE_ERRORS: u32 = 3;

/// Spotify provider state
pub struct SpotifyProvider {
    client: Option<AuthCodePkceSpotify>,
    is_authenticated: bool,
    access_token: Option<String>,
}

impl Default for SpotifyProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SpotifyProvider {
    /// Create a new Spotify provider
    pub fn new() -> Self {
        Self {
            client: None,
            is_authenticated: false,
            access_token: None,
        }
    }

    /// Helper method to create default OAuth configuration with PKCE
    fn default_oauth_config() -> (Credentials, OAuth) {
        // Use PKCE for public clients (desktop apps) that don't have/store a secret
        let credentials = Credentials::new_pkce(SPOTIFY_CLIENT_ID);
        let oauth = OAuth {
            redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
            scopes: scopes!(
                "playlist-read-private",
                "playlist-read-collaborative",
                "playlist-modify-public",
                "playlist-modify-private",
                "streaming",
                "user-modify-playback-state",
                "user-read-playback-state",
                "user-read-private",
                "user-read-email",
                "user-library-read",
                "user-library-modify",
                "user-top-read",
                "user-read-recently-played"
            ),
            ..Default::default()
        };
        (credentials, oauth)
    }

    /// Create a new Spotify provider with default OAuth configuration (PKCE - no secrets needed)
    pub fn with_default_oauth() -> Self {
        let (credentials, oauth) = Self::default_oauth_config();
        let client = AuthCodePkceSpotify::new(credentials, oauth)
            .with_middleware(spotify_retry_middleware());

        Self {
            client: Some(client),
            is_authenticated: false,
            access_token: None,
        }
    }

    /// Create a new Spotify provider with default OAuth and configured cache path
    pub fn with_default_oauth_and_cache(cache_path: PathBuf) -> Self {
        let (credentials, oauth) = Self::default_oauth_config();
        let mut client = AuthCodePkceSpotify::new(credentials, oauth)
            .with_middleware(spotify_retry_middleware());

        // Configure token cache
        client.config.token_cached = true;
        client.config.cache_path = cache_path;

        Self {
            client: Some(client),
            is_authenticated: false,
            access_token: None,
        }
    }

    /// Create a new Spotify provider with custom OAuth configuration
    pub fn with_oauth(client_id: String, client_secret: String, redirect_uri: String) -> Self {
        let credentials = Credentials::new(&client_id, &client_secret);
        let oauth = OAuth {
            redirect_uri: redirect_uri.clone(),
            scopes: scopes!(
                "playlist-read-private",
                "playlist-read-collaborative",
                "playlist-modify-public",
                "playlist-modify-private",
                "streaming",
                "user-modify-playback-state",
                "user-read-playback-state",
                "user-read-private",
                "user-read-email",
                "user-library-read",
                "user-library-modify",
                "user-top-read",
                "user-read-recently-played"
            ),
            ..Default::default()
        };

        let client = AuthCodePkceSpotify::new(credentials, oauth)
            .with_middleware(spotify_retry_middleware());

        Self {
            client: Some(client),
            is_authenticated: false,
            access_token: None,
        }
    }

    /// Get the authorization URL for OAuth flow
    pub fn get_auth_url(&mut self) -> Result<String, ProviderError> {
        self.client
            .as_mut()
            .map(|c| {
                // PKCE requires mutable reference to generate verifier
                c.get_authorize_url(None)
                    .map_err(|e| ProviderError(e.to_string()))
            })
            .ok_or_else(|| ProviderError("Client not configured".to_string()))?
    }

    /// Complete the authentication flow with an authorization code
    pub async fn authenticate_with_code(&mut self, code: &str) -> Result<(), ProviderError> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| ProviderError("Client not configured".to_string()))?;

        // Request access token
        client
            .request_token(code)
            .await
            .map_err(|e| ProviderError(format!("Failed to request access token: {}", e)))?;

        // Mark as authenticated after successful token request
        self.is_authenticated = true;

        // Try to cache the raw access token for convenience by reading the
        // client's in-memory token (avoids depending on file-based token cache
        // which may not be configured).
        let token_mutex = client.get_token();
        match token_mutex.lock().await {
            Ok(token_guard) => {
                if let Some(token) = token_guard.as_ref() {
                    tracing::info!("Caching Spotify access token in memory");
                    self.access_token = Some(token.access_token.clone());
                } else {
                    tracing::debug!("No token found in client in-memory token after request_token");
                }
            }
            Err(err) => {
                tracing::warn!(
                    "Failed to acquire Spotify token mutex during authentication: {:?}",
                    err
                );
            }
        }

        Ok(())
    }

    /// Get the current token if available
    pub async fn get_token(&self) -> Option<Token> {
        if let Some(client) = &self.client {
            match client.token.lock().await {
                Ok(guard) => guard.clone(),
                Err(err) => {
                    tracing::warn!(
                        "Failed to acquire Spotify token mutex in get_token: {:?}",
                        err
                    );
                    None
                }
            }
        } else {
            None
        }
    }

    /// Set a token for the client (used for restoring sessions)
    pub async fn set_token(&mut self, token: Token) -> Result<(), ProviderError> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| ProviderError("Client not configured".to_string()))?;

        // If the provided token is already expired, attempt to refresh it using its refresh token
        if token.is_expired() {
            if token.refresh_token.is_some() {
                tracing::info!("Token is expired, attempting to refresh using refresh token");

                // Temporarily set the expired token (containing the refresh token) on the client
                // so that rspotify can perform the refresh operation
                {
                    let mut token_guard = client
                        .token
                        .lock()
                        .await
                        .map_err(|_| ProviderError("Failed to lock token".to_string()))?;
                    *token_guard = Some(token.clone());
                }

                // Attempt to refresh the token via rspotify
                match client.refresh_token().await {
                    Ok(_) => {
                        tracing::info!("Token refreshed successfully");
                        // Read back the refreshed token from the client
                        let token_guard = client
                            .token
                            .lock()
                            .await
                            .map_err(|_| ProviderError("Failed to lock token".to_string()))?;

                        if let Some(refreshed_token) = token_guard.as_ref() {
                            // Keep internal metadata in sync with the newly refreshed token
                            self.access_token = Some(refreshed_token.access_token.clone());
                            self.is_authenticated = true;
                            drop(token_guard);

                            return Ok(());
                        } else {
                            return Err(ProviderError(
                                "Token refresh succeeded but no token found in client".to_string(),
                            ));
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh expired token: {}", e);
                        return Err(ProviderError(format!(
                            "Provided Spotify token is expired and refresh failed: {}",
                            e
                        )));
                    }
                }
            } else {
                return Err(ProviderError(
                    "Provided Spotify token is expired and has no refresh token".to_string(),
                ));
            }
        }

        // Token is not expired, proceed with normal set operation
        // Save access token string before moving `token` into the guard
        let access_token = token.access_token.clone();

        // Update the underlying client's token
        let mut token_guard = client
            .token
            .lock()
            .await
            .map_err(|_| ProviderError("Failed to lock token".to_string()))?;
        *token_guard = Some(token);
        drop(token_guard);

        // Keep internal metadata in sync with the newly set token
        self.access_token = Some(access_token);
        self.is_authenticated = true;

        Ok(())
    }

    /// Get the cache path if configured
    pub fn get_cache_path(&self) -> Option<PathBuf> {
        self.client.as_ref().map(|c| c.config.cache_path.clone())
    }

    /// Check if provider is authenticated
    pub fn is_authenticated_status(&self) -> bool {
        self.is_authenticated
    }

    /// Spotify no longer exposes account tier via the API; assume Premium.
    pub fn is_premium(&self) -> bool {
        true
    }

    /// Get the current access token for Spotify API
    ///
    /// Returns a placeholder access token if authenticated.
    /// This token can be used to initialize Spotify playback warm-up state and shared-engine playback.
    /// Note: In a production implementation, we'd need to extract the actual token from rspotify's internal state.
    pub async fn get_access_token(&self) -> Option<String> {
        // Return cached token if we stored it during authentication
        if let Some(token) = &self.access_token {
            tracing::debug!(
                "Returning cached Spotify access token (len={})",
                token.len()
            );
            return Some(token.clone());
        }

        if self.is_authenticated {
            if let Some(client) = &self.client {
                // Prefer the client's in-memory token if present (populated by
                // `request_token`). Fall back to file cache only if needed.
                let token_mutex = client.get_token();
                let guard = match token_mutex.lock().await {
                    Ok(guard) => guard,
                    Err(err) => {
                        tracing::warn!(
                            "Failed to acquire Spotify token mutex in get_access_token(): {:?}",
                            err
                        );
                        return None;
                    }
                };
                if let Some(token) = guard.as_ref() {
                    tracing::debug!(
                        "Returning access token from client memory (len={})",
                        token.access_token.len()
                    );
                    return Some(token.access_token.clone());
                } else {
                    tracing::debug!("Client in-memory token empty in get_access_token()");
                }
                // If the client was configured to use a file cache, try that as a
                // fallback (may return None if token caching is disabled).
                if let Ok(maybe_token) = client.read_token_cache(true).await {
                    if let Some(token) = maybe_token {
                        tracing::debug!(
                            "Returning access token from client cache (len={})",
                            token.access_token.len()
                        );
                        return Some(token.access_token.clone());
                    }
                } else {
                    tracing::warn!("Failed to read client token cache in get_access_token()");
                }
            }
        }

        None
    }

    /// Refresh the OAuth token when it is expired or close to expiry.
    ///
    /// Uses a proactive refresh window so long-running sessions keep working
    /// without waiting for the exact expiry boundary.
    pub async fn refresh_token(&mut self) -> Result<(), ProviderError> {
        const REFRESH_WINDOW_SECONDS: i64 = 10 * 60;

        let client = self
            .client
            .as_mut()
            .ok_or_else(|| ProviderError("Client not configured".to_string()))?;

        let current_token = client
            .token
            .lock()
            .await
            .map_err(|_| ProviderError("Failed to lock token".to_string()))?
            .clone();

        let Some(token) = current_token else {
            return Err(ProviderError(
                "No Spotify token is available to refresh".to_string(),
            ));
        };

        let now = Utc::now();
        let refresh_deadline = now + ChronoDuration::seconds(REFRESH_WINDOW_SECONDS);
        let should_refresh = token.is_expired()
            || token
                .expires_at
                .map(|expires_at| expires_at <= refresh_deadline)
                .unwrap_or(true);

        if !should_refresh {
            tracing::debug!("Spotify token refresh skipped (token still valid)");
            return Ok(());
        }

        if token.refresh_token.is_none() {
            return Err(ProviderError(
                "Spotify token is expiring/expired but has no refresh token".to_string(),
            ));
        }

        tracing::info!("Refreshing Spotify OAuth token");
        client
            .refresh_token()
            .await
            .map_err(|e| ProviderError(format!("Failed to refresh Spotify token: {}", e)))?;

        let refreshed_token = client
            .token
            .lock()
            .await
            .map_err(|_| ProviderError("Failed to lock refreshed token".to_string()))?
            .clone();

        let Some(refreshed_token) = refreshed_token else {
            return Err(ProviderError(
                "Spotify refresh reported success but no token is present".to_string(),
            ));
        };

        self.access_token = Some(refreshed_token.access_token.clone());
        self.is_authenticated = true;

        Ok(())
    }

    // --- Spotify Connect (`/v1/me/player/*`) playback control ---
    //
    // These drive playback on whichever Spotify Connect device is active on the
    // account (the same way the official Spotify app's own "Devices" picker
    // would), mirroring the Android app's `SpotifyConnectBridge`. They replace
    // in-process/SDK-based playback, which is no longer usable for this client
    // ID (see the module docs above).

    /// Clone the underlying rspotify client out from behind the provider's
    /// lock. `AuthCodePkceSpotify` is cheap to clone (its token and HTTP
    /// client are internally `Arc`-shared), so callers holding a provider
    /// lock (e.g. `SpotifyConnectBridge`) can grab an owned client and drop
    /// the lock *before* making the actual (potentially multi-retry) HTTP
    /// request, instead of holding the lock for the request's full duration.
    pub fn connect_client(&self) -> Result<AuthCodePkceSpotify, ProviderError> {
        self.client
            .clone()
            .ok_or_else(|| ProviderError("Not authenticated".to_string()))
    }

    /// List the user's available Spotify Connect devices.
    pub async fn connect_devices(&self) -> Result<Vec<rspotify::model::Device>, ProviderError> {
        spotify_connect_devices(&self.connect_client()?).await
    }

    /// The id of the currently active Connect device, if any.
    pub async fn connect_active_device_id(&self) -> Result<Option<String>, ProviderError> {
        spotify_connect_active_device_id(&self.connect_client()?).await
    }

    /// Get the account's current Connect playback state (device, position,
    /// track, shuffle/repeat, etc). Returns `None` when nothing is playing.
    pub async fn connect_playback_state(
        &self,
    ) -> Result<Option<rspotify::model::CurrentPlaybackContext>, ProviderError> {
        spotify_connect_playback_state(&self.connect_client()?).await
    }

    /// Start playback of one or more `spotify:track:...` ids on `device_id`,
    /// optionally seeking to `position_ms` as part of the same request.
    pub async fn connect_start_playback(
        &self,
        track_ids: &[String],
        device_id: Option<&str>,
        position_ms: Option<i64>,
    ) -> Result<(), ProviderError> {
        spotify_connect_start_playback(&self.connect_client()?, track_ids, device_id, position_ms)
            .await
    }

    pub async fn connect_resume(&self, device_id: Option<&str>) -> Result<(), ProviderError> {
        spotify_connect_resume(&self.connect_client()?, device_id).await
    }

    pub async fn connect_pause(&self, device_id: Option<&str>) -> Result<(), ProviderError> {
        spotify_connect_pause(&self.connect_client()?, device_id).await
    }

    pub async fn connect_seek(
        &self,
        position_ms: i64,
        device_id: Option<&str>,
    ) -> Result<(), ProviderError> {
        spotify_connect_seek(&self.connect_client()?, position_ms, device_id).await
    }

    pub async fn connect_set_volume(
        &self,
        volume_percent: u8,
        device_id: Option<&str>,
    ) -> Result<(), ProviderError> {
        spotify_connect_set_volume(&self.connect_client()?, volume_percent, device_id).await
    }
}

/// List the user's available Spotify Connect devices using an already-cloned
/// client, without needing to hold any provider lock for the request.
pub async fn spotify_connect_devices(
    client: &AuthCodePkceSpotify,
) -> Result<Vec<rspotify::model::Device>, ProviderError> {
    client
        .device()
        .await
        .map_err(|e| ProviderError(format!("Failed to list Spotify Connect devices: {}", e)))
}

/// The id of the currently active Connect device, if any.
pub async fn spotify_connect_active_device_id(
    client: &AuthCodePkceSpotify,
) -> Result<Option<String>, ProviderError> {
    let devices = spotify_connect_devices(client).await?;
    Ok(devices
        .into_iter()
        .find(|device| device.is_active)
        .and_then(|device| device.id))
}

/// Get the account's current Connect playback state (device, position,
/// track, shuffle/repeat, etc). Returns `None` when nothing is playing.
pub async fn spotify_connect_playback_state(
    client: &AuthCodePkceSpotify,
) -> Result<Option<rspotify::model::CurrentPlaybackContext>, ProviderError> {
    client
        .current_playback(None, None::<Vec<&rspotify::model::AdditionalType>>)
        .await
        .map_err(|e| {
            ProviderError(format!(
                "Failed to get Spotify Connect playback state: {}",
                e
            ))
        })
}

/// Start playback of one or more `spotify:track:...` ids on `device_id`,
/// optionally seeking to `position_ms` as part of the same request.
pub async fn spotify_connect_start_playback(
    client: &AuthCodePkceSpotify,
    track_ids: &[String],
    device_id: Option<&str>,
    position_ms: Option<i64>,
) -> Result<(), ProviderError> {
    let uris: Vec<rspotify::model::PlayableId> = track_ids
        .iter()
        .filter_map(|id| rspotify::model::TrackId::from_id(id.as_str()).ok())
        .map(rspotify::model::PlayableId::Track)
        .collect();
    if uris.is_empty() {
        return Err(ProviderError(
            "No valid Spotify track ids to play".to_string(),
        ));
    }

    client
        .start_uris_playback(
            uris,
            device_id,
            None,
            position_ms.map(chrono::Duration::milliseconds),
        )
        .await
        .map_err(|e| {
            let message = e.to_string();
            // Spotify's Connect API still rejects playback with a
            // PREMIUM_REQUIRED reason for free-tier accounts even though
            // the profile endpoint no longer exposes account tier - surface
            // that case with a clean, actionable message instead of the
            // raw API error.
            if message.contains("PREMIUM_REQUIRED") || message.to_lowercase().contains("premium") {
                ProviderError("Premium required for full Spotify playback via Connect".to_string())
            } else {
                ProviderError(format!(
                    "Failed to start Spotify Connect playback: {}",
                    message
                ))
            }
        })
}

pub async fn spotify_connect_resume(
    client: &AuthCodePkceSpotify,
    device_id: Option<&str>,
) -> Result<(), ProviderError> {
    client
        .resume_playback(device_id, None)
        .await
        .map_err(|e| ProviderError(format!("Failed to resume Spotify Connect playback: {}", e)))
}

pub async fn spotify_connect_pause(
    client: &AuthCodePkceSpotify,
    device_id: Option<&str>,
) -> Result<(), ProviderError> {
    client
        .pause_playback(device_id)
        .await
        .map_err(|e| ProviderError(format!("Failed to pause Spotify Connect playback: {}", e)))
}

pub async fn spotify_connect_seek(
    client: &AuthCodePkceSpotify,
    position_ms: i64,
    device_id: Option<&str>,
) -> Result<(), ProviderError> {
    client
        .seek_track(chrono::Duration::milliseconds(position_ms), device_id)
        .await
        .map_err(|e| ProviderError(format!("Failed to seek Spotify Connect playback: {}", e)))
}

pub async fn spotify_connect_set_volume(
    client: &AuthCodePkceSpotify,
    volume_percent: u8,
    device_id: Option<&str>,
) -> Result<(), ProviderError> {
    client
        .volume(volume_percent.min(100), device_id)
        .await
        .map_err(|e| {
            ProviderError(format!(
                "Failed to set Spotify Connect playback volume: {}",
                e
            ))
        })
}

#[async_trait]
impl MusicProvider for SpotifyProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn source(&self) -> Source {
        Source::Spotify
    }

    async fn authenticate(&mut self) -> Result<(), ProviderError> {
        // OAuth flow is handled via get_auth_url() and authenticate_with_code()
        self.client.is_some().then_some(()).ok_or_else(|| {
            ProviderError(
                "Not authenticated. Use get_auth_url() and authenticate_with_code()".to_string(),
            )
        })
    }

    async fn begin_auth(
        &mut self,
        _request: ProviderAuthRequest,
    ) -> Result<ProviderAuthResponse, ProviderError> {
        if self.client.is_none() {
            let (credentials, oauth) = Self::default_oauth_config();
            self.client = Some(
                AuthCodePkceSpotify::new(credentials, oauth)
                    .with_middleware(spotify_retry_middleware()),
            );
        }

        let auth_url = self.get_auth_url()?;
        Ok(ProviderAuthResponse::with_auth_url(auth_url))
    }

    async fn complete_auth(&mut self, request: ProviderAuthRequest) -> Result<(), ProviderError> {
        let code = request
            .get("code")
            .ok_or_else(|| ProviderError("Missing Spotify authorization code".to_string()))?;
        self.authenticate_with_code(code).await
    }

    fn is_authenticated(&self) -> bool {
        self.is_authenticated && self.client.is_some()
    }

    async fn get_playlists(&self) -> Result<Vec<Playlist>, ProviderError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ProviderError("Not authenticated".to_string()))?;

        // Use stream API for pagination
        let mut playlists_stream = client.current_user_playlists();
        let mut result = Vec::new();

        while let Some(playlist_item) = playlists_stream.next().await {
            let item = playlist_item
                .map_err(|e| ProviderError(format!("Failed to fetch playlist: {}", e)))?;
            result.push(Playlist {
                id: item.id.to_string(),
                name: item.name,
                description: None,
                owner: item
                    .owner
                    .display_name
                    .unwrap_or_else(|| item.owner.id.to_string()),
                image_url: item.images.first().map(|img| img.url.clone()),
                track_count: item.items.total as usize,
                tracks: Vec::new(),
                source: Source::Spotify,
            });
        }

        Ok(result)
    }

    async fn get_playlist(&self, id: &str) -> Result<Playlist, ProviderError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ProviderError("Not authenticated".to_string()))?;

        // Extract the ID part - it could be a full URI or just the ID
        let clean_id = if id.contains("spotify:playlist:") {
            id.split(':').next_back().unwrap_or(id)
        } else if id.contains("/playlist/") {
            id.split('/').next_back().unwrap_or(id)
        } else {
            id
        };

        let playlist_id = rspotify::model::PlaylistId::from_id(clean_id)
            .map_err(|e| ProviderError(format!("Invalid playlist ID: {}", e)))?;

        let playlist = client
            .playlist(playlist_id.clone(), None, None)
            .await
            .map_err(|e| ProviderError(format!("Failed to fetch playlist: {}", e)))?;

        let mut tracks = Vec::new();

        // Use stream to get all tracks with pagination
        // Retry transient network errors to handle large playlists more robustly
        let mut tracks_stream = client.playlist_items(playlist_id.clone(), None, None);
        let mut consecutive_errors = 0;

        while let Some(track_result) = tracks_stream.next().await {
            match track_result {
                Ok(item) => {
                    consecutive_errors = 0; // Reset error counter on success

                    if let Some(rspotify::model::PlayableItem::Track(t)) = item.item {
                        let duration_ms = t.duration.num_milliseconds() as u64;
                        // Return a Spotify URI for the active Connect device.
                        let url = t.id.as_ref().map(|id| format!("spotify:track:{}", id));
                        tracks.push(Track {
                            id: t.id.map(|id| id.to_string()).unwrap_or_default(),
                            title: t.name,
                            artist: t
                                .artists
                                .iter()
                                .map(|a| a.name.clone())
                                .collect::<Vec<_>>()
                                .join(", "),
                            album: t.album.name,
                            duration_ms,
                            image_url: t.album.images.first().map(|img| img.url.clone()),
                            source: Source::Spotify,
                            url,
                            bitrate_kbps: None,
                            sample_rate_hz: None,
                            auth_headers: None,
                            enriched: false,
                        });
                    }
                }
                Err(e) => {
                    consecutive_errors += 1;
                    tracing::warn!(
                        "Error fetching Spotify playlist track (attempt {}/{}): {}",
                        consecutive_errors,
                        MAX_CONSECUTIVE_ERRORS,
                        e
                    );

                    if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                        return Err(ProviderError(format!(
                            "Failed to fetch playlist tracks after {} consecutive errors: {}",
                            MAX_CONSECUTIVE_ERRORS, e
                        )));
                    }

                    // Brief delay before continuing to next item to avoid overwhelming API
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        }

        Ok(Playlist {
            id: playlist.id.to_string(),
            name: playlist.name,
            description: playlist.description,
            owner: playlist
                .owner
                .display_name
                .unwrap_or_else(|| playlist.owner.id.to_string()),
            image_url: playlist.images.first().map(|img| img.url.clone()),
            track_count: tracks.len(),
            tracks,
            source: Source::Spotify,
        })
    }
    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>, ProviderError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ProviderError("Not authenticated".to_string()))?;

        use rspotify::model::SearchType;

        let search_result = client
            .search(query, SearchType::Track, None, None, Some(20), None)
            .await
            .map_err(|e| ProviderError(format!("Failed to search Spotify tracks: {}", e)))?;

        let tracks = if let rspotify::model::SearchResult::Tracks(page) = search_result {
            page.items
                .iter()
                .map(|track| {
                    let artists = track
                        .artists
                        .iter()
                        .map(|a| a.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ");

                    let album_name = track.album.name.clone();
                    let duration_ms = track.duration.num_milliseconds() as u64;
                    let image_url = track.album.images.first().map(|img| img.url.clone());

                    Track {
                        id: track
                            .id
                            .as_ref()
                            .map(|id| id.to_string())
                            .unwrap_or_default(),
                        title: track.name.clone(),
                        artist: artists,
                        album: album_name,
                        duration_ms,
                        image_url,
                        source: Source::Spotify,
                        // Must be a `spotify:track:` URI, not the web link in
                        // `external_urls` - playback routing keys off this
                        // prefix to send the track through Spotify Connect.
                        url: track.id.as_ref().map(|id| format!("spotify:track:{}", id)),
                        bitrate_kbps: None,
                        sample_rate_hz: None,
                        auth_headers: None,
                        enriched: false,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(tracks)
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>, ProviderError> {
        let _client = self
            .client
            .as_ref()
            .ok_or_else(|| ProviderError("Not authenticated".to_string()))?;

        // TODO: Implement playlist search using rspotify search API
        Err(ProviderError(format!(
            "Playlist search not yet implemented for query: {}",
            query
        )))
    }

    async fn get_stream_url(&self, track_id: &str) -> Result<String, ProviderError> {
        // Premium playback only - extract track ID and return spotify:track: URI
        let clean_id = if track_id.contains("spotify:track:") {
            track_id.split(':').next_back().unwrap_or(track_id)
        } else if track_id.contains("/track/") {
            track_id.split('/').next_back().unwrap_or(track_id)
        } else {
            track_id
        };

        let spotify_uri = format!("spotify:track:{}", clean_id);
        tracing::info!(
            "Returning spotify URI for premium playback: {}",
            spotify_uri
        );
        Ok(spotify_uri)
    }

    async fn get_track(&self, track_id: &str) -> Result<Track, ProviderError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ProviderError("Not authenticated".to_string()))?;

        // Extract the ID part - it could be a full URI or just the ID
        let clean_id = if track_id.contains("spotify:track:") {
            track_id.split(':').next_back().unwrap_or(track_id)
        } else if track_id.contains("/track/") {
            track_id.split('/').next_back().unwrap_or(track_id)
        } else {
            track_id
        };

        let track_id_obj = rspotify::model::TrackId::from_id(clean_id)
            .map_err(|e| ProviderError(format!("Invalid track ID: {}", e)))?;

        let track = client
            .track(track_id_obj, None)
            .await
            .map_err(|e| ProviderError(format!("Failed to fetch track: {}", e)))?;

        let duration_ms = track.duration.num_milliseconds() as u64;
        // Return the full track URI for playback on the active Connect device.
        let url = Some(format!("spotify:track:{}", clean_id));

        Ok(Track {
            id: clean_id.to_string(),
            title: track.name,
            artist: track
                .artists
                .iter()
                .map(|a| a.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
            album: track.album.name,
            duration_ms,
            image_url: track.album.images.first().map(|img| img.url.clone()),
            source: Source::Spotify,
            url,
            bitrate_kbps: None,
            sample_rate_hz: None,
            auth_headers: None,
            enriched: false,
        })
    }

    async fn create_playlist(
        &self,
        _name: &str,
        _description: Option<&str>,
    ) -> Result<Playlist, ProviderError> {
        Err(ProviderError(
            "Playlist creation not yet implemented".to_string(),
        ))
    }

    async fn add_track_to_playlist(
        &self,
        _playlist_id: &str,
        _track: &Track,
    ) -> Result<(), ProviderError> {
        Err(ProviderError(
            "Add track to playlist not yet implemented".to_string(),
        ))
    }

    async fn remove_track_from_playlist(
        &self,
        _playlist_id: &str,
        _track_id: &str,
    ) -> Result<(), ProviderError> {
        Err(ProviderError(
            "Remove track from playlist not yet implemented".to_string(),
        ))
    }

    async fn get_recently_played(&self, _limit: usize) -> Result<Vec<Track>, ProviderError> {
        Err(ProviderError(
            "Get recently played not yet implemented".to_string(),
        ))
    }

    async fn get_access_token(&self) -> Option<String> {
        SpotifyProvider::get_access_token(self).await
    }

    async fn refresh_auth(&mut self) -> Result<(), ProviderError> {
        SpotifyProvider::refresh_token(self).await
    }

    async fn premium_status(&self) -> Option<bool> {
        Some(self.is_premium())
    }

    async fn disconnect(&mut self) -> Result<(), ProviderError> {
        self.client = None;
        self.is_authenticated = false;
        self.access_token = None;
        Ok(())
    }
}
