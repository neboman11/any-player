/// Jellyfin provider implementation
use crate::models::{Playlist, Source, Track};
use super::{MusicProvider, ProviderError};
use async_trait::async_trait;

/// Jellyfin provider state
pub struct JellyfinProvider {
    base_url: String,
    api_key: String,
    authenticated: bool,
    user_id: Option<String>,
}

impl JellyfinProvider {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            base_url,
            api_key,
            authenticated: false,
            user_id: None,
        }
    }
}

#[async_trait]
impl MusicProvider for JellyfinProvider {
    fn source(&self) -> Source {
        Source::Jellyfin
    }

    async fn authenticate(&mut self) -> Result<(), ProviderError> {
        // TODO: Verify connection to Jellyfin server
        // GET /System/Info with api_key header
        self.authenticated = true;
        Ok(())
    }

    fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    async fn get_playlists(&self) -> Result<Vec<Playlist>, ProviderError> {
        // TODO: Implement fetching playlists from Jellyfin
        // GET /Users/{userId}/Items with Filters=IsPlaylist
        Ok(Vec::new())
    }

    async fn get_playlist(&self, id: &str) -> Result<Playlist, ProviderError> {
        // TODO: Implement fetching specific playlist
        // GET /Users/{userId}/Items/{id}
        Err(ProviderError("Not implemented".to_string()))
    }

    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>, ProviderError> {
        // TODO: Implement track search
        // GET /Items with search query
        Ok(Vec::new())
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>, ProviderError> {
        // TODO: Implement playlist search
        Ok(Vec::new())
    }

    async fn get_stream_url(&self, track_id: &str) -> Result<String, ProviderError> {
        // TODO: Get direct stream URL from Jellyfin
        // Format: {base_url}/Audio/{track_id}/universal?api_key={api_key}
        Ok(format!(
            "{}/Audio/{}/universal?api_key={}",
            self.base_url, track_id, self.api_key
        ))
    }

    async fn create_playlist(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<Playlist, ProviderError> {
        // TODO: Implement playlist creation
        // POST /Playlists with playlist data
        Err(ProviderError("Not implemented".to_string()))
    }

    async fn add_track_to_playlist(
        &self,
        playlist_id: &str,
        track: &Track,
    ) -> Result<(), ProviderError> {
        // TODO: Implement adding tracks to playlist
        // POST /Playlists/{playlistId}/Items?ids={trackId}
        Ok(())
    }

    async fn remove_track_from_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
    ) -> Result<(), ProviderError> {
        // TODO: Implement removing tracks from playlist
        // DELETE /Playlists/{playlistId}/Items?ids={trackId}
        Ok(())
    }

    async fn get_recently_played(&self, limit: usize) -> Result<Vec<Track>, ProviderError> {
        // TODO: Implement fetching recently played tracks
        Ok(Vec::new())
    }
}
