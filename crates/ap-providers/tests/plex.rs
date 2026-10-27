mod common;

use ap_model::{SourceType, Track};
use ap_providers::{PlexClient, PlexConfig, ProviderClient, ProviderError};
use common::{FAST_RETRY, counting_json, http};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "plex-secret";
const CLIENT_ID: &str = "install-1";

fn config(base: &str, page_size: u32) -> PlexConfig {
    PlexConfig {
        base_url: base.into(),
        token: TOKEN.into(),
        page_size,
        allow_self_signed: false,
        client_identifier: CLIENT_ID.into(),
    }
}

fn client(server: &MockServer, page_size: u32) -> PlexClient {
    PlexClient::with_http(http(), &config(&server.uri(), page_size))
        .unwrap()
        .with_retry_policy(FAST_RETRY)
}

fn audio(id: &str, codec: &str) -> Value {
    json!({
        "ratingKey": id, "type": "audio", "title": format!("Song {id}"),
        "grandparentTitle": "Artist", "parentTitle": "Album", "duration": 180_000,
        "thumb": format!("/library/metadata/{id}/thumb/500"),
        "Media": [{
            "audioCodec": codec, "bitrate": 320,
            "Part": [{"key": format!("/library/metadata/{id}/stream"), "Stream": [{"streamType": 1}, {"streamType": 2, "samplingRate": 44_100}]}]
        }]
    })
}

fn container(rows: Value, total: Option<usize>) -> Value {
    let mut body = json!({"MediaContainer": {}});
    body["MediaContainer"]["Metadata"] = rows;
    if let Some(total) = total {
        body["MediaContainer"]["totalSize"] = json!(total);
    }
    body
}

