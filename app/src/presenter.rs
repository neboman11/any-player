use crate::{
    CardRow, ColumnRow, MainWindow, TrackRow,
    vm::{
        DjCommand, LibraryCommand, PlaybackCommand, PlaylistCard, ProviderForm, ProvidersCommand,
        ProvidersSnapshot, Result, SearchRequest, Services, SettingsCommand, SyncChoice,
        SyncCommand, TrackTable, TransferKind, TransferRequest,
    },
};
use ap_model::{RepeatMode, SourceType, Track};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

fn milliseconds(value: i64) -> Duration {
    Duration::from_millis(u64::try_from(value).unwrap_or_default())
}
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Slider values are rounded and clamped to the u8 range"
)]
fn volume_value(value: f32) -> u8 {
    value.round().clamp(0.0, 100.0) as u8
}

#[derive(Clone, Default)]
struct Context {
    selected: String,
    catalog_tab: i32,
    source_filter: i32,
    table: TrackTable,
    pending: Option<Track>,
    generation: u64,
    library_render: u64,
    playback_render: u64,
    search_task: Option<tokio::task::AbortHandle>,
}
#[derive(Clone)]
struct Bridge {
    ports: Arc<Services>,
    context: Arc<Mutex<Context>>,
    ui: slint::Weak<MainWindow>,
}
fn source(index: i32) -> SourceType {
    match index {
        1 => SourceType::Spotify,
        2 => SourceType::Jellyfin,
        3 => SourceType::Plex,
        4 => SourceType::Custom,
        _ => SourceType::All,
    }
}
fn duration(ms: i64) -> String {
    let seconds = ms.max(0) / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
fn row(track: &Track, key: String, index: usize) -> TrackRow {
    TrackRow {
        key: key.into(),
        title: track.title.clone().into(),
        artist: track.artist.clone().into(),
        album: track.album.clone().unwrap_or_else(|| "—".into()).into(),
        duration: track
            .duration_ms
            .map_or_else(|| "--:--".into(), duration)
            .into(),
        source: format!("{:?}", track.source).into(),
        quality: track
            .bitrate_kbps
            .map_or_else(
                || format!("{:?}", track.source),
                |bitrate| format!("{bitrate} kbps"),
            )
            .into(),
        index: i32::try_from(index).unwrap_or(i32::MAX),
    }
}
fn card(p: &PlaylistCard) -> CardRow {
    CardRow {
        id: p.id.clone().into(),
        name: p.name.clone().into(),
        detail: format!("{} · {} tracks · {:?}", p.owner, p.count, p.source).into(),
    }
}
fn swap<T: Clone + 'static>(model: ModelRc<T>, rows: Vec<T>) -> ModelRc<T> {
    if let Some(existing) = model.as_any().downcast_ref::<VecModel<T>>() {
        existing.set_vec(rows);
        model
    } else {
        Rc::new(VecModel::from(rows)).into()
    }
}
impl Bridge {
    fn toast(&self, text: String) {
        let _ = self
            .ui
            .upgrade_in_event_loop(move |ui| ui.set_toast(text.into()));
    }
    fn launch<F>(&self, future: F)
    where
        F: Future<Output = Result> + Send + 'static,
    {
        let bridge = self.clone();
        tokio::spawn(async move {
            if let Err(error) = future.await {
                bridge.toast(error.to_string());
            }
        });
    }
    fn refresh_library(&self) {
        let bridge = self.clone();
        let (generation, context) = {
            let mut context = self.context.lock().expect("UI context");
            context.library_render += 1;
            (context.library_render, context.clone())
        };
        let sort = context.table.sort.clone();
        tokio::spawn(async move {
            let receiver = bridge.ports.library.subscribe();
            let prepared = tokio::task::spawn_blocking(move || {
                let state = receiver.borrow().clone();
                let selected = state
                    .catalog
                    .iter()
                    .find(|p| p.id == context.selected)
                    .cloned();
                let selected_id = context.selected.clone();
                let all_rows = state.materialize(&selected_id);
                let rows = context.table.rows(&all_rows);
                let mut seen = std::collections::HashSet::new();
                let duplicates = all_rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| {
                        !seen.insert(ap_model::dedupe::key(&r.track.title, &r.track.artist))
                    })
                    .map(|(i, r)| row(&r.track, r.key.clone(), i))
                    .collect::<Vec<_>>();
                let rows = rows
                    .iter()
                    .enumerate()
                    .map(|(i, r)| row(&r.track, r.key.clone(), i))
                    .collect::<Vec<_>>();
                let tab_source = match context.catalog_tab {
                    1 => SourceType::Custom,
                    2 => SourceType::Spotify,
                    3 => SourceType::Jellyfin,
                    4 => SourceType::Plex,
                    _ => SourceType::All,
                };
                let cards = state
                    .catalog
                    .iter()
                    .filter(|p| tab_source == SourceType::All || p.source == tab_source)
                    .map(card)
                    .collect::<Vec<_>>();
                let standard = state
                    .catalog
                    .iter()
                    .filter(|p| p.source == SourceType::Custom && !p.union)
                    .map(card)
                    .collect::<Vec<_>>();
                let candidates = state
                    .union_candidates(&selected_id)
                    .into_iter()
                    .filter(|p| {
                        context.source_filter == 0 || p.source == source(context.source_filter)
                    })
                    .map(card)
                    .collect::<Vec<_>>();
                let sources = state
                    .sources
                    .get(&selected_id)
                    .into_iter()
                    .flatten()
                    .filter_map(|id| state.catalog.iter().find(|p| &p.id == id))
                    .map(card)
                    .collect::<Vec<_>>();
                (
                    selected,
                    rows,
                    duplicates,
                    cards,
                    standard,
                    candidates,
                    sources,
                    context.table.can_reorder(),
                )
            })
            .await;
            if let Ok((selected, rows, duplicates, cards, standard, candidates, sources, reorder)) =
                prepared
            {
                let context = bridge.context.clone();
                let _ = bridge.ui.upgrade_in_event_loop(move |ui| {
                    if context.lock().expect("UI context").library_render != generation {
                        return;
                    }
                    if let Some(p) = selected {
                        ui.set_selected_id(p.id.into());
                        ui.set_selected_name(p.name.into());
                        ui.set_selected_description(p.description.into());
                        ui.set_selected_custom(p.source == SourceType::Custom);
                        ui.set_selected_union(p.union);
                        ui.set_distinct(p.distinct);
                    }
                    ui.set_tracks(swap(ui.get_tracks(), rows));
                    ui.set_duplicates(swap(ui.get_duplicates(), duplicates));
                    ui.set_cards(swap(ui.get_cards(), cards));
                    ui.set_standard_playlists(swap(ui.get_standard_playlists(), standard));
                    ui.set_candidates(swap(ui.get_candidates(), candidates));
                    ui.set_union_sources(swap(ui.get_union_sources(), sources));
                    ui.set_reorder_enabled(reorder);
                    ui.set_sort_column(sort.column.into());
                    ui.set_sort_descending(sort.descending);
                });
            }
        });
    }
    fn refresh_playback(&self) {
        let bridge = self.clone();
        let generation = {
            let mut context = self.context.lock().expect("UI context");
            context.playback_render += 1;
            context.playback_render
        };
        let state = self.ports.playback.subscribe().borrow().clone();
        let dj = self.ports.dj.subscribe().borrow().clone();
        tokio::spawn(async move {
            let next: Vec<_> = state
                .queue
                .upcoming()
                .enumerate()
                .map(|(i, e)| row(&e.track, e.uid.to_string(), i))
                .collect();
            let history: Vec<_> = state
                .queue
                .history()
                .enumerate()
                .map(|(i, e)| row(&e.track, e.uid.to_string(), i))
                .collect();
            let context = bridge.context.clone();
            let _ = bridge.ui.upgrade_in_event_loop(move |ui| {
                if context.lock().expect("UI context").playback_render != generation {
                    return;
                }
                let current = state.queue.current();
                let url = current
                    .and_then(|e| e.track.image_url.as_deref())
                    .unwrap_or_default();
                if ui.get_artwork_url().as_str() != url {
                    ui.set_artwork(slint::Image::default());
                    ui.set_artwork_present(false);
                    ui.set_artwork_url(url.into());
                    if !url.is_empty() {
                        ui.invoke_artwork_requested(url.into());
                    }
                }
                ui.set_now_title(
                    current
                        .map_or("No track playing", |e| e.track.title.as_str())
                        .into(),
                );
                ui.set_now_artist(
                    current
                        .map_or("Select a track to play", |e| e.track.artist.as_str())
                        .into(),
                );
                ui.set_now_album(
                    current
                        .and_then(|e| e.track.album.as_deref())
                        .unwrap_or_default()
                        .into(),
                );
                ui.set_now_source(
                    current
                        .map_or_else(String::new, |e| format!("{:?}", e.track.source))
                        .into(),
                );
                ui.set_quality(
                    current
                        .map_or_else(
                            || "Unknown".into(),
                            |e| {
                                format!(
                                    "{} kbps · {} Hz",
                                    e.track.bitrate_kbps.unwrap_or_default(),
                                    e.track.sample_rate_hz.unwrap_or_default()
                                )
                            },
                        )
                        .into(),
                );
                ui.set_playing(state.playing);
                ui.set_shuffle(state.queue.shuffle);
                ui.set_repeat_label(
                    match state.queue.repeat {
                        RepeatMode::Off => "Repeat off",
                        RepeatMode::All => "Repeat all",
                        RepeatMode::One => "Repeat one",
                    }
                    .into(),
                );
                ui.set_volume(f32::from(state.volume));
                ui.set_device(state.device.into());
                ui.set_disabled_reason(state.disabled_reason.into());
                let mut rows = if ui.get_history() { history } else { next };
                if !ui.get_history() && dj.enabled && dj.show_queue {
                    let index = dj.songs_away.min(rows.len());
                    rows.insert(
                        index,
                        TrackRow {
                            key: "dj".into(),
                            title: "AnyPlayer DJ".into(),
                            artist: dj.status.into(),
                            ..TrackRow::default()
                        },
                    );
                }
                ui.set_queue(swap(ui.get_queue(), rows));
            });
        });
    }
    fn search(&self, query: String, page: i32, playlists: bool, source_index: i32) {
        let generation = {
            let mut context = self.context.lock().expect("context");
            context.generation += 1;
            if let Some(task) = context.search_task.take() {
                task.abort();
            }
            context.generation
        };
        let bridge = self.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(180)).await;
            if bridge.context.lock().expect("context").generation != generation {
                return;
            }
            let request = SearchRequest {
                query,
                source: source(source_index),
                playlists,
                page: usize::try_from(page).unwrap_or_default(),
                generation,
            };
            if let Err(error) = bridge.ports.search.search(request).await {
                bridge.toast(error.to_string());
            }
        });
        self.context.lock().expect("context").search_task = Some(task.abort_handle());
    }
    fn track_action(&self, action: &str, key: String, position: i32, search: bool) {
        let state = self.ports.library.subscribe().borrow().clone();
        let context = self.context.lock().expect("context");
        let selected = context.selected.clone();
        let rows = state.materialize(&selected);
        let track = if search {
            self.ports
                .search
                .subscribe()
                .borrow()
                .tracks
                .iter()
                .find(|t| format!("{:?}:{}", t.source, t.id) == key)
                .cloned()
        } else {
            rows.iter().find(|r| r.key == key).map(|r| r.track.clone())
        };
        let can_reorder = context.table.can_reorder();
        drop(context);
        let Some(track) = track else { return };
        let ports = self.ports.clone();
        match action {
            "play" => {
                self.launch(async move {
                    if search {
                        ports
                            .playback
                            .playback(PlaybackCommand::SetQueue {
                                tracks: vec![track],
                                start: 0,
                            })
                            .await
                    } else {
                        ports
                            .library
                            .library(LibraryCommand::Play {
                                id: selected,
                                key: Some(key),
                            })
                            .await
                    }
                });
            }
            "next" => self.launch(async move {
                ports
                    .playback
                    .playback(PlaybackCommand::PlayNext(track))
                    .await
            }),
            "add" => {
                self.context.lock().expect("context").pending = Some(track);
                let _ = self
                    .ui
                    .upgrade_in_event_loop(|ui| ui.set_dialog("add".into()));
            }
            "remove" => self.launch(async move {
                ports
                    .library
                    .library(LibraryCommand::Remove { id: selected, key })
                    .await
            }),
            "move" | "drag" if can_reorder => self.launch(async move {
                ports
                    .library
                    .library(LibraryCommand::Move {
                        id: selected,
                        key,
                        position: usize::try_from(position).unwrap_or_default(),
                    })
                    .await
            }),
            _ => {}
        }
    }
    fn action(&self, ui: &MainWindow, action: &str, key: String, number: i32) {
        let ports = self.ports.clone();
        let selected = self.context.lock().expect("context").selected.clone();
        let playback = match action {
            "toggle" => Some(PlaybackCommand::Toggle),
            "next" => Some(PlaybackCommand::Next),
            "previous" => Some(PlaybackCommand::Previous),
            "shuffle" => Some(PlaybackCommand::Shuffle),
            "repeat" => Some(PlaybackCommand::Repeat),
            "jump" => key.parse().ok().map(PlaybackCommand::Jump),
            "queue-next" => key.parse::<u64>().ok().and_then(|uid| {
                ports
                    .playback
                    .subscribe()
                    .borrow()
                    .queue
                    .entries
                    .iter()
                    .find(|e| e.uid == uid)
                    .map(|e| PlaybackCommand::PlayNext(e.track.clone()))
            }),
            _ => None,
        };
        if let Some(command) = playback {
            self.launch(async move { ports.playback.playback(command).await });
            return;
        }
        let library = match action {
            "refresh" => Some(LibraryCommand::Refresh),
            "clear-cache" => Some(LibraryCommand::ClearCache),
            "create" => Some(LibraryCommand::Create {
                name: key.clone(),
                union: number == 1,
            }),
            "edit" => Some(LibraryCommand::Update {
                id: selected.clone(),
                name: key.clone(),
                description: ui.get_draft_description().to_string(),
            }),
            "delete" => Some(LibraryCommand::Delete(selected.clone())),
            "distinct" => Some(LibraryCommand::Distinct {
                id: selected.clone(),
                enabled: number == 1,
            }),
            "source-add" => Some(LibraryCommand::AddSource {
                id: selected.clone(),
                source: key.clone(),
            }),
            "source-remove" => Some(LibraryCommand::RemoveSource {
                id: selected.clone(),
                source: key.clone(),
            }),
            "source-move" => Some(LibraryCommand::MoveSource {
                id: selected.clone(),
                source: key.clone(),
                position: usize::try_from(number).unwrap_or_default(),
            }),
            "add-to" => self
                .context
                .lock()
                .expect("context")
                .pending
                .clone()
                .map(|track| LibraryCommand::Add {
                    id: key.clone(),
                    track,
                }),
            _ => None,
        };
        if let Some(command) = library {
            let bridge = self.clone();
            let close = matches!(action, "create" | "edit" | "delete" | "add-to");
            let delete = action == "delete";
            self.launch(async move {
                ports.library.library(command).await?;
                if close {
                    let _ = bridge.ui.upgrade_in_event_loop(move |ui| {
                        ui.set_dialog("".into());
                        if delete {
                            ui.set_selected_id("".into());
                        }
                    });
                }
                Ok(())
            });
            return;
        }
        match action {
            "open" => {
                let open = key.clone();
                let service = ports.library.clone();
                self.launch(async move { service.library(LibraryCommand::Open(open)).await });
                let mut context = self.context.lock().expect("context");
                context.selected = key;
                context.table = TrackTable::default();
                context.table.sort = ports
                    .settings
                    .subscribe()
                    .borrow()
                    .sorts
                    .get(&context.selected)
                    .cloned()
                    .unwrap_or_default();
                drop(context);
                ui.set_filter("".into());
                ui.set_page(1);
                self.refresh_library();
            }
            "back" => {
                self.context.lock().expect("context").selected.clear();
                ui.set_selected_id("".into());
                self.refresh_library();
            }
            "catalog" => {
                self.context.lock().expect("context").catalog_tab = number;
                self.refresh_library();
            }
            "candidates" => {
                self.context.lock().expect("context").source_filter = number;
                self.refresh_library();
            }
            "filter" => {
                self.context.lock().expect("context").table.filter = key;
                self.refresh_library();
            }
            "queue" => self.refresh_playback(),
            "search" => self.search(
                key,
                number,
                ui.get_search_tab() == 1,
                ui.get_search_source(),
            ),
            "play-all" => {
                self.launch(async move {
                    ports
                        .library
                        .library(LibraryCommand::Play {
                            id: selected,
                            key: None,
                        })
                        .await
                });
            }
            "normalize" => self.launch(async move {
                ports
                    .settings
                    .settings(SettingsCommand::Normalize(number == 1))
                    .await
            }),
            "output" => self
                .launch(async move { ports.settings.settings(SettingsCommand::Output(key)).await }),
            "close-tray" => self.launch(async move {
                ports
                    .settings
                    .settings(SettingsCommand::CloseToTray(number == 1))
                    .await
            }),
            "column" => {
                let mut columns = ports.settings.subscribe().borrow().columns.clone();
                if let Some(column) = columns.iter_mut().find(|c| c.key == key) {
                    column.visible = number == 1;
                }
                self.launch(async move {
                    ports
                        .settings
                        .settings(SettingsCommand::Columns(columns))
                        .await
                });
            }
            "sync-connect" => {
                let url = ui.get_sync_url().to_string();
                let token = ui.get_sync_token().to_string();
                self.launch(async move {
                    ports.sync.sync(SyncCommand::Commit { url, token }).await?;
                    ports.sync.sync(SyncCommand::Connect).await
                });
            }
            "sync-pull" => self.launch(async move { ports.sync.sync(SyncCommand::Pull).await }),
            "sync-resolve" => self.launch(async move {
                ports
                    .sync
                    .sync(SyncCommand::Resolve(match number {
                        0 => SyncChoice::Server,
                        1 => SyncChoice::Device,
                        _ => SyncChoice::Cancel,
                    }))
                    .await
            }),
            "domain" => self.launch(async move {
                ports
                    .sync
                    .sync(SyncCommand::Domain {
                        index: usize::try_from(number / 2).unwrap_or_default(),
                        enabled: number % 2 == 1,
                    })
                    .await
            }),
            "dj-enable" => {
                self.launch(async move { ports.dj.dj(DjCommand::Enable(number == 1)).await });
            }
            "dj-queue" => {
                self.launch(async move { ports.dj.dj(DjCommand::ShowQueue(number == 1)).await });
            }
            "dj-debug" => {
                self.launch(async move { ports.dj.dj(DjCommand::Debug(number == 1)).await });
            }
            "dj-level" => self.launch(async move {
                ports
                    .dj
                    .dj(DjCommand::Level(
                        f32::from(i16::try_from(number.clamp(50, 200)).unwrap_or(100)) / 100.0,
                    ))
                    .await
            }),
            "dj-voice" => self.launch(async move { ports.dj.dj(DjCommand::Voice(key)).await }),
            "dj-clear" => self.launch(async move { ports.dj.dj(DjCommand::ClearLogs).await }),
            "device" => self.launch(async move {
                ports
                    .providers
                    .providers(ProvidersCommand::Preferred(key))
                    .await
            }),
            "cancel-login" => self.launch(async move {
                ports
                    .providers
                    .providers(ProvidersCommand::CancelLogin)
                    .await
            }),
            "provider-tab" => apply_provider(ui, &ports.providers.subscribe().borrow()),
            "provider-connect" => {
                let source = source(ui.get_provider_tab() + 1);
                let command = if ui.get_provider_connected() {
                    ProvidersCommand::Disconnect(source)
                } else {
                    ProvidersCommand::Connect(ProviderForm {
                        source,
                        url: ui.get_provider_url().to_string(),
                        secret: ui.get_provider_secret().to_string(),
                        page_size: ui
                            .get_provider_page_size()
                            .parse::<u16>()
                            .unwrap_or(300)
                            .clamp(1, 1000),
                        self_signed: ui.get_self_signed(),
                    })
                };
                self.launch(async move { ports.providers.providers(command).await });
            }
            "transfer" => {
                let bridge = self.clone();
                let private = ui.get_transfer_private();
                let passphrase = ui.get_transfer_passphrase().to_string();
                let replace = ui.get_transfer_replace();
                let dry_run = ui.get_transfer_dry_run();
                tokio::spawn(async move {
                    let export = number == 0 || number == 2;
                    let dialog = rfd::AsyncFileDialog::new()
                        .add_filter("JSON", &["json"])
                        .set_file_name(if number < 2 {
                            "any-player-state.json"
                        } else {
                            "any-player-config.json"
                        });
                    let file = if export {
                        dialog.save_file().await
                    } else {
                        dialog.pick_file().await
                    };
                    if let Some(file) = file {
                        let kind = match number {
                            0 => TransferKind::ExportState,
                            1 => TransferKind::ImportState,
                            2 => TransferKind::ExportConfig,
                            _ => TransferKind::ImportConfig,
                        };
                        match ports
                            .settings
                            .transfer(TransferRequest {
                                kind,
                                path: file.path().to_path_buf(),
                                private,
                                passphrase,
                                replace,
                                dry_run,
                            })
                            .await
                        {
                            Ok(message) => bridge.toast(message),
                            Err(error) => bridge.toast(error.to_string()),
                        }
                    }
                });
            }
            _ => {}
        }
    }
}
fn apply_provider(ui: &MainWindow, state: &ProvidersSnapshot) {
    let selected = source(ui.get_provider_tab() + 1);
    if let Some(profile) = state.profiles.iter().find(|p| p.source == selected) {
        ui.set_provider_connected(profile.connected);
        ui.set_provider_url(profile.server_url.clone().unwrap_or_default().into());
        ui.set_provider_status(
            profile
                .last_error
                .clone()
                .unwrap_or_else(|| {
                    if profile.connected {
                        "✓ Connected".into()
                    } else {
                        "✗ Not connected".into()
                    }
                })
                .into(),
        );
    }
    ui.set_provider_secret("".into());
    ui.set_login_url(state.login_url.clone().into());
    ui.set_preferred_device(state.preferred.clone().into());
    ui.set_devices(swap(
        ui.get_devices(),
        state
            .devices
            .iter()
            .map(|d| CardRow {
                id: d.id.clone().into(),
                name: d.name.clone().into(),
                detail: "".into(),
            })
            .collect(),
    ));
}

