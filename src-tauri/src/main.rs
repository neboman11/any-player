// Main Tauri application entry point
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;

use any_player::{PlaybackManager, ProviderRegistry};
use commands::AppState;
use std::sync::Arc;
use tokio::sync::Mutex;

fn main() {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create application state
    let playback = Arc::new(Mutex::new(PlaybackManager::new()));
    let providers = Arc::new(Mutex::new(ProviderRegistry::new()));

    let app_state = AppState {
        playback,
        providers,
    };

    tauri::Builder::default()
        .manage(app_state)
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
