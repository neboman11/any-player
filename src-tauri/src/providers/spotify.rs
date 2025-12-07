use super::{MusicProvider, ProviderError};
use crate::models::{Playlist, Source, Track};
use async_trait::async_trait;
use futures::stream::StreamExt;
use rspotify::{prelude::*, scopes, AuthCodePkceSpotify, Credentials, OAuth};
use std::path::PathBuf;

/// Public Spotify Client ID
const SPOTIFY_CLIENT_ID: &str = "243bb6667db04143b6586d8598aed48b";

/// Default OAuth redirect URI - must be localhost with specific port for Spotify
const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8989/callback";

/// Spotify provider state
pub struct SpotifyProvider {
    client: Option<AuthCodePkceSpotify>,
    redirect_uri: String,
    cache_path: Option<PathBuf>,
    is_authenticated: bool,
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
            redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
            cache_path: None,
            is_authenticated: false,
        }
    }

    /// Create a new Spotify provider with default OAuth configuration (PKCE - no secrets needed)
    pub fn with_default_oauth() -> Self {
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

        let client = AuthCodePkceSpotify::new(credentials, oauth);

        Self {
            client: Some(client),
            redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
            cache_path: None,
            is_authenticated: false,
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
            redirect_uri,
            cache_path: None,
            is_authenticated: false,
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

        Ok(())
    }

    /// Check if provider is authenticated
    pub fn is_authenticated_status(&self) -> bool {
        self.is_authenticated
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
                // Use preview URL if available (full Spotify streaming requires librespot)
                let url = t.preview_url.clone();
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

        // Use Spotify Web API preview URL if available, or external URL
        track
            .preview_url
            .or_else(|| track.external_urls.get("spotify").cloned())
            .ok_or_else(|| ProviderError("No stream URL available for this track".to_string()))
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
        // Use preview URL if available
        // Note: Spotify tracks without preview URLs require full librespot implementation
        let url = track.preview_url.clone();

        if url.is_none() {
            tracing::warn!("Track '{}' has no preview URL available", track.name);
        }

        Ok(Track {
            id: track.id.map(|id| id.to_string()).unwrap_or_default(),
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
