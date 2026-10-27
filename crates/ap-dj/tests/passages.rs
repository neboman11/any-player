//! Port of Android `PassageRepositoryTest` against an in-process wiremock server.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ap_dj::passages::{EXCLUSION_SECONDS, MAX_PENDING};
use ap_dj::{DjAccount, DjDiagnostics, MemoryPassageStateStore, Passage, PassageClient, Passages};
use ap_model::{SourceType, Track};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

struct Control {
    offered: Vec<Passage>,
    lookup_failures: u32,
    lookup_code: u16,
    receipt_code: u16,
    receipt_delay: Duration,
}

struct Fixture {
    server: MockServer,
    control: Arc<Mutex<Control>>,
    store: Arc<MemoryPassageStateStore>,
    now: Arc<Mutex<DateTime<Utc>>>,
    diagnostics: DjDiagnostics,
    account: DjAccount,
    client: PassageClient,
}

impl Fixture {
    async fn new() -> Self {
        let server = MockServer::start().await;
        let control = Arc::new(Mutex::new(Control {
            offered: vec![
                Passage::new(1, "First story", "source"),
                Passage::new(2, "Second story", "source"),
            ],
            lookup_failures: 0,
            lookup_code: 200,
            receipt_code: 204,
            receipt_delay: Duration::ZERO,
        }));
        let lookup = Arc::clone(&control);
        Mock::given(method("GET"))
            .and(path("/v1/dj-passages"))
            .respond_with(move |_: &Request| {
                let mut control = lookup.lock().unwrap();
                if control.lookup_failures > 0 {
                    control.lookup_failures -= 1;
                    return ResponseTemplate::new(503);
                }
                ResponseTemplate::new(control.lookup_code).set_body_json(
                    json!({"status": "ok", "passages": control.offered, "ignored": true}),
                )
            })
            .mount(&server)
            .await;
        let receipts = Arc::clone(&control);
        Mock::given(method("POST"))
            .and(path("/v1/dj-passages/played"))
            .respond_with(move |_: &Request| {
                let control = receipts.lock().unwrap();
                ResponseTemplate::new(control.receipt_code).set_delay(control.receipt_delay)
            })
            .mount(&server)
            .await;
        let store = Arc::new(MemoryPassageStateStore::default());
        let now = Arc::new(Mutex::new(
            DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z")
                .unwrap()
                .to_utc(),
        ));
        let diagnostics = DjDiagnostics::new();
        let account = DjAccount::new(&server.uri(), "token");
        let client = client(&store, &now, &diagnostics);
        Self {
            server,
            control,
            store,
            now,
            diagnostics,
            account,
            client,
        }
    }

    fn control(&self) -> std::sync::MutexGuard<'_, Control> {
        self.control.lock().unwrap()
    }

    async fn find(&self) -> Option<Vec<i64>> {
        self.client
            .find_passages(&self.account, &track())
            .await
            .map(|p| p.chunk_ids())
    }

    async fn record(&self, passages: Vec<Passage>) {
        self.client
            .record_played(&self.account, &Passages { passages })
            .await;
        self.client.settle().await;
    }

    async fn requests(&self, endpoint: &str) -> Vec<Request> {
        let requests = self.server.received_requests().await.unwrap();
        requests
            .into_iter()
            .filter(|r| r.url.path() == endpoint)
            .collect()
    }

    async fn receipts(&self) -> Vec<Value> {
        self.requests("/v1/dj-passages/played")
            .await
            .iter()
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect()
    }
}

fn client(
    store: &Arc<MemoryPassageStateStore>,
    now: &Arc<Mutex<DateTime<Utc>>>,
    diagnostics: &DjDiagnostics,
) -> PassageClient {
    let clock = Arc::clone(now);
    PassageClient::with_clock(store.clone(), diagnostics.clone(), move || {
        *clock.lock().unwrap()
    })
    .unwrap()
}

fn track() -> Track {
    let mut track = Track::new("id", "Song <&>", "Artist", SourceType::Custom);
    track.album = Some("Album".into());
    track
}

#[tokio::test]
async fn lookup_escapes_metadata_and_authenticates() {
    let f = Fixture::new().await;
    assert_eq!(f.find().await, Some(vec![1, 2]));
    let request = &f.requests("/v1/dj-passages").await[0];
    let query: Vec<(String, String)> = request
        .url
        .query_pairs()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    assert_eq!(
        query,
        [
            ("song".into(), "Song <&>".into()),
            ("artist".into(), "Artist".into()),
            ("album".into(), "Album".into())
        ]
    );
    assert_eq!(
        request.headers.get("authorization").unwrap(),
        "Bearer token"
    );
}

