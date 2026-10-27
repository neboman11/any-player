//! Spotify Web API catalog client (port of Android `SpotifyClient`, design §7), adapted to the
//! February 2026 Web API: `playlists/{id}/items` with `item` entries and search pages of ≤ 10.

use std::sync::Arc;

use ap_model::{Playlist, SourceType, Track, normalize_playlist_id};
use async_trait::async_trait;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use reqwest::{Client, Method, Request, Response, StatusCode, Url};
use serde_json::Value;

use super::normalize_track_id;
use super::token::SpotifyTokenProvider;
use crate::error::{ProviderError, Result};
use crate::http::{self, RetryPolicy};
use crate::json_ext::{array, number, obj, raw_len, text};
use crate::provider::{
    MAX_PAGES, MediaRequest, ProviderClient, SEARCH_LIMIT, Validation, is_blank, parse_url,
};

/// Production Web API base.
pub const API_BASE: &str = "https://api.spotify.com/v1/";

const PLAYLISTS_PAGE: usize = 50;
const PLAYLIST_ITEMS_PAGE: usize = 100;
/// Search `limit` maximum since February 2026.
const SEARCH_PAGE: usize = 10;

/// Sends a bearer-authorized request; a 401 triggers one forced refresh and one retry.
pub(crate) async fn send_authorized(
    http: &Client,
    tokens: &SpotifyTokenProvider,
    retry: RetryPolicy,
    build: impl Fn(&str) -> reqwest::Result<Request>,
) -> Result<Response> {
    let token = tokens.access_token().await?;
    let response = http::execute_retrying_with(http, build(&token)?, retry).await?;
    if response.status() != StatusCode::UNAUTHORIZED {
        return Ok(response);
    }
    drop(response);
    let refreshed = tokens.refresh_after_unauthorized(&token).await?;
    Ok(http::execute_retrying_with(http, build(&refreshed)?, retry).await?)
}

pub(crate) fn api_url(base: &str, path: &str, query: &[(&str, &str)]) -> Result<Url> {
    let mut url = parse_url(&format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    ))?;
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    Ok(url)
}

/// Spotify catalog client.
#[derive(Debug, Clone)]
pub struct SpotifyClient {
    http: Client,
    tokens: Arc<SpotifyTokenProvider>,
    api_base: String,
    retry: RetryPolicy,
}

impl SpotifyClient {
    pub fn new(http: Client, tokens: Arc<SpotifyTokenProvider>) -> Self {
        Self {
            http,
            tokens,
            api_base: API_BASE.into(),
            retry: RetryPolicy::default(),
        }
    }

    #[must_use]
    pub fn with_api_base(mut self, base: impl Into<String>) -> Self {
        self.api_base = base.into();
        self
    }

    #[must_use]
    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn tokens(&self) -> &Arc<SpotifyTokenProvider> {
        &self.tokens
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let url = api_url(&self.api_base, path, query)?;
        let response = send_authorized(&self.http, &self.tokens, self.retry, |token| {
            self.http
                .request(Method::GET, url.clone())
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header(ACCEPT, "application/json")
                .build()
        })
        .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(ProviderError::http(status));
        }
        http::decode_json(response).await
    }

    /// Offset paging: stops at `total`, on an empty page, on a short last page, or after
    /// [`MAX_PAGES`]. `rows` extracts the page object holding `items`/`total`/`next`.
    async fn paged(
        &self,
        path: &str,
        query: &[(&str, &str)],
        limit: usize,
        max_rows: usize,
        mut rows: impl FnMut(&Value) -> &Value,
    ) -> Result<Vec<Value>> {
        let size = limit.to_string();
        let mut offset = 0usize;
        let mut out = Vec::new();
        for _ in 0..MAX_PAGES {
            let start = offset.to_string();
            let mut params = query.to_vec();
            params.extend([("limit", size.as_str()), ("offset", start.as_str())]);
            let root = self.get(path, &params).await?;
            let page = rows(&root);
            let count = raw_len(page, "items");
            out.extend(array(page, "items").cloned());
            offset += limit;
            let total = number(page, "total").and_then(|t| usize::try_from(t).ok());
            let no_next = page.get("next").is_none_or(Value::is_null);
            if count == 0
                || (no_next && (count < limit || total.is_none()))
                || total.is_some_and(|t| offset >= t)
                || out.len() >= max_rows
            {
                break;
            }
        }
        Ok(out)
    }

    async fn search(&self, query: &str, kind: &str) -> Result<Vec<Value>> {
        let mut params = vec![("q", query), ("type", kind)];
        if kind == "track" {
            params.push(("market", "from_token"));
        }
        let key = format!("{kind}s");
        let mut rows = self
            .paged("search", &params, SEARCH_PAGE, SEARCH_LIMIT, |root| {
                obj(root, &key)
            })
            .await?;
        rows.truncate(SEARCH_LIMIT);
        Ok(rows)
    }
}

