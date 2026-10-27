mod common;
use ap_providers::spotify::{CLIENT_ID, REDIRECT_URI, SpotifyAccounts};
use serde_json::json;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn pkce_exchange_and_refresh_use_accounts_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/token")).and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code_verifier=verifier"))
        .and(body_string_contains(format!("client_id={CLIENT_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":"A","expires_in":3600,"refresh_token":"R","scope":"user-read-email"}))).expect(1).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=R"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"access_token":"B","expires_in":3600})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let accounts = SpotifyAccounts::new(common::http())
        .with_token_url(format!("{}/token", server.uri()))
        .with_retry_policy(common::FAST_RETRY);
    let token = accounts.exchange("code", "verifier").await.unwrap();
    assert_eq!(token.access_token, "A");
    assert!(token.expires_at.is_some());
    assert_eq!(accounts.refresh("R").await.unwrap().access_token, "B");
    let requests = server.received_requests().await.unwrap();
    let body = std::str::from_utf8(&requests[0].body).unwrap();
    let form: std::collections::HashMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(form["redirect_uri"], REDIRECT_URI);
}
