//! Shared HTTP client construction and the retry policy (port of Android `retrying`).

use std::time::Duration;

use reqwest::header::{HeaderValue, RETRY_AFTER};
use reqwest::{Client, ClientBuilder, Method, Request, Response, StatusCode};
use serde_json::Value;

use crate::error::{ProviderError, Result};

/// Connect timeout for the shared client.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Whole-request timeout for the shared client.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Plex keeps the legacy 6 s connect timeout.
pub const PLEX_CONNECT_TIMEOUT: Duration = Duration::from_secs(6);

/// `User-Agent` sent by every provider request.
pub const USER_AGENT: &str = concat!("AnyPlayer/", env!("CARGO_PKG_VERSION"));

/// The builder behind [`build_client`]: rustls, connect timeout 10 s, request timeout 30 s.
pub fn client_builder() -> ClientBuilder {
    Client::builder()
        .use_rustls_tls()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .user_agent(USER_AGENT)
}

/// The one `reqwest::Client` shared by the Jellyfin and Spotify clients.
pub fn build_client() -> Result<Client> {
    client_builder().build().map_err(ProviderError::from)
}

/// Retry policy of [`execute_retrying`].
///
/// 429 is retried for any method, 5xx only for GET/HEAD, at most `max_retries` times. The wait is
/// a numeric `Retry-After` capped at `max_retry_after`, otherwise `base_backoff << attempt`
/// capped at `max_backoff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
    pub max_retry_after: Duration,
}

impl Default for RetryPolicy {
    /// Android parity: 5 retries, 500 ms << attempt up to 10 s, `Retry-After` up to 60 s.
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(10),
            max_retry_after: Duration::from_secs(60),
        }
    }
}

impl RetryPolicy {
    /// Never retries.
    pub const NONE: Self = Self {
        max_retries: 0,
        base_backoff: Duration::ZERO,
        max_backoff: Duration::ZERO,
        max_retry_after: Duration::ZERO,
    };

    /// True when a response with `status` to a `method` request is retryable.
    pub fn is_retryable(method: &Method, status: StatusCode) -> bool {
        status == StatusCode::TOO_MANY_REQUESTS
            || (status.is_server_error() && (*method == Method::GET || *method == Method::HEAD))
    }

    /// Wait before retry number `attempt` (0-based).
    pub fn delay(&self, attempt: u32, retry_after: Option<&HeaderValue>) -> Duration {
        let header_secs = retry_after
            .and_then(|value| value.to_str().ok())
            .and_then(|text| text.trim().parse::<i64>().ok());
        if let Some(secs) = header_secs {
            let secs = u64::try_from(secs.max(0)).unwrap_or(0);
            return Duration::from_secs(secs).min(self.max_retry_after);
        }
        let factor = 1u32.checked_shl(attempt).unwrap_or(u32::MAX);
        self.base_backoff
            .saturating_mul(factor)
            .min(self.max_backoff)
    }
}

/// Executes `request` with the default [`RetryPolicy`].
pub async fn execute_retrying(client: &Client, request: Request) -> reqwest::Result<Response> {
    execute_retrying_with(client, request, RetryPolicy::default()).await
}

/// Executes `request`, retrying per `policy`. Rejected responses are dropped (closed) before the
/// wait. Requests with a non-clonable (streaming) body are sent once.
pub async fn execute_retrying_with(
    client: &Client,
    request: Request,
    policy: RetryPolicy,
) -> reqwest::Result<Response> {
    let mut attempt = 0;
    let mut request = request;
    loop {
        let retry_copy = if attempt < policy.max_retries {
            request.try_clone()
        } else {
            None
        };
        let method = request.method().clone();
        let response = client.execute(request).await?;
        let Some(next) = retry_copy else {
            return Ok(response);
        };
        if !RetryPolicy::is_retryable(&method, response.status()) {
            return Ok(response);
        }
        let wait = policy.delay(attempt, response.headers().get(RETRY_AFTER));
        drop(response);
        tokio::time::sleep(wait).await;
        request = next;
        attempt += 1;
    }
}

/// Executes with retries and decodes a 2xx JSON body; non-2xx becomes [`ProviderError::Http`].
pub(crate) async fn json(client: &Client, request: Request, policy: RetryPolicy) -> Result<Value> {
    let response = execute_retrying_with(client, request, policy).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(ProviderError::http(status));
    }
    decode_json(response).await
}

pub(crate) async fn decode_json(response: Response) -> Result<Value> {
    let bytes = response.bytes().await?;
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|e| ProviderError::Decode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_statuses_follow_android() {
        assert!(RetryPolicy::is_retryable(
            &Method::POST,
            StatusCode::TOO_MANY_REQUESTS
        ));
        assert!(RetryPolicy::is_retryable(
            &Method::GET,
            StatusCode::BAD_GATEWAY
        ));
        assert!(RetryPolicy::is_retryable(
            &Method::HEAD,
            StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(!RetryPolicy::is_retryable(
            &Method::POST,
            StatusCode::BAD_GATEWAY
        ));
        assert!(!RetryPolicy::is_retryable(
            &Method::PUT,
            StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(!RetryPolicy::is_retryable(
            &Method::GET,
            StatusCode::NOT_FOUND
        ));
    }

    #[test]
    fn delay_prefers_numeric_retry_after_capped_at_60s() {
        let policy = RetryPolicy::default();
        let header = |v: &str| HeaderValue::from_str(v).unwrap();
        assert_eq!(policy.delay(0, Some(&header("3"))), Duration::from_secs(3));
        assert_eq!(
            policy.delay(4, Some(&header(" 120 "))),
            Duration::from_secs(60)
        );
        assert_eq!(policy.delay(0, Some(&header("-4"))), Duration::ZERO);
        // HTTP-date and garbage fall back to exponential backoff.
        assert_eq!(
            policy.delay(1, Some(&header("Wed, 21 Oct 2015 07:28:00 GMT"))),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn delay_backs_off_exponentially_capped_at_10s() {
        let policy = RetryPolicy::default();
        let waits: Vec<_> = (0..7).map(|a| policy.delay(a, None).as_millis()).collect();
        assert_eq!(waits, [500, 1000, 2000, 4000, 8000, 10_000, 10_000]);
        assert_eq!(policy.delay(40, None), Duration::from_secs(10));
    }
}
