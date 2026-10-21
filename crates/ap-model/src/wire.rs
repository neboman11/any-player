//! kotlinx-compatible wire DTOs (design §10): the `any-player-state` transfer envelope, the
//! encrypted state file and the config file (`export_version` 1).
//!
//! Rules (AppJson and the sync `Json` both keep `encodeDefaults = false`): fields in Kotlin
//! declaration order; fields equal to their Kotlin default are omitted, including `null`
//! defaults; unknown keys are ignored. Compact `serde_json` output escapes strings exactly like
//! kotlinx (see `json::write_quoted`).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::enums::{PlaylistType, SourceType};
use crate::ids::name_uuid_from_bytes;
use crate::json::JsonElement;
use crate::models::{
    ColumnPreferences, CustomPlaylist, PlaybackStatus, PlaylistTrack, UnionPlaylistSource, is_false,
};
use crate::text::{kotlin_trim, sha256_hex};
use crate::transfer_crypto::{self, CryptoError};

pub const ANY_PLAYER_STATE_FORMAT: &str = "any-player-state";
pub const ANY_PLAYER_STATE_VERSION: i32 = 1;
pub const CONFIG_EXPORT_VERSION: i32 = 1;
pub const ENCRYPTION_ALGORITHM: &str = "AES-256-GCM";
pub const ENCRYPTION_KDF: &str = "PBKDF2-HMAC-SHA256";

fn default_format() -> String {
    ANY_PLAYER_STATE_FORMAT.to_owned()
}
fn is_default_format(v: &str) -> bool {
    v == ANY_PLAYER_STATE_FORMAT
}
fn default_version() -> i32 {
    ANY_PLAYER_STATE_VERSION
}
#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde skip_serializing_if signature")]
fn is_default_version(v: &i32) -> bool {
    *v == ANY_PLAYER_STATE_VERSION
}
fn default_true() -> bool {
    true
}
#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde skip_serializing_if signature")]
fn is_true(v: &bool) -> bool {
    *v
}
fn default_algorithm() -> String {
    ENCRYPTION_ALGORITHM.to_owned()
}
fn is_default_algorithm(v: &str) -> bool {
    v == ENCRYPTION_ALGORITHM
}
fn default_kdf() -> String {
    ENCRYPTION_KDF.to_owned()
}
fn is_default_kdf(v: &str) -> bool {
    v == ENCRYPTION_KDF
}

