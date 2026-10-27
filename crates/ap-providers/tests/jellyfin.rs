mod common;

use ap_model::{SourceType, Track};
use ap_providers::{JellyfinClient, JellyfinConfig, ProviderClient, ProviderError};
use common::{FAST_RETRY, http, raw_header};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "secret-key";
const AUTH: &str = r#"MediaBrowser Token="secret-key", Client="AnyPlayer", Device="AnyPlayer", DeviceId="install-1", Version="0.2.0""#;

fn client(server: &MockServer, user_id: Option<&str>, page_size: u32) -> JellyfinClient {
    JellyfinClient::new(
        http(),
        JellyfinConfig {
            base_url: format!("{}/", server.uri()),
            api_key: KEY.into(),
            user_id: user_id.map(str::to_owned),
            page_size,
            device_id: "install-1".into(),
            app_version: "0.2.0".into(),
        },
    )
    .unwrap()
    .with_retry_policy(FAST_RETRY)
}

async fn mount_users(server: &MockServer, expect: u64) {
    Mock::given(method("GET"))
        .and(path("/Users"))
        .and(header("X-Emby-Token", KEY))
        .and(raw_header("X-Emby-Authorization", AUTH))
        .and(raw_header("Authorization", AUTH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"Id": "u1", "Name": "First"},
            {"Id": "u2", "Name": "Second"}
        ])))
        .expect(expect)
        .mount(server)
        .await;
}

fn audio(id: &str) -> Value {
    json!({"Id": id, "Type": "Audio", "Name": format!("Song {id}"), "Artists": ["A"], "Album": "Al"})
}

#[tokio::test]
async fn validate_resolves_first_user_with_auth_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/System/Info"))
        .and(header("X-Emby-Token", KEY))
        .and(header("Accept", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ServerName": "S"})))
        .expect(1)
        .mount(&server)
        .await;
    mount_users(&server, 1).await;
    let v = client(&server, None, 300).validate().await.unwrap();
    assert_eq!(v.jellyfin_user_id.as_deref(), Some("u1"));
    assert_eq!(v.username.as_deref(), Some("First"));
}

#[tokio::test]
async fn stored_user_is_preferred_and_resolution_is_cached() {
    let server = MockServer::start().await;
    mount_users(&server, 1).await;
    Mock::given(method("GET"))
        .and(path("/Users/u2/Items"))
        .and(query_param("IncludeItemTypes", "Playlist"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"Items": [], "TotalRecordCount": 0})),
        )
        .expect(2)
        .mount(&server)
        .await;
    let c = client(&server, Some("u2"), 300);
    assert_eq!(c.playlists().await.unwrap(), [] as [ap_model::Playlist; 0]);
    assert_eq!(c.playlists().await.unwrap(), [] as [ap_model::Playlist; 0]);
    assert_eq!(c.resolve_user().await.unwrap().id, "u2");
}

#[tokio::test]
async fn unknown_stored_user_falls_back_to_first() {
    let server = MockServer::start().await;
    mount_users(&server, 1).await;
    assert_eq!(
        client(&server, Some("gone"), 300)
            .resolve_user()
            .await
            .unwrap()
            .id,
        "u1"
    );
}