#[tokio::test]
async fn validate_uses_headers_and_never_the_url() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/identity"))
        .and(header("X-Plex-Token", TOKEN))
        .and(header("Accept", "application/json"))
        .and(header("X-Plex-Client-Identifier", CLIENT_ID))
        .and(query_param_is_missing("X-Plex-Token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {}})))
        .expect(1)
        .mount(&server)
        .await;
    let v = client(&server, 300).validate().await.unwrap();
    assert!(v.username.is_none() && v.premium.is_none());
}

#[tokio::test]
async fn validate_maps_http_failures_to_legacy_messages() {
    let server = MockServer::start().await;
    Mock::given(path("/identity"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let err = client(&server, 300).validate().await.unwrap_err();
    assert_eq!(err.status(), Some(401));
    assert_eq!(err.to_string(), "Plex authentication failed: HTTP 401");
}

#[tokio::test]
async fn transient_5xx_is_retried_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/playlists/all"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/playlists/all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(container(json!([]), Some(0))))
        .mount(&server)
        .await;
    assert!(client(&server, 300).playlists().await.unwrap().is_empty());
}

async fn mount_page(
    server: &MockServer,
    start: usize,
    rows: Value,
    total: Option<usize>,
) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
    let (count, responder) = counting_json(200, container(rows, total));
    Mock::given(method("GET"))
        .and(path("/playlists/pl/items"))
        .and(query_param("X-Plex-Container-Start", start.to_string()))
        .and(query_param("X-Plex-Container-Size", "2"))
        .respond_with(responder)
        .mount(server)
        .await;
    count
}
#[tokio::test]
async fn playlist_tracks_page_until_short_page() {
    let server = MockServer::start().await;
    mount_page(
        &server,
        0,
        json!([audio("1", "mp3"), audio("2", "mp3")]),
        Some(5),
    )
    .await;
    mount_page(
        &server,
        2,
        json!([audio("3", "mp3"), audio("4", "mp3")]),
        Some(5),
    )
    .await;
    mount_page(&server, 4, json!([audio("5", "mp3")]), Some(5)).await;
    let tracks = client(&server, 2).playlist_tracks("pl").await.unwrap();
    assert_eq!(
        tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["1", "2", "3", "4", "5"]
    );
}

#[tokio::test]
async fn playlist_tracks_stop_at_total_without_extra_request() {
    let server = MockServer::start().await;
    mount_page(
        &server,
        0,
        json!([audio("1", "mp3"), audio("2", "mp3")]),
        Some(4),
    )
    .await;
    let last = mount_page(
        &server,
        2,
        json!([audio("3", "mp3"), audio("4", "mp3")]),
        Some(4),
    )
    .await;
    let tracks = client(&server, 2).playlist_tracks("pl").await.unwrap();
    assert_eq!(tracks.len(), 4);
    assert_eq!(last.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn playlist_tracks_stop_on_empty_page_without_total() {
    let server = MockServer::start().await;
    mount_page(
        &server,
        0,
        json!([audio("1", "mp3"), audio("2", "mp3")]),
        None,
    )
    .await;
    mount_page(&server, 2, json!([]), None).await;
    assert_eq!(
        client(&server, 2)
            .playlist_tracks("pl")
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn track_mapping_and_opus_transcode_selection() {
    let server = MockServer::start().await;
    let mut opus = audio("o", "opus");
    opus["Media"][0]["audioCodec"] = json!("opus");
    Mock::given(path("/playlists/pl/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(container(
            json!([
                opus,
                audio("m", "mp3"),
                {"ratingKey": "x", "type": "audio", "title": "No media"}
            ]),
            Some(3),
        )))
        .mount(&server)
        .await;
    let tracks = client(&server, 300).playlist_tracks("pl").await.unwrap();
    assert_eq!(tracks.len(), 3);
    let base = server.uri();

    let opus = &tracks[0];
    assert_eq!(opus.source, SourceType::Plex);
    assert_eq!(opus.title, "Song o");
    assert_eq!(opus.artist, "Artist");
    assert_eq!(opus.album.as_deref(), Some("Album"));
    assert_eq!(opus.duration_ms, Some(180_000));
    assert_eq!(opus.bitrate_kbps, Some(320));
    assert_eq!(opus.sample_rate_hz, Some(44_100));
    assert_eq!(opus.enriched, Some(true));
    assert_eq!(
        opus.image_url.as_deref(),
        Some(format!("{base}/library/metadata/o/thumb/500").as_str())
    );
    assert_eq!(
        opus.url.as_deref(),
        Some(format!("{base}/music/:/transcode/universal/start.mp3?path=/library/metadata/o&mediaIndex=0&partIndex=0&protocol=http&directPlay=0&directStream=0").as_str())
    );

    assert_eq!(
        tracks[1].url.as_deref(),
        Some(format!("{base}/library/metadata/m/stream").as_str())
    );
    assert_eq!(tracks[2].url, None);

    for track in &tracks {
        assert!(
            !track
                .url
                .as_ref()
                .is_some_and(|u| u.contains(TOKEN) || u.contains("X-Plex-Token"))
        );
        assert!(!track.image_url.as_ref().is_some_and(|u| u.contains(TOKEN)));
    }
}

#[tokio::test]
async fn playlists_map_leaf_count_and_summary() {
    let server = MockServer::start().await;
    Mock::given(path("/playlists/all"))
        .and(query_param("type", "15"))
        .respond_with(ResponseTemplate::new(200).set_body_json(container(
            json!([
                {"ratingKey": "p1", "type": "playlist", "title": "One", "leafCount": 3, "summary": "desc"},
                {"title": "no key"}
            ]),
            Some(2),
        )))
        .mount(&server)
        .await;
    let playlists = client(&server, 300).playlists().await.unwrap();
    assert_eq!(playlists.len(), 1);
    assert_eq!(playlists[0].track_count, 3);
    assert_eq!(playlists[0].description.as_deref(), Some("desc"));
    assert!(
        playlists
            .iter()
            .all(|p| p.owner == "Plex" && p.source == SourceType::Plex)
    );
}

#[tokio::test]
async fn search_filters_types_and_blank_is_local() {
    let server = MockServer::start().await;
    let (count, responder) = counting_json(
        200,
        container(
            json!([
                {
                    "ratingKey": "t1", "type": "track", "title": "Song t1", "grandparentTitle": "A",
                    "parentTitle": "B", "duration": 1, "Media": [{"audioCodec": "mp3", "Part": [{"key": "/s"}]}]
                },
                {
                    "ratingKey": "t2", "type": 10, "title": "Typed 10", "grandparentTitle": "A",
                    "parentTitle": "B", "duration": 1, "Media": [{"audioCodec": "mp3", "Part": [{"key": "/s"}]}]
                },
                {"ratingKey": "p1", "type": "playlist", "title": "Pl", "leafCount": 1},
                {"ratingKey": "p2", "type": "playlist", "title": "Pl2", "leafCount": 2},
                {"ratingKey": "a1", "type": "album", "title": "Al"}
            ]),
            Some(5),
        ),
    );
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("query", "song"))
        .and(query_param("limit", "50"))
        .respond_with(responder)
        .mount(&server)
        .await;
    let c = client(&server, 300);
    assert_eq!(c.search_tracks("song").await.unwrap().len(), 2);
    assert_eq!(c.search_playlists("song").await.unwrap().len(), 2);
    assert!(c.search_tracks("   ").await.unwrap().is_empty());
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[tokio::test]
async fn stream_request_uses_stored_url_and_strips_legacy_tokens() {
    let server = MockServer::start().await;
    let c = client(&server, 300);
    let mut track = Track::new("m", "T", "A", SourceType::Plex);
    track.url = Some(format!(
        "{}/library/metadata/m/stream?X-Plex-Token=old-token",
        server.uri()
    ));
    let request = c.stream_request(&track).await.unwrap();
    assert_eq!(
        request.url.as_str(),
        format!("{}/library/metadata/m/stream", server.uri())
    );
    assert_eq!(request.headers["x-plex-token"], TOKEN);
    assert_eq!(request.headers["x-plex-client-identifier"], CLIENT_ID);
}

#[tokio::test]
async fn stream_request_refetches_metadata_when_url_missing_or_foreign() {
    let server = MockServer::start().await;
    Mock::given(path("/library/metadata/o"))
        .and(query_param_is_missing("X-Plex-Token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(container(json!([audio("o", "opus")]), Some(1))),
        )
        .expect(2)
        .mount(&server)
        .await;
    let c = client(&server, 300);
    let mut track = Track::new("o", "T", "A", SourceType::Plex);
    let rebuilt = c.stream_request(&track).await.unwrap();
    assert_eq!(
        rebuilt.url.as_str(),
        format!(
            "{}/music/:/transcode/universal/start.mp3?path=/library/metadata/o&mediaIndex=0&partIndex=0&protocol=http&directPlay=0&directStream=0",
            server.uri()
        )
    );
    assert_eq!(rebuilt.headers["x-plex-token"], TOKEN);

    track.url = Some("https://other.example/x".into());
    assert!(c.stream_request(&track).await.is_ok());
}

#[tokio::test]
async fn image_request_strips_token_and_scopes_headers_to_server() {
    let server = MockServer::start().await;
    let c = client(&server, 300);
    let own = c
        .image_request(&format!(
            "{}/library/metadata/m/thumb/500?X-Plex-Token={TOKEN}",
            server.uri()
        ))
        .unwrap();
    assert_eq!(
        own.url.as_str(),
        format!("{}/library/metadata/m/thumb/500", server.uri())
    );
    assert_eq!(own.headers["x-plex-token"], TOKEN);
    let foreign = c
        .image_request("https://cdn.example.com/a.jpg?X-Plex-Token=leak")
        .unwrap();
    assert_eq!(foreign.url.as_str(), "https://cdn.example.com/a.jpg");
    assert!(foreign.headers.is_empty());
    assert!(c.image_request("not a url").is_none());
}

#[tokio::test]
async fn invalid_base_url_is_config_error() {
    let err = PlexClient::with_http(http(), &config("plex.local", 300)).unwrap_err();
    assert!(matches!(err, ProviderError::InvalidConfig(_)));
}

#[tokio::test]
async fn server_supplied_media_urls_do_not_persist_tokens() {
    let server = MockServer::start().await;
    let mut row = audio("1", "mp3");
    row["thumb"] = json!("/thumb?X-Plex-Token=secret&v=1");
    row["Media"][0]["Part"][0]["key"] = json!("/audio?X-Plex-Token=secret&v=1");
    Mock::given(path("/playlists/pl/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(container(json!([row]), Some(1))))
        .expect(1)
        .mount(&server)
        .await;
    let tracks = client(&server, 300).playlist_tracks("pl").await.unwrap();
    assert!(!tracks[0].url.as_ref().unwrap().contains("secret"));
    assert!(!tracks[0].image_url.as_ref().unwrap().contains("secret"));
    assert!(tracks[0].url.as_ref().unwrap().contains("v=1"));
}
