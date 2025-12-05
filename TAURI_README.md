# Tauri Desktop UI Guide

## Overview

A professional desktop application for Any Player has been added using **Tauri**, a lightweight framework for building cross-platform desktop applications with Rust and web technologies.

## Quick Links

- **Quick Start**: See `TAURI_QUICKSTART.md`
- **Detailed Setup**: See `TAURI_SETUP.md`
- **Implementation Details**: See `TAURI_IMPLEMENTATION.md`
- **Development Checklist**: See `TAURI_CHECKLIST.md`

## Getting Started in 2 Minutes

### 1. Install Prerequisites

```bash
# macOS
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Ubuntu/Debian
sudo apt-get install build-essential libgtk-3-dev libwebkit2gtk-4.0-dev

# Windows: Install Visual Studio Build Tools + WebView2 Runtime
```

### 2. Run the App

```bash
cd /home/nesbitt/Desktop/any-player
cargo tauri dev
```

The desktop application opens automatically with hot-reload enabled.

## Project Structure

```
any-player/
├── src/                          # Shared Rust library
│   ├── lib.rs
│   ├── models/                   # Data structures
│   ├── providers/                # Spotify, Jellyfin
│   ├── playback/                 # Playback engine
│   ├── config/                   # Configuration
│   └── ui/                       # UI components (TUI)
│
├── src-tauri/                    # Desktop app (Tauri)
│   ├── src/
│   │   ├── main.rs              # Tauri entry point
│   │   └── commands.rs          # IPC commands
│   ├── ui/dist/                 # Web frontend
│   │   ├── index.html           # UI structure
│   │   ├── styles.css           # Dark theme
│   │   ├── api.js               # API wrapper
│   │   ├── ui.js                # UI controller
│   │   ├── app.js               # Initialization
│   │   └── tauri.js             # Tauri stubs
│   ├── Cargo.toml
│   └── build.rs
│
├── Cargo.toml                    # Workspace config
├── tauri.conf.json               # Tauri settings
└── [Documentation Files]
```

## Features

### 🎵 Now Playing Page
- Current track display
- Album artwork (placeholder)
- Playback progress bar with seek
- Play, pause, next, previous buttons
- Volume control
- Shuffle and repeat mode controls
- Queue display

### 📋 Playlists Page
- Browse playlists from all sources
- Filter by source (Spotify, Jellyfin)
- Click to view playlist details
- Add to queue functionality

### 🔍 Search Page
- Search for tracks and playlists
- Multi-source search capability
- Filter by source
- Result display with metadata

### ⚙️ Settings Page
- Spotify connection
- Jellyfin configuration
- Provider status display
- Playback preferences

## Commands Available

All commands use the Tauri IPC bridge:

**Playback:**
- `play()` - Start playback
- `pause()` - Pause playback
- `toggle_play_pause()` - Toggle
- `next_track()` - Next track
- `previous_track()` - Previous track
- `seek(position)` - Seek to position
- `set_volume(volume)` - Set volume 0-100
- `toggle_shuffle()` - Toggle shuffle
- `set_repeat_mode(mode)` - Set repeat mode

**Playlists:**
- `get_playlists(source)` - Get playlists
- `queue_track(id, source)` - Queue track
- `get_playback_status()` - Get current status

## Building

### Development (Hot Reload)
```bash
cargo tauri dev
```

### Production Build
```bash
cargo tauri build
```

Creates installers in `src-tauri/target/release/bundle/`:
- **macOS**: `.dmg` file
- **Linux**: `.deb` and `.AppImage` files
- **Windows**: `.msi` installer

## Architecture

### IPC Model

```
Frontend (JavaScript)  ←→  Tauri Bridge  ←→  Backend (Rust)
```

1. User interacts with UI (click button, etc.)
2. JavaScript calls Tauri command via `tauriAPI.invoke()`
3. Command routed to Rust handler
4. Handler processes using `PlaybackManager` and `ProviderRegistry`
5. Response returned to JavaScript
6. UI updated with result

### State Management

- **Thread-safe**: Using `Arc<Mutex<T>>`
- **Async-first**: All I/O operations are non-blocking
- **Type-safe**: Serde for serialization

## CLI vs GUI

Run CLI (Terminal UI):
```bash
cargo build --features cli --bin any-player-cli
./target/debug/any-player-cli tui
```

Run GUI (Desktop):
```bash
cargo tauri dev
# or
cargo tauri build
```

## Theming

The UI uses a dark theme inspired by Spotify:

