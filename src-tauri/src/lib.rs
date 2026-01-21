pub mod config;
pub mod database;
/// Any Player - Multi-Source Music Client
pub mod models;
pub mod playback;
pub mod providers;

pub use config::Config;
pub use database::Database;
pub use models::{PlaybackInfo, PlaybackState, Playlist, RepeatMode, Source, Track};
pub use playback::PlaybackManager;
pub use providers::{MusicProvider, ProviderError, ProviderRegistry};
use tracing_subscriber::{filter, layer::SubscriberExt, util::SubscriberInitExt};

mod commands;

use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize logging
    let filter = filter::Targets::new()
        .with_default(filter::LevelFilter::TRACE)
        .with_target("any_player_lib", filter::LevelFilter::TRACE)
        .with_target("glycin", filter::LevelFilter::INFO)
        .with_target("hyper", filter::LevelFilter::INFO)
        .with_target("zbus", filter::LevelFilter::INFO);
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Initialize database
    let db_path = dirs::data_dir()
        .expect("Failed to get data directory")
        .join("any-player")
        .join("playlists.db");

    std::fs::create_dir_all(db_path.parent().unwrap()).expect("Failed to create data directory");

    let database = Arc::new(Mutex::new(
        Database::new(db_path).expect("Failed to initialize database"),
    ));

    // Create application state
    let providers = Arc::new(Mutex::new(ProviderRegistry::new()));
    let playback = Arc::new(Mutex::new(PlaybackManager::new(providers.clone())));
    let oauth_code: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let providers_clone = providers.clone();
    let playback_clone = playback.clone();

    let app_state = commands::AppState {
        playback,
        providers,
        oauth_code: oauth_code.clone(),
        database,
    };

    let oauth_code_for_server = oauth_code.clone();

    tauri::Builder::default()
        .manage(app_state)
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            // Playback commands
            commands::get_playback_status,
            commands::play,
            commands::pause,
            commands::toggle_play_pause,
            commands::next_track,
            commands::previous_track,
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
        ])
        .setup(move |_app| {
            // Start OAuth callback server in the Tauri runtime
            let oauth_code_clone = oauth_code_for_server.clone();
            tauri::async_runtime::spawn(start_oauth_server(oauth_code_clone));

            // Try to restore Spotify session on startup in the background
            // This allows the UI to load immediately while authentication is being restored
            let playback_for_init = playback_clone.clone();
            let providers_for_jellyfin = providers_clone.clone();
            tauri::async_runtime::spawn(async move {
                // Restore session without holding the lock during the entire process
                let restored = {
                    let mut providers = providers_clone.lock().await;
                    match providers.restore_spotify_session().await {
                        Ok(restored) => {
                            if restored {
                                tracing::info!("✓ Spotify session restored from cache on startup");
                            } else {
                                tracing::info!("No cached Spotify session found on startup");
                            }
                            restored
                        }
                        Err(e) => {
                            tracing::warn!("Failed to restore Spotify session: {}", e);
                            false
                        }
                    }
                };

                // Auto-initialize session for premium users without holding the providers lock
                if restored {
                    let is_premium = {
                        let providers = providers_clone.lock().await;
                        providers.is_spotify_premium().await
                    };

                    if let Some(true) = is_premium {
                        let access_token = {
                            let providers = providers_clone.lock().await;
                            providers.get_spotify_access_token().await
                        };

                        if let Some(access_token) = access_token {
                            tracing::info!("Auto-initializing Spotify session for premium user");

                            let playback = playback_for_init.lock().await;
                            match playback.initialize_spotify_session(&access_token).await {
                                Ok(()) => {
                                    if playback.is_spotify_session_ready().await {
                                        tracing::info!(
                                            "✓ Spotify session auto-initialized and ready"
                                        );
                                    } else {
                                        tracing::warn!(
                                            "Session initialized but not verified as ready"
                                        );
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to auto-initialize session: {}", e);
                                }
                            }
                        }
                    }
                }

                // Also try to restore Jellyfin session
                {
                    let mut providers = providers_for_jellyfin.lock().await;
                    match providers.restore_jellyfin_session().await {
                        Ok(restored) => {
                            if restored {
                                tracing::info!(
                                    "✓ Jellyfin session restored from keyring on startup"
                                );
                            } else {
                                tracing::info!("No cached Jellyfin credentials found on startup");
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to restore Jellyfin session: {}", e);
                        }
                    }
                }
            });

            Ok(())
        })
        .on_window_event(|_window, event| {
            // Clean up temporary audio files when the application is closing
            if let tauri::WindowEvent::Destroyed = event {
                commands::cleanup_all_temp_audio_files();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Start a simple HTTP server for OAuth callbacks
async fn start_oauth_server(oauth_code: Arc<Mutex<Option<String>>>) {
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
                tauri::async_runtime::spawn(handle_oauth_request(socket, oauth_code_clone));
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
