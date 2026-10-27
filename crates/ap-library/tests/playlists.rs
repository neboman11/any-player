mod support;
use ap_library::{CatalogRepository, PlaylistRepository};
use ap_model::{PlaylistType, SourceType};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::*;

#[tokio::test]
async fn standard_crud_dense_reorder_duplicates_and_play_from_track() {
    let (_dir, db) = database().await;
    let catalog = CatalogRepository::new(db.clone(), vec![]).await.unwrap();
    let repo = PlaylistRepository::new(db, catalog).await.unwrap();
    let p = repo.create(" ", PlaylistType::Standard).await.unwrap();
    assert_eq!(p.name, "Untitled Playlist");
    let mut observed = repo.observe_tracks(&p.id).await.unwrap();
    let mut a = track("a", SourceType::Plex);
    a.title = "Same".into();
    let mut b = track("b", SourceType::Spotify);
    b.title = " same ".into();
    let rows = repo
        .add_tracks(
            &p.id,
            vec![a.clone(), track("other", SourceType::Jellyfin), b.clone()],
        )
        .await
        .unwrap();
    observed.changed().await.unwrap();
    assert_eq!(observed.borrow().len(), 3);
    repo.set_distinct(&p.id, true).await.unwrap();
    let selection = repo.playback_selection(&p.id, Some(2)).await.unwrap();
    assert_eq!(selection.tracks.len(), 2);
    assert_eq!(selection.start_index, 0);
    let groups = repo.duplicate_groups(&p.id).await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].occurrences[0].index, 2);
    repo.reorder_tracks(&p.id, vec![rows[2].id, rows[2].id, -1])
        .await
        .unwrap();
    assert_eq!(
        repo.track_rows(&p.id)
            .await
            .unwrap()
            .iter()
            .map(|r| r.track_id.as_str())
            .collect::<Vec<_>>(),
        ["b", "a", "other"]
    );
    assert!(repo.remove_track_at(&p.id, 1).await.unwrap());
    let rows = repo.track_rows(&p.id).await.unwrap();
    assert_eq!(rows.iter().map(|r| r.position).collect::<Vec<_>>(), [0, 1]);
    repo.rename(&p.id, "new").await.unwrap();
    assert_eq!(repo.get(&p.id).await.unwrap().unwrap().name, "new");
    assert!(repo.delete(&p.id).await.unwrap());
    assert!(repo.cached_tracks(&p.id).await.unwrap().is_empty());
}
#[tokio::test]
async fn union_source_order_one_level_expansion_identity_then_title_dedupe() {
    let (_dir, db) = database().await;
    let fake = Arc::new(Fake::new(SourceType::Plex));
    let mut same = track("same", SourceType::Plex);
    same.title = "Shared".into();
    fake.tracks.lock().unwrap().insert(
        "remote".into(),
        vec![same.clone(), track("r", SourceType::Plex)],
    );
    let catalog = CatalogRepository::new(db.clone(), vec![fake.clone()])
        .await
        .unwrap();
    let repo = PlaylistRepository::new(db, catalog).await.unwrap();
    let standard = repo.create("s", PlaylistType::Standard).await.unwrap();
    let mut other = track("different", SourceType::Jellyfin);
    other.title = "shared".into();
    repo.add_tracks(&standard.id, vec![same, other])
        .await
        .unwrap();
    let nested = repo.create("nested", PlaylistType::Union).await.unwrap();
    repo.replace_union_sources(&nested.id, vec![(SourceType::Custom, standard.id.clone())])
        .await
        .unwrap();
    let union = repo.create("u", PlaylistType::Union).await.unwrap();
    repo.replace_union_sources(
        &union.id,
        vec![
            (SourceType::Plex, "remote".into()),
            (SourceType::Custom, nested.id.clone()),
        ],
    )
    .await
    .unwrap();
    let tracks = repo.materialize_union(&union.id, false).await.unwrap();
    assert_eq!(
        tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["same", "r", "different"]
    );
    assert!(tracks.iter().all(|t| t.enriched == Some(true)));
    repo.set_distinct(&union.id, true).await.unwrap();
    let queue = repo.playback_selection(&union.id, Some(2)).await.unwrap();
    assert_eq!(queue.tracks.len(), 2);
    assert_eq!(queue.start_index, 0);
    let sources = repo.union_sources(&union.id).await.unwrap();
    repo.reorder_union_sources(&union.id, vec![sources[1].id])
        .await
        .unwrap();
    assert_eq!(
        repo.cached_tracks(&union.id)
            .await
            .unwrap()
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        ["same", "different", "r"]
    );
    repo.remove_union_source(&union.id, sources[0].id)
        .await
        .unwrap();
    assert_eq!(repo.cached_tracks(&union.id).await.unwrap().len(), 2);
    repo.replace_union_sources(&union.id, vec![]).await.unwrap();
    assert!(repo.cached_tracks(&union.id).await.unwrap().is_empty());
}
#[tokio::test]
async fn observe_only_union_never_fetches_and_source_edit_survives_cancellation() {
    let (_dir, db) = database().await;
    let mut fake = Fake::new(SourceType::Plex);
    fake.gate = Some(Arc::new(tokio::sync::Semaphore::new(0)));
    let fake = Arc::new(fake);
    let catalog = CatalogRepository::new(db.clone(), vec![fake.clone()])
        .await
        .unwrap();
    let repo = PlaylistRepository::new(db, catalog).await.unwrap();
    let union = repo.create("u", PlaylistType::Union).await.unwrap();
    let _observed = repo.observe_tracks(&union.id).await.unwrap();
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    let edit = tokio::spawn({
        let repo = repo.clone();
        let id = union.id.clone();
        async move { repo.add_union_source(&id, SourceType::Plex, "remote").await }
    });
    tokio::time::timeout(Duration::from_secs(2), fake.started.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    edit.abort();
    let _ = edit.await;
    assert_eq!(repo.union_sources(&union.id).await.unwrap().len(), 1);
    assert!(repo.cached_tracks(&union.id).await.unwrap().is_empty());
    assert_eq!(fake.active.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn provider_distinct_flag_controls_queue_without_mutating_cache() {
    let (_dir, db) = database().await;
    let fake = Arc::new(Fake::new(SourceType::Plex));
    let a = track("a", SourceType::Plex);
    let mut b = a.clone();
    b.id = "b".into();
    fake.tracks.lock().unwrap().insert("p".into(), vec![a, b]);
    let catalog = CatalogRepository::new(db.clone(), vec![fake])
        .await
        .unwrap();
    let repo = PlaylistRepository::new(db, catalog.clone()).await.unwrap();
    catalog
        .set_distinct(SourceType::Plex, "p", true)
        .await
        .unwrap();
    let queue = repo
        .provider_playback_selection(SourceType::Plex, "p", Some(1))
        .await
        .unwrap();
    assert_eq!(queue.tracks.len(), 1);
    assert_eq!(queue.start_index, 0);
    assert_eq!(
        catalog
            .cached_tracks(SourceType::Plex, "p")
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn source_edit_wins_over_materialization_already_in_flight() {
    let (_dir, db) = database().await;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let mut fake = Fake::new(SourceType::Plex);
    fake.gate = Some(gate.clone());
    let fake = Arc::new(fake);
    fake.tracks
        .lock()
        .unwrap()
        .insert("remote".into(), vec![track("stale", SourceType::Plex)]);
    let catalog = CatalogRepository::new(db.clone(), vec![fake.clone()])
        .await
        .unwrap();
    let repo = PlaylistRepository::new(db.clone(), catalog).await.unwrap();
    let union = repo.create("u", PlaylistType::Union).await.unwrap();
    let id = union.id.clone();
    db.call(move |c| ap_store::playlists::add_union_source(c, &id, SourceType::Plex, "remote"))
        .await
        .unwrap();
    let materialize = tokio::spawn({
        let repo = repo.clone();
        let id = union.id.clone();
        async move { repo.materialize_union(&id, true).await }
    });
    fake.started.acquire().await.unwrap().forget();
    repo.replace_union_sources(&union.id, vec![]).await.unwrap();
    gate.add_permits(1);
    assert!(materialize.await.unwrap().unwrap().is_empty());
    assert!(repo.cached_tracks(&union.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn nested_custom_unions_expand_only_one_level_and_local_urls_survive() {
    let (_dir, db) = database().await;
    let catalog = CatalogRepository::new(db.clone(), vec![]).await.unwrap();
    let repo = PlaylistRepository::new(db, catalog).await.unwrap();
    let standard = repo.create("s", PlaylistType::Standard).await.unwrap();
    let mut local = track("file", SourceType::Custom);
    local.url = Some("file:///music.flac".into());
    repo.add_tracks(&standard.id, vec![local]).await.unwrap();
    assert_eq!(
        repo.cached_tracks(&standard.id).await.unwrap()[0]
            .url
            .as_deref(),
        Some("file:///music.flac")
    );
    let first = repo.create("first", PlaylistType::Union).await.unwrap();
    repo.replace_union_sources(&first.id, vec![(SourceType::Custom, standard.id)])
        .await
        .unwrap();
    let second = repo.create("second", PlaylistType::Union).await.unwrap();
    repo.replace_union_sources(&second.id, vec![(SourceType::Custom, first.id)])
        .await
        .unwrap();
    assert_eq!(repo.cached_tracks(&second.id).await.unwrap().len(), 1);
    let third = repo.create("third", PlaylistType::Union).await.unwrap();
    repo.replace_union_sources(&third.id, vec![(SourceType::Custom, second.id)])
        .await
        .unwrap();
    assert!(repo.cached_tracks(&third.id).await.unwrap().is_empty());
}
