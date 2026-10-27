//! Spotify OAuth: PKCE S256 + `state`, the authorize URL, and the Accounts token endpoint
//! (code exchange and refresh). Design §7 / §7.1, D10.

use std::collections::BTreeSet;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, TimeDelta, Utc};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rand::RngCore as _;
use reqwest::Client;
use reqwest::header::ACCEPT;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{ProviderError, Result};
use crate::http::{self, RetryPolicy};
use crate::json_ext::{number, text};

/// The app's Spotify client id (PKCE, no secret).
pub const CLIENT_ID: &str = "243bb6667db04143b6586d8598aed48b";
/// Loopback redirect registered for [`CLIENT_ID`].
pub const REDIRECT_URI: &str = "http://127.0.0.1:8989/login";
/// Android's scope set.
pub const SCOPES: &str = "playlist-read-private playlist-read-collaborative user-read-private user-read-email user-library-read user-read-playback-state user-modify-playback-state user-read-currently-playing";
pub const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
pub const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

/// RFC 3986 unreserved characters stay literal; everything else is percent-encoded (`%20`, not `+`).
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub(crate) fn encode_component(value: &str) -> String {
    utf8_percent_encode(value, COMPONENT).to_string()
}

/// Wall clock used for token expiry; injectable for tests.
pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// The system clock.
pub fn system_clock() -> Clock {
    Arc::new(Utc::now)
}

/// One sign-in attempt's secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkceSession {
    /// 64-char base64url code verifier.
    pub verifier: String,
    /// `BASE64URL(SHA256(verifier))`.
    pub challenge: String,
    /// Random `state` echoed by the callback.
    pub state: String,
}

impl PkceSession {
    /// A fresh verifier and state from the OS-seeded thread RNG.
    pub fn generate() -> Self {
        Self::from_parts(random_token(64), random_token(24))
    }

    pub fn from_parts(verifier: String, state: String) -> Self {
        let challenge = code_challenge(&verifier);
        Self {
            verifier,
            challenge,
            state,
        }
    }

    /// The browser URL for this attempt.
    pub fn authorize_url(&self) -> String {
        let params = [
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT_URI),
            ("scope", SCOPES),
            ("state", self.state.as_str()),
            ("code_challenge_method", "S256"),
            ("code_challenge", self.challenge.as_str()),
        ];
        let query: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{k}={}", encode_component(v)))
            .collect();
        format!("{AUTHORIZE_URL}?{}", query.join("&"))
    }
}

/// `BASE64URL-NOPAD(SHA256(ascii(verifier)))` (RFC 7636 S256).
pub fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// `length` base64url characters of randomness (Android `randomString`).
fn random_token(length: usize) -> String {
    let mut bytes = vec![0u8; length];
    rand::rng().fill_bytes(&mut bytes);
    let mut text = URL_SAFE_NO_PAD.encode(bytes);
    text.truncate(length);
    text
}

/// A Spotify token in the legacy rspotify `Token` JSON shape stored in the keyring entry
/// `spotify-token`: `{"access_token","expires_in"(s),"expires_at"(RFC 3339|null),"refresh_token"
/// (|null),"scope"(space-separated)}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotifyToken {
    pub access_token: String,
    /// Lifetime in seconds as granted.
    pub expires_in: i64,
    pub expires_at: Option<DateTime<Utc>>,
    pub refresh_token: Option<String>,
    #[serde(default, rename = "scope", with = "space_separated")]
    pub scopes: BTreeSet<String>,
}

impl SpotifyToken {
    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// True when the token expires before `now + margin`. Unknown expiry counts as valid.
    pub fn expires_within(&self, now: DateTime<Utc>, margin: TimeDelta) -> bool {
        self.expires_at.is_some_and(|at| at <= now + margin)
    }

    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_within(now, TimeDelta::zero())
    }
}

mod space_separated {
    use std::collections::BTreeSet;

    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        scopes: &BTreeSet<String>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        s.serialize_str(
            &scopes
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeSet<String>, D::Error> {
        let text = Option::<String>::deserialize(d)?.unwrap_or_default();
        Ok(text.split_whitespace().map(str::to_owned).collect())
    }
}

/// The Spotify Accounts token endpoint.
#[derive(Clone)]
pub struct SpotifyAccounts {
    http: Client,
    token_url: String,
    retry: RetryPolicy,
    clock: Clock,
}

impl std::fmt::Debug for SpotifyAccounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpotifyAccounts")
            .field("token_url", &self.token_url)
            .finish_non_exhaustive()
    }
}

impl SpotifyAccounts {
    pub fn new(http: Client) -> Self {
        Self {
            http,
            token_url: TOKEN_URL.into(),
            retry: RetryPolicy::default(),
            clock: system_clock(),
        }
    }

