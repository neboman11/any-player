pub mod jellyfin;
/// Provider trait and implementations
pub mod spotify;

use crate::models::{Playlist, Source, Track};
use async_trait::async_trait;
use std::sync::Arc;

/// Error type for provider operations
#[derive(Debug)]
pub struct ProviderError(pub String);

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProviderError {}

/// Core trait that all music providers must implement
#[async_trait]
pub trait MusicProvider: Send + Sync {
    /// Get the source provider type
    fn source(&self) -> Source;

    /// Authenticate with the provider
    async fn authenticate(&mut self) -> Result<(), ProviderError>;

    /// Check if provider is authenticated
    fn is_authenticated(&self) -> bool;

    /// Get user's playlists
    async fn get_playlists(&self) -> Result<Vec<Playlist>, ProviderError>;

    /// Get a specific playlist by ID
    async fn get_playlist(&self, id: &str) -> Result<Playlist, ProviderError>;

    /// Get a specific track by ID
    async fn get_track(&self, id: &str) -> Result<Track, ProviderError>;

    /// Search for tracks by query
    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>, ProviderError>;

    /// Search for playlists by query
    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>, ProviderError>;

    /// Get a streamable URL for a track
    /// Returns the URL where the track can be streamed from
    async fn get_stream_url(&self, track_id: &str) -> Result<String, ProviderError>;

    /// Create a new playlist
    async fn create_playlist(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<Playlist, ProviderError>;

    /// Add a track to a playlist
    async fn add_track_to_playlist(
        &self,
        playlist_id: &str,
        track: &Track,
    ) -> Result<(), ProviderError>;

    /// Remove a track from a playlist
    async fn remove_track_from_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
    ) -> Result<(), ProviderError>;

    /// Get recently played tracks
    async fn get_recently_played(&self, limit: usize) -> Result<Vec<Track>, ProviderError>;
}

