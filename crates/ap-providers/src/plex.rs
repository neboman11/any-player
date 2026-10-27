//! Plex client (port of Android `PlexClient`, design §7). The token travels only in the
//! `X-Plex-Token` header; stored track and artwork URLs never carry it.

use ap_model::{Playlist, SourceType, Track};
use async_trait::async_trait;
use reqwest::header::{ACCEPT, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Url};
use serde_json::Value;

use crate::error::{ProviderError, Result};
use crate::http::{self, PLEX_CONNECT_TIMEOUT, RetryPolicy};
use crate::json_ext::{array, number, obj, raw_len, text, to_i32};
use crate::provider::{
    MAX_PAGES, MediaRequest, ProviderClient, SEARCH_LIMIT, Validation, is_blank,
    normalize_base_url, parse_url, same_origin, server_media_request, strip_url_tokens,
};

/// Default `providers.plex.page_size`.
pub const DEFAULT_PAGE_SIZE: u32 = 300;

/// Audio codecs (Plex `Media.audioCodec`) the local decoder plays directly. Any other reported
/// codec (e.g. `opus`) streams through the universal transcoder as MP3.
pub const DIRECT_PLAY_CODECS: &[&str] = &["mp3", "aac", "flac", "vorbis", "alac", "pcm", "wav"];

const X_PLEX_TOKEN: HeaderName = HeaderName::from_static("x-plex-token");
const X_PLEX_CLIENT_IDENTIFIER: HeaderName = HeaderName::from_static("x-plex-client-identifier");
const X_PLEX_PRODUCT: HeaderName = HeaderName::from_static("x-plex-product");

/// One Plex connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlexConfig {
    pub base_url: String,
    pub token: String,
    /// Clamped to 1..=1000.
    pub page_size: u32,
    /// The per-connection "Allow self-signed certificate" setting. Never enabled implicitly.
    pub allow_self_signed: bool,
    /// Install id, sent as `X-Plex-Client-Identifier` (required by the transcoder).
    pub client_identifier: String,
}

/// Plex client bound to one connection.
#[derive(Debug)]
pub struct PlexClient {
    http: Client,
    base: String,
    base_url: Url,
    page_size: usize,
    headers: HeaderMap,
    retry: RetryPolicy,
}

impl PlexClient {
    /// Builds a dedicated HTTP client (6 s connect timeout; certificate checks disabled only when
    /// `allow_self_signed` is set).
    pub fn new(config: &PlexConfig) -> Result<Self> {
        let http = http::client_builder()
            .connect_timeout(PLEX_CONNECT_TIMEOUT)
            .danger_accept_invalid_certs(config.allow_self_signed)
            .build()?;
        Self::with_http(http, config)
    }

    /// Uses a caller-provided HTTP client; `allow_self_signed` is then the caller's business.
    pub fn with_http(http: Client, config: &PlexConfig) -> Result<Self> {
        let base = normalize_base_url(&config.base_url, "Plex")?;
        let base_url = parse_url(&base)?;
        let invalid = |what: &str| ProviderError::InvalidConfig(format!("Invalid Plex {what}"));
        let mut token = HeaderValue::from_str(config.token.trim()).map_err(|_| invalid("token"))?;
        token.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(X_PLEX_TOKEN, token);
        headers.insert(
            X_PLEX_CLIENT_IDENTIFIER,
            HeaderValue::from_str(&config.client_identifier)
                .map_err(|_| invalid("client identifier"))?,
        );
        headers.insert(X_PLEX_PRODUCT, HeaderValue::from_static("AnyPlayer"));
        Ok(Self {
            http,
            base,
            base_url,
            page_size: config.page_size.clamp(1, 1000) as usize,
            headers,
            retry: RetryPolicy::default(),
        })
    }

    #[must_use]
    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// `X-Plex-Token`, `X-Plex-Client-Identifier` and `X-Plex-Product`.
    pub fn auth_headers(&self) -> HeaderMap {
        self.headers.clone()
    }

