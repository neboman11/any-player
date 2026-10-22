//! Remote break generation and voice catalog against an in-process wiremock server.

use std::f64::consts::PI;

use ap_dj::remote::{BREAKS_NOT_CONFIGURED, VOICES_NOT_CONFIGURED};
use ap_dj::voice_processor::{read_wav, write_wav};
use ap_dj::{
    CatalogError, DjAccount, DjDiagnostics, Passage, Passages, RemoteBreakGenerator, VoiceCatalog,
    VoiceCatalogClient,
};
use ap_model::{SourceType, Track};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn track() -> Track {
    Track::new("id", "Song", "Artist", SourceType::Jellyfin)
}

fn passages() -> Passages {
    let mut first = Passage::new(1, "First story", "wiki");
    first.title = "not sent".into();
    Passages {
        passages: vec![first, Passage::new(2, "Second", "news")],
    }
}

/// A short tone burst bracketed by silence, encoded as the server does (mono PCM16).
#[allow(clippy::cast_possible_truncation)]
fn server_wav() -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("raw.wav");
    let fs = 24_000;
    let samples: Vec<f32> = (0..fs)
        .map(|i| {
            if (6_000..18_000).contains(&i) {
                (0.2 * (2.0 * PI * 220.0 * f64::from(i) / 24_000.0).sin()) as f32
            } else {
                0.0
            }
        })
        .collect();
    write_wav(&file, &samples, fs).unwrap();
    std::fs::read(file).unwrap()
}