/// Album/playlist image closest to 300 px wide, else the first.
fn image(row: &Value) -> Option<String> {
    let images: Vec<&Value> = array(row, "images").collect();
    images
        .iter()
        .filter_map(|i| number(i, "width").map(|w| ((w - 300).abs(), *i)))
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, i)| i)
        .or_else(|| images.first().copied())
        .and_then(|i| text(i, "url"))
}

pub(crate) fn map_playlist(row: &Value) -> Option<Playlist> {
    let id = text(row, "id")?;
    let count = number(obj(row, "items"), "total").or_else(|| number(obj(row, "tracks"), "total"));
    Some(Playlist {
        id,
        name: text(row, "name").unwrap_or_default(),
        owner: text(obj(row, "owner"), "display_name").unwrap_or_else(|| "Spotify".into()),
        track_count: count.and_then(|t| i32::try_from(t).ok()).unwrap_or(0),
        source: SourceType::Spotify,
        image_url: image(row),
        tracks: None,
        description: text(row, "description"),
    })
}

pub(crate) fn map_track(row: &Value) -> Option<Track> {
    if text(row, "type").is_some_and(|t| t != "track")
        || row.get("episode").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let id = text(row, "id").filter(|id| !id.is_empty())?;
    let artists: Vec<String> = array(row, "artists")
        .filter_map(|a| text(a, "name"))
        .collect();
    let artist = artists.join(", ");
    Some(Track {
        url: Some(format!("spotify:track:{id}")),
        title: text(row, "name").unwrap_or_default(),
        artist: if artist.trim().is_empty() {
            "Unknown Artist".into()
        } else {
            artist
        },
        album: text(obj(row, "album"), "name"),
        duration_ms: number(row, "duration_ms"),
        image_url: image(obj(row, "album")),
        enriched: Some(true),
        ..Track::new(id, "", "", SourceType::Spotify)
    })
}

#[async_trait]
impl ProviderClient for SpotifyClient {
    fn source(&self) -> SourceType {
        SourceType::Spotify
    }

    /// `GET me`. `premium` is `None` when Spotify omits `product` (removed for development-mode
    /// apps in February 2026).
    async fn validate(&self) -> Result<Validation> {
        let me = self.get("me", &[]).await.map_err(|e| match e {
            ProviderError::Http { status, .. } => ProviderError::Http {
                status,
                message: "Spotify token validation failed".into(),
            },
            other => other,
        })?;
        Ok(Validation {
            username: text(&me, "display_name")
                .filter(|n| !n.is_empty())
                .or_else(|| text(&me, "id")),
            premium: text(&me, "product").map(|p| p == "premium"),
            jellyfin_user_id: None,
        })
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        let rows = self
            .paged("me/playlists", &[], PLAYLISTS_PAGE, usize::MAX, |root| root)
            .await?;
        Ok(rows.iter().filter_map(map_playlist).collect())
    }

    /// `playlists/{id}/items`; each entry's `item` (legacy `track`) is mapped, episodes and local
    /// files are skipped.
    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let id = normalize_playlist_id(SourceType::Spotify, playlist_id);
        let rows = self
            .paged(
                &format!("playlists/{id}/items"),
                &[("market", "from_token")],
                PLAYLIST_ITEMS_PAGE,
                usize::MAX,
                |root| root,
            )
            .await?;
        Ok(rows
            .iter()
            .filter_map(|entry| {
                let item = entry
                    .get("item")
                    .filter(|v| v.is_object())
                    .or_else(|| entry.get("track"))?;
                map_track(item)
            })
            .collect())
    }

    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>> {
        if is_blank(query) {
            return Ok(Vec::new());
        }
        Ok(self
            .search(query, "track")
            .await?
            .iter()
            .filter_map(map_track)
            .collect())
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        if is_blank(query) {
            return Ok(Vec::new());
        }
        Ok(self
            .search(query, "playlist")
            .await?
            .iter()
            .filter_map(map_playlist)
            .collect())
    }

    /// Spotify audio is never fetched; it plays through Spotify Connect.
    async fn stream_request(&self, _track: &Track) -> Result<MediaRequest> {
        Err(ProviderError::Unsupported(
            "Spotify tracks play through Spotify Connect",
        ))
    }

    /// Spotify artwork is public CDN content: no credentials.
    fn image_request(&self, image_url: &str) -> Option<MediaRequest> {
        Some(MediaRequest {
            url: Url::parse(image_url.trim()).ok()?,
            headers: reqwest::header::HeaderMap::new(),
        })
    }
}

impl SpotifyClient {
    /// `GET tracks/{id}` for a bare id, `spotify:track:` URI or `open.spotify.com/track/` URL.
    pub async fn track(&self, track_id: &str) -> Result<Option<Track>> {
        let id = normalize_track_id(track_id);
        if id.is_empty() {
            return Ok(None);
        }
        let row = self
            .get(&format!("tracks/{id}"), &[("market", "from_token")])
            .await?;
        Ok(map_track(&row))
    }
}
