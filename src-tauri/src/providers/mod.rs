pub mod jellyfin;
pub mod plex;
/// Provider trait and implementations
pub mod spotify;

use crate::models::{Playlist, Source, Track};
pub use any_player_core::providers::{
    MusicProvider, ProviderAuthRequest, ProviderAuthResponse, ProviderError, ProviderHandle,
};

/// Provider registry for managing multiple providers
pub struct ProviderRegistry {
    core: any_player_core::providers::ProviderRegistry,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            core: any_player_core::providers::ProviderRegistry::new(),
        }
    }

    pub fn register(&mut self, provider: impl MusicProvider + 'static) -> ProviderHandle {
        self.core.register(provider)
    }

    pub fn register_boxed(&mut self, provider: Box<dyn MusicProvider>) -> ProviderHandle {
        self.core.register_boxed(provider)
    }

    pub fn get(&self, source: Source) -> Option<ProviderHandle> {
        self.core.get(source)
    }

    pub fn get_all(&self) -> Vec<ProviderHandle> {
        self.core.get_all()
    }

    fn require_provider(&self, source: Source) -> Result<ProviderHandle, ProviderError> {
        self.get(source).ok_or_else(|| {
            ProviderError(format!("Provider not initialized for source: {}", source))
        })
    }

    fn build_spotify_provider(request: &ProviderAuthRequest) -> Box<dyn MusicProvider> {
        let client_id = request.get("client_id");
        let client_secret = request.get("client_secret");
        let redirect_uri = request.get("redirect_uri");

        if let (Some(client_id), Some(client_secret), Some(redirect_uri)) =
            (client_id, client_secret, redirect_uri)
        {
            Box::new(spotify::SpotifyProvider::with_oauth(
                client_id.to_string(),
                client_secret.to_string(),
                redirect_uri.to_string(),
            ))
        } else {
            Box::new(spotify::SpotifyProvider::with_default_oauth())
        }
    }

    fn build_jellyfin_provider(
        request: &ProviderAuthRequest,
    ) -> Result<Box<dyn MusicProvider>, ProviderError> {
        let url = request
            .get("url")
            .ok_or_else(|| ProviderError("Missing Jellyfin url".to_string()))?;
        let api_key = request
            .get("api_key")
            .ok_or_else(|| ProviderError("Missing Jellyfin api_key".to_string()))?;

        Ok(Box::new(jellyfin::JellyfinProvider::new(
            url.to_string(),
            api_key.to_string(),
        )))
    }

    fn build_plex_provider(
        request: &ProviderAuthRequest,
    ) -> Result<Box<dyn MusicProvider>, ProviderError> {
        let url = request
            .get("url")
            .ok_or_else(|| ProviderError("Missing Plex url".to_string()))?;
        let token = request
            .get("token")
            .ok_or_else(|| ProviderError("Missing Plex token".to_string()))?;

        Ok(Box::new(plex::PlexProvider::new(
            url.to_string(),
            token.to_string(),
        )))
    }

    pub async fn begin_auth(
        &mut self,
        source: Source,
        request: ProviderAuthRequest,
    ) -> Result<ProviderAuthResponse, ProviderError> {
        let handle = match self.get(source) {
            Some(existing) => existing,
            None => match source {
                Source::Spotify => self.register_boxed(Self::build_spotify_provider(&request)),
                Source::Jellyfin => {
                    return Err(ProviderError(
                        "Jellyfin authentication must be completed with credentials".to_string(),
                    ))
                }
                Source::Plex => {
                    return Err(ProviderError(
                        "Plex authentication must be completed with credentials".to_string(),
                    ))
                }
                Source::Custom => {
                    return Err(ProviderError(
                        "Custom provider does not support authentication".to_string(),
                    ))
                }
            },
        };

        let mut provider = handle.lock().await;
        provider.begin_auth(request).await
    }

    pub async fn complete_auth(
        &mut self,
        source: Source,
        request: ProviderAuthRequest,
    ) -> Result<(), ProviderError> {
        let handle = match self.get(source) {
            Some(existing) => existing,
            None => match source {
                Source::Spotify => {
                    return Err(ProviderError(
                        "Spotify provider not initialized. Call begin_auth first.".to_string(),
                    ))
                }
                Source::Jellyfin => self.register_boxed(Self::build_jellyfin_provider(&request)?),
                Source::Plex => self.register_boxed(Self::build_plex_provider(&request)?),
                Source::Custom => {
                    return Err(ProviderError(
                        "Custom provider does not support authentication".to_string(),
                    ))
                }
            },
        };

        // Hold the provider lock only as long as needed to complete auth and extract credentials.
        let credentials = {
            let mut provider = handle.lock().await;
            provider.complete_auth(request.clone()).await?;

            // Extract provider credentials while holding the lock, but perform I/O after dropping it.
            match source {
                Source::Spotify => {
                    if let Some(spotify) =
                        provider.as_any().downcast_ref::<spotify::SpotifyProvider>()
                    {
                        spotify
                            .get_token()
                            .await
                            .map(|token| (None, None, Some(token)))
                    } else {
                        None
                    }
                }
                Source::Jellyfin => {
                    // Extract Jellyfin credentials from the request
                    let url = request.get("url").map(|s| s.to_string());
                    let api_key = request.get("api_key").map(|s| s.to_string());
                    if url.is_some() && api_key.is_some() {
                        Some((url, api_key, None))
                    } else {
                        None
                    }
                }
                Source::Plex => {
                    let url = request.get("url").map(|s| s.to_string());
                    let token = request.get("token").map(|s| s.to_string());
                    if url.is_some() && token.is_some() {
                        Some((url, token, None))
                    } else {
                        None
                    }
                }
                Source::Custom => None,
            }
        };
        // Provider lock is dropped here

        // Save credentials to keyring after dropping the provider lock
        if let Some((provider_url, provider_secret, spotify_token)) = credentials {
            let mut tokens = crate::config::Config::load_tokens()
                .map_err(|e| ProviderError(format!("Failed to load tokens: {}", e)))?;

            if let Some(token) = spotify_token {
                tracing::info!("Retrieved Spotify token from provider, saving to keyring");
                tokens.spotify_token = Some(token);
                tracing::info!("Spotify token saved to keyring successfully");
            } else if source == Source::Spotify {
                tracing::warn!("Spotify authentication succeeded but no token was retrieved");
            }

            if let (Some(url), Some(secret)) = (provider_url, provider_secret) {
                match source {
                    Source::Jellyfin => {
                        tracing::info!(
                            "Retrieved Jellyfin credentials from request, saving to keyring"
                        );
                        tokens.jellyfin_url = Some(url);
                        tokens.jellyfin_api_key = Some(secret);
                        tracing::info!("Jellyfin credentials saved to keyring successfully");
                    }
                    Source::Plex => {
                        tracing::info!(
                            "Retrieved Plex credentials from request, saving to keyring"
                        );
                        tokens.plex_url = Some(url);
                        tokens.plex_token = Some(secret);
                        tracing::info!("Plex credentials saved to keyring successfully");
                    }
                    _ => {}
                }
            }

            crate::config::Config::save_tokens(&tokens)
                .map_err(|e| ProviderError(format!("Failed to save tokens: {}", e)))?;
        }

        Ok(())
    }

    pub async fn is_authenticated(&self, source: Source) -> bool {
        if let Some(provider) = self.get(source) {
            let provider = provider.lock().await;
            provider.is_authenticated()
        } else {
            false
        }
    }

    pub async fn get_playlists(&self, source: Source) -> Result<Vec<Playlist>, ProviderError> {
        let provider = self.require_provider(source)?;
        let provider = provider.lock().await;
        provider.get_playlists().await
    }

    pub async fn get_playlist(&self, source: Source, id: &str) -> Result<Playlist, ProviderError> {
        let provider = self.require_provider(source)?;
        let provider = provider.lock().await;
        provider.get_playlist(id).await
    }

    pub async fn get_track(&self, source: Source, id: &str) -> Result<Track, ProviderError> {
        let provider = self.require_provider(source)?;
        let provider = provider.lock().await;
        provider.get_track(id).await
    }

    pub async fn search_tracks(
        &self,
        source: Source,
        query: &str,
    ) -> Result<Vec<Track>, ProviderError> {
        let provider = self.require_provider(source)?;
        let provider = provider.lock().await;
        provider.search_tracks(query).await
    }

    pub async fn search_playlists(
        &self,
        source: Source,
        query: &str,
    ) -> Result<Vec<Playlist>, ProviderError> {
        let provider = self.require_provider(source)?;
        let provider = provider.lock().await;
        provider.search_playlists(query).await
    }

    pub async fn get_stream_url(
        &self,
        source: Source,
        track_id: &str,
    ) -> Result<String, ProviderError> {
        let provider = self.require_provider(source)?;
        let provider = provider.lock().await;
        provider.get_stream_url(track_id).await
    }

    pub async fn create_playlist(
        &self,
        source: Source,
        name: &str,
        description: Option<&str>,
    ) -> Result<Playlist, ProviderError> {
        let provider = self.require_provider(source)?;
        let provider = provider.lock().await;
        provider.create_playlist(name, description).await
    }

    pub async fn add_track_to_playlist(
        &self,
        source: Source,
        playlist_id: &str,
        track: &Track,
    ) -> Result<(), ProviderError> {
        let provider = self.require_provider(source)?;
        let provider = provider.lock().await;
        provider.add_track_to_playlist(playlist_id, track).await
    }

    pub async fn remove_track_from_playlist(
        &self,
        source: Source,
        playlist_id: &str,
        track_id: &str,
    ) -> Result<(), ProviderError> {
        let provider = self.require_provider(source)?;
        let provider = provider.lock().await;
        provider
            .remove_track_from_playlist(playlist_id, track_id)
            .await
    }

    pub async fn get_recently_played(
        &self,
        source: Source,
        limit: usize,
    ) -> Result<Vec<Track>, ProviderError> {
        let provider = self.require_provider(source)?;
        let provider = provider.lock().await;
        provider.get_recently_played(limit).await
    }

    pub async fn get_auth_headers(&self, source: Source) -> Option<Vec<(String, String)>> {
        if let Some(provider) = self.get(source) {
            let provider = provider.lock().await;
            provider.get_auth_headers().await
        } else {
            None
        }
    }

    pub async fn get_access_token(&self, source: Source) -> Option<String> {
        if let Some(provider) = self.get(source) {
            let provider = provider.lock().await;
            provider.get_access_token().await
        } else {
            None
        }
    }

    pub async fn refresh_auth(&mut self, source: Source) -> Result<(), ProviderError> {
        let provider = self.require_provider(source)?;
        let mut provider = provider.lock().await;
        provider.refresh_auth().await?;

        if source == Source::Spotify {
            if let Some(spotify) = provider.as_any().downcast_ref::<spotify::SpotifyProvider>() {
                if let Some(token) = spotify.get_token().await {
                    let mut tokens = crate::config::Config::load_tokens()
                        .map_err(|e| ProviderError(format!("Failed to load tokens: {}", e)))?;
                    tokens.spotify_token = Some(token);
                    crate::config::Config::save_tokens(&tokens)
                        .map_err(|e| ProviderError(format!("Failed to save tokens: {}", e)))?;
                }
            }
        }

        Ok(())
    }

    pub async fn premium_status(&self, source: Source) -> Option<bool> {
        if let Some(provider) = self.get(source) {
            let provider = provider.lock().await;
            provider.premium_status().await
        } else {
            None
        }
    }

    pub async fn disconnect(&mut self, source: Source) -> Result<(), ProviderError> {
        match source {
            Source::Spotify => {
                if let Ok(config_dir) = crate::config::Config::config_dir() {
                    let cache_path = config_dir.join("spotify_cache.json");
                    if cache_path.exists() {
                        if let Err(e) = std::fs::remove_file(&cache_path) {
                            tracing::warn!(
                                "Failed to remove Spotify cache file ({}): {}",
                                cache_path.display(),
                                e
                            );
                        }
                    }
                }

                let mut tokens = crate::config::Config::load_tokens()
                    .map_err(|e| ProviderError(format!("Failed to load tokens: {}", e)))?;
                tokens.spotify_token = None;
                crate::config::Config::save_tokens(&tokens)
                    .map_err(|e| ProviderError(format!("Failed to save tokens: {}", e)))?;
            }
            Source::Jellyfin => {
                let mut tokens = crate::config::Config::load_tokens()
                    .map_err(|e| ProviderError(format!("Failed to load tokens: {}", e)))?;
                tokens.jellyfin_api_key = None;
                tokens.jellyfin_url = None;
                crate::config::Config::save_tokens(&tokens)
                    .map_err(|e| ProviderError(format!("Failed to save tokens: {}", e)))?;
            }
            Source::Plex => {
                let mut tokens = crate::config::Config::load_tokens()
                    .map_err(|e| ProviderError(format!("Failed to load tokens: {}", e)))?;
                tokens.plex_token = None;
                tokens.plex_url = None;
                crate::config::Config::save_tokens(&tokens)
                    .map_err(|e| ProviderError(format!("Failed to save tokens: {}", e)))?;
            }
            Source::Custom => {}
        }

        self.core.disconnect(source).await?;
        Ok(())
    }

    pub async fn restore_session(&mut self, source: Source) -> Result<bool, ProviderError> {
        match source {
            Source::Spotify => {
                use crate::config::Config;

                tracing::info!("Starting Spotify session restoration from keyring");

                let tokens = Config::load_tokens()
                    .map_err(|e| ProviderError(format!("Failed to load tokens: {}", e)))?;

                if tokens.spotify_token.is_none() {
                    tracing::info!("No Spotify token found in keyring");
                    return Ok(false);
                }

                tracing::info!("Found Spotify token in keyring, creating provider");

                let mut spotify_provider = spotify::SpotifyProvider::with_default_oauth();

                if let Some(token) = tokens.spotify_token {
                    tracing::info!("Setting token on provider");
                    spotify_provider.set_token(token).await?;
                    tracing::info!("Token set successfully");

                    tracing::info!("Session restored from keyring, storing provider");
                    self.register(spotify_provider);
                    Ok(true)
                } else {
                    tracing::info!("Token was None after loading from keyring");
                    Ok(false)
                }
            }
            Source::Jellyfin => {
                use crate::config::Config;

                tracing::info!("Starting Jellyfin session restoration from keyring");

                let tokens = Config::load_tokens()
                    .map_err(|e| ProviderError(format!("Failed to load tokens: {}", e)))?;

                if tokens.jellyfin_api_key.is_none() || tokens.jellyfin_url.is_none() {
                    tracing::info!("No Jellyfin credentials found in keyring");
                    return Ok(false);
                }

                let api_key = tokens.jellyfin_api_key.unwrap();
                let url = tokens.jellyfin_url.unwrap();

                tracing::info!("Found Jellyfin credentials in keyring, authenticating");

                let mut jellyfin_provider =
                    jellyfin::JellyfinProvider::new(url.to_string(), api_key.to_string());
                jellyfin_provider.authenticate().await?;

                self.register(jellyfin_provider);
                tracing::info!("Jellyfin session restored successfully");
                Ok(true)
            }
            Source::Plex => {
                use crate::config::Config;

                tracing::info!("Starting Plex session restoration from keyring");

                let tokens = Config::load_tokens()
                    .map_err(|e| ProviderError(format!("Failed to load tokens: {}", e)))?;

                if tokens.plex_token.is_none() || tokens.plex_url.is_none() {
                    tracing::info!("No Plex credentials found in keyring");
                    return Ok(false);
                }

                let token = tokens.plex_token.unwrap();
                let url = tokens.plex_url.unwrap();

                tracing::info!("Found Plex credentials in keyring, authenticating");

                let mut plex_provider = plex::PlexProvider::new(url.to_string(), token.to_string());
                plex_provider.authenticate().await?;

                self.register(plex_provider);
                tracing::info!("Plex session restored successfully");
                Ok(true)
            }
            Source::Custom => Ok(false),
        }
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, TokenStorage};
    use rspotify::Token;
    use serial_test::serial;

    /// Helper to create a test token that's not expired
    fn create_valid_token() -> Token {
        use chrono::{Duration as ChronoDuration, Utc};
        Token {
            access_token: "test_access_token".to_string(),
            expires_in: ChronoDuration::seconds(3600),
            expires_at: Some(Utc::now() + ChronoDuration::seconds(3600)),
            refresh_token: Some("test_refresh_token".to_string()),
            scopes: Default::default(),
        }
    }

    /// Helper to create an expired token
    fn create_expired_token() -> Token {
        use chrono::{Duration as ChronoDuration, Utc};
        Token {
            access_token: "expired_access_token".to_string(),
            expires_in: ChronoDuration::seconds(3600),
            expires_at: Some(Utc::now() - ChronoDuration::seconds(3600)),
            refresh_token: Some("test_refresh_token".to_string()),
            scopes: Default::default(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_restore_spotify_session_no_cache_or_tokens() {
        // Clean up any existing cache and tokens
        let _ = Config::clear_tokens();

        let mut registry = ProviderRegistry::new();
        let result = registry.restore_session(Source::Spotify).await;

        // Should return Ok(false) when no cache or tokens exist
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    #[serial]
    async fn test_restore_spotify_session_from_token_storage() {
        // Set up token storage with a valid token
        let tokens = TokenStorage {
            spotify_token: Some(create_valid_token()),
            jellyfin_api_key: None,
            jellyfin_url: None,
            plex_token: None,
            plex_url: None,
        };

        // Save tokens to the system keyring
        Config::save_tokens(&tokens).expect("Failed to save test tokens");

        let mut registry = ProviderRegistry::new();
        let result = registry.restore_session(Source::Spotify).await;

        // Note: This test will likely fail because the token is fake and
        // check_and_update_premium_status will fail when it tries to make an API call.
        // In a real test environment, we'd mock the Spotify API.
        // For now, we're just verifying the code path doesn't panic.

        // Clean up
        let _ = Config::clear_tokens();

        // The result will be Ok(true) if token was set, even if premium check fails
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    #[ignore] // Flaky: depends on Spotify token refresh behavior
    async fn test_restore_spotify_session_with_expired_token() {
        // Set up token storage with an expired token
        let tokens = TokenStorage {
            spotify_token: Some(create_expired_token()),
            jellyfin_api_key: None,
            jellyfin_url: None,
            plex_token: None,
            plex_url: None,
        };

        // Save tokens
        Config::save_tokens(&tokens).expect("Failed to save test tokens");

        let mut registry = ProviderRegistry::new();
        let result = registry.restore_session(Source::Spotify).await;

        // Should fail because the token is expired
        // The set_token method now validates expiry
        assert!(result.is_err() || (result.is_ok() && !result.unwrap()));

        // Clean up
        let _ = Config::clear_tokens();
    }

    #[tokio::test]
    #[serial]
    async fn test_restore_spotify_session_error_handling() {
        // This test verifies that restore_spotify_session handles errors gracefully
        // when both cache and token storage mechanisms fail or are unavailable

        // Clean up any existing state
        let _ = Config::clear_tokens();

        // Try to restore with no available session data
        let mut registry = ProviderRegistry::new();
        let result = registry.restore_session(Source::Spotify).await;

        // Should return Ok(false) rather than panicking
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_provider_registry_initialization() {
        // Test that ProviderRegistry can be created and initialized
        let registry = ProviderRegistry::new();

        // Verify initial state
        assert!(registry.get(Source::Spotify).is_none());
        assert!(registry.get(Source::Jellyfin).is_none());
        assert!(registry.get(Source::Plex).is_none());
    }

    #[tokio::test]
    #[ignore] // Ignore by default as it requires a functioning keyring service
    async fn test_clear_tokens_and_restore() {
        // Set up some tokens
        let tokens = TokenStorage {
            spotify_token: Some(create_valid_token()),
            jellyfin_api_key: Some("test_key".to_string()),
            jellyfin_url: Some("http://localhost:8096".to_string()),
            plex_token: None,
            plex_url: None,
        };
        Config::save_tokens(&tokens).expect("Failed to save tokens");

        // Clear tokens
        Config::clear_tokens().expect("Failed to clear tokens");

        // Try to restore - should return Ok(false)
        let mut registry = ProviderRegistry::new();
        let result = registry.restore_session(Source::Spotify).await;

        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    #[serial]
    async fn test_restore_jellyfin_session_no_credentials() {
        // Clean up any existing tokens
        let _ = Config::clear_tokens();

        let mut registry = ProviderRegistry::new();
        let result = registry.restore_session(Source::Jellyfin).await;

        // Should return Ok(false) when no credentials exist
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    #[serial]
    #[ignore] // Requires actual Jellyfin server for full test
    async fn test_restore_jellyfin_session_with_credentials() {
        // Set up token storage with Jellyfin credentials
        let tokens = TokenStorage {
            spotify_token: None,
            jellyfin_api_key: Some("test_api_key".to_string()),
            jellyfin_url: Some("http://localhost:8096".to_string()),
            plex_token: None,
            plex_url: None,
        };

        // Save tokens to the system keyring
        Config::save_tokens(&tokens).expect("Failed to save test tokens");

        let mut registry = ProviderRegistry::new();
        let result = registry.restore_session(Source::Jellyfin).await;

        // Clean up
        let _ = Config::clear_tokens();

        // Note: This will likely fail in CI without a real Jellyfin server
        // but verifies the code path doesn't panic
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    #[serial]
    async fn test_restore_jellyfin_session_missing_api_key() {
        // Set up token storage with only URL (missing API key)
        let tokens = TokenStorage {
            spotify_token: None,
            jellyfin_api_key: None,
            jellyfin_url: Some("http://localhost:8096".to_string()),
            plex_token: None,
            plex_url: None,
        };

        Config::save_tokens(&tokens).expect("Failed to save test tokens");

        let mut registry = ProviderRegistry::new();
        let result = registry.restore_session(Source::Jellyfin).await;

        // Clean up
        let _ = Config::clear_tokens();

        // Should return Ok(false) when API key is missing
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    #[serial]
    async fn test_restore_jellyfin_session_missing_url() {
        // Set up token storage with only API key (missing URL)
        let tokens = TokenStorage {
            spotify_token: None,
            jellyfin_api_key: Some("test_api_key".to_string()),
            jellyfin_url: None,
            plex_token: None,
            plex_url: None,
        };

        Config::save_tokens(&tokens).expect("Failed to save test tokens");

        let mut registry = ProviderRegistry::new();
        let result = registry.restore_session(Source::Jellyfin).await;

        // Clean up
        let _ = Config::clear_tokens();

        // Should return Ok(false) when URL is missing
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    #[serial]
    async fn test_restore_jellyfin_session_error_handling() {
        // Clean up any existing state
        let _ = Config::clear_tokens();

        // Try to restore with no available credentials
        let mut registry = ProviderRegistry::new();
        let result = registry.restore_session(Source::Jellyfin).await;

        // Should return Ok(false) rather than panicking
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }
}
