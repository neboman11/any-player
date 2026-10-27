use super::*;
use crate::vm::{ProviderForm, ProvidersCommand, ProvidersPort, SettingsCommand, SettingsPort};
use ap_model::SourceType;
use ap_playback::ports::{ConnectionGate, StreamResolver};
use ap_store::secrets::{MemoryBackend, SecretAccount, Secrets};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

#[tokio::test]
async fn jellyfin_connection_validates_persists_user_and_gates_playback() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/System/Info"))
        .and(header("x-emby-token", "secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"Id":"server"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Users"))
        .and(header("x-emby-token", "secret"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!([{"Id":"user-1","Name":"Listener"}])),
        )
        .mount(&server)
        .await;
    let temp = tempfile::tempdir().unwrap();
    let paths = ap_store::paths::Paths::rooted(temp.path());
    let backend = Arc::new(MemoryBackend::new());
    let secrets = Arc::new(Secrets::new(backend.clone()));
    let (services, _) = crate::startup::boot(&paths, secrets).await.unwrap();
    let track = ap_model::Track::new("t1", "Song", "Artist", SourceType::Jellyfin);
    assert!(
        services.providers.resolve(&track).await.is_err(),
        "no session and no URL cannot resolve"
    );
    assert!(
        services
            .providers
            .disabled_name(SourceType::Jellyfin)
            .is_some()
    );
    services
        .providers
        .providers(ProvidersCommand::Connect(ProviderForm {
            source: SourceType::Jellyfin,
            url: server.uri(),
            secret: "secret".into(),
            page_size: 77,
            self_signed: true,
        }))
        .await
        .unwrap();
    assert!(
        services
            .providers
            .disabled_name(SourceType::Jellyfin)
            .is_none()
    );
    assert_eq!(
        services
            .settings_store
            .get()
            .providers
            .jellyfin_user_id
            .as_deref(),
        Some("user-1")
    );
    assert_eq!(
        services.settings_store.get().providers.jellyfin_page_size,
        77
    );
    assert_eq!(
        backend.raw(SecretAccount::JellyfinApiKey).as_deref(),
        Some("secret")
    );
    let stream = services.providers.resolve(&track).await.unwrap();
    assert!(
        stream.allow_invalid_certs,
        "self-signed setting reaches the local HTTP source"
    );
    assert!(stream.url.as_str().starts_with(&server.uri()));
    assert_eq!(
        stream
            .headers
            .get("x-emby-token")
            .and_then(|v| v.to_str().ok()),
        Some("secret")
    );
    let artwork = services
        .providers
        .image_request(&format!("{}/Items/t1/Images/Primary", server.uri()))
        .expect("server artwork is authenticated");
    assert_eq!(
        artwork
            .headers
            .get("x-emby-token")
            .and_then(|v| v.to_str().ok()),
        Some("secret")
    );
    assert!(
        services
            .providers
            .image_request("https://i.scdn.co/image/abc")
            .is_none()
    );
    let restored_paths = ap_store::paths::Paths::rooted(temp.path());
    let (restored, status) =
        crate::startup::boot(&restored_paths, Arc::new(Secrets::new(backend.clone())))
            .await
            .unwrap();
    assert!(status.warnings.is_empty(), "{:?}", status.warnings);
    assert!(
        restored
            .providers
            .disabled_name(SourceType::Jellyfin)
            .is_none(),
        "saved connection restores at startup"
    );
    assert!(
        restored
            .providers
            .resolve(&track)
            .await
            .unwrap()
            .allow_invalid_certs,
        "self-signed preference restores"
    );
    let revalidation: Arc<dyn ap_sync::ProviderRevalidation> = restored.providers.clone();
    revalidation.revalidate(SourceType::Jellyfin).await.unwrap();
    server.reset().await;
    let failure = revalidation
        .revalidate(SourceType::Jellyfin)
        .await
        .unwrap_err();
    assert!(failure.contains("404"), "{failure}");
    assert!(
        !failure.starts_with("Jellyfin"),
        "sync engine adds the source prefix"
    );
    assert!(
        restored
            .providers
            .restore_step(SourceType::Jellyfin)
            .await
            .unwrap_err()
            .0
            .starts_with(SourceType::Jellyfin.name()),
        "startup retry messages keep the source prefix"
    );
    services
        .providers
        .providers(ProvidersCommand::Disconnect(SourceType::Jellyfin))
        .await
        .unwrap();
    assert!(backend.raw(SecretAccount::JellyfinApiKey).is_none());
    assert!(
        services
            .providers
            .disabled_name(SourceType::Jellyfin)
            .is_some()
    );
}
#[tokio::test]
async fn artwork_cache_injects_provider_auth_serves_hits_from_disk_and_evicts_oldest() {
    use crate::images::{DiskImageCache, ImageCache};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/System/Info"))
        .and(header("x-emby-token", "secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"Id":"server"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Users"))
        .and(header("x-emby-token", "secret"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!([{"Id":"user-1","Name":"Listener"}])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items/a/Images/Primary"))
        .and(header("x-emby-token", "secret"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8; 10]))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items/b/Images/Primary"))
        .and(header("x-emby-token", "secret"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![2u8; 10]))
        .expect(1)
        .mount(&server)
        .await;
    let temp = tempfile::tempdir().unwrap();
    let paths = ap_store::paths::Paths::rooted(temp.path());
    let (services, _) = crate::startup::boot(
        &paths,
        Arc::new(Secrets::new(Arc::new(MemoryBackend::new()))),
    )
    .await
    .unwrap();
    services
        .providers
        .providers(ProvidersCommand::Connect(ProviderForm {
            source: SourceType::Jellyfin,
            url: server.uri(),
            secret: "secret".into(),
            page_size: 50,
            self_signed: false,
        }))
        .await
        .unwrap();
    let directory = temp.path().join("artwork");
    let cache = DiskImageCache::authenticated(directory.clone(), services.providers.clone())
        .with_capacity(15);
    let a = format!("{}/Items/a/Images/Primary", server.uri());
    let b = format!("{}/Items/b/Images/Primary", server.uri());
    assert_eq!(cache.load(&a).await.unwrap(), vec![1u8; 10]);
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert_eq!(cache.load(&b).await.unwrap(), vec![2u8; 10]);
    assert_eq!(
        std::fs::read_dir(&directory).unwrap().count(),
        1,
        "oldest entry evicted past capacity"
    );
    assert_eq!(
        cache.load(&b).await.unwrap(),
        vec![2u8; 10],
        "hit served from disk"
    );
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert_eq!(
        cache.load(&a).await.unwrap(),
        vec![1u8; 10],
        "evicted entry refetched with auth"
    );
}
#[tokio::test]
async fn settings_apply_to_engine_and_persist_sort_and_close_policy() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ap_store::paths::Paths::rooted(temp.path());
    let secrets = Arc::new(Secrets::new(Arc::new(MemoryBackend::new())));
    let (services, _) = crate::startup::boot(&paths, secrets).await.unwrap();
    services
        .settings
        .settings(SettingsCommand::Normalize(true))
        .await
        .unwrap();
    services.engine.flush().await;
    assert!(services.engine.player().borrow().normalization);
    services
        .settings
        .settings(SettingsCommand::CloseToTray(false))
        .await
        .unwrap();
    services
        .settings
        .settings(SettingsCommand::Sort {
            key: "table".into(),
            preference: crate::vm::SortPreference {
                column: "artist".into(),
                descending: true,
            },
        })
        .await
        .unwrap();
    assert!(!services.settings_store.get().ui.close_to_tray);
    assert!(!services.settings_store.get().ui.track_sort["table"].ascending);
}
