mod common;

use ap_providers::{
    ProviderClient,
    spotify::{
        SpotifyAccounts, SpotifyClient, SpotifyErrorKind, SpotifyPlayerApi, SpotifyToken,
        SpotifyTokenProvider, TokenSink, WebSpotifyPlayerApi,
    },
};
use async_trait::async_trait;
use chrono::{TimeDelta, Utc};
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default)]
struct Sink(Mutex<Vec<SpotifyToken>>);
#[async_trait]
impl TokenSink for Sink {
    async fn token_updated(&self, token: &SpotifyToken) {
        self.0.lock().unwrap().push(token.clone());
    }
}
fn token(access: &str, seconds: i64) -> SpotifyToken {
    SpotifyToken {
        access_token: access.into(),
        expires_in: seconds,
        expires_at: Some(Utc::now() + TimeDelta::seconds(seconds)),
        refresh_token: Some("refresh".into()),
        scopes: BTreeSet::new(),
    }
}
fn setup(
    server: &MockServer,
    initial: SpotifyToken,
) -> (SpotifyClient, Arc<SpotifyTokenProvider>, Arc<Sink>) {
    let http = common::http();
    let sink = Arc::new(Sink::default());
    let accounts = SpotifyAccounts::new(http.clone())
        .with_token_url(format!("{}/token", server.uri()))
        .with_retry_policy(common::FAST_RETRY);
    let tokens = Arc::new(SpotifyTokenProvider::new(
        accounts,
        Some(initial),
        sink.clone(),
    ));
    let client = SpotifyClient::new(http, tokens.clone())
        .with_api_base(server.uri())
        .with_retry_policy(common::FAST_RETRY);
    (client, tokens, sink)
}

#[tokio::test]
async fn february_playlist_shapes_and_episodes() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/me/playlists")).and(header("authorization", "Bearer access")).and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[{"id":"a","name":"new","items":{"total":7}}, {"id":"b","name":"old","items":{},"tracks":{"total":3}}],"total":2,"next":null}))).expect(1).mount(&server).await;
    Mock::given(method("GET")).and(path("/playlists/a/items")).and(query_param("limit", "100")).and(query_param("offset", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[{"item":{"type":"track","id":"new","name":"New"}}, {"track":{"type":"track","id":"old","name":"Old"}}, {"item":{"type":"episode","id":"ep"}}],"total":3,"next":null}))).expect(1).mount(&server).await;
    let (client, _, _) = setup(&server, token("access", 3600));
    let playlists = client.playlists().await.unwrap();
    assert_eq!(
        playlists.iter().map(|p| p.track_count).collect::<Vec<_>>(),
        [7, 3]
    );
    let tracks = client.playlist_tracks("a").await.unwrap();
    assert_eq!(
        tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["new", "old"]
    );
}

#[tokio::test]
async fn search_pages_at_ten_and_stops_at_fifty() {
    let server = MockServer::start().await;
    for offset in (0..50).step_by(10) {
        let rows: Vec<_> = (offset..offset + 10)
            .map(|i| json!({"type":"track","id":format!("{i}"),"name":"track"}))
            .collect();
        Mock::given(path("/search"))
            .and(query_param("type", "track"))
            .and(query_param("limit", "10"))
            .and(query_param("offset", offset.to_string()))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"tracks":{"items":rows,"total":100,"next":"next"}})),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(path("/search")).and(query_param("type", "playlist"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"playlists":{"items":[{"id":"p","name":"result","items":{"total":2}}],"total":1,"next":null}}))).expect(1).mount(&server).await;
    let (client, _, _) = setup(&server, token("access", 3600));
    assert_eq!(client.search_tracks("music").await.unwrap().len(), 50);
    assert_eq!(client.search_playlists("music").await.unwrap()[0].id, "p");
}

#[tokio::test]
async fn concurrent_unauthorized_requests_refresh_once_and_persist() {
    let server = MockServer::start().await;
    Mock::given(path("/me"))
        .and(header("authorization", "Bearer old"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"fresh","expires_in":3600})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/me"))
        .and(header("authorization", "Bearer fresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"user"})))
        .expect(12)
        .mount(&server)
        .await;
    let (client, tokens, sink) = setup(&server, token("old", 3600));
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let c = client.clone();
        tasks.push(tokio::spawn(async move { c.validate().await.unwrap() }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().premium, None);
    }
    assert_eq!(sink.0.lock().unwrap().len(), 1);
    assert_eq!(
        tokens.current().await.unwrap().refresh_token.as_deref(),
        Some("refresh")
    );
}

#[tokio::test]
async fn refreshes_near_expiry_and_persists_at_startup() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"fresh","expires_in":3600})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (_, tokens, sink) = setup(&server, token("old", 599));
    let (a, b) = tokio::join!(tokens.access_token(), tokens.access_token());
    assert_eq!(a.unwrap(), "fresh");
    assert_eq!(b.unwrap(), "fresh");
    assert_eq!(sink.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn player_sends_run_and_reports_premium_required() {
    let server = MockServer::start().await;
    Mock::given(method("PUT")).and(path("/me/player/play")).and(query_param("device_id", "dev"))
        .and(body_json(json!({"uris":["spotify:track:4uLU6hMCjMI75M1A2tKUQC"],"offset":{"position":0},"position_ms":123})))
        .respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
    Mock::given(method("PUT"))
        .and(path("/me/player/pause"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({"error":{"reason":"PREMIUM_REQUIRED"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (_, tokens, _) = setup(&server, token("access", 3600));
    let player = WebSpotifyPlayerApi::new(common::http(), tokens).with_api_base(server.uri());
    player
        .start_playback(Some("dev"), &["4uLU6hMCjMI75M1A2tKUQC".into()], 0, 123)
        .await
        .unwrap();
    let error = player.pause(Some("dev")).await.unwrap_err();
    assert_eq!(error.kind, SpotifyErrorKind::PremiumRequired);
}

/// Spotify's front end rejects body-less PUTs without `Content-Length` (HTTP 411), which broke
/// pause/seek/volume/shuffle/repeat against the live API.
#[tokio::test]
async fn bodyless_player_commands_send_content_length_zero() {
    let server = MockServer::start().await;
    for p in ["/me/player/pause", "/me/player/seek", "/me/player/volume"] {
        Mock::given(method("PUT"))
            .and(path(p))
            .and(header("content-length", "0"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
    }
    let (_, tokens, _) = setup(&server, token("access", 3600));
    let player = WebSpotifyPlayerApi::new(common::http(), tokens).with_api_base(server.uri());
    player.pause(Some("dev")).await.unwrap();
    player.seek(1000, Some("dev")).await.unwrap();
    player.set_volume(40, Some("dev")).await.unwrap();
}
