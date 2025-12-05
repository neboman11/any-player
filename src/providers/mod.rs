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
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: std::collections::HashMap::new(),
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
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
