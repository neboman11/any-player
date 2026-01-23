use super::{MusicProvider, ProviderError};
/// Jellyfin provider implementation
use crate::models::{Playlist, Source, Track};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Jellyfin provider state
pub struct JellyfinProvider {
    base_url: String,
    api_key: String,
    authenticated: bool,
    user_id: Option<String>,
    client: Client,
}

/// Jellyfin API response types
#[derive(Debug, Deserialize)]
struct JellyfinUser {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    #[allow(dead_code)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct JellyfinItem {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Type")]
    item_type: String,
    #[serde(rename = "Album")]
    album: Option<String>,
    #[serde(rename = "Artists")]
    artists: Option<Vec<String>>,
    #[serde(rename = "RunTimeTicks")]
    runtime_ticks: Option<u64>,
    #[serde(rename = "ImageTags")]
    image_tags: Option<Value>,
    #[serde(rename = "UserData")]
    #[allow(dead_code)]
    user_data: Option<Value>,
    #[serde(rename = "ChildCount")]
    child_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct JellyfinItemsResponse {
    #[serde(rename = "Items")]
    items: Vec<JellyfinItem>,
    #[serde(rename = "TotalRecordCount")]
    #[allow(dead_code)]
    total_record_count: u32,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct CreatePlaylistRequest {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Ids")]
    ids: Vec<String>,
}

impl JellyfinProvider {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            base_url,
            api_key,
            authenticated: false,
            user_id: None,
            client: Client::new(),
        }
    }

    /// Get authentication headers for streaming requests
    /// Returns headers as Vec<(String, String)> for use with audio playback
    pub fn get_auth_headers(&self) -> Vec<(String, String)> {
        vec![
            ("X-Emby-Token".to_string(), self.api_key.clone()),
            ("X-Emby-Authorization".to_string(), format!(
                "MediaBrowser Token=\"{}\", Client=\"AnyPlayer\", Device=\"AnyPlayer\", DeviceId=\"AnyPlayer\", Version=\"1.0.0\"",
                self.api_key
            )),
        ]
    }

    /// Helper method to build API request headers
    fn build_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "X-Emby-Token",
            reqwest::header::HeaderValue::from_str(&self.api_key).unwrap(),
        );
        headers.insert(
            "X-Emby-Authorization",
            reqwest::header::HeaderValue::from_str(&format!(
                "MediaBrowser Token=\"{}\", Client=\"AnyPlayer\", Device=\"AnyPlayer\", DeviceId=\"AnyPlayer\", Version=\"1.0.0\"",
                self.api_key
            )).unwrap(),
        );
        headers
    }

    /// Helper method to get image URL if available
    fn get_image_url(&self, item_id: &str, image_tags: &Option<Value>) -> Option<String> {
        if let Some(tags) = image_tags {
            if let Some(primary_tag) = tags.get("Primary").and_then(|v| v.as_str()) {
                return Some(format!(
                    "{}/Items/{}/Images/Primary?tag={}",
                    self.base_url, item_id, primary_tag
                ));
            }
        }
        None
    }

    /// Create a fallback playlist with basic metadata when detailed metadata is unavailable
    fn create_fallback_playlist(&self, id: &str, tracks: Vec<Track>) -> Playlist {
        let track_count = tracks.len();
        Playlist {
            id: id.to_string(),
            name: format!("Playlist {}", id),
            description: None,
            owner: "Jellyfin".to_string(),
            image_url: None,
            track_count,
            tracks,
            source: Source::Jellyfin,
        }
    }

    /// Convert Jellyfin item to Track
    fn item_to_track(&self, item: &JellyfinItem) -> Track {
        let duration_ms = item.runtime_ticks.map(|ticks| ticks / 10_000).unwrap_or(0);
        let artist = item
            .artists
            .as_ref()
            .and_then(|artists| artists.first())
            .cloned()
            .unwrap_or_else(|| "Unknown Artist".to_string());
        let album = item
            .album
            .clone()
            .unwrap_or_else(|| "Unknown Album".to_string());
        let image_url = self.get_image_url(&item.id, &item.image_tags);

        // Generate the streaming URL for this track with required parameters
        // The universal endpoint requires UserId, Container format, and optionally AudioCodec
        // Authentication (API key) is handled via X-Emby-Token header to avoid exposing it in URL
        // Note: UserId is a required parameter for the universal endpoint and is not sensitive data
        let user_id = self.user_id.as_deref().unwrap_or("");
        let stream_url = format!(
            "{}/Audio/{}/universal?UserId={}&Container=opus,mp3,aac,m4a,flac,webma,webm,wav,ogg&AudioCodec=aac,mp3,vorbis,opus",
            self.base_url, item.id, user_id
        );

        // Prepare authentication headers for streaming requests
        let auth_headers = vec![
            ("X-Emby-Token".to_string(), self.api_key.clone()),
            ("X-Emby-Authorization".to_string(), format!(
                "MediaBrowser Token=\"{}\", Client=\"AnyPlayer\", Device=\"AnyPlayer\", DeviceId=\"AnyPlayer\", Version=\"1.0.0\"",
                self.api_key
            )),
        ];

        Track {
            id: item.id.clone(),
            title: item.name.clone(),
            artist,
            album,
            duration_ms,
            image_url,
            source: Source::Jellyfin,
            url: Some(stream_url),
            auth_headers: Some(auth_headers),
        }
    }

    /// Convert Jellyfin item to Playlist
    fn item_to_playlist(&self, item: &JellyfinItem) -> Playlist {
        let image_url = self.get_image_url(&item.id, &item.image_tags);

        Playlist {
            id: item.id.clone(),
            name: item.name.clone(),
            description: None,
            owner: "Jellyfin".to_string(),
            image_url,
            track_count: item.child_count.unwrap_or(0) as usize,
            tracks: Vec::new(),
            source: Source::Jellyfin,
        }
    }
}

