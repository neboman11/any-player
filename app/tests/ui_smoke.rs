use any_player::{MainWindow, attach, fake::DemoServices};
use i_slint_backend_testing::ElementHandle;
use slint::{ComponentHandle, Timer, TimerMode};
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

fn visible(ui: &MainWindow, label: &str) {
    assert!(
        ElementHandle::find_by_accessible_label(ui, label)
            .next()
            .is_some(),
        "Missing visible text/control: {label}"
    );
}

#[test]
fn navigate_every_page_and_dialog_with_fake_engine() {
    i_slint_backend_testing::init_integration_test_with_system_time();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();
    let service = Arc::new(DemoServices::new());
    let ui = MainWindow::new().unwrap();
    attach(&ui, service.ports());
    ui.window().set_size(slint::LogicalSize::new(800.0, 560.0));
    let step = Rc::new(Cell::new(0));
    let timer = Timer::default();
    let weak = ui.as_weak();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    timer.start(TimerMode::Repeated, Duration::from_millis(150), move || {
        assert!(
            std::time::Instant::now() < deadline,
            "UI smoke timed out at step {}",
            step.get()
        );
        let ui = weak.upgrade().unwrap();
        match step.get() {
            0 => {
                visible(&ui, "Now Playing");
                for label in [
                    "Shuffle",
                    "Previous",
                    "Play",
                    "Next",
                    "Repeat off",
                    "Playback position",
                    "Volume",
                ] {
                    visible(&ui, label);
                }
                assert!(ui.get_now_title().contains("Northern Lights"));
                ui.invoke_action("toggle".into(), "".into(), 0);
            }
            1 => {
                assert!(ui.get_playing());
                ui.set_page(1);
            }
            2 => {
                visible(&ui, "Your Playlists");
                ui.set_dialog("create".into());
            }
            3 => {
                visible(&ui, "Create New Playlist");
                visible(&ui, "Standard Playlist");
                visible(&ui, "Union Playlist");
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                        text: slint::platform::Key::Escape.into(),
                    });
                assert!(ui.get_dialog().is_empty(), "Escape must dismiss a dialog");
                ui.invoke_action("open".into(), "playlist-0".into(), 0);
            }
            4 => {
                visible(&ui, "Late Night Favorites");
                visible(&ui, "Edit");
                // 800 px: row actions fold into one menu button; headers stay sortable.
                visible(&ui, "More actions for Northern Lights 1");
                visible(&ui, "Sort by Title");
                ui.invoke_sort("title".into());
                ui.set_dialog("edit".into());
            }
            5 => {
                assert_eq!(ui.get_sort_column(), "title");
                assert!(!ui.get_sort_descending());
                visible(&ui, "Edit Playlist");
                ui.set_dialog("delete".into());
            }
            6 => {
                visible(&ui, "Delete Playlist");
                ui.set_dialog("add".into());
            }
            7 => {
                visible(&ui, "Add to Playlist");
                ui.set_dialog("".into());
                ui.invoke_action("open".into(), "playlist-1".into(), 0);
            }
            8 => {
                visible(&ui, "Discover Weekly");
                ui.invoke_action("open".into(), "union-1".into(), 0);
            }
            9 => {
                visible(&ui, "UNION");
                ui.set_dialog("source".into());
            }
            10 => {
                visible(&ui, "Add Playlist to Union");
                ui.set_dialog("".into());
                ui.set_page(2);
                ui.invoke_action("search".into(), "Northern".into(), 0);
            }
            11 => {
                visible(&ui, "Search query");
                visible(&ui, "Tracks");
                ui.set_search_tab(1);
            }
            12 => {
                visible(&ui, "Playlists");
                ui.set_page(3);
                ui.window()
                    .set_size(slint::LogicalSize::new(1100.0, 2600.0));
            }
            13 => {
                visible(&ui, "Sync server URL");
                visible(&ui, "Transfer passphrase");
                ui.set_settings_tab(1);
            }
            14 => {
                visible(&ui, "Spotify Connect devices");
                ui.set_provider_tab(1);
                ui.invoke_action("provider-tab".into(), "".into(), 1);
            }
            15 => {
                visible(&ui, "Provider server URL");
                visible(&ui, "Allow self-signed certificate");
                ui.set_provider_tab(2);
            }
            16 => {
                visible(&ui, "Provider secret");
                ui.set_settings_tab(2);
            }
            17 => {
                visible(
                    &ui,
                    "AI DJ needs a sync server configured in Settings → General.",
                );
                visible(&ui, "AI DJ diagnostic logs");
                ui.set_settings_tab(0);
                ui.invoke_action("sync-connect".into(), "".into(), 0);
            }
            18 => {
                visible(
                    &ui,
                    "Server already has synced data - choose which side to keep.",
                );
                ui.invoke_action("sync-resolve".into(), "".into(), 2);
            }
            19 => {
                // Actions run on Tokio tasks: wait for Resolve to finish before
                // launching Pull, or Resolve can erase Pull's overwrite prompt.
                if ui.get_sync_conflict() {
                    return;
                }
                ui.invoke_action("sync-pull".into(), "".into(), 0);
            }
            20 => {
                if !ui.get_sync_overwrite() {
                    return;
                }
                visible(&ui, "Confirm playlist overwrite");
                visible(&ui, "Continue");
                visible(&ui, "Keep this device's playlists");
                slint::quit_event_loop().unwrap();
            }
            _ => unreachable!(),
        }
        step.set(step.get() + 1);
    });
    ui.run().unwrap();
}
