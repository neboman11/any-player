//! Ports of Android `:core:model` `Models.kt`.
//!
//! Field order is Kotlin declaration order; fields equal to their Kotlin default and `None`s are
//! omitted on output (AppJson: `encodeDefaults = false`, `explicitNulls = false`). Unknown keys
//! are ignored on input.

use serde::{Deserialize, Serialize};

use crate::enums::{PlaybackStateType, PlaylistType, RepeatMode, SourceType};

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if signature"
)]
pub(crate) fn is_false(v: &bool) -> bool {
    !*v
}

/// A playable song from any provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "duration_ms",
        alias = "duration"
    )]
    pub duration_ms: Option<i64>,
    pub source: SourceType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "image_url")]
    pub image_url: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "bitrate_kbps"
    )]
    pub bitrate_kbps: Option<i32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "sample_rate_hz"
    )]
    pub sample_rate_hz: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enriched: Option<bool>,
    /// True for the synthetic AI DJ voice-over track. Never persisted as a real song.
    #[serde(default, skip_serializing_if = "is_false", alias = "is_dj_filler")]
    pub is_dj_filler: bool,
}

impl Track {
    /// A track with only the required fields set.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        artist: impl Into<String>,
        source: SourceType,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            artist: artist.into(),
            album: None,
            duration_ms: None,
            source,
            url: None,
            image_url: None,
            bitrate_kbps: None,
            sample_rate_hz: None,
            enriched: None,
            is_dj_filler: false,
        }
    }

    /// Android `sameTrack`: identity is `(source, id)`.
    #[must_use]
    pub fn same_track(&self, other: &Self) -> bool {
        self.id == other.id && self.source == other.source
    }
}

/// A provider playlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub track_count: i32,
    pub source: SourceType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracks: Option<Vec<Track>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A user playlist. Timestamps are ISO-8601 instants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomPlaylist {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub track_count: i32,
    pub playlist_type: PlaylistType,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_distinct: bool,
}

/// A row of a standard custom playlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistTrack {
    pub id: String,
    pub playlist_id: String,
    pub track_source: SourceType,
    pub track_id: String,
    pub position: i32,
    pub added_at: String,
    pub title: String,
    pub artist: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// A source playlist of a union playlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnionPlaylistSource {
    pub id: String,
    pub union_playlist_id: String,
    pub source_type: SourceType,
    pub source_playlist_id: String,
    pub position: i32,
    pub added_at: String,
}

/// Legacy sync/export transport shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackStatus {
    pub state: PlaybackStateType,
    pub shuffle: bool,
    pub repeat_mode: RepeatMode,
    pub volume: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_track: Option<Track>,
    pub position: i64,
    pub duration: i64,
    pub queue: Vec<Track>,
    /// Tracks in actual playback order; equals `queue` when shuffle is off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ordered_queue: Vec<Track>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Export wire format only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnPreferences {
    pub key: String,
    pub visible: bool,
    pub position: i32,
}

/// Provider connection summary shown in settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionProfile {
    pub source: SourceType,
    pub connected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_token: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_premium: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_ready: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_encodes_legacy_shape_without_defaults_or_nulls() {
        let t = Track::new("1", "T", "A", SourceType::Jellyfin);
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            r#"{"id":"1","title":"T","artist":"A","source":"jellyfin"}"#
        );
    }

    #[test]
    fn track_full_field_order_matches_kotlin() {
        let t = Track {
            album: Some("Al".into()),
            duration_ms: Some(1000),
            url: Some("http://x".into()),
            image_url: Some("i".into()),
            bitrate_kbps: Some(320),
            sample_rate_hz: Some(44100),
            enriched: Some(true),
            is_dj_filler: true,
            ..Track::new("1", "T", "A", SourceType::Plex)
        };
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            r#"{"id":"1","title":"T","artist":"A","album":"Al","durationMs":1000,"source":"plex","url":"http://x","imageUrl":"i","bitrateKbps":320,"sampleRateHz":44100,"enriched":true,"isDjFiller":true}"#
        );
    }

    #[test]
    fn track_decode_ignores_unknown_keys() {
        let t: Track = serde_json::from_str(
            r#"{"id":"1","title":"T","artist":"A","source":"spotify","extra":1}"#,
        )
        .unwrap();
        assert_eq!(t.source, SourceType::Spotify);
        assert!(!t.is_dj_filler);
    }

    #[test]
    fn track_decode_accepts_legacy_aliases() {
        let t: Track = serde_json::from_str(
            r#"{"id":"1","title":"T","artist":"A","source":"Spotify","duration_ms":5,"image_url":"i","bitrate_kbps":1,"sample_rate_hz":2}"#,
        )
        .unwrap();
        assert_eq!(t.duration_ms, Some(5));
        assert_eq!(t.image_url.as_deref(), Some("i"));
        assert_eq!((t.bitrate_kbps, t.sample_rate_hz), (Some(1), Some(2)));
        let d: Track = serde_json::from_str(
            r#"{"id":"1","title":"T","artist":"A","source":"plex","duration":7}"#,
        )
        .unwrap();
        assert_eq!(d.duration_ms, Some(7));
    }
}