/// The `any-player-state` transfer file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnyPlayerStateEnvelope {
    #[serde(default = "default_format", skip_serializing_if = "is_default_format")]
    pub format: String,
    #[serde(default = "default_version", skip_serializing_if = "is_default_version")]
    pub version: i32,
    pub created_at: String,
    pub source_app: SourceAppInfo,
    pub data: AnyPlayerStateData,
    pub integrity: Integrity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAppInfo {
    pub platform: String,
    pub app_version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnyPlayerStateData {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_playlists: Vec<CustomPlaylist>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub playlist_tracks: Vec<PlaylistTrack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub union_playlist_sources: Vec<UnionPlaylistSource>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub column_preferences: IndexMap<String, ColumnPreferences>,
    /// Keys are lowercase source serial names.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub connections: IndexMap<SourceType, ExportedConnection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_state: Option<PlaybackStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedConnection {
    pub source: SourceType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spotify_connected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spotify_premium: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Integrity {
    pub sha256: String,
}

/// A PRIVATE export: the AppJson envelope text encrypted with a passphrase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedStateFile {
    #[serde(default = "default_format", skip_serializing_if = "is_default_format")]
    pub format: String,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub encrypted: bool,
    #[serde(default = "default_algorithm", skip_serializing_if = "is_default_algorithm")]
    pub algorithm: String,
    #[serde(default = "default_kdf", skip_serializing_if = "is_default_kdf")]
    pub kdf: String,
    pub kdf_iterations: i32,
    pub salt_base64: String,
    pub nonce_base64: String,
    pub ciphertext_base64: String,
}

/// Why a state file could not be decoded.
#[derive(Debug, thiserror::Error)]
pub enum StateFileError {
    #[error("invalid JSON: {0}")]
    Syntax(#[from] crate::json::JsonSyntaxError),
    #[error("invalid state file: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("state file root is not a JSON object")]
    NotAnObject,
    #[error("Passphrase required for encrypted import")]
    PassphraseRequired,
    #[error("Invalid format")]
    InvalidFormat,
    #[error("Unsupported state version: {0}")]
    UnsupportedVersion(i32),
    #[error("Integrity check failed")]
    IntegrityFailed,
    #[error(transparent)]
    Crypto(#[from] CryptoError),
}

impl AnyPlayerStateEnvelope {
    /// A new envelope with its integrity hash computed.
    #[must_use]
    pub fn new(created_at: String, source_app: SourceAppInfo, data: AnyPlayerStateData) -> Self {
        let mut envelope = Self {
            format: default_format(),
            version: ANY_PLAYER_STATE_VERSION,
            created_at,
            source_app,
            data,
            integrity: Integrity { sha256: String::new() },
        };
        envelope.integrity.sha256 = envelope.compute_integrity();
        envelope
    }

    /// Compact AppJson text.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("envelope serialization cannot fail")
    }

    /// `sha256_hex` of the AppJson text of this envelope with `integrity.sha256 = ""`.
    #[must_use]
    pub fn compute_integrity(&self) -> String {
        let seed = Self { integrity: Integrity { sha256: String::new() }, ..self.clone() };
        sha256_hex(&seed.to_json())
    }
}

/// Android `decodeAndVerify`: detects encryption, decrypts with `passphrase`, decodes and checks
/// format, version and integrity.
pub fn decode_and_verify(raw: &str, passphrase: Option<&str>) -> Result<AnyPlayerStateEnvelope, StateFileError> {
    let root = JsonElement::parse(raw)?;
    let root = root.as_object().ok_or(StateFileError::NotAnObject)?;
    let primitive = |key: &str| -> Result<Option<&JsonElement>, StateFileError> {
        match root.get(key) {
            Some(JsonElement::Array(_) | JsonElement::Object(_)) => Err(StateFileError::NotAnObject),
            other => Ok(other),
        }
    };
    let flagged = primitive("encrypted")?.and_then(JsonElement::content_or_null).as_deref() == Some("true");
    let has_material = ["saltBase64", "nonceBase64", "ciphertextBase64"]
        .into_iter()
        .map(|k| primitive(k).map(|v| v.and_then(JsonElement::content_or_null).is_some()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .all(|present| present)
        && primitive("kdfIterations")?.is_some();
    let decrypted;
    let plain = if flagged || has_material {
        let file: EncryptedStateFile = serde_json::from_str(raw)?;
        decrypted = transfer_crypto::decrypt(&file, passphrase.ok_or(StateFileError::PassphraseRequired)?)?;
        decrypted.as_str()
    } else {
        raw
    };
    let envelope: AnyPlayerStateEnvelope = serde_json::from_str(plain)?;
    if envelope.format != ANY_PLAYER_STATE_FORMAT {
        return Err(StateFileError::InvalidFormat);
    }
    if envelope.version != ANY_PLAYER_STATE_VERSION {
        return Err(StateFileError::UnsupportedVersion(envelope.version));
    }
    if envelope.compute_integrity() != envelope.integrity.sha256 {
        return Err(StateFileError::IntegrityFailed);
    }
    Ok(envelope)
}

// ---- Config file (`export_version` 1) ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigFile {
    pub export_version: i32,
    pub provider_configs: ConfigProviderConfigs,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_playlists: Vec<ConfigCustomPlaylist>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigProviderConfigs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spotify: Option<ConfigSpotify>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jellyfin: Option<ConfigServer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plex: Option<ConfigServer>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSpotify {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
}

/// Android `ConfigJellyfin` / `ConfigPlex`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigServer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigCustomPlaylist {
    pub playlist: ConfigPlaylistEntry,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<ConfigTrack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub union_sources: Vec<ConfigUnionSource>,
}

/// Timestamps are epoch seconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigPlaylistEntry {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub track_count: i32,
    /// Free text; see [`parse_config_playlist_type`].
    pub playlist_type: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_distinct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigTrack {
    /// File-local row number.
    pub id: i32,
    pub playlist_id: String,
    /// Free text; see [`parse_config_source_type`].
    pub track_source: String,
    pub track_id: String,
    pub position: i32,
    pub added_at: i64,
    pub title: String,
    pub artist: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigUnionSource {
    /// File-local row number.
    pub id: i32,
    pub union_playlist_id: String,
    pub source_type: String,
    pub source_playlist_id: String,
    pub position: i32,
    pub added_at: i64,
}

/// Android `toConfigPlaylistTypeOrNull`: trimmed, case-insensitive.
#[must_use]
pub fn parse_config_playlist_type(value: &str) -> Option<PlaylistType> {
    match kotlin_trim(value).to_lowercase().as_str() {
        "standard" => Some(PlaylistType::Standard),
        "union" => Some(PlaylistType::Union),
        _ => None,
    }
}

/// Android `toConfigSourceTypeOrNull`: trimmed, case-insensitive.
#[must_use]
pub fn parse_config_source_type(value: &str) -> Option<SourceType> {
    let lower = kotlin_trim(value).to_lowercase();
    SourceType::ALL_VARIANTS.iter().copied().find(|s| s.serial_name() == lower)
}

fn occurrence_key(key: String, occurrence: usize) -> String {
    if occurrence == 0 { key } else { format!("{key}:occurrence:{occurrence}") }
}

/// Deterministic row id of the `occurrence`-th (0-based) config track with this natural key:
/// `nameUUIDFromBytes("track:<playlistId>:<SOURCE>:<trackId>[:occurrence:<n>]")`.
#[must_use]
pub fn config_track_row_id(playlist_id: &str, source: SourceType, track_id: &str, occurrence: usize) -> String {
    let key = occurrence_key(format!("track:{playlist_id}:{}:{track_id}", source.name()), occurrence);
    name_uuid_from_bytes(key.as_bytes())
}

/// Deterministic row id of a config union source:
/// `nameUUIDFromBytes("union:<unionId>:<SOURCE>:<sourcePlaylistId>[:occurrence:<n>]")`.
#[must_use]
pub fn config_union_source_row_id(
    union_playlist_id: &str,
    source: SourceType,
    source_playlist_id: &str,
    occurrence: usize,
) -> String {
    let key = occurrence_key(
        format!("union:{union_playlist_id}:{}:{source_playlist_id}", source.name()),
        occurrence,
    );
    name_uuid_from_bytes(key.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_envelope_omits_defaults() {
        let e = AnyPlayerStateEnvelope::new(
            "2026-05-18T12:00:00Z".into(),
            SourceAppInfo { platform: "android".into(), app_version: "test".into() },
            AnyPlayerStateData::default(),
        );
        assert_eq!(
            e.to_json(),
            r#"{"createdAt":"2026-05-18T12:00:00Z","sourceApp":{"platform":"android","appVersion":"test"},"data":{},"integrity":{"sha256":"773701283adb876dfcdb815f46225e12deff91c8fe544e020afe5a87ae77eb3f"}}"#
        );
    }

    #[test]
    fn integrity_mismatch_is_rejected() {
        let raw = r#"{"createdAt":"x","sourceApp":{"platform":"a","appVersion":"b"},"data":{},"integrity":{"sha256":"00"}}"#;
        assert!(matches!(decode_and_verify(raw, None), Err(StateFileError::IntegrityFailed)));
        let bad_version = r#"{"version":2,"createdAt":"x","sourceApp":{"platform":"a","appVersion":"b"},"data":{},"integrity":{"sha256":"00"}}"#;
        assert!(matches!(decode_and_verify(bad_version, None), Err(StateFileError::UnsupportedVersion(2))));
    }

    #[test]
    fn config_type_parsing_is_lenient() {
        assert_eq!(parse_config_source_type(" Jellyfin "), Some(SourceType::Jellyfin));
        assert_eq!(parse_config_source_type("tidal"), None);
        assert_eq!(parse_config_playlist_type("UNION"), Some(PlaylistType::Union));
    }

    #[test]
    fn config_file_decodes_explicit_nulls_and_unknown_keys() {
        let raw = r#"{"export_version":1,"provider_configs":{"spotify":null,"jellyfin":{"base_url":"http://j"},"extra":1},
            "custom_playlists":[{"playlist":{"id":"p","name":"n","description":null,"image_url":null,"created_at":1,"updated_at":2,
            "track_count":0,"playlist_type":"standard","is_distinct":false},"tracks":[],"union_sources":[]}]}"#;
        let config: ConfigFile = serde_json::from_str(raw).unwrap();
        assert_eq!(config.provider_configs.jellyfin.unwrap().base_url.as_deref(), Some("http://j"));
        assert_eq!(config.custom_playlists[0].playlist.description, None);
    }
}
