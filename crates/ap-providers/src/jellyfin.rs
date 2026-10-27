//! Jellyfin client (port of Android `JellyfinClient`, design §7).

use ap_model::{Playlist, SourceType, Track};
use async_trait::async_trait;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Url};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::error::{ProviderError, Result};
use crate::http::{self, RetryPolicy};
use crate::json_ext::{array, number, obj, raw_len, text, to_i32};
use crate::provider::{
    MAX_PAGES, MediaRequest, ProviderClient, SEARCH_LIMIT, Validation, is_blank,
    normalize_base_url, parse_url, server_media_request,
};

/// Default `providers.jellyfin.page_size`.
pub const DEFAULT_PAGE_SIZE: u32 = 300;

/// Containers requested from `/Audio/{id}/universal`: only formats the local decoder handles, so
/// Opus/WebM sources are transcoded by the server.
pub const STREAM_CONTAINERS: &str = "mp3,aac,m4a,flac,ogg,wav";
/// Codecs requested from `/Audio/{id}/universal`.
pub const STREAM_CODECS: &str = "aac,mp3,vorbis,flac";

const X_EMBY_TOKEN: HeaderName = HeaderName::from_static("x-emby-token");
const X_EMBY_AUTHORIZATION: HeaderName = HeaderName::from_static("x-emby-authorization");

/// One Jellyfin connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JellyfinConfig {
    pub base_url: String,
    pub api_key: String,
    /// Persisted `providers.jellyfin.user_id`; `None` resolves the first user.
    pub user_id: Option<String>,
    /// Clamped to 1..=1000.
    pub page_size: u32,
    /// Install id, sent as `DeviceId`.
    pub device_id: String,
    /// App version, sent as `Version`.
    pub app_version: String,
}

/// The user all reads run as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JellyfinUser {
    pub id: String,
    pub name: Option<String>,
}

/// Jellyfin client bound to one connection.
#[derive(Debug)]
pub struct JellyfinClient {
    http: Client,
    base: String,
    base_url: Url,
    preferred_user: Option<String>,
    page_size: usize,
    headers: HeaderMap,
    retry: RetryPolicy,
    user: Mutex<Option<JellyfinUser>>,
}

