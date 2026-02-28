use super::{MusicProvider, ProviderAuthRequest, ProviderAuthResponse, ProviderError};
/// Jellyfin provider implementation
use crate::models::{Playlist, Source, Track};
use any_player_core::provider_api::{ProviderApi, ProviderConnectionCheck};
use any_player_core::provider_clients::jellyfin::JellyfinApiClient;
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
    api_client: JellyfinApiClient,
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
    #[serde(rename = "AlbumId")]
    album_id: Option<String>,
    #[serde(rename = "Artists")]
    artists: Option<Vec<String>>,
    #[serde(rename = "RunTimeTicks")]
    runtime_ticks: Option<u64>,
    #[serde(rename = "ImageTags")]
    image_tags: Option<Value>,
    #[serde(rename = "AlbumPrimaryImageTag")]
    album_primary_image_tag: Option<String>,
    #[serde(rename = "UserData")]
    #[allow(dead_code)]
    user_data: Option<Value>,
    #[serde(rename = "ChildCount")]
    child_count: Option<u32>,
    #[serde(rename = "RecursiveItemCount")]
    recursive_item_count: Option<u32>,
    #[serde(rename = "Bitrate")]
    #[serde(alias = "BitRate")]
    bitrate: Option<u32>,
    #[serde(rename = "SampleRate")]
    #[serde(alias = "SamplingRate")]
    sample_rate: Option<u32>,
    #[serde(rename = "MediaStreams")]
    media_streams: Option<Vec<JellyfinMediaStream>>,
    #[serde(rename = "MediaSources")]
    media_sources: Option<Vec<JellyfinMediaSource>>,
}

