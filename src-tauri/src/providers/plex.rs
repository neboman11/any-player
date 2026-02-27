use super::{MusicProvider, ProviderAuthRequest, ProviderAuthResponse, ProviderError};
use crate::models::{Playlist, Source, Track};
use any_player_core::provider_api::ProviderApi;
use any_player_core::provider_clients::plex::PlexApiClient;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

pub struct PlexProvider {
    base_url: String,
    token: String,
    authenticated: bool,
    client: Client,
    insecure_client: Client,
    use_insecure_tls: bool,
    api_client: PlexApiClient,
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
        let insecure_client = Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            base_url,
            token,
            authenticated: false,
            client: Client::new(),
            insecure_client,
            use_insecure_tls: false,
            api_client: PlexApiClient::new(),
        }
    }

    fn session_request(&self) -> ProviderAuthRequest {
        ProviderAuthRequest::from_pairs([
            ("url", self.base_url.as_str()),
            ("token", self.token.as_str()),
        ])
    }

    fn active_client(&self) -> &Client {
        if self.use_insecure_tls {
            &self.insecure_client
        } else {
            &self.client
        }
    }

    fn is_tls_error(error: &reqwest::Error) -> bool {
        let message = error.to_string().to_lowercase();

        message.contains("certificate")
            || message.contains("tls")
            || message.contains("ssl")
            || message.contains("unknown issuer")
            || message.contains("self signed")
            || message.contains("invalid peer certificate")
    }

    fn normalize_base_url(url: &str) -> String {
        url.trim_end_matches('/').to_string()
    }

    fn validate_base_url(url: &str) -> Result<(), ProviderError> {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(ProviderError(
                "Invalid Plex URL. Include http:// or https:// (for example: http://127.0.0.1:32400)"
                    .to_string(),
            ));
        }

        url::Url::parse(url).map_err(|_| {
            ProviderError(
                "Invalid Plex URL format. Check the server URL and try again.".to_string(),
            )
        })?;

        Ok(())
    }

    fn map_connect_error(error: &reqwest::Error) -> String {
        let raw = error.to_string();
        let message = raw.to_lowercase();

        if Self::is_tls_error(error) {
            return "TLS/SSL handshake failed while connecting to Plex. This often happens with self-signed or locally-issued certificates.".to_string();
        }

        if error.is_timeout() || message.contains("timed out") {
            return "Timed out connecting to Plex. Check that the server URL is reachable and the server is online.".to_string();
        }

        if error.is_connect()
            && (message.contains("dns")
                || message.contains("name or service not known")
                || message.contains("failed to lookup address information")
                || message.contains("no such host"))
        {
            return "Could not resolve the Plex server host. Verify the URL hostname/IP and try again."
                .to_string();
        }

        if error.is_connect() && message.contains("connection refused") {
            return "Plex server refused the connection. Confirm the host/port and that Plex is running."
                .to_string();
        }

        if error.is_connect() {
            return "Could not connect to the Plex server. Check the server URL, network, and firewall settings."
                .to_string();
        }

        format!("Failed to connect to Plex: {}", raw)
    }

    fn map_auth_status(status: reqwest::StatusCode) -> String {
        match status.as_u16() {
            401 | 403 => {
                "Plex rejected the token (unauthorized). Verify your Plex token and try again."
                    .to_string()
            }
            404 => {
                "Plex server endpoint was not found. Verify the server URL and port.".to_string()
            }
            _ => format!("Plex authentication failed (HTTP {}).", status),
        }
    }

    fn apply_auth_request(&mut self, request: &ProviderAuthRequest) -> Result<(), ProviderError> {
        let url = request
            .get("url")
            .ok_or_else(|| ProviderError("Missing Plex url".to_string()))?;
        let token = request
            .get("token")
            .ok_or_else(|| ProviderError("Missing Plex token".to_string()))?;

        let normalized_url = Self::normalize_base_url(url);
        Self::validate_base_url(&normalized_url)?;

        self.base_url = normalized_url;
        self.token = token.to_string();
        self.use_insecure_tls = false;
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

    async fn get_tracks_from_endpoint(&self, endpoint: &str) -> Result<Vec<Track>, ProviderError> {
        let response = self
            .active_client()
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
        let identity_url = self.authed_url("identity");
        let mut used_insecure_tls = false;

        let response = match self
            .client
            .get(&identity_url)
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) if self.base_url.starts_with("https://") && Self::is_tls_error(&error) => {
                tracing::warn!(
                    "Plex HTTPS TLS validation failed; retrying with insecure certificate validation"
                );

                let insecure_response = self
                    .insecure_client
                    .get(&identity_url)
                    .header("Accept", "application/json")
                    .send()
                    .await
                    .map_err(|e| ProviderError(Self::map_connect_error(&e)))?;

                used_insecure_tls = true;
                insecure_response
            }
            Err(error) => return Err(ProviderError(Self::map_connect_error(&error))),
        };

        if !response.status().is_success() {
            return Err(ProviderError(Self::map_auth_status(response.status())));
        }

        self.use_insecure_tls = used_insecure_tls;
        self.api_client = if used_insecure_tls {
            PlexApiClient::with_client(self.insecure_client.clone())
        } else {
            PlexApiClient::with_client(self.client.clone())
        };
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
