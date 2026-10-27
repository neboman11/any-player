//! Spotify Connect Web API (`me/player…`), a port of Android `SpotifyPlayerApi` used by the
//! Connect backend (design §8.2). Calls are not retried: the backend owns polling cadence.

use std::sync::Arc;

use ap_model::{RepeatMode, is_valid_spotify_track_id};
use async_trait::async_trait;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE};
use reqwest::{Client, Method, Response, StatusCode};
use serde_json::{Value, json};

use super::client::{API_BASE, api_url, send_authorized};
use super::normalize_track_id;
use super::token::SpotifyTokenProvider;
use crate::error::ProviderError;
use crate::http::{self, RetryPolicy};
use crate::json_ext::{boolean, number, obj, text};

/// User-facing Spotify messages (legacy desktop wording).
pub mod messages {
    pub const NO_ACCOUNT: &str = "No Spotify account linked. Link Spotify in Settings.";
    pub const TOKEN_REFRESH_FAILED: &str =
        "Spotify access token refresh failed. Reconnect Spotify and retry.";
    pub const NOT_INSTALLED: &str = "Spotify isn't installed. Install it, open it, and try again.";
    pub const NO_DEVICE: &str = "No active Spotify Connect device found. Open Spotify on this computer (or another device on the same account) and try again.";
    pub const PREMIUM_REQUIRED: &str = "Premium required for full Spotify playback via Connect";
    pub const NO_VALID_TRACKS: &str = "No valid Spotify track ids to play";

    /// `Spotify command failed: <action>[. <detail>]`.
    pub fn command_failed(action: &str, detail: Option<&str>) -> String {
        match detail.map(str::trim).filter(|d| !d.is_empty()) {
            Some(detail) => format!("Spotify command failed: {action}. {detail}"),
            None => format!("Spotify command failed: {action}"),
        }
    }
}

/// What went wrong, for callers that branch on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotifyErrorKind {
    NotLinked,
    TokenRefreshFailed,
    PremiumRequired,
    NoActiveDevice,
    InvalidRequest,
    /// Any other non-2xx answer.
    Rejected,
    /// Connect/read failure.
    Transport,
}

/// A failed player call. `message` is user-facing; `http_status` is the Web API status, `None`
/// for token and transport failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct SpotifyCommandError {
    pub kind: SpotifyErrorKind,
    pub message: String,
    pub http_status: Option<u16>,
}

impl SpotifyCommandError {
    fn new(kind: SpotifyErrorKind, message: impl Into<String>, http_status: Option<u16>) -> Self {
        Self {
            kind,
            message: message.into(),
            http_status,
        }
    }

    fn from_provider(action: &str, error: &ProviderError) -> Self {
        match error {
            ProviderError::NotLinked => {
                Self::new(SpotifyErrorKind::NotLinked, messages::NO_ACCOUNT, None)
            }
            ProviderError::TokenRefreshFailed => Self::new(
                SpotifyErrorKind::TokenRefreshFailed,
                messages::TOKEN_REFRESH_FAILED,
                None,
            ),
            other => Self::new(
                SpotifyErrorKind::Transport,
                messages::command_failed(action, Some(&other.to_string())),
                other.status(),
            ),
        }
    }

    /// Maps a rejected write. Spotify explains rejections as
    /// `{"error":{"status":403,"message":"Player command failed: …","reason":"PREMIUM_REQUIRED"}}`.
    fn rejected(action: &str, status: StatusCode, body: &[u8]) -> Self {
        let root: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let error = obj(&root, "error");
        let message = text(error, "message");
        let reason = text(error, "reason").unwrap_or_default();
        let lower = message.as_deref().unwrap_or_default().to_ascii_lowercase();
        let code = Some(status.as_u16());
        if reason == "PREMIUM_REQUIRED" || lower.contains("premium") {
            return Self::new(
                SpotifyErrorKind::PremiumRequired,
                messages::PREMIUM_REQUIRED,
                code,
            );
        }
        if reason == "NO_ACTIVE_DEVICE"
            || (status == StatusCode::NOT_FOUND && lower.contains("no active device"))
        {
            return Self::new(SpotifyErrorKind::NoActiveDevice, messages::NO_DEVICE, code);
        }
        let detail = match message {
            Some(m) if !m.trim().is_empty() => format!("{m} HTTP {}", status.as_u16()),
            _ => format!("HTTP {}", status.as_u16()),
        };
        Self::new(
            SpotifyErrorKind::Rejected,
            messages::command_failed(action, Some(&detail)),
            code,
        )
    }
}

/// One `GET me/player` poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotifyPlaybackSnapshot {
    pub is_playing: bool,
    pub progress_ms: i64,
    pub duration_ms: i64,
    /// `item.id`.
    pub track_id: Option<String>,
    /// `item.linked_from.id` (track relinking), when Spotify still reports it.
    pub linked_from_id: Option<String>,
    /// Device volume; 100 when unknown.
    pub volume_percent: i32,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
}