    /// Universal-transcode MP3 URL for a library item.
    pub fn transcode_url(&self, rating_key: &str) -> String {
        format!(
            "{}/music/:/transcode/universal/start.mp3?path=/library/metadata/{rating_key}&mediaIndex=0&partIndex=0&protocol=http&directPlay=0&directStream=0",
            self.base
        )
    }

    fn server_url(&self, key: &str) -> String {
        strip_url_tokens(&format!("{}/{}", self.base, key.trim_start_matches('/')))
    }

    /// `{base}{Media[0].Part[0].key}`, or the transcode URL when `Media[0].audioCodec` is not
    /// locally decodable.
    fn stream_url(&self, rating_key: &str, row: &Value) -> Option<String> {
        let media = array(row, "Media").next()?;
        let codec = text(media, "audioCodec").map(|c| c.to_ascii_lowercase());
        if codec.is_some_and(|c| !c.is_empty() && !DIRECT_PLAY_CODECS.contains(&c.as_str())) {
            return Some(self.transcode_url(rating_key));
        }
        let key = array(media, "Part")
            .next()
            .and_then(|part| text(part, "key"))?;
        Some(self.server_url(&key))
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let mut url = parse_url(&self.server_url(path))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let request = self
            .http
            .get(url)
            .headers(self.auth_headers())
            .header(ACCEPT, "application/json")
            .build()?;
        let mut root = http::json(&self.http, request, self.retry).await?;
        Ok(root
            .get_mut("MediaContainer")
            .filter(|v| v.is_object())
            .map_or(Value::Null, Value::take))
    }

    /// Paged read with `X-Plex-Container-Start`/`-Size`: stops on an empty or short page, at
    /// `totalSize`, or after [`MAX_PAGES`].
    async fn pages(&self, path: &str, query: &[(&str, &str)]) -> Result<Vec<Value>> {
        let size = self.page_size.to_string();
        let mut offset = 0usize;
        let mut rows = Vec::new();
        for _ in 0..MAX_PAGES {
            let start = offset.to_string();
            let mut params = query.to_vec();
            params.extend([
                ("X-Plex-Container-Start", start.as_str()),
                ("X-Plex-Container-Size", size.as_str()),
            ]);
            let mut page = self.get(path, &params).await?;
            let count = raw_len(&page, "Metadata");
            offset += count;
            let total = number(&page, "totalSize").and_then(|t| usize::try_from(t).ok());
            if let Some(Value::Array(items)) = page.get_mut("Metadata").map(Value::take) {
                rows.extend(items.into_iter().filter(Value::is_object));
            }
            if count == 0 || count < self.page_size || total.is_some_and(|t| offset >= t) {
                break;
            }
        }
        Ok(rows)
    }

    fn image(&self, row: &Value) -> Option<String> {
        text(row, "thumb")
            .filter(|t| !t.is_empty())
            .map(|t| self.server_url(&t))
    }

    fn playlist(&self, row: &Value) -> Option<Playlist> {
        Some(Playlist {
            id: text(row, "ratingKey")?,
            name: text(row, "title").unwrap_or_default(),
            owner: "Plex".into(),
            track_count: number(row, "leafCount").and_then(to_i32).unwrap_or(0),
            source: SourceType::Plex,
            image_url: self.image(row),
            tracks: None,
            description: text(row, "summary"),
        })
    }

    fn track(&self, row: &Value) -> Option<Track> {
        let id = text(row, "ratingKey")?;
        let media = array(row, "Media").next();
        let sample_rate = media
            .and_then(|m| array(m, "Part").next())
            .and_then(|part| array(part, "Stream").find(|s| number(s, "streamType") == Some(2)))
            .and_then(|s| number(s, "samplingRate"));
        Some(Track {
            id: id.clone(),
            title: text(row, "title").unwrap_or_default(),
            artist: text(row, "grandparentTitle").unwrap_or_else(|| "Unknown Artist".into()),
            album: Some(text(row, "parentTitle").unwrap_or_else(|| "Unknown Album".into())),
            duration_ms: number(row, "duration").filter(|d| *d > 0),
            source: SourceType::Plex,
            url: self.stream_url(&id, row),
            image_url: self.image(row),
            bitrate_kbps: media.and_then(|m| number(m, "bitrate")).and_then(to_i32),
            sample_rate_hz: sample_rate.and_then(to_i32),
            enriched: Some(true),
            is_dj_filler: false,
        })
    }