- **Primary Color**: `#1DB954` (Spotify Green)
- **Background**: `#121212` (Dark)
- **Surface**: `#1e1e1e` (Lighter Dark)
- **Text**: `#ffffff` (White)
- **Secondary Text**: `#b3b3b3` (Gray)

Edit `src-tauri/ui/dist/styles.css` to customize.

## Troubleshooting

### Build Issues

**"WebKit2 not found" on Linux:**
```bash
sudo apt-get install libwebkit2gtk-4.0-dev
```

**"Workspace error":**
```bash
cargo clean
cargo tauri dev
```

### Runtime Issues

**"App won't start":**
- Check console for errors (DevTools)
- Verify `tauri.conf.json` paths
- Try: `cargo clean && cargo tauri dev`

**"Commands not responding":**
- Ensure Tauri dev server running
- Check command name matches (case-sensitive)
- Look at Rust console for errors

## File Structure Legend

```
src-tauri/
├── src/
│   ├── main.rs              ← Entry point, initializes Tauri
│   └── commands.rs          ← Command handlers (IPC)
├── ui/
│   └── dist/
│       ├── index.html       ← UI structure
│       ├── styles.css       ← Styling (1000+ lines)
│       ├── api.js           ← Tauri API wrapper
│       ├── ui.js            ← UI logic & event handlers
│       ├── app.js           ← Initialization
│       └── tauri.js         ← Development stubs
├── Cargo.toml               ← Tauri package config
└── build.rs                 ← Build script
```

## Development Workflow

### Frontend Development
1. Edit files in `src-tauri/ui/dist/`
2. Changes hot-reload automatically in `cargo tauri dev`
3. Use browser DevTools (F12) to debug

### Backend Development
1. Edit files in `src-tauri/src/`
2. Changes require restarting `cargo tauri dev`
3. View logs in terminal running the dev server

### Testing Commands
Use browser console while `cargo tauri dev` is running:

```javascript
// In browser DevTools console
await __TAURI__.invoke('get_playback_status')
await __TAURI__.invoke('play')
await __TAURI__.invoke('set_volume', { volume: 50 })
```

## Next Steps

1. **Run the app**: `cargo tauri dev`
2. **Test the UI**: Click buttons to test
3. **Connect Spotify**: Settings → Spotify (implementation pending)
4. **Connect Jellyfin**: Settings → Jellyfin (implementation pending)
5. **Browse playlists**: Playlists page (backend pending)
6. **Search music**: Search page (backend pending)

## Customization

### Change Window Size
Edit `tauri.conf.json`:
```json
{
  "tauri": {
    "windows": [{
      "width": 1400,
      "height": 900
    }]
  }
}
```

### Change App Title
Edit `tauri.conf.json`:
```json
{
  "package": {
    "productName": "My Music Player"
  }
}
```

### Change UI Colors
Edit `src-tauri/ui/dist/styles.css`:
```css
:root {
    --primary-color: #1DB954;     /* Change to your color */
    --background: #121212;
    /* ... */
}
```

## Performance Notes

- **Bundle Size**: ~50-70MB per platform
- **Memory**: ~100-150MB baseline
- **Startup Time**: 1-2 seconds
- **CPU**: Minimal when idle

## Security

- **CSP**: Content Security Policy restricts scripts
- **Allowlist**: Only `shell.open` enabled by default
- **No Remote Code**: UI is local only
- **Serialization**: Serde for type-safe data transfer

## Distribution

### Code Signing (macOS/Windows)
See `TAURI_SETUP.md` for detailed code signing instructions.

### Auto-Updates
Tauri has built-in auto-update support. Configure in `tauri.conf.json`.

## Resources

- **Tauri Docs**: https://tauri.app/docs/
- **Tauri API**: https://tauri.app/docs/api/js/
- **Command System**: https://tauri.app/docs/features/command
- **Security**: https://tauri.app/docs/security/

## Support

For issues:
1. Check `TAURI_SETUP.md` troubleshooting section
2. Check browser console (F12)
3. Check terminal output
4. See `TAURI_CHECKLIST.md` for known limitations

## Contributing

To contribute to the desktop UI:
1. Read `TAURI_IMPLEMENTATION.md` for architecture details
2. Check `TAURI_CHECKLIST.md` for tasks
3. Follow existing code style
4. Test on multiple platforms if possible

## License

Same as main Any Player project (see LICENSE file)

---

**Version**: 1.0
**Last Updated**: December 5, 2025
**Status**: Foundation Complete, Features In Progress
