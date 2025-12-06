#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

/// GUI entry point for Any Player Desktop (Tauri)
/// This is the main entry point when building the desktop GUI version
fn main() {
    // This file serves as a placeholder for conditional compilation
    // The actual Tauri app is in src-tauri/src/main.rs
    eprintln!("Please use 'cargo build -p any-player-tauri' to build the Tauri desktop app");
    std::process::exit(1);
}
