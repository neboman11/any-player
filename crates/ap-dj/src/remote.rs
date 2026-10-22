//! Server-side break generation (Android `RemoteBreakGenerator.kt`) and the voice catalog
//! (`GET /v1/dj-voice-models`, Android `ArtifactStore.refreshCatalog` for voices).

use std::path::Path;
use std::time::Duration;

use ap_model::Track;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::account::{DjAccount, describe_status_failure, describe_transport_failure, error_kind};
use crate::diagnostics::DjDiagnostics;
use crate::passages::Passages;
use crate::voice_processor;

const TAG: &str = "DjRemote";
const VOICE_TAG: &str = "DjVoiceSynthesizer";
const CATALOG_TAG: &str = "DjArtifacts";
const BREAK_READ_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const BREAK_CALL_TIMEOUT: Duration = Duration::from_secs(6 * 60);
/// OkHttp's default connect/read timeouts, which Android's catalog client keeps.
const CATALOG_TIMEOUT: Duration = Duration::from_secs(10);

pub const BREAKS_NOT_CONFIGURED: &str =
    "This sync server doesn't have AI DJ break generation configured yet. Ask the server admin.";
pub const VOICES_NOT_CONFIGURED: &str =
    "This sync server doesn't have an AI DJ voice catalog configured yet. Ask the server admin.";

#[derive(Serialize)]
struct BodyPassage<'a> {
    chunk_id: i64,
    text: &'a str,
    source: &'a str,
}

#[derive(Serialize)]
struct Body<'a> {
    title: &'a str,
    artist: &'a str,
    passages: Vec<BodyPassage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice_id: Option<&'a str>,
}

/// `POST /v1/dj-breaks` → WAV file → [`voice_processor::normalize_wave_file`].
#[derive(Clone)]
pub struct RemoteBreakGenerator {
    http: reqwest::Client,
    diagnostics: DjDiagnostics,
}

impl RemoteBreakGenerator {
    pub fn new(diagnostics: DjDiagnostics) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .read_timeout(BREAK_READ_TIMEOUT)
            .timeout(BREAK_CALL_TIMEOUT)
            .build()?;
        Ok(Self { http, diagnostics })
    }

    /// Generates one break into `file` (normally `dj_filler/<breakId>.wav`). Returns `true`
    /// only when the server returned audio and post-processing produced a non-empty WAV.
    /// `level` is the user's voice level, clamped to [`voice_processor::LEVEL_RANGE`].
    /// On `false` the file may hold partial or unprocessed bytes; the caller owns its cleanup.
    pub async fn generate(
        &self,
        account: &DjAccount,
        track: &Track,
        passages: &Passages,
        voice_id: Option<&str>,
        level: f32,
        file: &Path,
    ) -> bool {
        if !account.is_configured() {
            return false;
        }
        let body = Body {
            title: &track.title,
            artist: &track.artist,
            passages: passages
                .passages
                .iter()
                .map(|p| BodyPassage {
                    chunk_id: p.chunk_id,
                    text: &p.text,
                    source: &p.source,
                })
                .collect(),
            voice_id,
        };
        let Ok(body) = serde_json::to_vec(&body) else {
            return false;
        };
        let request = account
            .authorize(self.http.post(account.url("v1/dj-breaks")))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => return self.failed(error_kind(&error)),
        };
        match response.status().as_u16() {
            200 => {}
            204 => {
                self.diagnostics
                    .log(TAG, "Server fact-check rejected script", None);
                return false;
            }
            code => {
                self.diagnostics.log(
                    TAG,
                    &describe_status_failure(code, BREAKS_NOT_CONFIGURED),
                    None,
                );
                return false;
            }
        }
        if let Err(kind) = save_body(response, file).await {
            return self.failed(kind);
        }
        let path = file.to_path_buf();
        match tokio::task::spawn_blocking(move || {
            voice_processor::normalize_wave_file(&path, level)
        })
        .await
        {
            Ok(Ok(written)) => written,
            Ok(Err(error)) => {
                let kind = match error {
                    voice_processor::WavError::Hound(hound::Error::IoError(_)) => "Io",
                    _ => "Format",
                };
                self.diagnostics
                    .log(VOICE_TAG, "Server audio could not be decoded", Some(kind));
                false
            }
            Err(_) => self.failed("Panic"),
        }
    }

    fn failed(&self, kind: &str) -> bool {
        self.diagnostics
            .log(TAG, "Remote break generation failed", Some(kind));
        false
    }
}

async fn save_body(mut response: reqwest::Response, file: &Path) -> Result<(), &'static str> {
    let mut out = tokio::fs::File::create(file).await.map_err(|_| "Io")?;
    while let Some(chunk) = response.chunk().await.map_err(|e| error_kind(&e))? {
        out.write_all(&chunk).await.map_err(|_| "Io")?;
    }
    out.flush().await.map_err(|_| "Io")
}

/// One entry of the server voice catalog (Android `VoiceDescriptor`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceDescriptor {
    pub id: String,
    pub name: String,
    pub version: String,
    pub size_bytes: i64,
    pub sha256: String,
}

