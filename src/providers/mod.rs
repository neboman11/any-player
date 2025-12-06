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
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: std::collections::HashMap::new(),
            spotify_provider: None,
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

    /// Initialize Spotify provider with OAuth configuration
    pub fn get_spotify_auth_url(
        &mut self,
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> Result<String, ProviderError> {
        let spotify_provider = spotify::SpotifyProvider::with_oauth(
            client_id.to_string(),
            client_secret.to_string(),
            redirect_uri.to_string(),
        );

        let auth_url = spotify_provider.get_auth_url()?;
        self.spotify_provider = Some(Arc::new(tokio::sync::Mutex::new(spotify_provider)));

        Ok(auth_url)
    }

    /// Complete Spotify authentication with authorization code
    pub async fn authenticate_spotify(&self, code: &str) -> Result<(), ProviderError> {
        if let Some(provider) = &self.spotify_provider {
            let mut spotify = provider.lock().await;
            spotify.authenticate_with_code(code).await?;
        } else {
            return Err(ProviderError(
                "Spotify provider not initialized".to_string(),
            ));
        }
        Ok(())
    }

    /// Check if Spotify is authenticated
    pub fn is_spotify_authenticated(&self) -> bool {
        self.spotify_provider.is_some()
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
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
