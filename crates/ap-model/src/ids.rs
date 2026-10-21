//! Identifier normalization: Spotify track/playlist ids and Java `UUID.nameUUIDFromBytes`.

use md5::{Digest, Md5};

use crate::enums::SourceType;
use crate::text::{hex_lower, kotlin_is_blank, kotlin_trim};

const SPOTIFY_TRACK_URI_PREFIX: &str = "spotify:track:";

/// Android `normalizeSpotifyTrackId`: strips a `spotify:track:` prefix, then trims.
#[must_use]
pub fn normalize_spotify_track_id(value: &str) -> &str {
    kotlin_trim(value.strip_prefix(SPOTIFY_TRACK_URI_PREFIX).unwrap_or(value))
}

/// Android `trackIdsMatch`.
#[must_use]
pub fn track_ids_match(left: &str, right: &str) -> bool {
    left == right || normalize_spotify_track_id(left) == normalize_spotify_track_id(right)
}

/// True when `value` normalizes to a base-62 Spotify track id (`^[A-Za-z0-9]{22}$`).
#[must_use]
pub fn is_valid_spotify_track_id(value: &str) -> bool {
    let id = normalize_spotify_track_id(value);
    id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Kotlin `Char.equals(other, ignoreCase = true)`.
fn kotlin_char_eq_ignore_case(a: char, b: char) -> bool {
    fn single(mut it: impl Iterator<Item = char>, fallback: char) -> char {
        match (it.next(), it.next()) {
            (Some(c), None) => c,
            _ => fallback,
        }
    }
    if a == b {
        return true;
    }
    let (ua, ub) = (single(a.to_uppercase(), a), single(b.to_uppercase(), b));
    ua == ub || single(ua.to_lowercase(), ua) == single(ub.to_lowercase(), ub)
}

fn starts_with_ignore_case(value: &str, prefix: &str) -> bool {
    let mut chars = value.chars();
    prefix
        .chars()
        .all(|p| chars.next().is_some_and(|c| kotlin_char_eq_ignore_case(c, p)))
}

fn substring_before(value: &str, delimiter: char) -> &str {
    value.split_once(delimiter).map_or(value, |(head, _)| head)
}

/// Android `normalizePlaylistId`: Spotify URIs and URLs reduce to the bare id; every other
/// source is only trimmed.
#[must_use]
pub fn normalize_playlist_id(source: SourceType, playlist_id: &str) -> &str {
    let trimmed = kotlin_trim(playlist_id);
    if source != SourceType::Spotify || kotlin_is_blank(trimmed) {
        return trimmed;
    }
    let rest = if starts_with_ignore_case(trimmed, "spotify:playlist:") {
        trimmed.rsplit_once(':').map_or(trimmed, |(_, tail)| tail)
    } else if let Some(index) = trimmed.find("/playlist/") {
        &trimmed[index + "/playlist/".len()..]
    } else {
        trimmed
    };
    substring_before(substring_before(rest, '?'), '/')
}

/// Java `UUID.nameUUIDFromBytes(name).toString()`: an MD5-based version 3 UUID.
#[must_use]
pub fn name_uuid_from_bytes(name: &[u8]) -> String {
    let mut hash: [u8; 16] = Md5::digest(name).into();
    hash[6] = (hash[6] & 0x0f) | 0x30;
    hash[8] = (hash[8] & 0x3f) | 0x80;
    let hex = hex_lower(&hash);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_uri_prefix_and_trims() {
        assert_eq!(
            normalize_spotify_track_id("spotify:track:4uLU6hMCjMI75M1A2tKUQC "),
            "4uLU6hMCjMI75M1A2tKUQC"
        );
        assert_eq!(normalize_spotify_track_id(" abc "), "abc");
    }

    #[test]
    fn track_ids_match_across_formats() {
        assert!(track_ids_match("spotify:track:4uLU6hMCjMI75M1A2tKUQC", "4uLU6hMCjMI75M1A2tKUQC"));
        assert!(track_ids_match("x", "x"));
        assert!(!track_ids_match("a", "b"));
    }

    #[test]
    fn spotify_track_id_validation() {
        assert!(is_valid_spotify_track_id(" 4uLU6hMCjMI75M1A2tKUQC "));
        assert!(!is_valid_spotify_track_id("4uLU6hMCjMI75M1A2tKUQ-"));
        assert!(!is_valid_spotify_track_id("4uLU6hMCjMI75M1A2tKUQCX"));
        assert!(!is_valid_spotify_track_id(""));
    }

    #[test]
    fn playlist_ids_normalize_like_android() {
        let s = SourceType::Spotify;
        assert_eq!(normalize_playlist_id(s, " Spotify:Playlist:abc?si=1 "), "abc");
        assert_eq!(normalize_playlist_id(s, "https://open.spotify.com/playlist/abc/x?si=1"), "abc");
        assert_eq!(normalize_playlist_id(s, "abc?si=2"), "abc");
        assert_eq!(normalize_playlist_id(SourceType::Plex, " a/b?c "), "a/b?c");
        assert_eq!(normalize_playlist_id(s, "  "), "");
    }

    #[test]
    fn name_uuid_matches_java() {
        // java.util.UUID.nameUUIDFromBytes("".getBytes()) and ("hello".getBytes()).
        assert_eq!(name_uuid_from_bytes(b""), "d41d8cd9-8f00-3204-a980-0998ecf8427e");
        assert_eq!(name_uuid_from_bytes(b"hello"), "5d41402a-bc4b-3a76-b971-9d911017c592");
    }
}