#[tokio::test]
async fn blank_album_is_omitted_and_no_account_makes_no_requests() {
    let f = Fixture::new().await;
    let mut track = track();
    track.album = Some("  ".into());
    f.client.find_passages(&f.account, &track).await.unwrap();
    assert!(
        f.requests("/v1/dj-passages").await[0]
            .url
            .query_pairs()
            .all(|(k, _)| k != "album")
    );

    let none = DjAccount::new("", "token");
    assert_eq!(f.client.find_passages(&none, &track).await, None);
    f.client
        .record_played(
            &none,
            &Passages {
                passages: vec![Passage::new(1, "t", "s")],
            },
        )
        .await;
    f.client.flush_pending(&none).await;
    f.client.settle().await;
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn locally_played_passages_excluded_for_thirty_days_across_restart() {
    let mut f = Fixture::new().await;
    let first = f.control().offered[0].clone();
    f.record(vec![first]).await;
    f.client = client(&f.store, &f.now, &f.diagnostics);
    assert_eq!(f.find().await, Some(vec![2]));
    *f.now.lock().unwrap() += chrono::Duration::seconds(EXCLUSION_SECONDS + 1);
    assert_eq!(f.find().await, Some(vec![1, 2]));
}

#[tokio::test]
async fn all_excluded_returns_no_break() {
    let f = Fixture::new().await;
    let offered = f.control().offered.clone();
    f.record(offered).await;
    assert_eq!(f.find().await, None);
}

#[tokio::test]
async fn exclusion_happens_before_rank_ordered_token_budget() {
    let f = Fixture::new().await;
    let offered = vec![
        Passage::new(1, "x".repeat(12_000), "x"),
        Passage::new(2, "y".repeat(6_000), "x"),
        Passage::new(3, "z".repeat(6_000), "x"),
        Passage::new(4, "short", "x"),
    ];
    f.control().offered = offered.clone();
    f.record(vec![offered[0].clone()]).await;
    assert_eq!(f.find().await, Some(vec![2]));
}

#[tokio::test]
async fn first_over_budget_passage_is_retained() {
    let f = Fixture::new().await;
    f.control().offered = vec![
        Passage::new(1, "漢".repeat(3_000), "x"),
        Passage::new(2, "short", "x"),
    ];
    assert_eq!(f.find().await, Some(vec![1]));
}

#[tokio::test]
async fn retries_503_twice_then_succeeds() {
    let f = Fixture::new().await;
    f.control().lookup_failures = 2;
    let started = Instant::now();
    assert!(f.find().await.is_some());
    assert_eq!(f.requests("/v1/dj-passages").await.len(), 3);
    assert!(
        started.elapsed() >= Duration::from_millis(750),
        "backoff is 250 ms then 500 ms"
    );
}

#[tokio::test]
async fn other_http_errors_are_not_retried() {
    let f = Fixture::new().await;
    f.control().lookup_code = 401;
    assert_eq!(f.find().await, None);
    assert_eq!(f.requests("/v1/dj-passages").await.len(), 1);
}

#[tokio::test]
async fn exhausts_three_attempts_on_503() {
    let f = Fixture::new().await;
    f.control().lookup_failures = 8;
    assert_eq!(f.find().await, None);
    assert_eq!(f.requests("/v1/dj-passages").await.len(), 3);
}

#[tokio::test]
async fn non_ok_status_and_malformed_bodies_yield_none() {
    let server = MockServer::start().await;
    Mock::given(path("/v1/dj-passages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "no_match"})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(path("/v1/dj-passages"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;
    let diagnostics = DjDiagnostics::new();
    let client = PassageClient::new(
        Arc::new(MemoryPassageStateStore::default()),
        diagnostics.clone(),
    )
    .unwrap();
    let account = DjAccount::new(&server.uri(), "");
    assert_eq!(client.find_passages(&account, &track()).await, None);
    assert_eq!(diagnostics.entries(), [] as [ap_dj::DjLogEntry; 0]);
    assert_eq!(client.find_passages(&account, &track()).await, None);
    assert_eq!(
        diagnostics.entries().last().unwrap().message,
        "Passage lookup failed: Decode"
    );
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .all(|r| !r.headers.contains_key("authorization"))
    );
}

#[tokio::test]
async fn receipts_split_sixteen_per_event_and_reuse_event_ids_until_accepted() {
    let f = Fixture::new().await;
    f.control().receipt_code = 503;
    f.record((1..=35).map(|id| Passage::new(id, "text", "x")).collect())
        .await;
    f.client.flush_pending(&f.account).await;
    f.control().receipt_code = 204;
    f.client.flush_pending(&f.account).await;
    f.client.flush_pending(&f.account).await;
    let receipts = f.receipts().await;
    // Background flush and first explicit flush each stop at the 503; then all three land.
    assert_eq!(receipts.len(), 5);
    let first_id = &receipts[0]["event_id"];
    assert_eq!(&receipts[1]["event_id"], first_id);
    assert_eq!(&receipts[2]["event_id"], first_id);
    let sizes: Vec<usize> = receipts[2..]
        .iter()
        .map(|r| r["chunk_ids"].as_array().unwrap().len())
        .collect();
    assert_eq!(sizes, [16, 16, 3]);
    assert_eq!(receipts[2]["played_at"], "2026-10-01T00:00:00Z");
    assert_eq!(receipts[2]["chunk_ids"][0], 1);
    let mut keys: Vec<&str> = receipts[2]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["chunk_ids", "event_id", "played_at"]);
}

#[tokio::test]
async fn client_errors_drop_but_transient_errors_retain_receipts() {
    for code in [400u16, 401, 403, 404, 408, 429, 500] {
        let f = Fixture::new().await;
        f.control().receipt_code = code;
        f.record(vec![Passage::new(1, "First story", "source")])
            .await;
        f.control().receipt_code = 204;
        f.client.flush_pending(&f.account).await;
        let expected = if [408, 429, 500].contains(&code) {
            2
        } else {
            1
        };
        assert_eq!(f.receipts().await.len(), expected, "HTTP {code}");
        if expected == 1 {
            assert_eq!(
                f.diagnostics.entries().last().unwrap().message,
                format!("Played receipt rejected (HTTP {code}); dropping")
            );
        }
    }
}

#[tokio::test]
async fn accounts_preserve_separate_queues_and_exclusions() {
    let f = Fixture::new().await;
    f.control().receipt_code = 500;
    let offered = f.control().offered.clone();
    f.record(offered).await;
    let other = DjAccount::new(&f.server.uri(), "other");
    assert_eq!(
        f.client
            .find_passages(&other, &track())
            .await
            .map(|p| p.chunk_ids()),
        Some(vec![1, 2])
    );
    f.client.settle().await;
    f.control().receipt_code = 204;
    let before = f.receipts().await.len();
    f.client.flush_pending(&other).await;
    assert_eq!(
        f.receipts().await.len(),
        before,
        "other account has nothing queued"
    );
    f.client.flush_pending(&f.account).await;
    assert_eq!(f.receipts().await.len(), before + 1);
    let keys: Vec<String> = f.store.values().into_keys().collect();
    assert_ne!(keys, [] as [std::string::String; 0]);
    assert!(keys.iter().all(|k| k.len() == 70
        && k.starts_with("state_")
        && k[6..].bytes().all(|b| b.is_ascii_hexdigit())));
}

#[tokio::test]
async fn pending_receipts_bounded_and_legacy_json_field_names_preserved() {
    let f = Fixture::new().await;
    f.control().receipt_code = 500;
    let total = i64::try_from((MAX_PENDING + 2) * 16).unwrap();
    let chunks: Vec<Passage> = (1..=total)
        .map(|id| Passage::new(id, "text", "x"))
        .collect();
    f.record(chunks).await;
    let stored: Value = serde_json::from_str(f.store.values().values().next().unwrap()).unwrap();
    let played = stored["played"].as_array().unwrap();
    assert_eq!(played.len(), MAX_PENDING);
    // Oldest dropped: the first kept event starts at chunk 2 * 16 + 1.
    assert_eq!(played[0]["chunkIds"][0], 33);
    let mut keys: Vec<&str> = played[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["chunkIds", "eventId", "playedAt"]);
    let mut keys: Vec<&str> = stored["usage"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["chunkId", "playedAt"]);
    assert!(
        f.diagnostics
            .entries()
            .iter()
            .any(|e| e.message == "Dropping oldest pending receipts at queue limit")
    );
}

#[tokio::test]
async fn reads_android_written_state() {
    let f = Fixture::new().await;
    let android = r#"{"played":[{"chunkIds":[7],"eventId":"e-1","playedAt":"2026-09-30T12:00:00.123Z"}],"usage":[{"chunkId":1,"playedAt":"2026-09-30T12:00:00.123Z"},{"chunkId":2,"playedAt":"2026-08-01T00:00:00Z"}]}"#;
    ap_dj::PassageStateStore::write(f.store.as_ref(), &f.account.state_key(), android.into())
        .await
        .unwrap();
    assert_eq!(f.find().await, Some(vec![2]));
    f.client.settle().await;
    let receipts = f.receipts().await;
    assert_eq!(
        receipts,
        [json!({"event_id": "e-1", "played_at": "2026-09-30T12:00:00.123Z", "chunk_ids": [7]})]
    );
    let stored: Value = serde_json::from_str(&f.store.values()[&f.account.state_key()]).unwrap();
    assert!(
        stored.get("played").is_none(),
        "empty lists are omitted like kotlinx"
    );
}

#[tokio::test]
async fn lookup_never_waits_for_background_receipt_flush() {
    let f = Fixture::new().await;
    f.control().receipt_delay = Duration::from_secs(3);
    let first = f.control().offered[0].clone();
    f.client
        .record_played(
            &f.account,
            &Passages {
                passages: vec![first],
            },
        )
        .await;
    let started = Instant::now();
    assert_eq!(f.find().await, Some(vec![2]));
    assert!(started.elapsed() < Duration::from_secs(2));
}
