/// Any Player - Multi-Source Music Client
pub mod cache;
pub mod config;
pub mod database;
pub mod models;
pub mod playback;
pub mod providers;
pub mod state;
pub mod websocket;

pub use config::Config;
pub use database::Database;
pub use models::{PlaybackInfo, PlaybackState, Playlist, RepeatMode, Source, Track};
pub use playback::PlaybackManager;
pub use providers::{MusicProvider, ProviderError, ProviderRegistry};
pub use state::PersistentPlaybackState;
use tracing_subscriber::{filter, layer::SubscriberExt, util::SubscriberInitExt};

mod commands;

// Re-export command functions from auth and custom_playlists modules only
// Other modules (cache, playback, providers) share names with top-level modules
pub use commands::{auth, custom_playlists};

use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::broadcast;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize logging
    // Default log level is INFO - anything above (DEBUG, TRACE) will drastically
    // increase log output and may impact performance. Use higher levels only for debugging.
    let filter = filter::Targets::new()
        .with_default(filter::LevelFilter::INFO)
        .with_target("any_player_lib", filter::LevelFilter::INFO)
        .with_target("glycin", filter::LevelFilter::WARN)
        .with_target("hyper", filter::LevelFilter::WARN)
        .with_target("zbus", filter::LevelFilter::WARN);
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Initialize database with graceful error handling
    let db_path = match dirs::data_dir() {
        Some(dir) => dir.join("any-player").join("playlists.db"),
        None => {
            eprintln!("Failed to get data directory. Using current directory.");
            std::path::PathBuf::from("playlists.db")
        }
    };

    if let Some(parent) = db_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("Failed to create data directory: {}", e);
            std::process::exit(1);
        }
    }

    let database = match Database::new(db_path.clone()) {
        Ok(db) => Arc::new(Mutex::new(db)),
        Err(e) => {
            eprintln!("Failed to initialize database at {:?}: {}", db_path, e);
            eprintln!("Please check file permissions and disk space.");
            std::process::exit(1);
        }
    };

    // Create application state
    let providers = Arc::new(Mutex::new(ProviderRegistry::new()));
    let oauth_code: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let (ws_sender, _) = broadcast::channel(256);

    let providers_clone = providers.clone();
    let oauth_code_for_server = oauth_code.clone();
    let database_clone = database.clone();
    let providers_for_state = providers.clone();
    let ws_sender_for_state = ws_sender.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            // Playback commands
            commands::get_playback_status,
            commands::play,
            commands::pause,
            commands::toggle_play_pause,
            commands::next_track,
            commands::previous_track,
            commands::skip_to_queue_index,
            commands::seek,
            commands::set_volume,
            commands::toggle_shuffle,
            commands::set_repeat_mode,
            // Playlist commands
            commands::get_playlists,
            commands::play_track,
            commands::queue_track,
            commands::clear_queue,
            commands::play_playlist,
            commands::play_tracks_immediate,
            // Spotify commands
            commands::get_spotify_auth_url,
            commands::authenticate_spotify,
            commands::is_spotify_authenticated,
            commands::check_spotify_premium,
            commands::initialize_spotify_session,
            commands::initialize_spotify_session_from_provider,
            commands::is_spotify_session_ready,
            commands::refresh_spotify_token,
            commands::get_spotify_playlists,
            commands::get_spotify_playlist,
            commands::check_oauth_code,
            commands::disconnect_spotify,
            commands::restore_spotify_session,
            commands::clear_spotify_session,
            // Jellyfin commands
            commands::authenticate_jellyfin,
            commands::is_jellyfin_authenticated,
            commands::get_jellyfin_playlists,
            commands::get_jellyfin_playlist,
            commands::search_jellyfin_tracks,
            commands::search_jellyfin_playlists,
            commands::get_jellyfin_recently_played,
            commands::disconnect_jellyfin,
            commands::get_jellyfin_credentials,
            commands::restore_jellyfin_session,
            // Plex commands
            commands::authenticate_plex,
            commands::is_plex_authenticated,
            commands::get_plex_playlists,
            commands::get_plex_playlist,
            commands::search_plex_tracks,
            commands::search_plex_playlists,
            commands::get_plex_recently_played,
            commands::disconnect_plex,
            commands::get_plex_credentials,
            commands::restore_plex_session,
            // Search commands
            commands::search_spotify_tracks,
            // Audio commands
            commands::get_audio_file,
            // Custom playlist commands
            commands::create_custom_playlist,
            commands::get_custom_playlists,
            commands::get_custom_playlist,
            commands::update_custom_playlist,
            commands::delete_custom_playlist,
            commands::add_track_to_custom_playlist,
            commands::get_custom_playlist_tracks,
            commands::remove_track_from_custom_playlist,
            commands::reorder_custom_playlist_tracks,
            commands::get_column_preferences,
            commands::save_column_preferences,
            // Union playlist commands
            commands::create_union_playlist,
            commands::add_source_to_union_playlist,
            commands::get_union_playlist_sources,
            commands::remove_source_from_union_playlist,
            commands::reorder_union_playlist_sources,
            commands::get_union_playlist_tracks,
            // Cache commands
            commands::write_playlists_cache,
            commands::read_playlists_cache,
            commands::clear_playlists_cache,
            commands::write_custom_playlists_cache,
            commands::read_custom_playlists_cache,
            commands::clear_custom_playlists_cache,
            commands::write_custom_playlist_tracks_cache,
            commands::read_custom_playlist_tracks_cache,
            commands::clear_custom_playlist_tracks_cache,
            commands::write_union_playlist_tracks_cache,
            commands::read_union_playlist_tracks_cache,
            commands::clear_union_playlist_tracks_cache,
            // Playback state commands
            commands::save_playback_state,
            commands::restore_playback_state,
        ])
        .setup(move |app| {
            // Initialize PlaybackManager inside the Tauri runtime context
            // This ensures the Tokio runtime is available for spawning tasks
            let playback = Arc::new(Mutex::new(PlaybackManager::new(
                providers_for_state.clone(),
            )));

            // Note: State saver will be started AFTER restoration completes
            // to prevent overwriting the saved state during startup

            // Create app state and manage it
            let app_state = commands::AppState {
                playback: playback.clone(),
                providers: providers_for_state.clone(),
                oauth_code: oauth_code_for_server.clone(),
                database: database_clone.clone(),
                ws_sender: ws_sender_for_state.clone(),
            };
            app.manage(app_state);

            let handle = app.handle().clone();
            let handle_for_ws = handle.clone();

            let ws_sender_for_server = ws_sender.clone();
            let ws_sender_for_playback = ws_sender.clone();
            let playback_for_ws = playback.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) =
                    websocket::start_ws_server(handle_for_ws, ws_sender_for_server).await
                {
                    tracing::error!(?err, "WebSocket server failed");
                }
            });

            tauri::async_runtime::spawn(async move {
                websocket::start_playback_broadcast(playback_for_ws, ws_sender_for_playback).await;
            });

            // Spawn a task to listen for track completion and emit events
            let playback_for_listener = playback.clone();
            tauri::async_runtime::spawn(async move {
                let playback_locked = playback_for_listener.lock().await;
                if let Some(mut rx) = playback_locked.take_completion_receiver().await {
                    drop(playback_locked); // Release lock

                    while let Some(()) = rx.recv().await {
                        tracing::info!("Track completed, emitting event to frontend");
                        if let Err(err) = handle.emit("track-completed", ()) {
                            tracing::error!(
                                ?err,
                                "Failed to emit 'track-completed' event to frontend"
                            );
                        }
                    }
                } else {
                    tracing::error!(
                        "PlaybackManager did not provide a completion receiver; \
                         'track-completed' events will not be emitted to the frontend"
                    );
                }
            });

            // Start OAuth callback server in the Tauri runtime
            let oauth_code_clone = oauth_code_for_server.clone();
            let ws_sender_for_oauth = ws_sender.clone();
            tauri::async_runtime::spawn(start_oauth_server(oauth_code_clone, ws_sender_for_oauth));

            // Try to restore provider sessions on startup in the background.
            // Keep provider lock contention low so frontend auth/status checks remain responsive.
            let playback_for_restore = playback.clone();
            let handle_for_status = app.handle().clone();
            let ws_sender_for_startup = ws_sender.clone();
            tauri::async_runtime::spawn(async move {
                // Track if any critical failures occurred during initialization
                let mut has_failures = false;

                websocket::emit_backend_init_status(
                    &ws_sender_for_startup,
                    "startup",
                    "Restoring provider sessions...",
                    false,
                    true,
                );

                let restored = restore_spotify_provider_on_startup(providers_clone.clone()).await;
                if restored {
                    tracing::info!("✓ Spotify session restored from cache on startup");
                } else {
                    tracing::info!("No cached Spotify session found on startup");
                }

                // Auto-initialize session for premium users without holding the providers lock
                if restored {
                    let (is_premium, access_token) = {
                        let providers = providers_clone.lock().await;
                        (
                            providers.premium_status(Source::Spotify).await,
                            providers.get_access_token(Source::Spotify).await,
                        )
                    };

                    if let (Some(true), Some(access_token)) = (is_premium, access_token) {
                        websocket::emit_backend_init_status(
                            &ws_sender_for_startup,
                            "spotify-session",
                            "Initializing Spotify playback session...",
                            false,
                            true,
                        );

                        tracing::info!("Auto-initializing Spotify session for premium user");

                        let init_result = timeout(Duration::from_secs(12), async {
                            let playback = playback_for_restore.lock().await;
                            playback.initialize_spotify_session(&access_token).await
                        })
                        .await;

                        match init_result {
                            Ok(Ok(())) => {
                                let playback = playback_for_restore.lock().await;
                                if playback.is_spotify_session_ready().await {
                                    tracing::info!("✓ Spotify session auto-initialized and ready");
                                } else {
                                    tracing::warn!("Session initialized but not verified as ready");
                                }
                            }
                            Ok(Err(e)) => {
                                tracing::warn!("Failed to auto-initialize session: {}", e);
                                has_failures = true;
                                websocket::emit_backend_init_status(
                                    &ws_sender_for_startup,
                                    "spotify-session",
                                    &format!("Failed to initialize Spotify session: {}", e),
                                    false,
                                    false,
                                );
                            }
                            Err(_) => {
                                tracing::warn!(
                                    "Timed out while auto-initializing Spotify session on startup"
                                );
                                has_failures = true;
                                websocket::emit_backend_init_status(
                                    &ws_sender_for_startup,
                                    "spotify-session",
                                    "Spotify session initialization timed out",
                                    false,
                                    false,
                                );
                            }
                        }
                    }
                }

                let app_state = handle_for_status.state::<commands::AppState>();
                websocket::emit_spotify_status(&app_state).await;

                websocket::emit_backend_init_status(
                    &ws_sender_for_startup,
                    "jellyfin-restore",
                    "Restoring Jellyfin session...",
                    false,
                    true,
                );

                let jellyfin_restored =
                    restore_jellyfin_provider_on_startup(providers_clone.clone()).await;
                if jellyfin_restored {
                    tracing::info!("✓ Jellyfin session restored from keyring on startup");
                } else {
                    tracing::info!("No cached Jellyfin credentials found on startup");
                }

                let app_state = handle_for_status.state::<commands::AppState>();
                websocket::emit_jellyfin_status(&app_state).await;

                websocket::emit_backend_init_status(
                    &ws_sender_for_startup,
                    "plex-restore",
                    "Restoring Plex session...",
                    false,
                    true,
                );

                let plex_restored = restore_plex_provider_on_startup(providers_clone.clone()).await;
                if plex_restored {
                    tracing::info!("✓ Plex session restored from keyring on startup");
                } else {
                    tracing::info!("No cached Plex credentials found on startup");
                }

                let app_state = handle_for_status.state::<commands::AppState>();
                websocket::emit_plex_status(&app_state).await;

                // Restore playback state from disk after providers are ready
                {
                    let playback = playback_for_restore.lock().await;
                    match playback.restore_state().await {
                        Ok(()) => {
                            tracing::info!("✓ Playback state restored from disk");
                        }
                        Err(e) => {
                            tracing::warn!("Failed to restore playback state from disk: {}", e);
                            has_failures = true;
                            websocket::emit_backend_init_status(
                                &ws_sender_for_startup,
                                "playback-restore",
                                &format!("Failed to restore playback state: {}", e),
                                false,
                                false,
                            );
                        }
                    }

                    // Start the state saver task AFTER restoration completes
                    // This prevents overwriting the restored state during startup
                    playback.start_state_saver().await;
                    tracing::info!("✓ State saver task started");
                }

                // Emit final status based on whether any failures occurred
                if has_failures {
                    websocket::emit_backend_init_status(
                        &ws_sender_for_startup,
                        "complete",
                        "Backend startup completed with some failures",
                        true,
                        false,
                    );
                } else {
                    // For successful completion, send an empty message
                    // The UI will hide the banner when done=true and success=true,
                    // without displaying any success message
                    websocket::emit_backend_init_status(
                        &ws_sender_for_startup,
                        "complete",
                        "",
                        true,
                        true,
                    );
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // Clean up temporary audio files when the application is closing
            if let tauri::WindowEvent::Destroyed = event {
                commands::cleanup_all_temp_audio_files();

                // Save playback state before closing - block to ensure it completes
                if let Some(app_state) = window.try_state::<commands::AppState>() {
                    let playback_clone = app_state.playback.clone();
                    tauri::async_runtime::block_on(async move {
                        let playback_locked = playback_clone.lock().await;
                        match playback_locked.save_state().await {
                            Ok(()) => tracing::info!("✓ Playback state saved on exit"),
                            Err(e) => {
                                tracing::error!("Failed to save playback state on exit: {}", e)
                            }
                        }
                    });
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn restore_spotify_provider_on_startup(providers: Arc<Mutex<ProviderRegistry>>) -> bool {
    use crate::config::Config;
    use crate::providers::spotify::SpotifyProvider;

    let tokens = match Config::load_tokens() {
        Ok(tokens) => tokens,
        Err(e) => {
            tracing::warn!(
                "Failed to load tokens while restoring Spotify session: {}",
                e
            );
            return false;
        }
    };

    let Some(token) = tokens.spotify_token else {
        return false;
    };

    let mut spotify_provider = SpotifyProvider::with_default_oauth();
    let restored = timeout(Duration::from_secs(10), spotify_provider.set_token(token)).await;

    match restored {
        Ok(Ok(())) => {
            let mut providers = providers.lock().await;
            providers.register(spotify_provider);
            true
        }
        Ok(Err(e)) => {
            tracing::warn!("Failed to restore Spotify provider token: {}", e);
            false
        }
        Err(_) => {
            tracing::warn!("Timed out restoring Spotify provider token on startup");
            false
        }
    }
}

async fn restore_jellyfin_provider_on_startup(providers: Arc<Mutex<ProviderRegistry>>) -> bool {
    use crate::config::Config;
    use crate::providers::jellyfin::JellyfinProvider;

    let tokens = match Config::load_tokens() {
        Ok(tokens) => tokens,
        Err(e) => {
            tracing::warn!(
                "Failed to load tokens while restoring Jellyfin session: {}",
                e
            );
            return false;
        }
    };

    let (Some(url), Some(api_key)) = (tokens.jellyfin_url, tokens.jellyfin_api_key) else {
        return false;
    };

    let mut jellyfin_provider = JellyfinProvider::new(url, api_key);
    let restored = timeout(Duration::from_secs(10), jellyfin_provider.authenticate()).await;

    match restored {
        Ok(Ok(())) => {
            let mut providers = providers.lock().await;
            providers.register(jellyfin_provider);
            true
        }
        Ok(Err(e)) => {
            tracing::warn!("Failed to restore Jellyfin provider: {}", e);
            false
        }
        Err(_) => {
            tracing::warn!("Timed out restoring Jellyfin provider on startup");
            false
        }
    }
}

async fn restore_plex_provider_on_startup(providers: Arc<Mutex<ProviderRegistry>>) -> bool {
    use crate::config::Config;
    use crate::providers::plex::PlexProvider;

    let tokens = match Config::load_tokens() {
        Ok(tokens) => tokens,
        Err(e) => {
            tracing::warn!("Failed to load tokens while restoring Plex session: {}", e);
            return false;
        }
    };

    let (Some(url), Some(token)) = (tokens.plex_url, tokens.plex_token) else {
        return false;
    };

    let mut plex_provider = PlexProvider::new(url, token);
    let restored = timeout(Duration::from_secs(10), plex_provider.authenticate()).await;

    match restored {
        Ok(Ok(())) => {
            let mut providers = providers.lock().await;
            providers.register(plex_provider);
            true
        }
        Ok(Err(e)) => {
            tracing::warn!("Failed to restore Plex provider: {}", e);
            false
        }
        Err(_) => {
            tracing::warn!("Timed out restoring Plex provider on startup");
            false
        }
    }
}

/// Start a simple HTTP server for OAuth callbacks
async fn start_oauth_server(
    oauth_code: Arc<Mutex<Option<String>>>,
    ws_sender: broadcast::Sender<String>,
) {
    use std::net::SocketAddr;

    let addr: SocketAddr = "127.0.0.1:8989".parse().expect("Failed to parse address");

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => {
            tracing::info!("OAuth callback server listening on {}", addr);
            l
        }
        Err(e) => {
            tracing::error!("Failed to bind OAuth server: {}", e);
            return;
        }
    };

    loop {
        match listener.accept().await {
            Ok((socket, _)) => {
                let oauth_code_clone = oauth_code.clone();
                let ws_sender_clone = ws_sender.clone();
                tauri::async_runtime::spawn(handle_oauth_request(
                    socket,
                    oauth_code_clone,
                    ws_sender_clone,
                ));
            }
            Err(e) => {
                tracing::error!("Error accepting connection: {}", e);
            }
        }
    }
}

/// Handle a single OAuth callback request
async fn handle_oauth_request(
    socket: tokio::net::TcpStream,
    oauth_code: Arc<Mutex<Option<String>>>,
    ws_sender: broadcast::Sender<String>,
) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (reader, mut writer) = socket.into_split();
    let mut reader = BufReader::new(reader);
    let mut request_line = String::new();

    if (reader.read_line(&mut request_line).await).is_ok() {
        // Extract the request path
        if let Some(path) = request_line.split_whitespace().nth(1) {
            // Parse the authorization code from the callback URL
            if path.contains("code=") {
                if let Some(code_part) = path.split("code=").nth(1) {
                    if let Some(code) = code_part.split('&').next() {
                        let code_str = code.to_string();

                        // Store the code for the UI to retrieve
                        {
                            let mut code_storage = oauth_code.lock().await;
                            *code_storage = Some(code_str.clone());
                        }

                        websocket::broadcast_event(
                            &ws_sender,
                            "oauth-code-received",
                            serde_json::json!({ "source": "spotify" }),
                        );

                        // Send a response to the browser
                        let response = b"HTTP/1.1 200 OK\r\n\
                                      Content-Type: text/html\r\n\
                                      Content-Length: 220\r\n\
                                      \r\n\
                                      <!DOCTYPE html>\r\n\
                                      <html>\r\n\
                                      <head><title>Authentication Complete</title></head>\r\n\
                                      <body style=\"font-family: Arial, sans-serif; text-align: center; padding: 50px;\">\r\n\
                                      <h1>Authentication Successful</h1>\r\n\
                                      <p>You can close this window.</p>\r\n\
                                      </body>\r\n\
                                      </html>\r\n";

                        let _ = writer.write_all(response).await;
                        let _ = writer.flush().await;

                        tracing::info!("OAuth callback received and code stored");
                        return;
                    }
                }
            }

            // Handle error case
            if path.contains("error=") {
                let response = b"HTTP/1.1 400 Bad Request\r\n\
                              Content-Type: text/html\r\n\
                              Content-Length: 150\r\n\
                              \r\n\
                              <!DOCTYPE html>\r\n\
                              <html>\r\n\
                              <body>\r\n\
                              <p>Authentication failed. Please try again.</p>\r\n\
                              </body>\r\n\
                              </html>\r\n";
                let _ = writer.write_all(response).await;
                let _ = writer.flush().await;
                return;
            }
        }
    }

    // Default response for other requests
    let response = b"HTTP/1.1 404 Not Found\r\n\
                  Content-Length: 0\r\n\
                  \r\n";
    let _ = writer.write_all(response).await;
    let _ = writer.flush().await;
}
