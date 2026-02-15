use super::{MusicProvider, ProviderAuthRequest, ProviderAuthResponse, ProviderError};
use crate::models::{Playlist, Source, Track};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

// Plex item type constants
const PLEX_TYPE_PLAYLIST: &str = "15";

pub struct PlexProvider {
    base_url: String,
    token: String,
    authenticated: bool,
    client: Client,
}

#[derive(Debug, Deserialize)]
struct PlexResponse {
    #[serde(rename = "MediaContainer")]
    media_container: PlexMediaContainer,
}

#[derive(Debug, Deserialize)]
struct PlexMediaContainer {
    #[serde(default, rename = "Metadata")]
    metadata: Vec<PlexMetadata>,
}

#[derive(Debug, Deserialize)]
struct PlexMetadata {
    #[serde(rename = "ratingKey")]
    rating_key: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default, rename = "leafCount")]
    leaf_count: Option<u32>,
    #[serde(default)]
    thumb: Option<String>,
    #[serde(default)]
    duration: Option<u64>,
    #[serde(default, rename = "grandparentTitle")]
    grandparent_title: Option<String>,
    #[serde(default, rename = "parentTitle")]
    parent_title: Option<String>,
    #[serde(default)]
    #[serde(rename = "Media")]
    media: Option<Vec<PlexMedia>>,
    #[serde(default, rename = "type")]
    item_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PlexMedia {
    #[serde(default)]
    bitrate: Option<u32>,
    #[serde(default, rename = "audioSamplingRate")]
    audio_sampling_rate: Option<u32>,
    #[serde(default, rename = "Part")]
    part: Option<Vec<PlexPart>>,
}

#[derive(Debug, Deserialize)]
struct PlexPart {
    #[serde(default)]
    key: Option<String>,
}

impl PlexProvider {
    pub fn new(base_url: String, token: String) -> Self {
        Self {
            base_url,
            token,
            authenticated: false,
            client: Client::new(),
        }
    }

    fn normalize_base_url(url: &str) -> String {
        url.trim_end_matches('/').to_string()
    }

    fn apply_auth_request(&mut self, request: &ProviderAuthRequest) -> Result<(), ProviderError> {
        let url = request
            .get("url")
            .ok_or_else(|| ProviderError("Missing Plex url".to_string()))?;
        let token = request
            .get("token")
            .ok_or_else(|| ProviderError("Missing Plex token".to_string()))?;

        self.base_url = Self::normalize_base_url(url);
        self.token = token.to_string();
        Ok(())
    }

    fn authed_url(&self, path_with_query: &str) -> String {
        let path = path_with_query.trim_start_matches('/');
        if path.contains('?') {
            format!("{}/{}&X-Plex-Token={}", self.base_url, path, self.token)
        } else {
            format!("{}/{}?X-Plex-Token={}", self.base_url, path, self.token)
        }
    }

    fn parse_json_response<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T, ProviderError> {
        serde_json::from_str::<T>(body)
            .map_err(|e| ProviderError(format!("Failed to parse Plex response: {}", e)))
    }

    fn image_url_from_path(&self, maybe_path: &Option<String>) -> Option<String> {
        maybe_path.as_ref().map(|path| {
            let path = path.trim_start_matches('/');
            format!("{}/{}?X-Plex-Token={}", self.base_url, path, self.token)
        })
    }

    fn track_from_metadata(&self, item: PlexMetadata) -> Option<Track> {
        let id = item.rating_key?;

        let artist = item
            .grandparent_title
            .unwrap_or_else(|| "Unknown Artist".to_string());
        let album = item
            .parent_title
            .unwrap_or_else(|| "Unknown Album".to_string());
        let image_url = self.image_url_from_path(&item.thumb);

        let stream_key = item
            .media
            .as_ref()
            .and_then(|medias| medias.first())
            .and_then(|media| media.part.as_ref())
            .and_then(|parts| parts.first())
            .and_then(|part| part.key.as_ref())
            .cloned();

        let stream_url = stream_key.map(|key| {
            let key = key.trim_start_matches('/');
            format!("{}/{}?X-Plex-Token={}", self.base_url, key, self.token)
        });

        let bitrate_kbps = item
            .media
            .as_ref()
            .and_then(|medias| medias.first())
            .and_then(|media| media.bitrate);

        let sample_rate_hz = item
            .media
            .as_ref()
            .and_then(|medias| medias.first())
            .and_then(|media| media.audio_sampling_rate);

        Some(Track {
            id,
            title: item.title,
            artist,
            album,
            duration_ms: item.duration.unwrap_or(0),
            image_url,
            source: Source::Plex,
            url: stream_url,
            bitrate_kbps,
            sample_rate_hz,
            auth_headers: None,
            enriched: false,
        })
    }

    fn playlist_from_metadata(&self, item: PlexMetadata) -> Option<Playlist> {
        let id = item.rating_key?;

        Some(Playlist {
            id,
            name: item.title,
            description: item.summary,
            owner: "Plex".to_string(),
            image_url: self.image_url_from_path(&item.thumb),
            track_count: item.leaf_count.unwrap_or(0) as usize,
            tracks: Vec::new(),
            source: Source::Plex,
        })
    }

