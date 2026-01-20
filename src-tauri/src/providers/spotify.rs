use super::{MusicProvider, ProviderError};
use crate::models::{Playlist, Source, Track};
use async_trait::async_trait;
use futures::stream::StreamExt;
use rspotify::{prelude::*, scopes, AuthCodePkceSpotify, Credentials, OAuth, Token};
use std::path::PathBuf;

/// Public Spotify Client ID - used across the application
pub const SPOTIFY_CLIENT_ID: &str = "243bb6667db04143b6586d8598aed48b";

/// Default OAuth redirect URI - must be localhost with specific port for Spotify
const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8989/callback";

/// Spotify provider state
pub struct SpotifyProvider {
    client: Option<AuthCodePkceSpotify>,
    is_authenticated: bool,
    is_premium: bool,
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
            is_premium: false,
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
        let client = AuthCodePkceSpotify::new(credentials, oauth);

        Self {
            client: Some(client),
            is_authenticated: false,
            is_premium: false,
            access_token: None,
        }
    }

    /// Create a new Spotify provider with default OAuth and configured cache path
    pub fn with_default_oauth_and_cache(cache_path: PathBuf) -> Self {
        let (credentials, oauth) = Self::default_oauth_config();
        let mut client = AuthCodePkceSpotify::new(credentials, oauth);

        // Configure token cache
        client.config.token_cached = true;
        client.config.cache_path = cache_path;

        Self {
            client: Some(client),
            is_authenticated: false,
            is_premium: false,
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
                "user-read-private",
                "user-read-email",
                "user-library-read",
                "user-library-modify",
                "user-top-read",
                "user-read-recently-played"
            ),
            ..Default::default()
        };

        let client = AuthCodePkceSpotify::new(credentials, oauth);

        Self {
            client: Some(client),
            is_authenticated: false,
            is_premium: false,
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

    /// Fetch current user profile and check premium status
    async fn get_current_user_profile(&mut self) -> Result<bool, ProviderError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| ProviderError("Client not configured".to_string()))?;

        let user = client
            .current_user()
            .await
            .map_err(|e| ProviderError(format!("Failed to fetch user profile: {}", e)))?;

        let is_premium = user.product.is_some();

        tracing::info!("Spotify user subscription type: {:?}", user.product);

        Ok(is_premium)
    }

    /// Check and update premium status from Spotify API
    pub async fn check_and_update_premium_status(&mut self) -> Result<(), ProviderError> {
        match self.get_current_user_profile().await {
            Ok(is_premium) => {
                self.is_premium = is_premium;
                if !is_premium {
                    tracing::warn!(
                        "Premium required for full Spotify playback. User has free tier account."
                    );
                }
                Ok(())
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to check premium status: {}. Defaulting to free tier.",
                    e
                );
                self.is_premium = false;
                Err(e)
            }
        }
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

        // Check premium status
        match self.get_current_user_profile().await {
            Ok(is_premium) => {
                self.is_premium = is_premium;
                if !is_premium {
                    tracing::warn!(
                        "Premium required for full Spotify playback. User has free tier account."
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to check premium status: {}. Defaulting to free tier.",
                    e
                );
                self.is_premium = false;
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
        // Basic validation: do not accept an already-expired token
        if token.is_expired() {
            return Err(ProviderError(
                "Provided Spotify token is expired or invalid".to_string(),
            ));
        }

        let client = self
            .client
            .as_mut()
            .ok_or_else(|| ProviderError("Client not configured".to_string()))?;

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

        // Ensure premium status is updated based on the new token
        if let Err(err) = self.check_and_update_premium_status().await {
            tracing::warn!(
                "Failed to update premium status after setting token: {}",
                err
            );
            // Don't fail the whole operation if premium check fails
            // The token is still valid for basic operations
        }

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

    /// Check if user has Spotify Premium
    pub fn is_premium(&self) -> bool {
        self.is_premium
    }

    /// Get the current access token for Spotify API
    ///
    /// Returns a placeholder access token if authenticated.
    /// This token can be used to initialize the librespot session for premium users.
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

    /// Refresh the OAuth token if needed
    ///
    /// In a production implementation, this would refresh the token with Spotify's API
    /// and notify the playback manager to reinitialize the session if needed.
    /// For now, this is a placeholder that logs the intention.
    pub async fn refresh_token(&mut self) -> Result<(), ProviderError> {
        // TODO: Implement actual token refresh when rspotify provides refresh capability
        // Steps:
        // 1. Check if token is expired
        // 2. Request new token from Spotify
        // 3. Update internal client with new token
        // 4. Return new token for session reinitialization

        tracing::debug!("Spotify token refresh called (placeholder)");
        Ok(())
    }
}

#[async_trait]
impl MusicProvider for SpotifyProvider {
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
            .playlist(playlist_id, None, None)
            .await
            .map_err(|e| ProviderError(format!("Failed to fetch playlist: {}", e)))?;

        let mut tracks = Vec::new();

        // Collect items from current page
        for item in playlist.tracks.items {
            if let Some(rspotify::model::PlayableItem::Track(t)) = item.track {
                let duration_ms = t.duration.num_milliseconds() as u64;
                // Premium playback only - return spotify:track: URI for librespot
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
                });
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
            tracks,
            source: Source::Spotify,
        })
    }
    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>, ProviderError> {
        let _client = self
            .client
            .as_ref()
            .ok_or_else(|| ProviderError("Not authenticated".to_string()))?;

        // TODO: Implement track search using rspotify search API
        Err(ProviderError(format!(
            "Track search not yet implemented for query: {}",
            query
        )))
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

        // Verify user is premium
        if !self.is_premium {
            tracing::warn!("Premium required for track playback. User has free tier account.");
            return Err(ProviderError(
                "Premium required for full Spotify playback".to_string(),
            ));
        }

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
        // Return full track URI for premium streaming via librespot
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
}