impl JellyfinClient {
    pub fn new(http: Client, config: JellyfinConfig) -> Result<Self> {
        let base = normalize_base_url(&config.base_url, "Jellyfin")?;
        let base_url = parse_url(&base)?;
        let api_key = config.api_key.trim();
        let authorization = format!(
            "MediaBrowser Token=\"{api_key}\", Client=\"AnyPlayer\", Device=\"AnyPlayer\", DeviceId=\"{}\", Version=\"{}\"",
            config.device_id, config.app_version
        );
        let invalid = || ProviderError::InvalidConfig("Invalid Jellyfin API key".into());
        let mut token = HeaderValue::from_str(api_key).map_err(|_| invalid())?;
        token.set_sensitive(true);
        let mut authorization = HeaderValue::from_str(&authorization).map_err(|_| invalid())?;
        authorization.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(X_EMBY_TOKEN, token);
        headers.insert(X_EMBY_AUTHORIZATION, authorization.clone());
        // Servers with legacy authorization disabled only read the standard header.
        headers.insert(AUTHORIZATION, authorization);
        let preferred_user = config
            .user_id
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty());
        Ok(Self {
            http,
            base,
            base_url,
            preferred_user,
            page_size: config.page_size.clamp(1, 1000) as usize,
            headers,
            retry: RetryPolicy::default(),
            user: Mutex::new(None),
        })
    }

    #[must_use]
    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// `X-Emby-Token`, `X-Emby-Authorization` and `Authorization` (all `MediaBrowser Token=…`).
    pub fn auth_headers(&self) -> HeaderMap {
        self.headers.clone()
    }

    /// The stored user if the server still has it, else the first user. Cached for the client's
    /// lifetime; the caller persists `id` when it differs from the configured `user_id`.
    pub async fn resolve_user(&self) -> Result<JellyfinUser> {
        let mut cached = self.user.lock().await;
        if let Some(user) = cached.as_ref() {
            return Ok(user.clone());
        }
        let users = self.get("Users", &[]).await?;
        let users: Vec<&Value> = users
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v.is_object())
            .collect();
        let selected = self
            .preferred_user
            .as_deref()
            .and_then(|preferred| {
                users
                    .iter()
                    .find(|u| text(u, "Id").as_deref() == Some(preferred))
            })
            .or_else(|| users.first())
            .ok_or(ProviderError::NoUsers)?;
        let id = text(selected, "Id")
            .filter(|id| !id.is_empty())
            .ok_or(ProviderError::NoUsers)?;
        let user = JellyfinUser {
            id,
            name: text(selected, "Name"),
        };
        *cached = Some(user.clone());
        Ok(user)
    }

    /// Legacy-style persisted stream URL (plain concatenation, literal commas, no credentials).
    pub fn stream_url(&self, item_id: &str, user_id: &str) -> String {
        format!(
            "{}/Audio/{item_id}/universal?UserId={user_id}&Container={STREAM_CONTAINERS}&AudioCodec={STREAM_CODECS}&TranscodingContainer=mp3&TranscodingProtocol=http",
            self.base
        )
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let mut url = parse_url(&format!("{}/{path}", self.base))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let request = self
            .http
            .get(url)
            .headers(self.auth_headers())
            .header(ACCEPT, "application/json")
            .build()?;
        http::json(&self.http, request, self.retry).await
    }

    /// Paged `Users/{uid}/Items` read: stops on an empty or short page, at `TotalRecordCount`,
    /// or after [`MAX_PAGES`].
    async fn items(&self, uid: &str, query: &[(&str, &str)]) -> Result<Vec<Value>> {
        let path = format!("Users/{uid}/Items");
        let limit = self.page_size.to_string();
        let mut offset = 0usize;
        let mut rows = Vec::new();
        for _ in 0..MAX_PAGES {
            let start = offset.to_string();
            let mut params = query.to_vec();
            params.extend([("Limit", limit.as_str()), ("StartIndex", start.as_str())]);
            let mut page = self.get(&path, &params).await?;
            let count = raw_len(&page, "Items");
            offset += count;
            let total = number(&page, "TotalRecordCount").and_then(|t| usize::try_from(t).ok());
            if let Some(Value::Array(items)) = page.get_mut("Items").map(Value::take) {
                rows.extend(items.into_iter().filter(Value::is_object));
            }
            if count == 0 || count < self.page_size || total.is_some_and(|t| offset >= t) {
                break;
            }
        }
        Ok(rows)
    }

    fn image(&self, row: &Value) -> Option<String> {
        let album = text(row, "AlbumId").filter(|s| !s.trim().is_empty());
        let album_tag = text(row, "AlbumPrimaryImageTag").filter(|s| !s.trim().is_empty());
        let (id, tag) = match (album, album_tag) {
            (Some(album), Some(tag)) => (album, tag),
            _ => (text(row, "Id")?, text(obj(row, "ImageTags"), "Primary")?),
        };
        Some(format!("{}/Items/{id}/Images/Primary?tag={tag}", self.base))
    }

    fn playlist(&self, row: &Value) -> Option<Playlist> {
        let id = text(row, "Id")?;
        let count = number(row, "ChildCount")
            .or_else(|| number(row, "RecursiveItemCount"))
            .unwrap_or(0);
        Some(Playlist {
            id,
            name: text(row, "Name").unwrap_or_default(),
            owner: "Jellyfin".into(),
            track_count: to_i32(count).unwrap_or(0),
            source: SourceType::Jellyfin,
            image_url: self.image(row),
            tracks: None,
            description: None,
        })
    }

    fn track(&self, uid: &str, row: &Value) -> Option<Track> {
        let id = text(row, "Id")?;
        let media = array(row, "MediaSources").next();
        let stream = media
            .into_iter()
            .flat_map(|m| array(m, "MediaStreams"))
            .chain(array(row, "MediaStreams"))
            .find(|s| text(s, "Type").as_deref() == Some("Audio"));
        let artists: Vec<&str> = row
            .get("Artists")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let artist = artists.join(", ");
        let bitrate = stream
            .and_then(|s| number(s, "BitRate"))
            .or_else(|| media.and_then(|m| number(m, "Bitrate")));
        Some(Track {
            title: text(row, "Name").unwrap_or_default(),
            artist: if artist.trim().is_empty() {
                "Unknown Artist".into()
            } else {
                artist
            },
            album: Some(text(row, "Album").unwrap_or_else(|| "Unknown Album".into())),
            duration_ms: number(row, "RunTimeTicks")
                .map(|ticks| ticks / 10_000)
                .filter(|ms| *ms > 0),
            url: Some(self.stream_url(&id, uid)),
            image_url: self.image(row),
            bitrate_kbps: bitrate.and_then(|b| to_i32(b / 1000)),
            sample_rate_hz: stream
                .and_then(|s| number(s, "SampleRate"))
                .and_then(to_i32),
            enriched: Some(true),
            ..Track::new(id, "", "", SourceType::Jellyfin)
        })
    }

    async fn search(&self, query: &str, item_type: &str) -> Result<(String, Vec<Value>)> {
        let uid = self.resolve_user().await?.id;
        let limit = SEARCH_LIMIT.to_string();
        let mut params = vec![
            ("searchTerm", query),
            ("Recursive", "true"),
            ("IncludeItemTypes", item_type),
        ];
        if item_type == "Audio" {
            params.push(("Fields", "AudioInfo,MediaSources"));
        }
        params.extend([("Limit", limit.as_str()), ("StartIndex", "0")]);
        let page = self.get(&format!("Users/{uid}/Items"), &params).await?;
        let rows = array(&page, "Items").take(SEARCH_LIMIT).cloned().collect();
        Ok((uid, rows))
    }
}

