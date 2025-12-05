# Quick Start: Desktop UI with Tauri

Get the Any Player desktop application running in minutes.

## 1. Prerequisites

**macOS:**
```bash
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**Linux (Ubuntu/Debian):**
```bash
sudo apt-get install -y build-essential curl wget libssl-dev libgtk-3-dev libayatana-appindicator3-dev libwebkit2gtk-4.0-dev
rustup install stable
```

**Windows:**
- Install Visual Studio Build Tools
- WebView2 Runtime (usually pre-installed)

## 2. Run Development Build

```bash
cd /home/nesbitt/Desktop/any-player
cargo tauri dev
```

This starts the app with hot-reload. The window opens automatically.

## 3. Build for Release

```bash
cargo tauri build
```

This creates platform-specific installers in `src-tauri/target/release/bundle/`.

## UI Features

### Pages

- **Now Playing**: Shows current track, playback controls, queue
- **Playlists**: Browse playlists from Spotify/Jellyfin
- **Search**: Search for music across sources
- **Settings**: Configure provider connections

### Controls

- **Play/Pause**: Center button
- **Next/Previous**: Arrow buttons
- **Shuffle**: Toggles with 🔀 button
- **Repeat**: Cycles through off → one → all
- **Volume**: Slider control

## Running CLI Instead

```bash
cargo build --features cli --bin any-player-cli
./target/debug/any-player-cli tui
```

## Documentation

- **Full Setup Guide**: See `TAURI_SETUP.md`
- **Architecture**: See `ARCHITECTURE.md`
- **Implementation**: See `IMPLEMENTATION.md`

## Next Steps

1. Connect Spotify (Settings → Spotify)
2. Connect Jellyfin (Settings → Jellyfin)
3. Browse playlists
4. Search and queue tracks
5. Enjoy your music!

## Troubleshooting

**App won't start:**
```bash
cargo clean
cargo tauri dev
```

**On Linux - WebKit2 error:**
```bash
sudo apt-get install libwebkit2gtk-4.0-dev
```

**On macOS - Permission denied:**
```bash
sudo xattr -rd com.apple.quarantine target/release/bundle/macos/Any\ Player.app
```
