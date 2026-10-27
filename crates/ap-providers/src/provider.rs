//! The [`ProviderClient`] trait and helpers shared by the Jellyfin, Plex and Spotify clients.

use ap_model::{Playlist, SourceType, Track};
use async_trait::async_trait;
use reqwest::Url;
use reqwest::header::HeaderMap;

use crate::error::{ProviderError, Result};

/// Result of [`ProviderClient::validate`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validation {
    /// Display name of the signed-in or resolved user, when known.
    pub username: Option<String>,
    /// Spotify only: `product == "premium"`.
    pub premium: Option<bool>,
    /// Jellyfin only: the resolved user id. The caller persists it as `providers.jellyfin.user_id`.
    pub jellyfin_user_id: Option<String>,
}

/// An HTTP request for audio or artwork. Credentials live only in `headers`, never in `url`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRequest {
    pub url: Url,
    pub headers: HeaderMap,
}

/// A provider catalog client bound to one connection (base URL + credentials). Rebuild the client
/// when the connection changes.
#[async_trait]
pub trait ProviderClient: Send + Sync {
    fn source(&self) -> SourceType;
    /// Checks the credentials against the server.
    async fn validate(&self) -> Result<Validation>;
    async fn playlists(&self) -> Result<Vec<Playlist>>;
    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>>;
    /// First page (up to 50) of matching tracks; a blank query returns nothing.
    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>>;
    /// First page (up to 50) of matching playlists; a blank query returns nothing.
    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>>;
    /// Where and how to fetch `track`'s audio; auth goes in headers only.
    async fn stream_request(&self, track: &Track) -> Result<MediaRequest>;
    /// Request for a stored artwork URL. Tokens embedded by legacy versions are stripped, and
    /// credentials are added as headers only when the URL points at this connection's server.
    /// `None` when `image_url` is not a valid absolute URL.
    fn image_request(&self, image_url: &str) -> Option<MediaRequest>;
}

/// Query parameters that carry provider credentials in legacy URLs.
const TOKEN_PARAMS: [&str; 4] = ["api_key", "apikey", "x-plex-token", "x-emby-token"];

/// Removes credential query parameters (`api_key`, `ApiKey`, `X-Plex-Token`, `X-Emby-Token`,
/// any case) from `url`, keeping every other byte (order, encoding, literal commas) unchanged.
pub fn strip_url_tokens(url: &str) -> String {
    let (without_fragment, fragment) = match url.split_once('#') {
        Some((head, tail)) => (head, Some(tail)),
        None => (url, None),
    };
    let Some((path, query)) = without_fragment.split_once('?') else {
        return url.to_owned();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            let key = pair.split_once('=').map_or(*pair, |(k, _)| k);
            let key = percent_encoding::percent_decode_str(key).decode_utf8_lossy();
            !TOKEN_PARAMS.iter().any(|t| key.eq_ignore_ascii_case(t))
        })
        .collect();
    let mut out = String::with_capacity(url.len());
    out.push_str(path);
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

/// Trims whitespace and trailing `/`, and requires an absolute http(s) URL with a host.
pub(crate) fn normalize_base_url(raw: &str, provider: &str) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    let invalid = || {
        ProviderError::InvalidConfig(format!(
            "Invalid {provider} URL. Include http:// or https:// (for example: http://127.0.0.1:{})",
            if provider == "Plex" { 32400 } else { 8096 }
        ))
    };
    let parsed = Url::parse(trimmed).map_err(|_| invalid())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(invalid());
    }
    Ok(trimmed.to_owned())
}

/// Parses a URL built by this crate.
pub(crate) fn parse_url(url: &str) -> Result<Url> {
    Url::parse(url).map_err(|e| ProviderError::InvalidConfig(format!("Invalid URL {url}: {e}")))
}

/// Scheme, host and effective port match.
pub(crate) fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// [`ProviderClient::image_request`] for header-authenticated servers.
pub(crate) fn server_media_request(
    raw: &str,
    base: &Url,
    auth: impl FnOnce() -> HeaderMap,
) -> Option<MediaRequest> {
    let url = Url::parse(&strip_url_tokens(raw.trim())).ok()?;
    let headers = if same_origin(&url, base) {
        auth()
    } else {
        HeaderMap::new()
    };
    Some(MediaRequest { url, headers })
}

/// True when `query` has nothing to search for.
pub(crate) fn is_blank(query: &str) -> bool {
    query.trim().is_empty()
}

/// Upper bound on paged reads (legacy `*_MAX_PLAYLIST_PAGES`).
pub(crate) const MAX_PAGES: usize = 1000;

/// Search result page size.
pub(crate) const SEARCH_LIMIT: usize = 50;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_token_params() {
        assert_eq!(
            strip_url_tokens("http://h/Items/1/Images/Primary?tag=abc&api_key=SECRET"),
            "http://h/Items/1/Images/Primary?tag=abc"
        );
        assert_eq!(strip_url_tokens("http://h/t?X-Plex-Token=S"), "http://h/t");
        assert_eq!(
            strip_url_tokens("http://h/a?Container=mp3,aac&ApiKey=S&x=1#frag"),
            "http://h/a?Container=mp3,aac&x=1#frag"
        );
        assert_eq!(
            strip_url_tokens("http://h/a?x-plex-token=S&b=2"),
            "http://h/a?b=2"
        );
        assert_eq!(strip_url_tokens("http://h/plain"), "http://h/plain");
    }

    #[test]
    fn base_url_normalization() {
        assert_eq!(
            normalize_base_url(" http://h:8096/ ", "Jellyfin").unwrap(),
            "http://h:8096"
        );
        assert!(normalize_base_url("h:32400", "Plex").is_err());
        assert!(normalize_base_url("ftp://h", "Plex").is_err());
        assert!(
            normalize_base_url("", "Plex")
                .unwrap_err()
                .to_string()
                .contains("http://127.0.0.1:32400")
        );
    }

    #[test]
    fn origin_comparison_uses_default_ports() {
        let a = Url::parse("https://h/x").unwrap();
        let b = Url::parse("https://h:443/y").unwrap();
        let c = Url::parse("http://h/x").unwrap();
        assert!(same_origin(&a, &b));
        assert!(!same_origin(&a, &c));
    }
}