#[async_trait]
impl ProviderClient for JellyfinClient {
    fn source(&self) -> SourceType {
        SourceType::Jellyfin
    }

    /// `GET /System/Info`, then user resolution.
    async fn validate(&self) -> Result<Validation> {
        self.get("System/Info", &[]).await?;
        let user = self.resolve_user().await?;
        Ok(Validation {
            username: user.name,
            premium: None,
            jellyfin_user_id: Some(user.id),
        })
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        let uid = self.resolve_user().await?.id;
        let rows = self
            .items(
                &uid,
                &[
                    ("Filters", "IsFolder"),
                    ("Recursive", "true"),
                    ("IncludeItemTypes", "Playlist"),
                ],
            )
            .await?;
        Ok(rows.iter().filter_map(|row| self.playlist(row)).collect())
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let uid = self.resolve_user().await?.id;
        let rows = self
            .items(
                &uid,
                &[
                    ("ParentId", playlist_id.trim()),
                    ("Fields", "AudioInfo,MediaSources,ParentId"),
                ],
            )
            .await?;
        Ok(rows
            .iter()
            .filter(|row| text(row, "Type").as_deref() == Some("Audio"))
            .filter_map(|row| self.track(&uid, row))
            .collect())
    }

    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>> {
        if is_blank(query) {
            return Ok(Vec::new());
        }
        let (uid, rows) = self.search(query, "Audio").await?;
        Ok(rows
            .iter()
            .filter_map(|row| self.track(&uid, row))
            .collect())
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        if is_blank(query) {
            return Ok(Vec::new());
        }
        let (_, rows) = self.search(query, "Playlist").await?;
        Ok(rows.iter().filter_map(|row| self.playlist(row)).collect())
    }

    /// Always rebuilt from the item id so persisted legacy URLs (wider codec lists, embedded
    /// `api_key`) are never replayed.
    async fn stream_request(&self, track: &Track) -> Result<MediaRequest> {
        let uid = self.resolve_user().await?.id;
        let url = parse_url(&self.stream_url(track.id.trim(), &uid))?;
        Ok(MediaRequest {
            url,
            headers: self.auth_headers(),
        })
    }

    fn image_request(&self, image_url: &str) -> Option<MediaRequest> {
        server_media_request(image_url, &self.base_url, || self.auth_headers())
    }
}
