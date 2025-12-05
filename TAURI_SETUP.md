# Tauri Desktop UI Setup Guide

This guide explains how to build and run the Any Player desktop application using Tauri.

## Overview

The desktop UI for Any Player has been added using [Tauri](https://tauri.app/), a lightweight framework for building desktop applications with Rust and web technologies. The architecture includes:

- **Rust Backend** (`src-tauri/src/main.rs`): Tauri app with command handlers
- **Web Frontend** (`src-tauri/ui/dist/`): Vanilla JavaScript + HTML/CSS UI
- **IPC Bridge** (`src-tauri/src/commands.rs`): Type-safe communication layer

## Project Structure

```
any-player/
├── src/                    # Rust library and CLI
├── src-tauri/              # Tauri desktop app
│   ├── src/
│   │   ├── main.rs         # Tauri app entry point
│   │   └── commands.rs     # Command handlers
│   ├── ui/
│   │   └── dist/           # Web frontend
│   │       ├── index.html
│   │       ├── styles.css
│   │       ├── app.js
│   │       ├── api.js
│   │       ├── ui.js
│   │       └── tauri.js
│   ├── Cargo.toml
│   └── build.rs
├── Cargo.toml              # Workspace manifest
└── tauri.conf.json         # Tauri configuration
```

## Prerequisites

### System Requirements

#### macOS
- Xcode command line tools: `xcode-select --install`
- Rust 1.70+: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

#### Linux (Ubuntu/Debian)
```bash
sudo apt-get update
sudo apt-get install -y \
    build-essential \
    curl \
    wget \
    libssl-dev \
    libgtk-3-dev \
    libayatana-appindicator3-dev \
    libwebkit2gtk-4.0-dev
```

#### Windows
- Visual Studio Build Tools (for C++ build tools)
- WebView2 Runtime (auto-installed on most systems)

### Rust Setup

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Update Rust
rustup update

# Verify installation
rustc --version
cargo --version
```

## Building the Desktop App

### Development Build

```bash
# From the project root
cd /home/nesbitt/Desktop/any-player

# Install dependencies (one-time)
cargo build -p any-player-tauri

# Run in development mode
cargo tauri dev
```

This starts a development server with hot-reload enabled.

### Production Build

```bash
# Build the optimized release version
cargo tauri build -p any-player-tauri
```

The compiled binary will be in `src-tauri/target/release/`.

Platform-specific installers will be generated:
- **macOS**: `.dmg` file
- **Linux**: `.deb` and `.AppImage` files
- **Windows**: `.msi` installer

## Running the App

### From Development Build
```bash
cargo tauri dev
```

### From Release Build

**macOS:**
```bash
open src-tauri/target/release/bundle/macos/Any\ Player.app
```

**Linux:**
```bash
./src-tauri/target/release/any-player-tauri
# or install the .deb
sudo dpkg -i src-tauri/target/release/bundle/deb/*.deb
any-player-tauri
```

**Windows:**
Run the `.msi` installer or execute the `.exe` directly.

## Features

### Current Implementation

✅ **Playback Controls**
- Play, pause, next, previous
- Seek to position
- Volume control
- Shuffle and repeat modes

✅ **UI Pages**
- **Now Playing**: Current track display with full playback controls
- **Playlists**: Browse playlists from connected services
- **Search**: Search for tracks and playlists
- **Settings**: Configure Spotify and Jellyfin connections

✅ **Backend Integration**
- Fully type-safe Tauri command system
- Integration with existing Rust backend
- Async playback management

### Architecture

The application uses an IPC (Inter-Process Communication) model:

1. **Frontend** (JavaScript) sends user interactions via Tauri commands
2. **Backend** (Rust) processes commands and manages playback state
3. **Response** is sent back to frontend to update the UI

Example flow:
```
User clicks "Play" → JS calls tauriAPI.play() → Tauri invokes play() command
→ Rust backend starts playback → Response sent back → UI updates
```

## Tauri Commands

The following commands are available from the frontend:

### Playback
- `get_playback_status()` - Get current playback state
- `play()` - Start playback
- `pause()` - Pause playback
- `toggle_play_pause()` - Toggle between play and pause
- `next_track()` - Play next track
- `previous_track()` - Play previous track
- `seek(position: u64)` - Seek to position (ms)
- `set_volume(volume: u32)` - Set volume (0-100)
- `toggle_shuffle()` - Toggle shuffle mode
- `set_repeat_mode(mode: String)` - Set repeat mode (off/one/all)

### Playlists
- `get_playlists(source: String)` - Get playlists from a source
- `queue_track(track_id: String, source: String)` - Add track to queue
- `clear_queue()` - Clear the playback queue

## Configuration

Edit `tauri.conf.json` to customize:

```json
{
  "tauri": {
    "windows": [
      {
        "width": 1200,
        "height": 800,
        "minWidth": 800,
        "minHeight": 600,
        "title": "Any Player"
      }
    ]
  }
}
```

### Security Settings

The app uses a restrictive Content Security Policy. To add features, update `allowlist` in `tauri.conf.json`:

```json
{
  "tauri": {
    "allowlist": {
      "core": {
        "all": false
      },
      "shell": {
        "open": true
      }
    }
  }
}
```

## Development Workflow

### Frontend Development

Edit files in `src-tauri/ui/dist/`:
- `index.html` - UI structure
- `styles.css` - Styling
- `api.js` - Tauri API wrapper
- `ui.js` - UI controller
- `app.js` - App initialization

Changes are hot-reloaded in `cargo tauri dev`.

### Backend Development

Edit files in `src-tauri/src/`:
- `main.rs` - Tauri app setup
- `commands.rs` - Command implementations

Changes require restarting the development server.

### Testing Commands

Use Tauri's developer tools (DevTools) to test commands:

```javascript
// In browser console while running cargo tauri dev
await __TAURI__.invoke('play')
await __TAURI__.invoke('get_playback_status')
```

## Troubleshooting

### Build Errors

**Missing WebKit2 development libraries (Linux)**
```bash
sudo apt-get install libwebkit2gtk-4.0-dev
```

**Cargo workspace issues**
```bash
# Clean and rebuild
cargo clean
cargo build -p any-player-tauri
```

**Tauri CLI not found**
```bash
cargo install tauri-cli
```

### Runtime Issues

**Window won't appear**
- Check `tauri.conf.json` configuration
- Verify `devPath` or `frontendDist` paths are correct
- Check browser console for JavaScript errors

**Commands not responding**
- Ensure Tauri dev server is running
- Check command name matches exactly (case-sensitive)
- Verify command is registered in `invoke_handler!`

### Platform-Specific Issues

**macOS: "App is damaged" when launching**
```bash
sudo xattr -rd com.apple.quarantine path/to/app
```

**Linux: AppImage won't run**
```bash
chmod +x any-player-tauri.AppImage
./any-player-tauri.AppImage
```

**Windows: Visual C++ Runtime error**
- Download Visual C++ Redistributable from Microsoft
- Install WebView2 Runtime

## Next Steps

1. **Implement Search**: Add search functionality using backend providers
2. **Add Spotify Integration**: Implement OAuth flow for Spotify authentication
3. **Jellyfin Connection**: Build configuration UI for Jellyfin servers
4. **Track Display**: Show full track information with album artwork
5. **Queue Management**: Build playlist and queue UI
6. **System Integration**: Add media keys support and native notifications

## Useful Resources

- [Tauri Documentation](https://tauri.app/docs/)
- [Tauri API Reference](https://tauri.app/docs/api/js/)
- [Tauri Command System](https://tauri.app/docs/features/command)
- [Tauri IPC Security](https://tauri.app/docs/security/allowlist/)

## Building for Distribution

### macOS
```bash
cargo tauri build
# Creates: src-tauri/target/release/bundle/macos/Any\ Player.app
# And: src-tauri/target/release/bundle/dmg/Any\ Player_*.dmg
```

### Linux
```bash
cargo tauri build
# Creates: src-tauri/target/release/bundle/deb/*.deb
# And: src-tauri/target/release/bundle/appimage/*.AppImage
```

### Windows
```bash
cargo tauri build
# Creates: src-tauri/target/release/bundle/msi/*.msi
```

### Code Signing (Optional)

For production, you can sign binaries. See [Tauri Code Signing Guide](https://tauri.app/docs/distribution/sign/).

## CLI vs GUI

Both CLI and GUI can be run:

```bash
# CLI (Terminal UI)
cargo build --features cli
./target/debug/any-player-cli

# GUI (Desktop)
cargo tauri dev
# or
cargo tauri build
```

Modify `Cargo.toml` `[features]` section to change defaults.