#[tokio::test]
async fn empty_user_list_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(path("/Users"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    assert!(matches!(
        client(&server, None, 300).resolve_user().await,
        Err(ProviderError::NoUsers)
    ));
}

async fn mount_page(server: &MockServer, start: usize, ids: &[&str], total: Option<usize>) {
    let mut body = json!({"Items": ids.iter().map(|id| audio(id)).collect::<Vec<_>>()});
    if let Some(total) = total {
        body["TotalRecordCount"] = json!(total);
    }
    Mock::given(method("GET"))
        .and(path("/Users/u1/Items"))
        .and(query_param("ParentId", "pl"))
        .and(query_param("Limit", "2"))
        .and(query_param("StartIndex", start.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn playlist_tracks_page_until_short_page() {
    let server = MockServer::start().await;
    mount_users(&server, 1).await;
    mount_page(&server, 0, &["1", "2"], Some(5)).await;
    mount_page(&server, 2, &["3", "4"], Some(5)).await;
    mount_page(&server, 4, &["5"], Some(5)).await;
    let tracks = client(&server, None, 2)
        .playlist_tracks("pl")
        .await
        .unwrap();
    assert_eq!(
        tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["1", "2", "3", "4", "5"]
    );
}

#[tokio::test]
async fn playlist_tracks_stop_at_total_without_extra_request() {
    let server = MockServer::start().await;
    mount_users(&server, 1).await;
    mount_page(&server, 0, &["1", "2"], Some(4)).await;
    mount_page(&server, 2, &["3", "4"], Some(4)).await;
    assert_eq!(
        client(&server, None, 2)
            .playlist_tracks("pl")
            .await
            .unwrap()
            .len(),
        4
    );
}

#[tokio::test]
async fn playlist_tracks_stop_on_empty_page_without_total() {
    let server = MockServer::start().await;
    mount_users(&server, 1).await;
    mount_page(&server, 0, &["1", "2"], None).await;
    mount_page(&server, 2, &[], None).await;
    assert_eq!(
        client(&server, None, 2)
            .playlist_tracks("pl")
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn track_mapping_matches_android() {
    let server = MockServer::start().await;
    mount_users(&server, 1).await;
    Mock::given(path("/Users/u1/Items"))
        .and(query_param("Fields", "AudioInfo,MediaSources,ParentId"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"Items": [
            {"Id": "a", "Type": "Audio", "Name": "Full", "Artists": ["X", "Y"], "Album": "Al",
             "RunTimeTicks": 1_800_000_000_i64, "AlbumId": "alb", "AlbumPrimaryImageTag": "t1",
             "MediaSources": [{"Bitrate": 900_000, "MediaStreams": [{"Type": "Video"}, {"Type": "Audio", "BitRate": 320_000, "SampleRate": 44_100}]}]},
            {"Id": "b", "Type": "Audio", "Name": "Bare", "Artists": [], "RunTimeTicks": 0,
             "ImageTags": {"Primary": "t2"}, "MediaSources": [{"Bitrate": 128_000}]},
            {"Id": "c", "Type": "Video", "Name": "Skipped"}
        ], "TotalRecordCount": 3})))
        .mount(&server)
        .await;
    let tracks = client(&server, None, 300)
        .playlist_tracks(" pl ")
        .await
        .unwrap();
    assert_eq!(tracks.len(), 2);
    let base = server.uri();
    let full = &tracks[0];
    assert_eq!(full.source, SourceType::Jellyfin);
    assert_eq!(full.artist, "X, Y");
    assert_eq!(full.album.as_deref(), Some("Al"));
    assert_eq!(full.duration_ms, Some(180_000));
    assert_eq!(full.bitrate_kbps, Some(320));
    assert_eq!(full.sample_rate_hz, Some(44_100));
    assert_eq!(full.enriched, Some(true));
    assert_eq!(
        full.image_url.as_deref(),
        Some(format!("{base}/Items/alb/Images/Primary?tag=t1").as_str())
    );
    assert_eq!(
        full.url.as_deref(),
        Some(format!("{base}/Audio/a/universal?UserId=u1&Container=mp3,aac,m4a,flac,ogg,wav&AudioCodec=aac,mp3,vorbis,flac&TranscodingContainer=mp3&TranscodingProtocol=http").as_str())
    );
    let bare = &tracks[1];
    assert_eq!(bare.artist, "Unknown Artist");
    assert_eq!(bare.album.as_deref(), Some("Unknown Album"));
    assert_eq!(bare.duration_ms, None);
    assert_eq!(bare.bitrate_kbps, Some(128));
    assert_eq!(bare.sample_rate_hz, None);
    assert_eq!(
        bare.image_url.as_deref(),
        Some(format!("{base}/Items/b/Images/Primary?tag=t2").as_str())
    );
    assert!(
        tracks
            .iter()
            .all(|t| !t.url.as_deref().unwrap().contains(KEY)
                && !t.image_url.as_deref().unwrap().contains(KEY))
    );
}

#[tokio::test]
async fn playlists_map_counts_and_owner() {
    let server = MockServer::start().await;
    mount_users(&server, 1).await;
    Mock::given(path("/Users/u1/Items"))
        .and(query_param("IncludeItemTypes", "Playlist"))
        .and(query_param("Recursive", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"Items": [
            {"Id": "p1", "Name": "One", "ChildCount": 3},
            {"Id": "p2", "Name": "Two", "RecursiveItemCount": 7},
            {"Name": "no id"}
        ], "TotalRecordCount": 3})))
        .mount(&server)
        .await;
    let playlists = client(&server, None, 300).playlists().await.unwrap();
    assert_eq!(playlists.len(), 2);
    assert_eq!((playlists[0].track_count, playlists[1].track_count), (3, 7));
    assert!(
        playlists
            .iter()
            .all(|p| p.owner == "Jellyfin" && p.source == SourceType::Jellyfin)
    );
}

#[tokio::test]
async fn search_is_one_page_of_fifty_and_blank_is_local() {
    let server = MockServer::start().await;
    mount_users(&server, 1).await;
    Mock::given(path("/Users/u1/Items"))
        .and(query_param("searchTerm", "rock & roll"))
        .and(query_param("IncludeItemTypes", "Audio"))
        .and(query_param("Limit", "50"))
        .and(query_param("StartIndex", "0"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"Items": [audio("1")], "TotalRecordCount": 900})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/Users/u1/Items"))
        .and(query_param("IncludeItemTypes", "Playlist"))
        .and(query_param("Limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"Items": [{"Id": "p"}]})))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server, None, 300);
    assert_eq!(c.search_tracks("rock & roll").await.unwrap().len(), 1);
    assert_eq!(c.search_playlists("x").await.unwrap().len(), 1);
    assert_eq!(
        c.search_tracks("   ").await.unwrap(),
        [] as [ap_model::Track; 0]
    );
}

#[tokio::test]
async fn stream_request_puts_auth_in_headers_only() {
    let server = MockServer::start().await;
    mount_users(&server, 1).await;
    let c = client(&server, None, 300);
    let mut track = Track::new("abc", "T", "A", SourceType::Jellyfin);
    track.url = Some(format!(
        "{}/Audio/abc/universal?UserId=u1&Container=opus,webm&api_key={KEY}",
        server.uri()
    ));
    let request = c.stream_request(&track).await.unwrap();
    assert_eq!(
        request.url.as_str(),
        format!(
            "{}/Audio/abc/universal?UserId=u1&Container=mp3,aac,m4a,flac,ogg,wav&AudioCodec=aac,mp3,vorbis,flac&TranscodingContainer=mp3&TranscodingProtocol=http",
            server.uri()
        )
    );
    assert_eq!(request.headers["x-emby-token"], KEY);
    assert_eq!(request.headers["authorization"], AUTH);
}

#[tokio::test]
async fn image_request_strips_legacy_token_and_scopes_headers_to_server() {
    let server = MockServer::start().await;
    let c = client(&server, None, 300);
    let legacy = format!(
        "{}/Items/alb/Images/Primary?tag=t1&api_key={KEY}",
        server.uri()
    );
    let own = c.image_request(&legacy).unwrap();
    assert_eq!(
        own.url.as_str(),
        format!("{}/Items/alb/Images/Primary?tag=t1", server.uri())
    );
    assert_eq!(own.headers["x-emby-token"], KEY);
    let foreign = c
        .image_request("https://cdn.example.com/a.jpg?api_key=leak")
        .unwrap();
    assert_eq!(foreign.url.as_str(), "https://cdn.example.com/a.jpg");
    assert!(foreign.headers.is_empty());
    assert!(c.image_request("not a url").is_none());
}

#[tokio::test]
async fn http_errors_and_retries_surface() {
    let server = MockServer::start().await;
    Mock::given(path("/System/Info"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(path("/System/Info"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let err = client(&server, None, 300).validate().await.unwrap_err();
    assert_eq!(err.status(), Some(401));
}

#[test]
fn rejects_non_http_urls() {
    let err = JellyfinClient::new(
        http(),
        JellyfinConfig {
            base_url: "jellyfin.local".into(),
            api_key: KEY.into(),
            user_id: None,
            page_size: 300,
            device_id: "d".into(),
            app_version: "v".into(),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ProviderError::InvalidConfig(_)));
}