/// Attach a service graph after creating Tokio and before entering Slint's event loop.
/// All callbacks post typed commands; background tasks only upgrade handles inside Slint.
pub fn attach(ui: &MainWindow, services: Services) {
    let bridge = Bridge {
        ports: Arc::new(services),
        context: Arc::new(Mutex::new(Context::default())),
        ui: ui.as_weak(),
    };
    crate::images::attach(ui);
    {
        let b = bridge.clone();
        let weak = ui.as_weak();
        ui.on_action(move |action, key, n| {
            if let Some(ui) = weak.upgrade() {
                b.action(&ui, &action, key.to_string(), n);
            }
        });
    }
    {
        let b = bridge.clone();
        ui.on_track_action(move |a, k, n| b.track_action(&a, k.to_string(), n, false));
    }
    {
        let b = bridge.clone();
        ui.on_search_action(move |a, k, n| b.track_action(&a, k.to_string(), n, true));
    }
    {
        let b = bridge.clone();
        ui.on_sort(move |key| {
            let (id, preference) = {
                let mut context = b.context.lock().expect("context");
                context.table.cycle_sort(&key);
                (context.selected.clone(), context.table.sort.clone())
            };
            let ports = b.ports.clone();
            b.launch(async move {
                ports
                    .settings
                    .settings(SettingsCommand::Sort {
                        key: id,
                        preference,
                    })
                    .await
            });
            b.refresh_library();
        });
    }
    {
        let b = bridge.clone();
        ui.on_sync_commit(move |url, token| {
            let ports = b.ports.clone();
            b.launch(async move {
                ports
                    .sync
                    .sync(SyncCommand::Commit {
                        url: url.to_string(),
                        token: token.to_string(),
                    })
                    .await
            });
        });
    }
    {
        let b = bridge.clone();
        let weak = ui.as_weak();
        ui.on_query_change(move |query| {
            if let Some(ui) = weak.upgrade() {
                b.search(
                    query.to_string(),
                    0,
                    ui.get_search_tab() == 1,
                    ui.get_search_source(),
                );
            }
        });
    }
    {
        let b = bridge.clone();
        ui.on_seek(move |percent| {
            let ports = b.ports.clone();
            let duration = ports
                .playback
                .subscribe()
                .borrow()
                .queue
                .current()
                .and_then(|e| e.track.duration_ms)
                .unwrap_or_default();
            let position = i64::try_from(
                milliseconds(duration)
                    .mul_f64(f64::from(percent.clamp(0.0, 100.0)) / 100.0)
                    .as_millis(),
            )
            .unwrap_or(i64::MAX);
            b.launch(async move {
                ports
                    .playback
                    .playback(PlaybackCommand::Seek(position))
                    .await
            });
        });
    }
    let volume_timer = Rc::new(slint::Timer::default());
    {
        let b = bridge.clone();
        let timer = volume_timer.clone();
        ui.on_volume_change(move |value| {
            let b = b.clone();
            timer.start(
                slint::TimerMode::SingleShot,
                Duration::from_millis(50),
                move || {
                    let ports = b.ports.clone();
                    b.launch(async move {
                        ports
                            .playback
                            .playback(PlaybackCommand::Volume(volume_value(value)))
                            .await
                    });
                },
            );
        });
    }
    {
        let b = bridge.clone();
        // Linux clipboard ownership must outlive the copy callback.
        let clipboard = RefCell::new(arboard::Clipboard::new());
        ui.on_copy(move |text| match clipboard.borrow_mut().as_mut() {
            Ok(clipboard) => {
                if let Err(error) = clipboard.set_text(text.to_string()) {
                    b.toast(error.to_string());
                }
            }
            Err(error) => b.toast(error.to_string()),
        });
    }
    let position_timer = Rc::new(slint::Timer::default());
    {
        let weak = ui.as_weak();
        let receiver = bridge.ports.playback.subscribe();
        position_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(250),
            move || {
                if let Some(ui) = weak.upgrade() {
                    let state = receiver.borrow();
                    let position = state.position();
                    let total = state
                        .queue
                        .current()
                        .and_then(|e| e.track.duration_ms)
                        .unwrap_or_default();
                    ui.set_progress(if total > 0 {
                        milliseconds(position).as_secs_f32() / milliseconds(total).as_secs_f32()
                            * 100.0
                    } else {
                        0.0
                    });
                    ui.set_position_text(
                        format!("{} / {}", duration(position), duration(total)).into(),
                    );
                }
            },
        );
    }
    // Keep timers alive through a callback owned by the component, not a leaked global.
    ui.on_navigate(move |_| {
        let _ = (&position_timer, &volume_timer);
    });
    bridge.refresh_library();
    bridge.refresh_playback();
    {
        let b = bridge.clone();
        let mut rx = b.ports.library.subscribe();
        tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                b.refresh_library();
            }
        });
    }
    {
        let b = bridge.clone();
        let mut rx = b.ports.playback.subscribe();
        tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                b.refresh_playback();
            }
        });
    }
    {
        let b = bridge.clone();
        let mut rx = b.ports.settings.subscribe();
        tokio::spawn(async move {
            loop {
                let state = rx.borrow_and_update().clone();
                let _ = b.ui.upgrade_in_event_loop(move |ui| {
                    ui.set_normalize(state.normalize);
                    ui.set_output_device(state.output_device.into());
                    ui.set_outputs(
                        Rc::new(VecModel::from(
                            state
                                .output_devices
                                .into_iter()
                                .map(Into::into)
                                .collect::<Vec<_>>(),
                        ))
                        .into(),
                    );
                    ui.set_close_to_tray(state.close_to_tray);
                    ui.set_tray_available(state.tray_available);
                    let mut columns = state.columns;
                    columns.sort_by_key(|c| c.position);
                    for column in &columns {
                        match column.key.as_str() {
                            "title" => ui.set_column_title(column.visible),
                            "artist" => ui.set_column_artist(column.visible),
                            "album" => ui.set_column_album(column.visible),
                            "duration" | "duration_ms" => ui.set_column_duration(column.visible),
                            "source" => ui.set_column_source(column.visible),
                            _ => {}
                        }
                    }
                    ui.set_columns(swap(
                        ui.get_columns(),
                        columns
                            .into_iter()
                            .filter(|c| c.visible)
                            .map(|c| {
                                let key = if c.key == "duration_ms" {
                                    "duration".into()
                                } else {
                                    c.key
                                };
                                ColumnRow {
                                    label: match key.as_str() {
                                        "title" => "Title",
                                        "artist" => "Artist",
                                        "album" => "Album",
                                        "duration" => "Duration",
                                        "source" => "Source",
                                        _ => &key,
                                    }
                                    .into(),
                                    width: state
                                        .column_widths
                                        .get(&key)
                                        .or_else(|| {
                                            if key == "duration" {
                                                state.column_widths.get("duration_ms")
                                            } else {
                                                None
                                            }
                                        })
                                        .copied()
                                        .unwrap_or(if key == "title" {
                                            150.0
                                        } else if key == "duration" || key == "source" {
                                            68.0
                                        } else {
                                            110.0
                                        })
                                        .clamp(40.0, 600.0),
                                    key: key.into(),
                                }
                            })
                            .collect(),
                    ));
                });
                if rx.changed().await.is_err() {
                    break;
                }
            }
        });
    }
    {
        let b = bridge.clone();
        let mut rx = b.ports.sync.subscribe();
        let initial = rx.borrow().clone();
        ui.set_sync_url(initial.url.into());
        ui.set_sync_token(initial.token.into());
        tokio::spawn(async move {
            loop {
                let state = rx.borrow_and_update().clone();
                let _ = b.ui.upgrade_in_event_loop(move |ui| {
                    ui.set_sync_status(state.status.into());
                    ui.set_sync_conflict(state.conflict);
                    ui.set_sync_overwrite(state.overwrite);
                    ui.set_sync_app(state.domains[0]);
                    ui.set_sync_playlists(state.domains[1]);
                    ui.set_sync_providers(state.domains[2]);
                    ui.set_sync_settings(state.domains[3]);
                    ui.set_transfer_status(state.import_status.into());
                });
                if rx.changed().await.is_err() {
                    break;
                }
            }
        });
    }
    {
        let b = bridge.clone();
        let mut rx = b.ports.dj.subscribe();
        tokio::spawn(async move {
            loop {
                let state = rx.borrow_and_update().clone();
                b.refresh_playback();
                let _ = b.ui.upgrade_in_event_loop(move |ui| {
                    ui.set_dj_enabled(state.enabled);
                    ui.set_dj_show_queue(state.show_queue);
                    ui.set_dj_debug(state.debug);
                    ui.set_dj_level(state.level);
                    ui.set_dj_status(state.status.into());
                    ui.set_voice(state.voice.into());
                    ui.set_voices(
                        Rc::new(VecModel::from(
                            state.voices.into_iter().map(Into::into).collect::<Vec<_>>(),
                        ))
                        .into(),
                    );
                    ui.set_dj_logs(state.logs.join("\n").into());
                });
                if rx.changed().await.is_err() {
                    break;
                }
            }
        });
    }
    {
        let b = bridge.clone();
        let mut rx = b.ports.providers.subscribe();
        tokio::spawn(async move {
            loop {
                let state = rx.borrow_and_update().clone();
                let _ =
                    b.ui.upgrade_in_event_loop(move |ui| apply_provider(&ui, &state));
                if rx.changed().await.is_err() {
                    break;
                }
            }
        });
    }
    {
        let b = bridge;
        let mut rx = b.ports.search.subscribe();
        tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                let state = rx.borrow_and_update().clone();
                if state.generation != b.context.lock().expect("context").generation {
                    continue;
                }
                let rows = state
                    .tracks
                    .iter()
                    .enumerate()
                    .map(|(i, t)| row(t, format!("{:?}:{}", t.source, t.id), i))
                    .collect();
                let cards = state.playlists.iter().map(card).collect();
                let _ = b.ui.upgrade_in_event_loop(move |ui| {
                    ui.set_search_tracks(swap(ui.get_search_tracks(), rows));
                    ui.set_search_playlists(swap(ui.get_search_playlists(), cards));
                    ui.set_search_page(i32::try_from(state.page).unwrap_or_default());
                    ui.set_search_has_next((state.page + 1) * 20 < state.total);
                    ui.set_search_status(
                        if state.error.is_empty() {
                            format!(
                                "{} results{}",
                                state.total,
                                if state.warnings.is_empty() {
                                    String::new()
                                } else {
                                    format!(" · {}", state.warnings.join(", "))
                                }
                            )
                        } else {
                            state.error
                        }
                        .into(),
                    );
                });
            }
        });
    }
}