async fn generate(
    server: &MockServer,
    voice_id: Option<&str>,
    level: f32,
) -> (bool, DjDiagnostics, tempfile::TempDir) {
    let diagnostics = DjDiagnostics::new();
    let generator = RemoteBreakGenerator::new(diagnostics.clone()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("break.wav");
    let ok = generator
        .generate(
            &DjAccount::new(&server.uri(), "token"),
            &track(),
            &passages(),
            voice_id,
            level,
            &file,
        )
        .await;
    (ok, diagnostics, dir)
}

#[tokio::test]
async fn success_posts_body_and_writes_processed_wav() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/dj-breaks"))
        .and(header("authorization", "Bearer token"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(server_wav(), "audio/wav"))
        .expect(1)
        .mount(&server)
        .await;
    let (ok, diagnostics, dir) = generate(&server, Some("deep"), 1.0).await;
    assert!(ok, "{:?}", diagnostics.entries());
    let body: Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(
        body,
        json!({
            "title": "Song",
            "artist": "Artist",
            "passages": [
                {"chunk_id": 1, "text": "First story", "source": "wiki"},
                {"chunk_id": 2, "text": "Second", "source": "news"}
            ],
            "voice_id": "deep"
        })
    );
    let wave = read_wav(&dir.path().join("break.wav")).unwrap();
    assert_eq!(wave.sample_rate, 24_000);
    // Audible tone spans samples 6001..=17999 (sample 6000 is a zero crossing), plus 2 × 20 ms
    // trim margins and the 50/200 ms pads.
    assert_eq!(wave.samples.len(), 11_999 + 2 * 480 + 1_200 + 4_800);
    assert!(diagnostics.entries().is_empty());
}

#[tokio::test]
async fn voice_id_is_omitted_when_unset() {
    let server = MockServer::start().await;
    Mock::given(path("/v1/dj-breaks"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let (ok, diagnostics, _dir) = generate(&server, None, 1.0).await;
    assert!(!ok);
    let body: Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert!(body.get("voice_id").is_none());
    assert_eq!(
        diagnostics.entries()[0].message,
        "Server fact-check rejected script"
    );
    assert_eq!(diagnostics.entries()[0].tag, "DjRemote");
}

#[tokio::test]
async fn failure_statuses_log_android_messages() {
    for (code, message) in [
        (404u16, BREAKS_NOT_CONFIGURED.to_owned()),
        (
            401,
            "Sync server rejected the auth token. Check the Sync Auth Token setting.".to_owned(),
        ),
        (503, "Sync server returned an error (HTTP 503).".to_owned()),
    ] {
        let server = MockServer::start().await;
        Mock::given(path("/v1/dj-breaks"))
            .respond_with(ResponseTemplate::new(code))
            .mount(&server)
            .await;
        let (ok, diagnostics, _dir) = generate(&server, None, 1.0).await;
        assert!(!ok);
        assert_eq!(diagnostics.entries()[0].message, message, "HTTP {code}");
    }
}

#[tokio::test]
async fn undecodable_or_silent_audio_fails() {
    let server = MockServer::start().await;
    Mock::given(path("/v1/dj-breaks"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"garbage".to_vec()))
        .mount(&server)
        .await;
    let (ok, diagnostics, _dir) = generate(&server, None, 1.0).await;
    assert!(!ok);
    assert_eq!(
        diagnostics.entries()[0].message,
        "Server audio could not be decoded: Format"
    );

    let silent_dir = tempfile::tempdir().unwrap();
    let silent = silent_dir.path().join("s.wav");
    write_wav(&silent, &[0.0; 2_400], 24_000).unwrap();
    let server = MockServer::start().await;
    Mock::given(path("/v1/dj-breaks"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(std::fs::read(&silent).unwrap()))
        .mount(&server)
        .await;
    let (ok, diagnostics, _dir) = generate(&server, None, 1.0).await;
    assert!(!ok);
    assert!(diagnostics.entries().is_empty());
}

#[tokio::test]
async fn no_account_or_unreachable_server() {
    let diagnostics = DjDiagnostics::new();
    let generator = RemoteBreakGenerator::new(diagnostics.clone()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("b.wav");
    assert!(
        !generator
            .generate(
                &DjAccount::new(" ", "t"),
                &track(),
                &passages(),
                None,
                1.0,
                &file
            )
            .await
    );
    assert!(diagnostics.entries().is_empty());
    // Port 9 (discard) on loopback is closed in test environments.
    assert!(
        !generator
            .generate(
                &DjAccount::new("http://127.0.0.1:9", ""),
                &track(),
                &passages(),
                None,
                1.0,
                &file
            )
            .await
    );
    assert_eq!(
        diagnostics.entries()[0].message,
        "Remote break generation failed: Connect"
    );
}

fn catalog_json() -> Value {
    json!({
        "default_id": "deep",
        "voices": [
            {"id": "deep", "name": "Deep", "version": "1", "size_bytes": 10, "sha256": "a".repeat(64)},
            {"id": "bright", "name": "Bright", "version": "2.0", "size_bytes": 20, "sha256": "B".repeat(64), "extra": 1}
        ]
    })
}

#[tokio::test]
async fn voice_catalog_fetch_and_validation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/dj-voice-models"))
        .and(header("authorization", "Bearer token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(catalog_json()))
        .mount(&server)
        .await;
    let client = VoiceCatalogClient::new(DjDiagnostics::new()).unwrap();
    let catalog: VoiceCatalog = client
        .fetch(&DjAccount::new(&server.uri(), "token"))
        .await
        .unwrap();
    assert_eq!(catalog.default_id.as_deref(), Some("deep"));
    assert_eq!(
        catalog
            .voices
            .iter()
            .map(|v| v.id.as_str())
            .collect::<Vec<_>>(),
        ["deep", "bright"]
    );
    assert_eq!(catalog.voices[1].size_bytes, 20);

    let invalid = MockServer::start().await;
    let mut body = catalog_json();
    body["default_id"] = json!("missing");
    Mock::given(path("/v1/dj-voice-models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&invalid)
        .await;
    let diagnostics = DjDiagnostics::new();
    let client = VoiceCatalogClient::new(diagnostics.clone()).unwrap();
    assert_eq!(
        client.fetch(&DjAccount::new(&invalid.uri(), "")).await,
        Err(CatalogError::Invalid)
    );
    assert_eq!(
        diagnostics.entries()[0].message,
        "Invalid AI DJ voice catalog."
    );
}

#[tokio::test]
async fn voice_catalog_failures_are_described() {
    let server = MockServer::start().await;
    Mock::given(path("/v1/dj-voice-models"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let client = VoiceCatalogClient::new(DjDiagnostics::new()).unwrap();
    assert_eq!(
        client.fetch(&DjAccount::new(&server.uri(), "")).await,
        Err(CatalogError::Status(VOICES_NOT_CONFIGURED.to_owned()))
    );
    assert_eq!(
        client.fetch(&DjAccount::new("", "")).await,
        Err(CatalogError::NoAccount)
    );
    let unreachable = client
        .fetch(&DjAccount::new("http://127.0.0.1:9", ""))
        .await
        .unwrap_err();
    assert!(
        unreachable
            .to_string()
            .starts_with("Could not reach the sync server.")
    );
}
