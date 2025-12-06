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
    user_data: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct JellyfinItemsResponse {
    #[serde(rename = "Items")]
    items: Vec<JellyfinItem>,
    #[serde(rename = "TotalRecordCount")]
    total_record_count: u32,
}

#[derive(Debug, Serialize)]
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

    /// Convert Jellyfin item to Track
    fn item_to_track(&self, item: &JellyfinItem) -> Track {
        let duration_ms = item.runtime_ticks.map(|ticks| ticks / 10_000).unwrap_or(0);
        let artist = item
            .artists
            .as_ref()
            .and_then(|artists| artists.first())
            .map(|s| s.clone())
            .unwrap_or_else(|| "Unknown Artist".to_string());
        let album = item
            .album
            .clone()
            .unwrap_or_else(|| "Unknown Album".to_string());
        let image_url = self.get_image_url(&item.id, &item.image_tags);

        Track {
            id: item.id.clone(),
            title: item.name.clone(),
            artist,
            album,
            duration_ms,
            image_url,
            source: Source::Jellyfin,
            url: None,
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

        // Get current user info
        let user_url = format!("{}/Users/Me", self.base_url);
        let user_response = self
            .client
            .get(&user_url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to get user info: {}", e)))?;

        if user_response.status().is_success() {
            let user: JellyfinUser = user_response
                .json()
                .await
                .map_err(|e| ProviderError(format!("Failed to parse user info: {}", e)))?;
            self.user_id = Some(user.id);
        }

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

        // GET /Users/{userId}/Items/{id}
        let url = format!("{}/Users/{}/Items/{}", self.base_url, user_id, id);

        let response = self
            .client
            .get(&url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to fetch playlist: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError(format!(
                "Failed to fetch playlist: HTTP {}",
                response.status()
            )));
        }

        let item: JellyfinItem = response
            .json()
            .await
            .map_err(|e| ProviderError(format!("Failed to parse playlist: {}", e)))?;

        // Get playlist items
        let items_url = format!("{}/Playlists/{}/Items", self.base_url, id);
        let items_response = self
            .client
            .get(&items_url)
            .headers(self.build_headers())
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to fetch playlist items: {}", e)))?;

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

        let mut playlist = self.item_to_playlist(&item);
        playlist.tracks = tracks;
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
