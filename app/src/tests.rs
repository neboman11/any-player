use crate::{fake::DemoServices, vm::*};
use std::sync::Arc;

#[tokio::test]
async fn playback_occurrences_and_play_next_are_stable() {
    let service = DemoServices::new();
    let before = PlaybackPort::subscribe(&service).borrow().clone();
    let track = before.queue.current().unwrap().track.clone();
    service
        .playback(PlaybackCommand::PlayNext(track.clone()))
        .await
        .unwrap();
    let after = PlaybackPort::subscribe(&service).borrow().clone();
    assert_eq!(after.queue.upcoming().next().unwrap().track.id, track.id);
    assert_ne!(
        after.queue.current().unwrap().uid,
        after.queue.upcoming().next().unwrap().uid
    );
}

#[tokio::test]
async fn failed_command_leaves_state_intact() {
    let service = DemoServices::new();
    service.fail_next("offline");
    assert!(service.playback(PlaybackCommand::Toggle).await.is_err());
    assert!(!PlaybackPort::subscribe(&service).borrow().playing);
}

#[tokio::test]
async fn union_creation_honors_type_and_excludes_unions() {
    let service = DemoServices::new();
    service
        .library(LibraryCommand::Create {
            name: "Union test".into(),
            union: true,
        })
        .await
        .unwrap();
    let state = LibraryPort::subscribe(&service).borrow().clone();
    let item = state.catalog.last().unwrap();
    assert!(item.union);
    assert!(
        state
            .union_candidates(&item.id)
            .iter()
            .all(|p| !p.union && p.id != item.id)
    );
}

#[tokio::test]
async fn sync_account_commits_and_conflict_choices_resolve() {
    let service = DemoServices::new();
    service
        .sync(SyncCommand::Commit {
            url: " demo.example/ ".into(),
            token: format!("Bearer {} ", "demo-value"),
        })
        .await
        .unwrap();
    let state = SyncPort::subscribe(&service).borrow().clone();
    assert_eq!(state.url, "https://demo.example");
    assert_eq!(state.token, "demo-value");
    service.sync(SyncCommand::Connect).await.unwrap();
    assert!(SyncPort::subscribe(&service).borrow().conflict);
    service
        .sync(SyncCommand::Resolve(SyncChoice::Cancel))
        .await
        .unwrap();
    assert!(!SyncPort::subscribe(&service).borrow().connected);
}

#[tokio::test]
async fn search_keeps_partial_success_and_pages_twenty() {
    let service = DemoServices::new();
    service
        .search(SearchRequest {
            query: String::new(),
            source: ap_model::SourceType::All,
            playlists: false,
            page: 0,
            generation: 1,
        })
        .await
        .unwrap();
    assert_eq!(SearchPort::subscribe(&service).borrow().tracks.len(), 20);
    service.fail_provider(ap_model::SourceType::Spotify);
    service
        .search(SearchRequest {
            query: String::new(),
            source: ap_model::SourceType::All,
            playlists: false,
            page: 1,
            generation: 2,
        })
        .await
        .unwrap();
    let state = SearchPort::subscribe(&service).borrow().clone();
    assert_eq!(state.error, "");
    assert!(
        state
            .tracks
            .iter()
            .all(|t| t.source != ap_model::SourceType::Spotify)
    );
}

#[test]
fn sorted_or_filtered_tables_cannot_reorder() {
    let mut table = TrackTable::default();
    assert!(table.can_reorder());
    table.filter = "track".into();
    assert!(!table.can_reorder());
    table.filter.clear();
    table.cycle_sort("title");
    assert!(!table.can_reorder());
    table.cycle_sort("title");
    table.cycle_sort("title");
    assert!(table.can_reorder());
}

