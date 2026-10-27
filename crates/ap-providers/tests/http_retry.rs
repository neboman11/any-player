mod common;

use std::time::{Duration, Instant};

use ap_providers::{RetryPolicy, execute_retrying_with};
use common::{FAST_RETRY, http};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn send(
    server: &MockServer,
    verb: reqwest::Method,
    policy: RetryPolicy,
) -> reqwest::Response {
    let client = http();
    let request = client
        .request(verb, format!("{}/r", server.uri()))
        .body("x")
        .build()
        .unwrap();
    execute_retrying_with(&client, request, policy)
        .await
        .unwrap()
}

#[tokio::test]
async fn get_retries_5xx_until_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/r"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/r"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        send(&server, reqwest::Method::GET, FAST_RETRY)
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn post_does_not_retry_5xx() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(502))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        send(&server, reqwest::Method::POST, FAST_RETRY)
            .await
            .status(),
        502
    );
}

#[tokio::test]
async fn any_method_retries_429() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        send(&server, reqwest::Method::PUT, FAST_RETRY)
            .await
            .status(),
        204
    );
}

#[tokio::test]
async fn gives_up_after_five_retries_and_returns_last_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429))
        .expect(6)
        .mount(&server)
        .await;
    assert_eq!(
        send(&server, reqwest::Method::GET, FAST_RETRY)
            .await
            .status(),
        429
    );
}

#[tokio::test]
async fn non_retryable_status_returns_immediately() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        send(&server, reqwest::Method::GET, FAST_RETRY)
            .await
            .status(),
        404
    );
}

#[tokio::test]
async fn honours_numeric_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let started = Instant::now();
    // Backoff would be 1 ms; the header asks for a full second.
    assert_eq!(
        send(&server, reqwest::Method::GET, FAST_RETRY)
            .await
            .status(),
        200
    );
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "waited {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn retry_after_is_capped() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "3600"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let policy = RetryPolicy {
        max_retry_after: Duration::from_millis(20),
        ..FAST_RETRY
    };
    let started = Instant::now();
    assert_eq!(
        send(&server, reqwest::Method::GET, policy).await.status(),
        200
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}
