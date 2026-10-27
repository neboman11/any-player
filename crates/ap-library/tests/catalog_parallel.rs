mod support;
use ap_library::CatalogRepository;
use ap_model::SourceType;
use std::sync::Arc;
use support::{Fake, database};

#[tokio::test]
async fn both_provider_requests_start_before_either_finishes() {
    let (_dir, db) = database().await;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let mut j = Fake::new(SourceType::Jellyfin);
    j.gate = Some(gate.clone());
    let j = Arc::new(j);
    let mut p = Fake::new(SourceType::Plex);
    p.gate = Some(gate.clone());
    let p = Arc::new(p);
    let repo = CatalogRepository::new(db, vec![j.clone(), p.clone()])
        .await
        .unwrap();
    let refresh = tokio::spawn(async move { repo.refresh_playlists().await });
    j.started.acquire().await.unwrap().forget();
    p.started.acquire().await.unwrap().forget();
    gate.add_permits(2);
    assert!(refresh.await.unwrap().unwrap().is_empty());
}