#[tokio::test]
async fn all_page_view_models_accept_service_ports() {
    let service = Arc::new(DemoServices::new());
    let playback = playback::PlaybackVm::new(service.clone());
    playback.dispatch(PlaybackCommand::Toggle).await.unwrap();
    assert!(playback.state.borrow().playing);
    let library = library::LibraryVm::new(service.clone());
    library
        .dispatch(LibraryCommand::Create {
            name: "VM playlist".into(),
            union: false,
        })
        .await
        .unwrap();
    assert!(
        library
            .state
            .borrow()
            .catalog
            .iter()
            .any(|p| p.name == "VM playlist")
    );
    let search = search::SearchVm::new(service.clone());
    search
        .dispatch(SearchRequest {
            query: "Northern".into(),
            source: ap_model::SourceType::All,
            playlists: false,
            page: 0,
            generation: 1,
        })
        .await
        .unwrap();
    assert_ne!(search.state.borrow().tracks, [] as [ap_model::Track; 0]);
    let settings = settings::SettingsVm::new(service.clone());
    settings
        .dispatch(SettingsCommand::Normalize(false))
        .await
        .unwrap();
    assert!(!settings.state.borrow().normalize);
    let sync = sync::SyncVm::new(service.clone());
    sync.dispatch(SyncCommand::Connect).await.unwrap();
    assert!(sync.state.borrow().conflict);
    let dj = dj::DjVm::new(service.clone());
    dj.dispatch(DjCommand::Enable(true)).await.unwrap();
    assert!(dj.state.borrow().enabled);
    let providers = providers::ProvidersVm::new(service);
    providers
        .dispatch(ProvidersCommand::Preferred("speaker".into()))
        .await
        .unwrap();
    assert_eq!(providers.state.borrow().preferred, "speaker");
}

#[tokio::test]
async fn search_discards_obsolete_generations_and_reports_total_failure() {
    let service = DemoServices::new();
    for source in [
        ap_model::SourceType::Spotify,
        ap_model::SourceType::Jellyfin,
        ap_model::SourceType::Plex,
    ] {
        service.fail_provider(source);
    }
    let request = SearchRequest {
        query: String::new(),
        source: ap_model::SourceType::All,
        playlists: false,
        page: 0,
        generation: 9,
    };
    service.search(request.clone()).await.unwrap();
    assert_eq!(
        SearchPort::subscribe(&service).borrow().error,
        "Search failed"
    );
    service
        .search(SearchRequest {
            generation: 1,
            ..request
        })
        .await
        .unwrap();
    assert_eq!(SearchPort::subscribe(&service).borrow().generation, 9);
}

#[tokio::test]
async fn reordering_uses_occurrence_not_track_identity() {
    let service = DemoServices::new();
    let before = LibraryPort::subscribe(&service)
        .borrow()
        .materialize("playlist-0");
    let key = before[3].key.clone();
    service
        .library(LibraryCommand::Move {
            id: "playlist-0".into(),
            key: key.clone(),
            position: 0,
        })
        .await
        .unwrap();
    let after = LibraryPort::subscribe(&service)
        .borrow()
        .materialize("playlist-0");
    assert_eq!(after[0].key, key);
    assert_eq!(after[1].key, before[0].key);
    service
        .library(LibraryCommand::Remove {
            id: "playlist-0".into(),
            key,
        })
        .await
        .unwrap();
    assert_eq!(
        LibraryPort::subscribe(&service)
            .borrow()
            .materialize("playlist-0")
            .len(),
        before.len() - 1
    );
}

#[tokio::test]
async fn column_visibility_and_sort_are_persisted_in_service_snapshot() {
    let service = DemoServices::new();
    let mut columns = SettingsPort::subscribe(&service).borrow().columns.clone();
    columns[1].visible = false;
    columns[0].position = 4;
    service
        .settings(SettingsCommand::Columns(columns.clone()))
        .await
        .unwrap();
    service
        .settings(SettingsCommand::Sort {
            key: "table".into(),
            preference: SortPreference {
                column: "duration".into(),
                descending: true,
            },
        })
        .await
        .unwrap();
    let state = SettingsPort::subscribe(&service).borrow().clone();
    assert_eq!(state.columns, columns);
    assert!(state.sorts["table"].descending);
    assert_eq!(state.sorts["table"].column, "duration");
}