/// One entry of `GET me/player/devices`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotifyDevice {
    pub id: String,
    pub name: String,
    /// Spotify device type, e.g. `Computer`, `Smartphone`, `Speaker`.
    pub device_type: String,
    pub is_active: bool,
    pub is_restricted: bool,
    pub volume_percent: Option<i32>,
}

/// Spotify Connect remote control. Write calls fail with the legacy messages.
#[async_trait]
pub trait SpotifyPlayerApi: Send + Sync {
    /// `None` on 204, empty body or non-2xx ("nothing to report"). Fails on token or transport errors.
    async fn playback_state(&self) -> Result<Option<SpotifyPlaybackSnapshot>, SpotifyCommandError>;
    /// Empty on non-2xx. Fails on token or transport errors.
    async fn devices(&self) -> Result<Vec<SpotifyDevice>, SpotifyCommandError>;
    /// `PUT me/player/play[?device_id=]` with `{"uris":[…],"offset":{"position":n}[,"position_ms":p]}`.
    /// Ids may be bare, `spotify:track:` URIs or `open.spotify.com/track/` URLs.
    async fn start_playback(
        &self,
        device_id: Option<&str>,
        track_ids: &[String],
        offset: usize,
        position_ms: u64,
    ) -> Result<(), SpotifyCommandError>;
    /// `PUT me/player/play` with `{}`.
    async fn resume(&self, device_id: Option<&str>) -> Result<(), SpotifyCommandError>;
    async fn pause(&self, device_id: Option<&str>) -> Result<(), SpotifyCommandError>;
    async fn seek(
        &self,
        position_ms: u64,
        device_id: Option<&str>,
    ) -> Result<(), SpotifyCommandError>;
    /// Clamped to 0..=100. Pass the just-started device: right after `PUT play` Spotify has no
    /// active device yet and answers 404 to untargeted calls.
    async fn set_volume(
        &self,
        percent: i32,
        device_id: Option<&str>,
    ) -> Result<(), SpotifyCommandError>;
    async fn set_shuffle(
        &self,
        enabled: bool,
        device_id: Option<&str>,
    ) -> Result<(), SpotifyCommandError>;
    /// OFF → `off`, ONE → `track`, ALL → `context`.
    async fn set_repeat(
        &self,
        mode: RepeatMode,
        device_id: Option<&str>,
    ) -> Result<(), SpotifyCommandError>;
}

/// [`SpotifyPlayerApi`] over the Web API.
#[derive(Debug, Clone)]
pub struct WebSpotifyPlayerApi {
    http: Client,
    tokens: Arc<SpotifyTokenProvider>,
    api_base: String,
}

impl WebSpotifyPlayerApi {
    pub fn new(http: Client, tokens: Arc<SpotifyTokenProvider>) -> Self {
        Self {
            http,
            tokens,
            api_base: API_BASE.into(),
        }
    }

    #[must_use]
    pub fn with_api_base(mut self, base: impl Into<String>) -> Self {
        self.api_base = base.into();
        self
    }