/// Provider registry for managing multiple providers
pub struct ProviderRegistry {
    providers: std::collections::HashMap<Source, Arc<dyn MusicProvider>>,
    spotify_provider: Option<Arc<tokio::sync::Mutex<spotify::SpotifyProvider>>>,
    jellyfin_provider: Option<Arc<tokio::sync::Mutex<jellyfin::JellyfinProvider>>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: std::collections::HashMap::new(),
            spotify_provider: None,
            jellyfin_provider: None,
        }
    }

    pub fn register(&mut self, provider: Arc<dyn MusicProvider>) {
        self.providers.insert(provider.source(), provider);
    }

    pub fn get(&self, source: Source) -> Option<Arc<dyn MusicProvider>> {
        self.providers.get(&source).cloned()
    }

    pub fn get_all(&self) -> Vec<Arc<dyn MusicProvider>> {
        self.providers.values().cloned().collect()
    }

    /// Initialize Spotify provider with default OAuth configuration (PKCE - no secrets needed)
    pub fn get_spotify_auth_url_default(&mut self) -> Result<String, ProviderError> {
        // Use cache path from config
        let config_dir = crate::config::Config::config_dir()
            .map_err(|e| ProviderError(format!("Failed to get config dir: {}", e)))?;

        // Ensure config directory exists before setting cache path
        std::fs::create_dir_all(&config_dir)
            .map_err(|e| ProviderError(format!("Failed to create config directory: {}", e)))?;

        let cache_path = config_dir.join("spotify_cache.json");

        let mut spotify_provider =
            spotify::SpotifyProvider::with_default_oauth_and_cache(cache_path);

        // PKCE requires mutable reference to generate verifier
        let auth_url = spotify_provider.get_auth_url()?;
        self.spotify_provider = Some(Arc::new(tokio::sync::Mutex::new(spotify_provider)));

        Ok(auth_url)
    }

    /// Initialize Spotify provider with OAuth configuration
    pub fn get_spotify_auth_url(
        &mut self,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> Result<String, ProviderError> {
        let mut spotify_provider = spotify::SpotifyProvider::with_oauth(
            client_id.to_string(),
            client_secret.to_string(),
            redirect_uri.to_string(),
        );

        // PKCE requires mutable reference to generate verifier
        let auth_url = spotify_provider.get_auth_url()?;
        self.spotify_provider = Some(Arc::new(tokio::sync::Mutex::new(spotify_provider)));

        Ok(auth_url)
    }
    /// Complete Spotify authentication with authorization code
    pub async fn authenticate_spotify(&self, code: &str) -> Result<(), ProviderError> {
        if let Some(provider) = &self.spotify_provider {
            let mut spotify = provider.lock().await;
            spotify.authenticate_with_code(code).await?;

            // Save the token after successful authentication
            if let Some(token) = spotify.get_token().await {
                let mut tokens = crate::config::Config::load_tokens()
                    .map_err(|e| ProviderError(format!("Failed to load tokens: {}", e)))?;
                tokens.spotify_token = Some(token);
                crate::config::Config::save_tokens(&tokens)
                    .map_err(|e| ProviderError(format!("Failed to save tokens: {}", e)))?;
            }
        } else {
            return Err(ProviderError(
                "Spotify provider not initialized".to_string(),
            ));
        }
        Ok(())
    }

    /// Check if Spotify is authenticated
    pub async fn is_spotify_authenticated(&self) -> bool {
        if let Some(provider) = &self.spotify_provider {
            let spotify = provider.lock().await;
            spotify.is_authenticated_status()
        } else {
            false
        }
    }

    /// Get Spotify playlists
    pub async fn get_spotify_playlists(&self) -> Result<Vec<Playlist>, ProviderError> {
        if let Some(provider) = &self.spotify_provider {
            let spotify = provider.lock().await;
            spotify.get_playlists().await
        } else {
            Err(ProviderError(
                "Spotify provider not authenticated".to_string(),
            ))
        }
    }

    /// Get a specific Spotify track by ID
    pub async fn get_spotify_track(&self, id: &str) -> Result<Track, ProviderError> {
        if let Some(provider) = &self.spotify_provider {
            let spotify = provider.lock().await;
            spotify.get_track(id).await
        } else {
            Err(ProviderError(
                "Spotify provider not authenticated".to_string(),
            ))
        }
    }

    /// Get a specific Spotify playlist by ID
    pub async fn get_spotify_playlist(&self, id: &str) -> Result<Playlist, ProviderError> {
        if let Some(provider) = &self.spotify_provider {
            let spotify = provider.lock().await;
            spotify.get_playlist(id).await
        } else {
            Err(ProviderError(
                "Spotify provider not authenticated".to_string(),
            ))
        }
    }

    /// Authenticate with Jellyfin
    pub async fn authenticate_jellyfin(
        &mut self,
        url: &str,
        api_key: &str,
    ) -> Result<(), ProviderError> {
        let mut jellyfin_provider =
            jellyfin::JellyfinProvider::new(url.to_string(), api_key.to_string());
        jellyfin_provider.authenticate().await?;
        self.jellyfin_provider = Some(Arc::new(tokio::sync::Mutex::new(jellyfin_provider)));
        Ok(())
    }

    /// Check if Jellyfin is authenticated
    pub async fn is_jellyfin_authenticated(&self) -> bool {
        if let Some(provider) = &self.jellyfin_provider {
            let jellyfin = provider.lock().await;
            jellyfin.is_authenticated()
        } else {
            false
        }
    }

    /// Get Jellyfin playlists
    pub async fn get_jellyfin_playlists(&self) -> Result<Vec<Playlist>, ProviderError> {
        if let Some(provider) = &self.jellyfin_provider {
            let jellyfin = provider.lock().await;
            jellyfin.get_playlists().await
        } else {
            Err(ProviderError(
                "Jellyfin provider not authenticated".to_string(),
            ))
        }
    }

    /// Get a specific Jellyfin track by ID
    pub async fn get_jellyfin_track(&self, id: &str) -> Result<Track, ProviderError> {
        if let Some(provider) = &self.jellyfin_provider {
            let jellyfin = provider.lock().await;
            jellyfin.get_track(id).await
        } else {
            Err(ProviderError(
                "Jellyfin provider not authenticated".to_string(),
            ))
        }
    }

    /// Get a specific Jellyfin playlist
    pub async fn get_jellyfin_playlist(&self, id: &str) -> Result<Playlist, ProviderError> {
        if let Some(provider) = &self.jellyfin_provider {
            let jellyfin = provider.lock().await;
            jellyfin.get_playlist(id).await
        } else {
            Err(ProviderError(
                "Jellyfin provider not authenticated".to_string(),
            ))
        }
    }

    /// Search tracks on Jellyfin
    pub async fn search_jellyfin_tracks(&self, query: &str) -> Result<Vec<Track>, ProviderError> {
        if let Some(provider) = &self.jellyfin_provider {
            let jellyfin = provider.lock().await;
            jellyfin.search_tracks(query).await
        } else {
            Err(ProviderError(
                "Jellyfin provider not authenticated".to_string(),
            ))
        }
    }

    /// Search playlists on Jellyfin
    pub async fn search_jellyfin_playlists(
        &self,
        query: &str,
    ) -> Result<Vec<Playlist>, ProviderError> {
        if let Some(provider) = &self.jellyfin_provider {
            let jellyfin = provider.lock().await;
            jellyfin.search_playlists(query).await
        } else {
            Err(ProviderError(
                "Jellyfin provider not authenticated".to_string(),
            ))
        }
    }

    /// Get recently played tracks from Jellyfin
    pub async fn get_jellyfin_recently_played(
        &self,
        limit: usize,
    ) -> Result<Vec<Track>, ProviderError> {
        if let Some(provider) = &self.jellyfin_provider {
            let jellyfin = provider.lock().await;
            jellyfin.get_recently_played(limit).await
        } else {
            Err(ProviderError(
                "Jellyfin provider not authenticated".to_string(),
            ))
        }
    }

    /// Disconnect Spotify
    pub async fn disconnect_spotify(&mut self) -> Result<(), ProviderError> {
        // Clear the cache file when disconnecting
        if let Ok(config_dir) = crate::config::Config::config_dir() {
            let cache_path = config_dir.join("spotify_cache.json");
            if cache_path.exists() {
                if let Err(e) = std::fs::remove_file(&cache_path) {
                    eprintln!(
                        "Warning: failed to remove Spotify cache file ({}): {}",
                        cache_path.display(),
                        e
                    );
                }
            }
        }

        // Clear stored tokens
        crate::config::Config::clear_tokens()
            .map_err(|e| ProviderError(format!("Failed to clear tokens: {}", e)))?;

        self.spotify_provider = None;
        Ok(())
    }

    /// Check if Spotify user has Premium subscription
    ///
    /// Returns Some(true) if user is premium, Some(false) if free tier,
    /// or None if Spotify is not authenticated
    pub async fn is_spotify_premium(&self) -> Option<bool> {
        if let Some(provider) = &self.spotify_provider {
            let spotify = provider.lock().await;
            Some(spotify.is_premium())
        } else {
            None
        }
    }

    /// Get Spotify access token for session initialization
    ///
    /// Returns the OAuth access token if authenticated, None otherwise.
    /// This token can be used to initialize the librespot session.
    pub async fn get_spotify_access_token(&self) -> Option<String> {
        if let Some(provider) = &self.spotify_provider {
            let spotify = provider.lock().await;
            spotify.get_access_token().await
        } else {
            None
        }
    }

    /// Refresh Spotify token and reinitialize session if needed
    ///
    /// This is called periodically to ensure the OAuth token stays valid.
    /// After token refresh, the playback session may need to be reinitialized.
    pub async fn refresh_spotify_token(&mut self) -> Result<(), ProviderError> {
        if let Some(provider) = &self.spotify_provider {
            let mut spotify = provider.lock().await;
            spotify.refresh_token().await?;
            Ok(())
        } else {
            Err(ProviderError(
                "Spotify provider not authenticated".to_string(),
            ))
        }
    }

    /// Disconnect Jellyfin
    pub async fn disconnect_jellyfin(&mut self) -> Result<(), ProviderError> {
        self.jellyfin_provider = None;
        Ok(())
    }

    /// Get Spotify provider for token access (internal use)
    pub fn get_spotify_provider(
        &self,
    ) -> Option<&Arc<tokio::sync::Mutex<spotify::SpotifyProvider>>> {
        self.spotify_provider.as_ref()
    }

    /// Check if we have saved tokens that can be used for authentication
    pub fn has_saved_tokens(&self) -> bool {
        // This will be called before the provider is initialized
        // We'll check for token files in the config directory
        use crate::config::Config;

        if let Ok(tokens) = Config::load_tokens() {
            tokens.spotify_token.is_some()
        } else {
            false
        }
    }

    /// Restore Spotify session from saved tokens
    pub async fn restore_spotify_session(&mut self) -> Result<bool, ProviderError> {
        use crate::config::Config;

        // Get config directory and ensure it exists
        let config_dir = Config::config_dir()
            .map_err(|e| ProviderError(format!("Failed to get config dir: {}", e)))?;

        std::fs::create_dir_all(&config_dir)
            .map_err(|e| ProviderError(format!("Failed to create config directory: {}", e)))?;

        let cache_path = config_dir.join("spotify_cache.json");

        if cache_path.exists() {
            // Create provider with cache path - rspotify will automatically load from cache
            let mut spotify_provider =
                spotify::SpotifyProvider::with_default_oauth_and_cache(cache_path.clone());

            // Try to verify that the restored session is valid by checking premium status.
            // This makes an authenticated API call which serves two purposes:
            // 1. Confirms the cached token is valid and not expired
            // 2. Updates the premium status flag for the user
            //
            // Note: If the API call succeeds but the token expires shortly after,
            // the provider's refresh token mechanism will handle re-authentication.
            if spotify_provider.check_and_update_premium_status().await.is_ok() {
                self.spotify_provider = Some(Arc::new(tokio::sync::Mutex::new(spotify_provider)));
                return Ok(true);
            }
        }

        // Fallback to our token storage if cache doesn't work
        let tokens = Config::load_tokens()
            .map_err(|e| ProviderError(format!("Failed to load tokens: {}", e)))?;

        if tokens.spotify_token.is_none() {
            return Ok(false);
        }

        // Create provider and try to restore with our tokens
        let mut spotify_provider =
            spotify::SpotifyProvider::with_default_oauth_and_cache(cache_path);

        if let Some(token) = tokens.spotify_token {
            spotify_provider.set_token(token).await?;
            // Check premium status after setting token
            let _ = spotify_provider.check_and_update_premium_status().await;

            self.spotify_provider = Some(Arc::new(tokio::sync::Mutex::new(spotify_provider)));
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