/// `GET /v1/dj-voice-models` response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceCatalog {
    #[serde(default)]
    pub default_id: Option<String>,
    pub voices: Vec<VoiceDescriptor>,
}

impl VoiceCatalog {
    /// Android `validCatalog`: safe ids and versions, 64-hex sha256, non-blank names,
    /// non-negative sizes, unique ids, and a default (if any) that is listed.
    pub fn is_valid(&self) -> bool {
        let mut ids: Vec<&str> = self.voices.iter().map(|v| v.id.as_str()).collect();
        let entries_valid = self.voices.iter().all(|v| {
            is_safe_component(&v.id)
                && is_safe_component(&v.version)
                && v.sha256.len() == 64
                && v.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                && !v.name.trim().is_empty()
                && v.size_bytes >= 0
        });
        ids.sort_unstable();
        ids.dedup();
        entries_valid
            && ids.len() == self.voices.len()
            && self
                .default_id
                .as_deref()
                .is_none_or(|d| self.voices.iter().any(|v| v.id == d))
    }
}

/// Android `SAFE_COMPONENT`: `[A-Za-z0-9][A-Za-z0-9._-]{0,127}`.
pub fn is_safe_component(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes[1..]
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Why the voice catalog could not be loaded; `Display` is the user-facing reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("Sync server target is not set.")]
    NoAccount,
    #[error("{0}")]
    Status(String),
    #[error("{0}")]
    Transport(String),
    #[error("Invalid AI DJ voice catalog.")]
    Invalid,
}

/// Fetches the voice catalog for the Settings → AI DJ voice picker.
#[derive(Clone)]
pub struct VoiceCatalogClient {
    http: reqwest::Client,
    diagnostics: DjDiagnostics,
}

impl VoiceCatalogClient {
    pub fn new(diagnostics: DjDiagnostics) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .connect_timeout(CATALOG_TIMEOUT)
            .read_timeout(CATALOG_TIMEOUT)
            .build()?;
        Ok(Self { http, diagnostics })
    }

    /// Failures are also logged to diagnostics.
    pub async fn fetch(&self, account: &DjAccount) -> Result<VoiceCatalog, CatalogError> {
        let result = self.fetch_inner(account).await;
        if let Err(error) = &result {
            self.diagnostics.log(CATALOG_TAG, &error.to_string(), None);
        }
        result
    }

    async fn fetch_inner(&self, account: &DjAccount) -> Result<VoiceCatalog, CatalogError> {
        if !account.is_configured() {
            return Err(CatalogError::NoAccount);
        }
        let transport = |e: reqwest::Error| CatalogError::Transport(describe_transport_failure(&e));
        let response = account
            .authorize(self.http.get(account.url("v1/dj-voice-models")))
            .send()
            .await
            .map_err(transport)?;
        let status = response.status();
        if !status.is_success() {
            return Err(CatalogError::Status(describe_status_failure(
                status.as_u16(),
                VOICES_NOT_CONFIGURED,
            )));
        }
        let body = response.bytes().await.map_err(transport)?;
        let catalog: VoiceCatalog =
            serde_json::from_slice(&body).map_err(|_| CatalogError::Invalid)?;
        if catalog.is_valid() {
            Ok(catalog)
        } else {
            Err(CatalogError::Invalid)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(id: &str) -> VoiceDescriptor {
        VoiceDescriptor {
            id: id.into(),
            name: "Name".into(),
            version: "1".into(),
            size_bytes: 1,
            sha256: "a".repeat(64),
        }
    }

    #[test]
    fn catalog_validation_matches_android() {
        assert!(
            VoiceCatalog {
                default_id: Some("a".into()),
                voices: vec![voice("a"), voice("b")]
            }
            .is_valid()
        );
        assert!(VoiceCatalog::default().is_valid());
        assert!(
            !VoiceCatalog {
                default_id: Some("c".into()),
                voices: vec![voice("a")]
            }
            .is_valid()
        );
        assert!(
            !VoiceCatalog {
                default_id: None,
                voices: vec![voice("a"), voice("a")]
            }
            .is_valid()
        );
        assert!(
            !VoiceCatalog {
                default_id: None,
                voices: vec![voice("../x")]
            }
            .is_valid()
        );
        let mut bad_hash = voice("a");
        bad_hash.sha256 = "zz".into();
        assert!(
            !VoiceCatalog {
                default_id: None,
                voices: vec![bad_hash]
            }
            .is_valid()
        );
        assert!(is_safe_component(&"a".repeat(128)));
        assert!(!is_safe_component(&"a".repeat(129)));
        assert!(!is_safe_component(".hidden"));
    }

    #[test]
    fn body_omits_absent_voice() {
        let body = Body {
            title: "T",
            artist: "A",
            passages: vec![],
            voice_id: None,
        };
        assert_eq!(
            serde_json::to_string(&body).unwrap(),
            r#"{"title":"T","artist":"A","passages":[]}"#
        );
    }
}