#[derive(Debug, Deserialize)]
struct JellyfinMediaStream {
    #[serde(rename = "Type")]
    stream_type: Option<String>,
    #[serde(rename = "BitRate")]
    #[serde(alias = "Bitrate")]
    bit_rate: Option<u32>,
    #[serde(rename = "SampleRate")]
    #[serde(alias = "SamplingRate")]
    sample_rate: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct JellyfinMediaSource {
    #[serde(rename = "Bitrate")]
    #[serde(alias = "BitRate")]
    bitrate: Option<u32>,
    #[serde(rename = "MediaStreams")]
    media_streams: Option<Vec<JellyfinMediaStream>>,
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
            api_client: JellyfinApiClient::new(),
        }
    }

    fn session_request(&self) -> ProviderAuthRequest {
        let mut request = ProviderAuthRequest::from_pairs([
            ("url", self.base_url.as_str()),
            ("api_key", self.api_key.as_str()),
        ]);
        if let Some(user_id) = &self.user_id {
            request.insert("user_id", user_id.clone());
        }
        request
    }

    /// Get authentication headers for streaming requests
    /// Returns headers as Vec<(String, String)> for use with audio playback
    fn build_auth_headers_vec(&self) -> Vec<(String, String)> {
        vec![
            ("X-Emby-Token".to_string(), self.api_key.clone()),
            ("X-Emby-Authorization".to_string(), format!(
                "MediaBrowser Token=\"{}\", Client=\"AnyPlayer\", Device=\"AnyPlayer\", DeviceId=\"AnyPlayer\", Version=\"1.0.0\"",
                self.api_key
            )),
        ]
    }

    fn apply_auth_request(&mut self, request: &ProviderAuthRequest) -> Result<(), ProviderError> {
        let url = request
            .get("url")
            .ok_or_else(|| ProviderError("Missing Jellyfin url".to_string()))?;
        let api_key = request
            .get("api_key")
            .ok_or_else(|| ProviderError("Missing Jellyfin api_key".to_string()))?;

        self.base_url = url.to_string();
        self.api_key = api_key.to_string();
        Ok(())
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
    /// For tracks, tries to use album artwork first, then falls back to item's own image
    fn get_image_url(&self, item: &JellyfinItem) -> Option<String> {
        // For audio tracks, use the album's primary image
        if item.item_type == "Audio" {
            if let (Some(album_id), Some(album_tag)) =
                (&item.album_id, &item.album_primary_image_tag)
            {
                return Some(format!(
                    "{}/Items/{}/Images/Primary?tag={}&api_key={}",
                    self.base_url, album_id, album_tag, self.api_key
                ));
            }
        }

        // Fallback to item's own image tags
        if let Some(tags) = &item.image_tags {
            if let Some(primary_tag) = tags.get("Primary").and_then(|v| v.as_str()) {
                return Some(format!(
                    "{}/Items/{}/Images/Primary?tag={}&api_key={}",
                    self.base_url, item.id, primary_tag, self.api_key
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
            .cloned()
            .unwrap_or_else(|| "Unknown Artist".to_string());
        let album = item
            .album
            .clone()
            .unwrap_or_else(|| "Unknown Album".to_string());
        let image_url = self.get_image_url(item);
        let (bitrate_kbps, sample_rate_hz) = self.get_audio_quality(item);

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
        let auth_headers = self.build_auth_headers_vec();

        Track {
            id: item.id.clone(),
            title: item.name.clone(),
            artist,
            album,
            duration_ms,
            image_url,
            source: Source::Jellyfin,
            url: Some(stream_url),
            bitrate_kbps,
            sample_rate_hz,
            auth_headers: Some(auth_headers),
            enriched: false,
        }
    }

    fn get_audio_quality(&self, item: &JellyfinItem) -> (Option<u32>, Option<u32>) {
        if let Some(sources) = &item.media_sources {
            if let Some(source) = sources.first() {
                if let Some(streams) = &source.media_streams {
                    if let Some(audio_stream) = streams.iter().find(|stream| {
                        stream
                            .stream_type
                            .as_deref()
                            .map(|value| value.eq_ignore_ascii_case("Audio"))
                            .unwrap_or(false)
                    }) {
                        let bitrate_kbps = audio_stream.bit_rate.map(|value| value / 1000);
                        let fallback_bitrate = source.bitrate.map(|value| value / 1000);
                        return (bitrate_kbps.or(fallback_bitrate), audio_stream.sample_rate);
                    }
                }

                if source.bitrate.is_some() {
                    return (source.bitrate.map(|value| value / 1000), None);
                }
            }
        }

        if let Some(streams) = &item.media_streams {
            if let Some(audio_stream) = streams.iter().find(|stream| {
                stream
                    .stream_type
                    .as_deref()
                    .map(|value| value.eq_ignore_ascii_case("Audio"))
                    .unwrap_or(false)
            }) {
                let bitrate_kbps = audio_stream.bit_rate.map(|value| value / 1000);
                return (bitrate_kbps, audio_stream.sample_rate);
            }
        }

        (item.bitrate.map(|value| value / 1000), item.sample_rate)
    }

    /// Convert Jellyfin item to Playlist
    fn item_to_playlist(&self, item: &JellyfinItem) -> Playlist {
        let image_url = self.get_image_url(item);
        let track_count = item.child_count.or(item.recursive_item_count).unwrap_or(0) as usize;

        Playlist {
            id: item.id.clone(),
            name: item.name.clone(),
            description: None,
            owner: "Jellyfin".to_string(),
            image_url,
            track_count,
            tracks: Vec::new(),
            source: Source::Jellyfin,
        }
    }
}

#[async_trait]
impl MusicProvider for JellyfinProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn source(&self) -> Source {
        Source::Jellyfin
    }

    async fn begin_auth(
        &mut self,
        _request: ProviderAuthRequest,
    ) -> Result<ProviderAuthResponse, ProviderError> {
        Err(ProviderError(
            "Jellyfin authentication requires url and api_key".to_string(),
        ))
    }

    async fn complete_auth(&mut self, request: ProviderAuthRequest) -> Result<(), ProviderError> {
        self.apply_auth_request(&request)?;
        self.authenticate().await
    }

    async fn authenticate(&mut self) -> Result<(), ProviderError> {
        match self
            .api_client
            .validate_connection(&self.session_request())
            .await?
        {
            ProviderConnectionCheck::Connected { metadata, .. } => {
                self.user_id = metadata
                    .get("user_id")
                    .cloned()
                    .or_else(|| metadata.get("userId").cloned())
                    .or_else(|| self.user_id.clone());
                self.authenticated = true;
                Ok(())
            }
            ProviderConnectionCheck::Failed(message) => Err(ProviderError(message)),
        }
    }

    fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    async fn get_playlists(&self) -> Result<Vec<Playlist>, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        self.api_client.get_playlists(&self.session_request()).await
    }

    async fn get_playlist(&self, id: &str) -> Result<Playlist, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        self.api_client
            .get_playlist(&self.session_request(), id)
            .await
    }

    async fn get_track(&self, id: &str) -> Result<Track, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        self.api_client.get_track(&self.session_request(), id).await
    }

    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        self.api_client
            .search_tracks(&self.session_request(), query)
            .await
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        self.api_client
            .search_playlists(&self.session_request(), query)
            .await
    }

    async fn get_stream_url(&self, track_id: &str) -> Result<String, ProviderError> {
        self.api_client
            .get_stream_url(&self.session_request(), track_id)
            .await
    }

    async fn get_auth_headers(&self) -> Option<Vec<(String, String)>> {
        Some(self.build_auth_headers_vec())
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
            "{}/Users/{}/Items?SortBy=DatePlayed&SortOrder=Descending&Limit={}&Filters=IsPlayed&IncludeItemTypes=Audio&Recursive=true&Fields=AudioInfo,MediaSources",
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

    async fn disconnect(&mut self) -> Result<(), ProviderError> {
        self.authenticated = false;
        self.user_id = None;
        Ok(())
    }
}
