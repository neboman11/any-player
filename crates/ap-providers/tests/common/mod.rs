#![allow(dead_code, reason = "each test binary uses a subset")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ap_providers::RetryPolicy;
use ap_providers::spotify::{
    Clock, SpotifyAccounts, SpotifyToken, SpotifyTokenProvider, TokenSink,
};
use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use wiremock::{MockServer, Request, ResponseTemplate};

/// Android's retry semantics with millisecond waits.
pub const FAST_RETRY: RetryPolicy = RetryPolicy {
    max_retries: 5,
    base_backoff: Duration::from_millis(1),
    max_backoff: Duration::from_millis(5),
    max_retry_after: Duration::from_secs(60),
};

pub fn http() -> reqwest::Client {
    ap_providers::build_client().unwrap()
}

pub fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-21T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

pub fn fixed_clock() -> Clock {
    Arc::new(now)
}

#[derive(Default)]
pub struct RecordingSink {
    pub tokens: Mutex<Vec<SpotifyToken>>,
    pub rejected: AtomicUsize,
}

impl RecordingSink {
    pub fn saved(&self) -> Vec<SpotifyToken> {
        self.tokens.lock().unwrap().clone()
    }

    pub fn rejected(&self) -> usize {
        self.rejected.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TokenSink for RecordingSink {
    async fn token_updated(&self, token: &SpotifyToken) {
        self.tokens.lock().unwrap().push(token.clone());
    }

    async fn refresh_rejected(&self) {
        self.rejected.fetch_add(1, Ordering::SeqCst);
    }
}

/// A JSON responder that counts how many times it was invoked (deterministic call-count
/// assertions without relying on wiremock expectations).
pub fn counting_json(
    status: u16,
    body: serde_json::Value,
) -> (
    Arc<std::sync::atomic::AtomicUsize>,
    impl Fn(&Request) -> ResponseTemplate,
) {
    let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let shared = counter.clone();
    let responder = move |_: &Request| {
        shared.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ResponseTemplate::new(status).set_body_json(body.clone())
    };
    (counter, responder)
}

/// `header()` splits comma-separated values; `X-Emby-Authorization` is a comma-separated
/// structured value, so tests must match the raw header bytes with a closure.
pub fn raw_header(
    name: &'static str,
    value: &'static str,
) -> impl Fn(&Request) -> bool + Send + Sync {
    move |req: &Request| {
        req.headers
            .get(name)
            .is_some_and(|v| v.as_bytes() == value.as_bytes())
    }
}

/// A token expiring `minutes` after [`now`].
pub fn token(access: &str, minutes: i64, refresh: Option<&str>) -> SpotifyToken {
    SpotifyToken {
        access_token: access.into(),
        expires_in: 3600,
        expires_at: Some(now() + TimeDelta::minutes(minutes)),
        refresh_token: refresh.map(str::to_owned),
        scopes: ["user-read-email".to_owned()].into_iter().collect(),
    }
}

pub fn accounts(server: &MockServer) -> SpotifyAccounts {
    SpotifyAccounts::new(http())
        .with_token_url(format!("{}/api/token", server.uri()))
        .with_retry_policy(FAST_RETRY)
        .with_clock(fixed_clock())
}

pub fn token_provider(
    server: &MockServer,
    token: Option<SpotifyToken>,
) -> (Arc<SpotifyTokenProvider>, Arc<RecordingSink>) {
    let sink = Arc::new(RecordingSink::default());
    let provider = Arc::new(SpotifyTokenProvider::new(
        accounts(server),
        token,
        sink.clone(),
    ));
    (provider, sink)
}
