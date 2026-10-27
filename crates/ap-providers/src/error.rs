//! Provider error type. `Display` texts are user-facing (legacy wording where one existed).

use std::error::Error as _;

/// A failed provider call.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The server answered with a non-2xx status.
    #[error("{message}")]
    Http { status: u16, message: String },
    /// The server's TLS certificate was rejected. For Plex the user may enable
    /// "Allow self-signed certificate" for this connection.
    #[error("{0}")]
    Tls(String),
    /// Connect/read failure, timeout, DNS, refused connection.
    #[error("{0}")]
    Network(String),
    /// The response body was not the JSON shape we expect.
    #[error("Invalid response from server: {0}")]
    Decode(String),
    /// A configured URL or credential is unusable.
    #[error("{0}")]
    InvalidConfig(String),
    /// No Jellyfin user could be resolved.
    #[error("No users found on Jellyfin server")]
    NoUsers,
    /// No Spotify account is linked.
    #[error("No Spotify account linked. Link Spotify in Settings.")]
    NotLinked,
    /// The Spotify token is expired and could not be refreshed.
    #[error("Spotify access token refresh failed. Reconnect Spotify and retry.")]
    TokenRefreshFailed,
    /// The operation does not exist for this provider.
    #[error("{0}")]
    Unsupported(&'static str),
}

impl ProviderError {
    /// HTTP status of an [`ProviderError::Http`] failure.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub(crate) fn http(status: reqwest::StatusCode) -> Self {
        Self::Http {
            status: status.as_u16(),
            message: format!("HTTP {}", status.as_u16()),
        }
    }

    /// Maps a transport error, recognising certificate failures anywhere in the source chain.
    pub(crate) fn from_transport(error: &reqwest::Error) -> Self {
        let text = error_chain_text(error);
        if is_tls_failure(&text) {
            Self::Tls(text)
        } else if error.is_timeout() {
            Self::Network(format!("Request timed out: {text}"))
        } else {
            Self::Network(text)
        }
    }
}

impl From<reqwest::Error> for ProviderError {
    fn from(error: reqwest::Error) -> Self {
        Self::from_transport(&error)
    }
}

pub(crate) fn error_chain_text(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// The legacy desktop's certificate-error heuristic.
pub(crate) fn is_tls_failure(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "certificate",
        "tls",
        "ssl",
        "unknown issuer",
        "unknownissuer",
        "self signed",
        "self-signed",
        "invalid peer certificate",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub type Result<T, E = ProviderError> = std::result::Result<T, E>;