    #[must_use]
    pub fn with_token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = url.into();
        self
    }

    #[must_use]
    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    #[must_use]
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub fn now(&self) -> DateTime<Utc> {
        (self.clock)()
    }

    /// Exchanges an authorization code (PKCE) for a token.
    pub async fn exchange(&self, code: &str, verifier: &str) -> Result<SpotifyToken> {
        self.token(
            &[
                ("grant_type", "authorization_code"),
                ("client_id", CLIENT_ID),
                ("code", code),
                ("code_verifier", verifier),
                ("redirect_uri", REDIRECT_URI),
            ],
            "Unable to exchange Spotify authorization code",
        )
        .await
    }

    /// Refreshes a token. The returned token's `refresh_token` is `None` when Spotify did not
    /// rotate it; callers keep the previous one.
    pub async fn refresh(&self, refresh_token: &str) -> Result<SpotifyToken> {
        self.token(
            &[
                ("grant_type", "refresh_token"),
                ("client_id", CLIENT_ID),
                ("refresh_token", refresh_token),
            ],
            "Spotify token refresh failed",
        )
        .await
    }

    async fn token(&self, form: &[(&str, &str)], failure: &str) -> Result<SpotifyToken> {
        let body: Vec<String> = form
            .iter()
            .map(|(k, v)| format!("{k}={}", encode_component(v)))
            .collect();
        let request = self
            .http
            .post(&self.token_url)
            .header(ACCEPT, "application/json")
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body.join("&"))
            .build()?;
        let response = http::execute_retrying_with(&self.http, request, self.retry).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(ProviderError::Http {
                status: status.as_u16(),
                message: format!("{failure}: HTTP {}", status.as_u16()),
            });
        }
        let json = http::decode_json(response).await?;
        let access_token = text(&json, "access_token")
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| ProviderError::Decode(format!("{failure}: no access_token")))?;
        let expires_in = number(&json, "expires_in").unwrap_or(0);
        let expires_at = (expires_in > 0).then(|| self.now() + TimeDelta::seconds(expires_in));
        Ok(SpotifyToken {
            access_token,
            expires_in,
            expires_at,
            refresh_token: text(&json, "refresh_token").filter(|t| !t.is_empty()),
            scopes: text(&json, "scope")
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_matches_rfc7636_vector() {
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generated_session_has_expected_shape() {
        let s = PkceSession::generate();
        assert_eq!(s.verifier.len(), 64);
        assert_eq!(s.state.len(), 24);
        assert!(
            s.verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_ne!(PkceSession::generate().state, s.state);
        assert_eq!(s.challenge, code_challenge(&s.verifier));
    }

    #[test]
    fn authorize_url_carries_all_params() {
        let s = PkceSession::from_parts("v".repeat(64), "st/ate".into());
        let url = reqwest::Url::parse(&s.authorize_url()).unwrap();
        assert!(
            s.authorize_url()
                .contains("scope=playlist-read-private%20playlist-read-collaborative")
        );
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], CLIENT_ID);
        assert_eq!(q["redirect_uri"], REDIRECT_URI);
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["scope"], SCOPES);
        assert_eq!(q["state"], "st/ate");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["code_challenge"], code_challenge(&"v".repeat(64)));
    }

    #[test]
    fn token_round_trips_legacy_rspotify_json() {
        let legacy = r#"{"access_token":"A","expires_in":3600,"expires_at":"2024-05-01T12:00:00.123456789Z","refresh_token":"R","scope":"user-read-email playlist-read-private"}"#;
        let token = SpotifyToken::from_json(legacy).unwrap();
        assert_eq!(token.expires_in, 3600);
        assert_eq!(token.expires_at.unwrap().timestamp(), 1_714_564_800);
        assert_eq!(token.scopes.len(), 2);
        let again = SpotifyToken::from_json(&token.to_json()).unwrap();
        assert_eq!(again, token);
        let json: serde_json::Value = serde_json::from_str(&token.to_json()).unwrap();
        assert_eq!(json["scope"], "playlist-read-private user-read-email");
        assert_eq!(json["expires_at"], "2024-05-01T12:00:00.123456789Z");

        let minimal = SpotifyToken::from_json(
            r#"{"access_token":"A","expires_in":0,"expires_at":null,"refresh_token":null}"#,
        )
        .unwrap();
        assert!(minimal.scopes.is_empty() && minimal.expires_at.is_none());
        assert!(
            serde_json::to_string(&minimal)
                .unwrap()
                .contains(r#""refresh_token":null"#)
        );
    }
}
