//! The DJ's view of the committed sync account (Android `DjHttp.kt`).

use sha2::{Digest, Sha256};

/// Committed sync server base URL and auth token, normalized like Android
/// (`normalizeSyncServerBaseUrl` / `normalizeSyncServerAuthToken`). A blank base means
/// "no account": every DJ call becomes a no-op.
#[derive(Clone, PartialEq, Eq)]
pub struct DjAccount {
    base: String,
    token: String,
}

impl std::fmt::Debug for DjAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DjAccount")
            .field("base", &self.base)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl DjAccount {
    pub fn new(server_target: &str, auth_token: &str) -> Self {
        Self {
            base: normalize_base_url(server_target),
            token: normalize_auth_token(auth_token),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn is_configured(&self) -> bool {
        !self.base.trim().is_empty()
    }

    /// Persisted passage-state key: `"state_" + hex(sha256(base + "\0" + token))`, the same key
    /// Android stores under, so state is partitioned per account.
    pub fn state_key(&self) -> String {
        let digest = Sha256::digest(format!("{}\0{}", self.base, self.token).as_bytes());
        let mut key = String::with_capacity(6 + 64);
        key.push_str("state_");
        for byte in digest {
            key.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
            key.push(char::from_digit(u32::from(byte & 0xf), 16).unwrap_or('0'));
        }
        key
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.base)
    }

    /// Adds `Authorization: Bearer <token>` when a token is set.
    pub(crate) fn authorize(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.token.is_empty() {
            request
        } else {
            request.bearer_auth(&self.token)
        }
    }
}

fn normalize_base_url(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.trim().is_empty() {
        String::new()
    } else if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    }
}

fn normalize_auth_token(value: &str) -> String {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() > 6 && bytes[..6].eq_ignore_ascii_case(b"bearer") {
        let rest = &trimmed[6..];
        let stripped = rest.trim_start_matches([' ', '\t', '\n', '\u{b}', '\u{c}', '\r']);
        if stripped.len() < rest.len() {
            return stripped.to_owned();
        }
    }
    trimmed.to_owned()
}

/// Android `describeFailure` for HTTP status failures.
pub(crate) fn describe_status_failure(code: u16, not_configured: &str) -> String {
    match code {
        401 | 403 => {
            "Sync server rejected the auth token. Check the Sync Auth Token setting.".to_owned()
        }
        404 => not_configured.to_owned(),
        _ => format!("Sync server returned an error (HTTP {code})."),
    }
}

/// Android `describeFailure` for transport failures.
pub(crate) fn describe_transport_failure(error: &reqwest::Error) -> String {
    format!(
        "Could not reach the sync server. Check the Sync Server Target and your network connection. ({})",
        error_kind(error)
    )
}

/// Secret-free classification of a transport error for diagnostics (Android logs the
/// exception's simple class name).
pub(crate) fn error_kind(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "Timeout"
    } else if error.is_connect() {
        "Connect"
    } else if error.is_decode() || error.is_body() {
        "Body"
    } else if error.is_request() {
        "Request"
    } else {
        "Http"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_like_android() {
        let account = DjAccount::new("  sync.example.com/ ", " Bearer  abc ");
        assert_eq!(account.base(), "https://sync.example.com");
        assert_eq!(account.token(), "abc");
        assert_eq!(
            DjAccount::new("http://h:1//", "bearerX").base(),
            "http://h:1"
        );
        assert_eq!(DjAccount::new("x", "bearerX").token(), "bearerX");
        assert!(!DjAccount::new("  ", "t").is_configured());
    }

    #[test]
    fn state_key_is_sha256_of_base_nul_token() {
        let key = DjAccount::new("http://h", "t").state_key();
        let expected = Sha256::digest(b"http://h\0t")
            .iter()
            .fold(String::new(), |mut hex, b| {
                use std::fmt::Write;
                let _ = write!(hex, "{b:02x}");
                hex
            });
        assert_eq!(key, format!("state_{expected}"));
    }
}