#[tokio::test]
async fn dj_level_is_clamped_and_logs_are_cleared() {
    let service = DemoServices::new();
    service.dj(DjCommand::Debug(true)).await.unwrap();
    service.dj(DjCommand::Level(9.0)).await.unwrap();
    assert!((DjPort::subscribe(&service).borrow().level - 2.0).abs() < f32::EPSILON);
    assert_ne!(
        DjPort::subscribe(&service).borrow().logs,
        [] as [std::string::String; 0]
    );
    service.dj(DjCommand::Debug(false)).await.unwrap();
    service.dj(DjCommand::ClearLogs).await.unwrap();
    assert_eq!(
        DjPort::subscribe(&service).borrow().logs,
        [] as [std::string::String; 0]
    );
}

#[tokio::test]
async fn provider_validation_and_device_preference_use_ports() {
    let service = DemoServices::new();
    let form = ProviderForm {
        source: ap_model::SourceType::Plex,
        url: String::new(),
        secret: String::new(),
        page_size: 300,
        self_signed: false,
    };
    assert_eq!(
        service
            .providers(ProvidersCommand::Connect(form))
            .await
            .unwrap_err()
            .to_string(),
        "Please enter both URL and token"
    );
    service
        .providers(ProvidersCommand::Preferred("speaker".into()))
        .await
        .unwrap();
    assert_eq!(
        ProvidersPort::subscribe(&service).borrow().preferred,
        "speaker"
    );
}

#[test]
fn ten_thousand_table_rows_sort_filter_and_keep_occurrence_keys() {
    let rows: Vec<_> = (0..10_000)
        .map(|i| TrackOccurrence {
            key: format!("occurrence-{i}"),
            track: ap_model::Track::new(
                "same",
                format!("Track {i:05}"),
                "Artist",
                ap_model::SourceType::Jellyfin,
            ),
        })
        .collect();
    let table = TrackTable {
        filter: "track 099".into(),
        sort: SortPreference {
            column: "title".into(),
            descending: true,
        },
    };
    let result = table.rows(&rows);
    assert_eq!(result.len(), 100);
    assert_eq!(result[0].key, "occurrence-9999");
    assert_eq!(result[99].key, "occurrence-9900");
}

#[tokio::test]
async fn transfer_round_trips_encrypted_state_and_config_with_dry_run() {
    let directory =
        std::env::temp_dir().join(format!("any-player-ui-transfer-{}", std::process::id()));
    tokio::fs::create_dir_all(&directory).await.unwrap();
    let service = DemoServices::new();
    for (name, export, import, private) in [
        (
            "private.json",
            TransferKind::ExportState,
            TransferKind::ImportState,
            true,
        ),
        (
            "config.json",
            TransferKind::ExportConfig,
            TransferKind::ImportConfig,
            false,
        ),
    ] {
        let path = directory.join(name);
        let request = TransferRequest {
            kind: export,
            path: path.clone(),
            private,
            passphrase: "correct horse battery staple".into(),
            replace: true,
            dry_run: false,
        };
        service.transfer(request.clone()).await.unwrap();
        let before = LibraryPort::subscribe(&service).borrow().revision;
        let message = service
            .transfer(TransferRequest {
                kind: import.clone(),
                dry_run: true,
                ..request.clone()
            })
            .await
            .unwrap();
        assert!(message.contains("Dry run"));
        assert_eq!(LibraryPort::subscribe(&service).borrow().revision, before);
        service
            .transfer(TransferRequest {
                kind: import,
                ..request
            })
            .await
            .unwrap();
        assert!(
            LibraryPort::subscribe(&service)
                .borrow()
                .catalog
                .iter()
                .any(|p| p.name == "Late Night Favorites")
        );
        assert!(!PlaybackPort::subscribe(&service).borrow().playing);
        tokio::fs::remove_file(path).await.unwrap();
    }
    tokio::fs::remove_dir(directory).await.unwrap();
}

