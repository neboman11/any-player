//! Spotify: OAuth (PKCE), token provider, catalog client and Connect player API.

pub mod auth;
pub mod client;
pub mod player;
pub mod token;

pub use auth::{
    AUTHORIZE_URL, CLIENT_ID, Clock, PkceSession, REDIRECT_URI, SCOPES, SpotifyAccounts,
    SpotifyToken, TOKEN_URL, code_challenge, system_clock,
};
pub use client::{API_BASE, SpotifyClient};
pub use player::{
    SpotifyCommandError, SpotifyDevice, SpotifyErrorKind, SpotifyPlaybackSnapshot,
    SpotifyPlayerApi, WebSpotifyPlayerApi, messages,
};
pub use token::{REFRESH_MARGIN, SpotifyTokenProvider, TokenSink};

/// Normalizes a track id from a bare id, a `spotify:track:` URI, or an
/// `https://open.spotify.com/track/<id>?si=…` URL. Other input is only trimmed.
pub fn normalize_track_id(value: &str) -> &str {
    let trimmed = value.trim();
    if let Some(index) = trimmed.find("/track/") {
        let rest = &trimmed[index + "/track/".len()..];
        let end = rest.find(['?', '/', '#']).unwrap_or(rest.len());
        return &rest[..end];
    }
    ap_model::normalize_spotify_track_id(trimmed)
}

#[cfg(test)]
mod tests {
    use super::normalize_track_id;

    #[test]
    fn track_ids_normalize_from_all_forms() {
        let id = "4uLU6hMCjMI75M1A2tKUQC";
        assert_eq!(normalize_track_id(id), id);
        assert_eq!(
            normalize_track_id(" spotify:track:4uLU6hMCjMI75M1A2tKUQC "),
            id
        );
        assert_eq!(
            normalize_track_id("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC?si=abc"),
            id
        );
        assert_eq!(
            normalize_track_id("https://open.spotify.com/intl-de/track/4uLU6hMCjMI75M1A2tKUQC"),
            id
        );
    }
}