#[async_trait]
impl MusicProvider for JellyfinProvider {
    fn source(&self) -> Source {
        Source::Jellyfin
    }

    async fn authenticate(&mut self) -> Result<(), ProviderError> {
        // Verify connection to Jellyfin server
        // GET /System/Info with api_key header
        let url = format!("{}/System/Info", self.base_url);
        let response = self
            .client
            .get(&url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to connect to Jellyfin: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError(format!(
                "Jellyfin authentication failed: HTTP {}",
                response.status()
            )));
        }

        // Get list of users from the /Users endpoint and pick the first one
        // (since API keys don't have a "current user"; typically this is the admin/main user)
        let users_url = format!("{}/Users", self.base_url);
        let users_response = self
            .client
            .get(&users_url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to get users: {}", e)))?;

        if !users_response.status().is_success() {
            return Err(ProviderError(format!(
                "Failed to get user list: HTTP {}",
                users_response.status()
            )));
        }

        let users: Vec<JellyfinUser> = users_response
            .json()
            .await
            .map_err(|e| ProviderError(format!("Failed to parse users: {}", e)))?;

        if users.is_empty() {
            return Err(ProviderError(
                "No users found on Jellyfin server".to_string(),
            ));
        }

        // If a user_id was preconfigured on this provider, try to match it against the
        // users returned by the server. This allows multi-user instances to explicitly
        // select which user to act as.
        //
        // If no user_id is configured or the configured id is not found, we fall back
        // to using the first user in the list (typically the admin/main user). This
        // preserves existing behavior but means that, in multi-user setups, the caller
        // SHOULD provide an explicit user_id if the default is not appropriate.
        let selected_user_id = if let Some(ref configured_id) = self.user_id {
            if users.iter().any(|u| &u.id == configured_id) {
                configured_id.clone()
            } else {
                users[0].id.clone()
            }
        } else {
            users[0].id.clone()
        };

        self.user_id = Some(selected_user_id);
        self.authenticated = true;
        Ok(())
    }

    fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    async fn get_playlists(&self) -> Result<Vec<Playlist>, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| ProviderError("User ID not available".to_string()))?;

        // GET /Users/{userId}/Items with Filters=IsFolder
        let url = format!(
            "{}/Users/{}/Items?Filters=IsFolder&Recursive=true&IncludeItemTypes=Playlist",
            self.base_url, user_id
        );

        let response = self
            .client
            .get(&url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to fetch playlists: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError(format!(
                "Failed to fetch playlists: HTTP {}",
                response.status()
            )));
        }

        let data: JellyfinItemsResponse = response
            .json()
            .await
            .map_err(|e| ProviderError(format!("Failed to parse playlists: {}", e)))?;

        let playlists: Vec<Playlist> = data
            .items
            .into_iter()
            .map(|item| self.item_to_playlist(&item))
            .collect();

        Ok(playlists)
    }

    async fn get_playlist(&self, id: &str) -> Result<Playlist, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| ProviderError("User ID not available".to_string()))?;

        // Fetch all playlist items with pagination
        let mut all_tracks = Vec::new();
        let limit = 300; // Jellyfin default limit
        let mut start_index = 0;
        // Safety limit to prevent infinite loops. With limit=300, this allows for
        // playlists with up to 300,000 items (1000 * 300), which should be sufficient
        // for any realistic use case while protecting against API issues.
        const MAX_ITERATIONS: usize = 1000;
        let mut iteration_count = 0;

        loop {
            iteration_count += 1;
            if iteration_count > MAX_ITERATIONS {
                tracing::warn!(
                    "Reached maximum iteration count ({}) while fetching Jellyfin playlist items",
                    MAX_ITERATIONS
                );
                break;
            }

            let items_url = format!(
                "{}/Users/{}/Items?ParentId={}&Fields=AudioInfo,ParentId&Limit={}&StartIndex={}",
                self.base_url, user_id, id, limit, start_index
            );
            let items_response = self
                .client
                .get(&items_url)
                .headers(self.build_headers())
                .send()
                .await
                .map_err(|e| ProviderError(format!("Failed to fetch playlist items: {}", e)))?;

            if !items_response.status().is_success() {
                return Err(ProviderError(format!(
                    "Failed to fetch playlist items: HTTP {}",
                    items_response.status()
                )));
            }

            let items_data: JellyfinItemsResponse = items_response
                .json()
                .await
                .map_err(|e| ProviderError(format!("Failed to parse playlist items: {}", e)))?;

            let tracks: Vec<Track> = items_data
                .items
                .into_iter()
                .filter(|item| item.item_type == "Audio")
                .map(|item| self.item_to_track(&item))
                .collect();

            let fetched_count = tracks.len();
            all_tracks.extend(tracks);

            // Check if we've fetched all items
            // Break if: no items returned, fewer items than requested, or we've reached the total
            if fetched_count == 0 || fetched_count < limit || all_tracks.len() >= items_data.total_record_count as usize {
                break;
            }

            start_index += limit;
        }

        // Try to get playlist metadata using the direct Playlists endpoint first
        let metadata_url = format!("{}/Playlists/{}", self.base_url, id);
        let metadata_response = self
            .client
            .get(&metadata_url)
            .headers(self.build_headers())
            .send()
            .await;

        // If direct endpoint works, use it; otherwise fall back to basic metadata
        let playlist = if let Ok(response) = metadata_response {
            if response.status().is_success() {
                if let Ok(item) = response.json::<JellyfinItem>().await {
                    let mut playlist = self.item_to_playlist(&item);
                    playlist.tracks = all_tracks;
                    return Ok(playlist);
                }
            }
            // Fallback: create basic playlist from ID
            self.create_fallback_playlist(id, all_tracks)
        } else {
            // Fallback: create basic playlist from ID
            self.create_fallback_playlist(id, all_tracks)
        };

        Ok(playlist)
    }

    async fn get_track(&self, id: &str) -> Result<Track, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| ProviderError("User ID not available".to_string()))?;

        // GET /Users/{userId}/Items/{id}
        let url = format!("{}/Users/{}/Items/{}", self.base_url, user_id, id);

        let response = self
            .client
            .get(&url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to fetch track: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError(format!(
                "Failed to fetch track: HTTP {}",
                response.status()
            )));
        }

        let item: JellyfinItem = response
            .json()
            .await
            .map_err(|e| ProviderError(format!("Failed to parse track: {}", e)))?;

        Ok(self.item_to_track(&item))
    }

    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| ProviderError("User ID not available".to_string()))?;

        // GET /Items with search query
        let url = format!(
            "{}/Users/{}/Items?searchTerm={}&IncludeItemTypes=Audio&Recursive=true",
            self.base_url, user_id, query
        );

        let response = self
            .client
            .get(&url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to search tracks: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError(format!(
                "Failed to search tracks: HTTP {}",
                response.status()
            )));
        }

        let data: JellyfinItemsResponse = response
            .json()
            .await
            .map_err(|e| ProviderError(format!("Failed to parse search results: {}", e)))?;

        let tracks: Vec<Track> = data
            .items
            .into_iter()
            .map(|item| self.item_to_track(&item))
            .collect();

        Ok(tracks)
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| ProviderError("User ID not available".to_string()))?;

        // GET /Items with search query for playlists
        let url = format!(
            "{}/Users/{}/Items?searchTerm={}&IncludeItemTypes=Playlist&Recursive=true",
            self.base_url, user_id, query
        );

        let response = self
            .client
            .get(&url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to search playlists: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError(format!(
                "Failed to search playlists: HTTP {}",
                response.status()
            )));
        }

        let data: JellyfinItemsResponse = response
            .json()
            .await
            .map_err(|e| ProviderError(format!("Failed to parse search results: {}", e)))?;

        let playlists: Vec<Playlist> = data
            .items
            .into_iter()
            .map(|item| self.item_to_playlist(&item))
            .collect();

        Ok(playlists)
    }

    async fn get_stream_url(&self, track_id: &str) -> Result<String, ProviderError> {
        // Get direct stream URL from Jellyfin
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
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| ProviderError("User ID not available".to_string()))?;

        // POST /Playlists with playlist data
        let url = format!(
            "{}/Playlists?userId={}&name={}",
            self.base_url, user_id, name
        );

        let response = self
            .client
            .post(&url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to create playlist: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError(format!(
                "Failed to create playlist: HTTP {}",
                response.status()
            )));
        }

        let item: JellyfinItem = response
            .json()
            .await
            .map_err(|e| ProviderError(format!("Failed to parse created playlist: {}", e)))?;

        let mut playlist = self.item_to_playlist(&item);
        if let Some(desc) = description {
            playlist.description = Some(desc.to_string());
        }
        Ok(playlist)
    }

    async fn add_track_to_playlist(
        &self,
        playlist_id: &str,
        track: &Track,
    ) -> Result<(), ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        // POST /Playlists/{playlistId}/Items?ids={trackId}
        let url = format!(
            "{}/Playlists/{}/Items?ids={}",
            self.base_url, playlist_id, track.id
        );

        let response = self
            .client
            .post(&url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to add track to playlist: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError(format!(
                "Failed to add track to playlist: HTTP {}",
                response.status()
            )));
        }

        Ok(())
    }

    async fn remove_track_from_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
    ) -> Result<(), ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        // DELETE /Playlists/{playlistId}/Items?ids={trackId}
        let url = format!(
            "{}/Playlists/{}/Items?ids={}",
            self.base_url, playlist_id, track_id
        );

        let response = self
            .client
            .delete(&url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to remove track from playlist: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError(format!(
                "Failed to remove track from playlist: HTTP {}",
                response.status()
            )));
        }

        Ok(())
    }

    async fn get_recently_played(&self, limit: usize) -> Result<Vec<Track>, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        let user_id = self
            .user_id
            .as_ref()
            .ok_or_else(|| ProviderError("User ID not available".to_string()))?;

        // Get recently played items
        let url = format!(
            "{}/Users/{}/Items?SortBy=DatePlayed&SortOrder=Descending&Limit={}&Filters=IsPlayed&IncludeItemTypes=Audio&Recursive=true",
            self.base_url, user_id, limit
        );

        let response = self
            .client
            .get(&url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to fetch recently played: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError(format!(
                "Failed to fetch recently played: HTTP {}",
                response.status()
            )));
        }

        let data: JellyfinItemsResponse = response
            .json()
            .await
            .map_err(|e| ProviderError(format!("Failed to parse recently played: {}", e)))?;

        let tracks: Vec<Track> = data
            .items
            .into_iter()
            .map(|item| self.item_to_track(&item))
            .collect();

        Ok(tracks)
    }
}