struct BrokenArtwork;
#[async_trait::async_trait]
impl crate::images::ImageCache for BrokenArtwork {
    async fn load(&self, _url: &str) -> Result<Vec<u8>> {
        Ok(vec![0; 8 * 1024 * 1024 + 1])
    }
}
#[tokio::test]
async fn oversized_artwork_returns_error_instead_of_decoding() {
    let result = crate::images::ImageLoader::decode(Arc::new(BrokenArtwork), "test".into()).await;
    assert!(matches!(result,Err(ServiceError(message)) if message=="Artwork exceeds 8 MiB"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_library_commands_do_not_lose_updates() {
    let service = Arc::new(DemoServices::new());
    let mut jobs = tokio::task::JoinSet::new();
    for i in 0..20 {
        let service = service.clone();
        jobs.spawn(async move {
            service
                .library(LibraryCommand::Create {
                    name: format!("Concurrent {i}"),
                    union: false,
                })
                .await
                .unwrap();
        });
    }
    while let Some(result) = jobs.join_next().await {
        result.unwrap();
    }
    let state = LibraryPort::subscribe(&*service).borrow().clone();
    assert_eq!(
        state
            .catalog
            .iter()
            .filter(|p| p.name.starts_with("Concurrent "))
            .count(),
        20
    );
    let ids: std::collections::HashSet<_> = state.catalog.iter().map(|p| &p.id).collect();
    assert_eq!(ids.len(), state.catalog.len());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_search_keeps_latest_generation() {
    let service = Arc::new(DemoServices::new());
    let mut jobs = tokio::task::JoinSet::new();
    for generation in 1..=20 {
        let service = service.clone();
        jobs.spawn(async move {
            service
                .search(SearchRequest {
                    query: String::new(),
                    source: ap_model::SourceType::All,
                    playlists: false,
                    page: 0,
                    generation,
                })
                .await
                .unwrap();
        });
    }
    while let Some(result) = jobs.join_next().await {
        result.unwrap();
    }
    assert_eq!(SearchPort::subscribe(&*service).borrow().generation, 20);
}

#[tokio::test]
async fn clearing_provider_cache_preserves_custom_and_refresh_reloads_providers() {
    let service = DemoServices::new();
    let custom = LibraryPort::subscribe(&service)
        .borrow()
        .materialize("playlist-0");
    service.library(LibraryCommand::ClearCache).await.unwrap();
    let cleared = LibraryPort::subscribe(&service).borrow().clone();
    assert!(
        cleared
            .catalog
            .iter()
            .all(|p| p.source == ap_model::SourceType::Custom)
    );
    assert!(cleared.materialize("playlist-2").is_empty());
    assert_eq!(cleared.materialize("playlist-0").len(), custom.len());
    service.library(LibraryCommand::Refresh).await.unwrap();
    let refreshed = LibraryPort::subscribe(&service).borrow().clone();
    assert_eq!(refreshed.catalog.len(), 5);
    assert!(!refreshed.materialize("playlist-2").is_empty());
}

#[test]
fn distinct_play_from_duplicate_starts_at_its_keeper_not_queue_start() {
    let row = |key: &str, id: &str, title: &str| TrackOccurrence {
        key: key.into(),
        track: ap_model::Track::new(id, title, "Artist", ap_model::SourceType::Jellyfin),
    };
    let rows = vec![
        row("a", "1", "Song A"),
        row("b", "2", "Song B"),
        row("c", "3", " Song B "),
    ];
    let (tracks, start) = library::playback_selection(&rows, "c", true).unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(start, 1);
    assert_eq!(tracks[start].id, "2");
}

#[test]
fn ordinary_play_from_duplicate_preserves_selected_occurrence() {
    let track = ap_model::Track::new("same", "Song", "Artist", ap_model::SourceType::Plex);
    let rows = vec![
        TrackOccurrence {
            key: "first".into(),
            track: track.clone(),
        },
        TrackOccurrence {
            key: "second".into(),
            track,
        },
    ];
    let (tracks, start) = library::playback_selection(&rows, "second", false).unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(start, 1);
    assert!(library::playback_selection(&rows, "missing", false).is_none());
}
