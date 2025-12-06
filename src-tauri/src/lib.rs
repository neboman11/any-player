pub mod config;
/// Any Player - Multi-Source Music Client
pub mod models;
pub mod playback;
pub mod providers;

pub use config::Config;
pub use models::{PlaybackInfo, PlaybackState, Playlist, RepeatMode, Source, Track};
pub use playback::PlaybackManager;
pub use providers::{MusicProvider, ProviderError, ProviderRegistry};

mod commands;

use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create application state
    let playback = Arc::new(Mutex::new(PlaybackManager::new()));
    let providers = Arc::new(Mutex::new(ProviderRegistry::new()));
    let oauth_code: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let app_state = commands::AppState {
        playback,
        providers,
        oauth_code: oauth_code.clone(),
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
            commands::queue_track,
            commands::clear_queue,
            // Spotify commands
            commands::get_spotify_auth_url,
            commands::authenticate_spotify,
            commands::is_spotify_authenticated,
            commands::get_spotify_playlists,
            commands::check_oauth_code,
            // Jellyfin commands
            commands::authenticate_jellyfin,
            commands::is_jellyfin_authenticated,
            commands::get_jellyfin_playlists,
            commands::get_jellyfin_playlist,
            commands::search_jellyfin_tracks,
            commands::search_jellyfin_playlists,
            commands::get_jellyfin_recently_played,
        ])
        .setup(move |_app| {
            // Start OAuth callback server in the Tauri runtime
            let oauth_code_clone = oauth_code_for_server.clone();
            tauri::async_runtime::spawn(start_oauth_server(oauth_code_clone));
            Ok(())
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

    if let Ok(_) = reader.read_line(&mut request_line).await {
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