    async fn send(
        &self,
        action: &str,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Response, SpotifyCommandError> {
        let url = api_url(&self.api_base, path, query)
            .map_err(|e| SpotifyCommandError::from_provider(action, &e))?;
        let body = body.map(Value::to_string);
        send_authorized(&self.http, &self.tokens, RetryPolicy::NONE, |token| {
            let mut request = self
                .http
                .request(method.clone(), url.clone())
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header(ACCEPT, "application/json");
            if method != Method::GET {
                let body = body.clone().unwrap_or_default();
                // Spotify answers 411 to a body-less PUT/POST that omits Content-Length, and
                // reqwest/hyper omit it for an empty body.
                request = request
                    .header(CONTENT_TYPE, "application/json")
                    .header(CONTENT_LENGTH, body.len())
                    .body(body);
            }
            request.build()
        })
        .await
        .map_err(|e| SpotifyCommandError::from_provider(action, &e))
    }

    async fn get_json(
        &self,
        action: &str,
        path: &str,
    ) -> Result<Option<Value>, SpotifyCommandError> {
        let response = self.send(action, Method::GET, path, &[], None).await?;
        if !response.status().is_success() || response.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        Ok(http::decode_json(response)
            .await
            .ok()
            .filter(Value::is_object))
    }

    async fn write(
        &self,
        action: &str,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<(), SpotifyCommandError> {
        let response = self.send(action, Method::PUT, path, query, body).await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let bytes = response.bytes().await.unwrap_or_default();
        Err(SpotifyCommandError::rejected(action, status, &bytes))
    }
}

fn with_device<'a>(
    mut query: Vec<(&'a str, &'a str)>,
    device_id: Option<&'a str>,
) -> Vec<(&'a str, &'a str)> {
    if let Some(id) = device_id.map(str::trim).filter(|id| !id.is_empty()) {
        query.push(("device_id", id));
    }
    query
}

fn parse_snapshot(root: &Value) -> SpotifyPlaybackSnapshot {
    let item = obj(root, "item");
    let device = obj(root, "device");
    SpotifyPlaybackSnapshot {
        is_playing: boolean(root, "is_playing").unwrap_or(false),
        progress_ms: number(root, "progress_ms").unwrap_or(0),
        duration_ms: number(item, "duration_ms").unwrap_or(0),
        track_id: item.get("id").and_then(Value::as_str).map(str::to_owned),
        linked_from_id: obj(item, "linked_from")
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        volume_percent: number(device, "volume_percent")
            .and_then(|v| i32::try_from(v).ok())
            .unwrap_or(100),
        shuffle: boolean(root, "shuffle_state").unwrap_or(false),
        repeat: match text(root, "repeat_state")
            .map(|s| s.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("track") => RepeatMode::One,
            Some("context") => RepeatMode::All,
            _ => RepeatMode::Off,
        },
        device_id: device.get("id").and_then(Value::as_str).map(str::to_owned),
        device_name: device
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

#[async_trait]
impl SpotifyPlayerApi for WebSpotifyPlayerApi {
    async fn playback_state(&self) -> Result<Option<SpotifyPlaybackSnapshot>, SpotifyCommandError> {
        Ok(self
            .get_json("playbackState", "me/player")
            .await?
            .as_ref()
            .map(parse_snapshot))
    }

    async fn devices(&self) -> Result<Vec<SpotifyDevice>, SpotifyCommandError> {
        let Some(root) = self.get_json("devices", "me/player/devices").await? else {
            return Ok(Vec::new());
        };
        Ok(root
            .get("devices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|d| {
                Some(SpotifyDevice {
                    id: d.get("id").and_then(Value::as_str)?.to_owned(),
                    name: d
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    device_type: d
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    is_active: boolean(d, "is_active").unwrap_or(false),
                    is_restricted: boolean(d, "is_restricted").unwrap_or(false),
                    volume_percent: number(d, "volume_percent").and_then(|v| i32::try_from(v).ok()),
                })
            })
            .collect())
    }

    async fn start_playback(
        &self,
        device_id: Option<&str>,
        track_ids: &[String],
        offset: usize,
        position_ms: u64,
    ) -> Result<(), SpotifyCommandError> {
        let uris: Vec<String> = track_ids
            .iter()
            .map(|id| normalize_track_id(id))
            .filter(|id| is_valid_spotify_track_id(id))
            .map(|id| format!("spotify:track:{id}"))
            .collect();
        if uris.is_empty() {
            return Err(SpotifyCommandError::new(
                SpotifyErrorKind::InvalidRequest,
                messages::NO_VALID_TRACKS,
                None,
            ));
        }
        let mut body = json!({ "uris": uris, "offset": { "position": offset } });
        if position_ms > 0 {
            body["position_ms"] = json!(position_ms);
        }
        self.write(
            "startQueue",
            "me/player/play",
            &with_device(Vec::new(), device_id),
            Some(&body),
        )
        .await
    }

    async fn resume(&self, device_id: Option<&str>) -> Result<(), SpotifyCommandError> {
        self.write(
            "play",
            "me/player/play",
            &with_device(Vec::new(), device_id),
            Some(&json!({})),
        )
        .await
    }

    async fn pause(&self, device_id: Option<&str>) -> Result<(), SpotifyCommandError> {
        self.write(
            "pause",
            "me/player/pause",
            &with_device(Vec::new(), device_id),
            None,
        )
        .await
    }

    async fn seek(
        &self,
        position_ms: u64,
        device_id: Option<&str>,
    ) -> Result<(), SpotifyCommandError> {
        let position = position_ms.to_string();
        self.write(
            "seek",
            "me/player/seek",
            &with_device(vec![("position_ms", &position)], device_id),
            None,
        )
        .await
    }

    async fn set_volume(
        &self,
        percent: i32,
        device_id: Option<&str>,
    ) -> Result<(), SpotifyCommandError> {
        let volume = percent.clamp(0, 100).to_string();
        self.write(
            "setVolume",
            "me/player/volume",
            &with_device(vec![("volume_percent", &volume)], device_id),
            None,
        )
        .await
    }

    async fn set_shuffle(
        &self,
        enabled: bool,
        device_id: Option<&str>,
    ) -> Result<(), SpotifyCommandError> {
        let state = if enabled { "true" } else { "false" };
        self.write(
            "setShuffle",
            "me/player/shuffle",
            &with_device(vec![("state", state)], device_id),
            None,
        )
        .await
    }

    async fn set_repeat(
        &self,
        mode: RepeatMode,
        device_id: Option<&str>,
    ) -> Result<(), SpotifyCommandError> {
        let state = match mode {
            RepeatMode::Off => "off",
            RepeatMode::One => "track",
            RepeatMode::All => "context",
        };
        self.write(
            "setRepeatMode",
            "me/player/repeat",
            &with_device(vec![("state", state)], device_id),
            None,
        )
        .await
    }
}