    async fn get_tracks_from_endpoint(&self, endpoint: &str) -> Result<Vec<Track>, ProviderError> {
        let response = self
            .client
            .get(self.authed_url(endpoint))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to fetch Plex tracks: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError(format!(
                "Plex track request failed: HTTP {}",
                response.status()
            )));
        }

        let body = response
            .text()
            .await
            .map_err(|e| ProviderError(format!("Failed to read Plex response body: {}", e)))?;

        let parsed = Self::parse_json_response::<PlexResponse>(&body)?;

        Ok(parsed
            .media_container
            .metadata
            .into_iter()
            .filter_map(|item| self.track_from_metadata(item))
            .collect())
    }

    async fn get_playlists_from_endpoint(
        &self,
        endpoint: &str,
    ) -> Result<Vec<Playlist>, ProviderError> {
        self.get_playlists_from_endpoint_with_filter(endpoint, None).await
    }

    async fn get_playlists_from_endpoint_with_filter(
        &self,
        endpoint: &str,
        type_filter: Option<&str>,
    ) -> Result<Vec<Playlist>, ProviderError> {
        let response = self
            .client
            .get(self.authed_url(endpoint))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to fetch Plex playlists: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError(format!(
                "Plex playlist request failed: HTTP {}",
                response.status()
            )));
        }

        let body = response
            .text()
            .await
            .map_err(|e| ProviderError(format!("Failed to read Plex response body: {}", e)))?;

        let parsed = Self::parse_json_response::<PlexResponse>(&body)?;

        Ok(parsed
            .media_container
            .metadata
            .into_iter()
            .filter(|item| {
                // Apply type filter if provided
                type_filter.map_or(true, |filter_type| {
                    item.item_type.as_deref() == Some(filter_type)
                })
            })
            .filter_map(|item| self.playlist_from_metadata(item))
            .collect())
    }
}

#[async_trait]
impl MusicProvider for PlexProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn source(&self) -> Source {
        Source::Plex
    }

    async fn begin_auth(
        &mut self,
        _request: ProviderAuthRequest,
    ) -> Result<ProviderAuthResponse, ProviderError> {
        Err(ProviderError(
            "Plex authentication requires url and token".to_string(),
        ))
    }

    async fn complete_auth(&mut self, request: ProviderAuthRequest) -> Result<(), ProviderError> {
        self.apply_auth_request(&request)?;
        self.authenticate().await
    }

    async fn authenticate(&mut self) -> Result<(), ProviderError> {
        let response = self
            .client
            .get(self.authed_url("identity"))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| ProviderError(format!("Failed to connect to Plex: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError(format!(
                "Plex authentication failed: HTTP {}",
                response.status()
            )));
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

        self.get_playlists_from_endpoint("playlists/all?type=15")
            .await
    }

    async fn get_playlist(&self, id: &str) -> Result<Playlist, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        let mut playlists = self
            .get_playlists_from_endpoint(&format!("library/metadata/{}", id))
            .await?;

        let mut playlist = playlists
            .pop()
            .ok_or_else(|| ProviderError(format!("Plex playlist not found: {}", id)))?;

        let tracks = self
            .get_tracks_from_endpoint(&format!("playlists/{}/items", id))
            .await?;
        playlist.track_count = tracks.len();
        playlist.tracks = tracks;

        Ok(playlist)
    }

    async fn get_track(&self, id: &str) -> Result<Track, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        let tracks = self
            .get_tracks_from_endpoint(&format!("library/metadata/{}", id))
            .await?;

        tracks
            .into_iter()
            .next()
            .ok_or_else(|| ProviderError(format!("Plex track not found: {}", id)))
    }

    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        let encoded_query: String =
            url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        self.get_tracks_from_endpoint(&format!("search?query={}&limit=50", encoded_query))
            .await
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        // Plex search API returns mixed results (tracks, albums, artists, playlists, etc.).
        // We filter to only playlist types using type "15" (PLEX_TYPE_PLAYLIST).
        // This is necessary because the Plex API doesn't support type filtering in search queries.
        let encoded_query: String =
            url::form_urlencoded::byte_serialize(query.as_bytes()).collect();

        self.get_playlists_from_endpoint_with_filter(
            &format!("search?query={}&limit=50", encoded_query),
            Some(PLEX_TYPE_PLAYLIST),
        ).await
    }

    async fn get_stream_url(&self, track_id: &str) -> Result<String, ProviderError> {
        let track = self.get_track(track_id).await?;
        track
            .url
            .ok_or_else(|| ProviderError("No stream URL available for Plex track".to_string()))
    }

    async fn create_playlist(
        &self,
        _name: &str,
        _description: Option<&str>,
    ) -> Result<Playlist, ProviderError> {
        Err(ProviderError(
            "Creating Plex playlists is not currently supported".to_string(),
        ))
    }

    async fn add_track_to_playlist(
        &self,
        _playlist_id: &str,
        _track: &Track,
    ) -> Result<(), ProviderError> {
        Err(ProviderError(
            "Adding tracks to Plex playlists is not currently supported".to_string(),
        ))
    }

    async fn remove_track_from_playlist(
        &self,
        _playlist_id: &str,
        _track_id: &str,
    ) -> Result<(), ProviderError> {
        Err(ProviderError(
            "Removing tracks from Plex playlists is not currently supported".to_string(),
        ))
    }

    async fn get_recently_played(&self, limit: usize) -> Result<Vec<Track>, ProviderError> {
        if !self.authenticated {
            return Err(ProviderError("Not authenticated".to_string()));
        }

        let mut tracks = self
            .get_tracks_from_endpoint("hubs/home/recentlyPlayed?type=10")
            .await?;
        if tracks.len() > limit {
            tracks.truncate(limit);
        }
        Ok(tracks)
    }

    async fn disconnect(&mut self) -> Result<(), ProviderError> {
        self.authenticated = false;
        Ok(())
    }
}
