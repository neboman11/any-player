use slint::ComponentHandle;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _runtime_guard = runtime.enter();
    if !std::env::args().any(|arg| arg == "--demo") {
        return production(&runtime);
    }
    let services = Arc::new(any_player::fake::DemoServices::new());
    let progression = services.start_progression();
    let window = any_player::MainWindow::new()?;
    any_player::attach(&window, services.ports());
    any_player::images::attach_cache(&window, Arc::new(any_player::fake::DemoImageCache));
    if std::env::args().any(|arg| arg == "--demo-playback-error") {
        services.show_playback_error();
    }
    // Demo-only route selection makes native-window screenshots reproducible.
    let args: Vec<_> = std::env::args().collect();
    if let Some(index) = args.iter().position(|arg| arg == "--demo-page") {
        match args.get(index + 1).map(String::as_str) {
            Some("now-playing") => window.set_page(0),
            Some("playlists") => window.set_page(1),
            Some("playlist") => {
                window.set_page(1);
                window.invoke_action("open".into(), "playlist-0".into(), 0);
            }
            Some("union") => {
                window.set_page(1);
                window.invoke_action("open".into(), "union-1".into(), 0);
            }
            Some("search") => {
                window.set_page(2);
                window.set_query("Northern".into());
                window.invoke_action("search".into(), "Northern".into(), 0);
            }
            Some("settings" | "providers" | "dj") => {
                window.set_page(3);
                window.set_settings_tab(match args[index + 1].as_str() {
                    "providers" => 1,
                    "dj" => 2,
                    _ => 0,
                });
            }
            _ => return Err("Unknown --demo-page route".into()),
        }
    }
    if args.iter().any(|arg| arg == "--demo-small") {
        window
            .window()
            .set_size(slint::LogicalSize::new(800.0, 560.0));
    }
    window.run()?;
    progression.abort();
    Ok(())
}

fn production(runtime: &tokio::runtime::Runtime) -> Result<(), Box<dyn std::error::Error>> {
    let result = production_inner(runtime);
    if let Err(error) = &result {
        rfd::MessageDialog::new()
            .set_title("Any Player could not start")
            .set_description(error.to_string())
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
    result
}

fn production_inner(runtime: &tokio::runtime::Runtime) -> Result<(), Box<dyn std::error::Error>> {
    let paths = ap_store::paths::Paths::system()?;
    let (show_tx, show_rx) = tokio::sync::mpsc::unbounded_channel();
    let instance = runtime.block_on(ap_platform::single_instance::acquire(
        paths.data_dir(),
        show_tx,
    ))?;
    let ap_platform::single_instance::Instance::Primary(_primary) = instance else {
        return Ok(());
    };
    let _logging = any_player::startup::tracing(&paths)?;
    let (services, status) = runtime.block_on(any_player::startup::boot(
        &paths,
        Arc::new(ap_store::secrets::Secrets::keyring()),
    ))?;
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .select()?;
    let window = any_player::MainWindow::new()?;
    any_player::attach(&window, services.ports());
    any_player::images::attach_cache(
        &window,
        Arc::new(any_player::images::DiskImageCache::authenticated(
            paths.images_cache_dir(),
            services.providers.clone(),
        )),
    );
    window.set_banner(if status.sync_migration_banner {
        "Sync settings didn't carry over from the previous version. Re-enter them in Settings → General.".into()
    } else {
        "".into()
    });
    if status.sync_migration_banner {
        let db = services.db.clone();
        window.on_banner_dismissed(move || {
            let db = db.clone();
            tokio::spawn(async move {
                if let Err(e) = any_player::startup::dismiss_sync_banner(&db).await {
                    tracing::warn!(error = %e, "could not persist the sync banner dismissal");
                }
            });
        });
        let mut settings = services.settings_store.subscribe();
        let db = services.db.clone();
        let weak = window.as_weak();
        tokio::spawn(async move {
            let mut banner = any_player::vm::playback::SyncMigrationBanner::new(true);
            loop {
                banner.observe_target(&settings.borrow_and_update().sync.server_target);
                if !banner.visible() {
                    let _ = weak.upgrade_in_event_loop(|ui| ui.set_banner("".into()));
                    if let Err(e) = any_player::startup::dismiss_sync_banner(&db).await {
                        tracing::warn!(error = %e, "could not persist the sync banner dismissal");
                    }
                    break;
                }
                if settings.changed().await.is_err() {
                    break;
                }
            }
        });
    }
    window.set_startup_warning(status.warnings.join("\n").into());
    let retry_services = services.clone();
    let weak = window.as_weak();
    window.on_startup_retry(move |step| {
        let services = retry_services.clone();
        let weak = weak.clone();
        tokio::spawn(async move {
            let result = match step.as_str() {
                "keyring" => services.providers.retry_keyring().await,
                "spotify" => {
                    services
                        .providers
                        .restore_step(ap_model::SourceType::Spotify)
                        .await
                }
                "jellyfin" => {
                    services
                        .providers
                        .restore_step(ap_model::SourceType::Jellyfin)
                        .await
                }
                "plex" => {
                    services
                        .providers
                        .restore_step(ap_model::SourceType::Plex)
                        .await
                }
                _ => tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    services.engine.await_restored(),
                )
                .await
                .map_err(|e| any_player::vm::ServiceError(e.to_string())),
            };
            let message = result.map_or_else(
                |e| format!("Retry failed: {e}"),
                |()| format!("{step} restored"),
            );
            let _ = weak.upgrade_in_event_loop(move |ui| ui.set_toast(message.into()));
        });
    });
    let deferred_services = services.clone();
    let desktop = any_player::desktop::DesktopIntegration::start(
        &window,
        services.engine.clone(),
        services.settings_store.clone(),
        show_rx,
        std::env::args().any(|arg| arg == "--minimized"),
        Arc::new(move || deferred_services.start_deferred()),
    )?;
    let timer = slint::Timer::default();
    let availability = desktop.availability();
    let settings = services.settings.clone();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(250),
        move || {
            settings.set_tray_available(availability.load(std::sync::atomic::Ordering::Relaxed));
        },
    );
    slint::run_event_loop_until_quit()?;
    runtime.block_on(services.engine.flush());
    Ok(())
}