    async fn search(&self, query: &str) -> Result<Vec<Value>> {
        let limit = SEARCH_LIMIT.to_string();
        let mut page = self
            .get("search", &[("query", query), ("limit", limit.as_str())])
            .await?;
        Ok(match page.get_mut("Metadata").map(Value::take) {
            Some(Value::Array(items)) => items,
            _ => Vec::new(),
        })
    }

    fn is_type(row: &Value, name: &str, code: i64) -> bool {
        text(row, "type").as_deref() == Some(name) || number(row, "type") == Some(code)
    }
}

#[async_trait]
impl ProviderClient for PlexClient {
    fn source(&self) -> SourceType {
        SourceType::Plex
    }

    /// `GET /identity`. Certificate failures surface as [`ProviderError::Tls`] so the UI can offer
    /// "Allow self-signed certificate".
    async fn validate(&self) -> Result<Validation> {
        match self.get("identity", &[]).await {
            Ok(_) => Ok(Validation::default()),
            Err(ProviderError::Http { status, .. }) => Err(ProviderError::Http {
                status,
                message: format!("Plex authentication failed: HTTP {status}"),
            }),
            Err(ProviderError::Tls(detail)) => Err(ProviderError::Tls(format!(
                "Plex server certificate is not trusted ({detail}). Enable \"Allow self-signed certificate\" for this connection only if you trust this server."
            ))),
            Err(ProviderError::Network(detail)) => Err(ProviderError::Network(format!(
                "Plex connection test failed: {detail}"
            ))),
            Err(other) => Err(other),
        }
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        let rows = self.pages("playlists/all", &[("type", "15")]).await?;
        Ok(rows.iter().filter_map(|row| self.playlist(row)).collect())
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let rows = self
            .pages(&format!("playlists/{}/items", playlist_id.trim()), &[])
            .await?;
        Ok(rows.iter().filter_map(|row| self.track(row)).collect())
    }

    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>> {
        if is_blank(query) {
            return Ok(Vec::new());
        }
        let rows = self.search(query).await?;
        Ok(rows
            .iter()
            .filter(|row| Self::is_type(row, "track", 10))
            .take(SEARCH_LIMIT)
            .filter_map(|row| self.track(row))
            .collect())
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        if is_blank(query) {
            return Ok(Vec::new());
        }
        let rows = self.search(query).await?;
        Ok(rows
            .iter()
            .filter(|row| Self::is_type(row, "playlist", 15))
            .take(SEARCH_LIMIT)
            .filter_map(|row| self.playlist(row))
            .collect())
    }

    /// The stored URL when it points at this server (legacy tokens stripped); otherwise the item's
    /// metadata is fetched to rebuild it.
    async fn stream_request(&self, track: &Track) -> Result<MediaRequest> {
        let stored = track
            .url
            .as_deref()
            .and_then(|raw| Url::parse(&strip_url_tokens(raw.trim())).ok())
            .filter(|url| same_origin(url, &self.base_url));
        let url = if let Some(url) = stored {
            url
        } else {
            let id = track.id.trim();
            let page = self.get(&format!("library/metadata/{id}"), &[]).await?;
            let row = array(&page, "Metadata")
                .next()
                .unwrap_or(obj(&page, "Metadata"));
            let built = self.stream_url(id, row).ok_or_else(|| {
                ProviderError::Decode(format!("Plex item {id} has no playable media"))
            })?;
            parse_url(&built)?
        };
        Ok(MediaRequest {
            url,
            headers: self.auth_headers(),
        })
    }

    fn image_request(&self, image_url: &str) -> Option<MediaRequest> {
        server_media_request(image_url, &self.base_url, || self.auth_headers())
    }
}
