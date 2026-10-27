mod support;
use ap_library::{CatalogRepository, SearchType};
use ap_model::SourceType;
use ap_store::catalog as cache;
use std::sync::{Arc, atomic::Ordering};
use support::*;

#[tokio::test]
async fn refresh_is_parallel_isolated_and_retains_offline_cache() {
    let (_dir, db) = database().await;
    db.call(|c| {
        cache::replace_playlists(
            c,
            SourceType::Plex,
            &[playlist("offline", SourceType::Plex)],
            0,
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let j = Arc::new(Fake::new(SourceType::Jellyfin));
    j.listing
        .lock()
        .unwrap()
        .push(playlist("fresh", SourceType::Jellyfin));
    let p = Arc::new(Fake::new(SourceType::Plex));
    p.fail.store(true, Ordering::SeqCst);
    let repo = CatalogRepository::new(db, vec![p, j]).await.unwrap();
    let mut observed = repo.observe_playlists();
    let failures = repo.refresh_playlists().await.unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].source, SourceType::Plex);
    observed.changed().await.unwrap();
    assert_eq!(
        observed
            .borrow()
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["fresh", "offline"]
    );
}
#[tokio::test]
async fn cache_only_reads_stale_background_revalidation_and_empty_remote_retention() {
    let (_dir, db) = database().await;
    db.call(|c| {
        cache::replace_provider_tracks(
            c,
            SourceType::Jellyfin,
            "p",
            &[track("old", SourceType::Jellyfin)],
            1,
        )
    })
    .await
    .unwrap();
    let fake = Arc::new(Fake::new(SourceType::Jellyfin));
    fake.tracks
        .lock()
        .unwrap()
        .insert("p".into(), vec![track("new", SourceType::Jellyfin)]);
    let repo = CatalogRepository::new(db.clone(), vec![fake.clone()])
        .await
        .unwrap();
    let mut changes = repo.subscribe();
    assert_eq!(
        repo.cached_tracks(SourceType::Jellyfin, "p").await.unwrap()[0].id,
        "old"
    );
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        repo.tracks(SourceType::Jellyfin, "p", false).await.unwrap()[0].id,
        "old"
    );
    changes.recv().await.unwrap();
    assert_eq!(
        repo.cached_tracks(SourceType::Jellyfin, "p").await.unwrap()[0].id,
        "new"
    );
    fake.tracks.lock().unwrap().clear();
    assert_eq!(
        repo.tracks(SourceType::Jellyfin, "p", true).await.unwrap()[0].id,
        "new"
    );
    fake.fail.store(true, Ordering::SeqCst);
    assert!(repo.tracks(SourceType::Jellyfin, "p", true).await.is_err());
    assert_eq!(
        repo.cached_tracks(SourceType::Jellyfin, "p").await.unwrap()[0].id,
        "new"
    );
}
#[tokio::test]
async fn search_retains_successes_in_provider_order_and_errors_only_if_all_fail() {
    let (_dir, db) = database().await;
    let j = Arc::new(Fake::new(SourceType::Jellyfin));
    let p = Arc::new(Fake::new(SourceType::Plex));
    let s = Arc::new(Fake::new(SourceType::Spotify));
    let repo = CatalogRepository::new(db, vec![s.clone(), p.clone(), j.clone()])
        .await
        .unwrap();
    assert!(
        repo.search(" ", SourceType::All, SearchType::Tracks)
            .await
            .unwrap()
            .tracks
            .is_empty()
    );
    assert_eq!(j.calls.load(Ordering::SeqCst), 0);
    let result = repo
        .search("hi", SourceType::All, SearchType::Tracks)
        .await
        .unwrap();
    assert_eq!(
        result.tracks.iter().map(|t| t.source).collect::<Vec<_>>(),
        [SourceType::Jellyfin, SourceType::Plex, SourceType::Spotify]
    );
    p.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        repo.search("hi", SourceType::All, SearchType::Tracks)
            .await
            .unwrap()
            .tracks
            .len(),
        2
    );
    j.fail.store(true, Ordering::SeqCst);
    s.fail.store(true, Ordering::SeqCst);
    let error = repo
        .search("hi", SourceType::All, SearchType::Tracks)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Jellyfin") && error.contains("Plex") && error.contains("Spotify"));
}
#[tokio::test]
async fn cancellation_drops_search_fetches() {
    let (_dir, db) = database().await;
    let mut fake = Fake::new(SourceType::Jellyfin);
    fake.gate = Some(Arc::new(tokio::sync::Semaphore::new(0)));
    let fake = Arc::new(fake);
    let repo = CatalogRepository::new(db, vec![fake.clone()])
        .await
        .unwrap();
    let running = tokio::spawn({
        let repo = repo.clone();
        async move {
            repo.search("query", SourceType::All, SearchType::Tracks)
                .await
        }
    });
    fake.started.acquire().await.unwrap().forget();
    running.abort();
    let _ = running.await;
    assert_eq!(fake.active.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn refresh_all_limits_track_fetches_to_four_and_keeps_listing_order() {
    let (_dir, db) = database().await;
    let fake = Arc::new(Fake::new(SourceType::Jellyfin));
    *fake.listing.lock().unwrap() = (0..12)
        .map(|i| playlist(&i.to_string(), SourceType::Jellyfin))
        .collect();
    let repo = CatalogRepository::new(db, vec![fake.clone()])
        .await
        .unwrap();
    assert!(repo.refresh_all().await.unwrap().is_empty());
    assert!(fake.peak.load(Ordering::SeqCst) <= 4);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 13);
    assert_eq!(repo.cached_playlists().await.unwrap()[11].id, "11");
}
#[tokio::test]
async fn spotify_uri_and_url_share_cache_and_distinct_preference() {
    let (_dir, db) = database().await;
    let repo = CatalogRepository::new(db, vec![]).await.unwrap();
    repo.set_distinct(
        SourceType::Spotify,
        "https://open.spotify.com/playlist/abc?si=1",
        true,
    )
    .await
    .unwrap();
    assert!(
        repo.is_distinct(SourceType::Spotify, "spotify:playlist:abc")
            .await
            .unwrap()
    );
}
