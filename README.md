# Any Player

A multi-source music client for the terminal with support for Spotify and Jellyfin.

## Features

- 🎵 **Multi-Source Playback**: Play music from Spotify and Jellyfin
- 🎼 **Unified Interface**: Browse playlists from both sources in one application
- 🎹 **Playlist Management**: Create and manage playlists across sources
- 🎹 **TUI**: Beautiful terminal user interface with keyboard controls
- ⚙️ **Configuration**: Easy TOML-based configuration
- 🔊 **Full Playback Control**: Play, pause, seek, volume, shuffle, repeat

## Installation

### Prerequisites

- Rust 1.70+ ([Install](https://rustup.rs/))
- Linux/macOS/Windows
- For audio output:
  - Linux: ALSA development libraries (`libasound2-dev` on Debian)
  - macOS: No additional dependencies
  - Windows: No additional dependencies

### From Source

```bash
git clone https://github.com/yourusername/any-player
cd any-player
cargo build --release
./target/release/any-player
```

## Quick Start

### 1. Start the Application

```bash
any-player
```

This launches the interactive TUI.

### 2. Configure Sources

Before using Spotify or Jellyfin, you need to configure them:

#### Spotify Setup

1. Create a Spotify Developer application at https://developer.spotify.com/
2. Note your Client ID and Client Secret
3. Set the redirect URI to `http://127.0.0.1:8989/login`
4. Authenticate via:
   ```bash
   any-player auth spotify
   ```

#### Jellyfin Setup

Configure your Jellyfin server details:

```bash
any-player auth jellyfin
```

You'll be prompted for:
- Server URL (e.g., `http://192.168.1.100:8096`)
- API key (from Jellyfin admin panel)

### 3. Browse and Play

- Use `p` to view playlists
- Press `/` to search
- Press `space` to play/pause
- Press `n` for next track
- Press `q` to quit

## Commands

### Interactive Mode

```bash
any-player                              # Start TUI
any-player tui                          # Explicit TUI mode
```

### CLI Commands

```bash
# List playlists
any-player list --source spotify
any-player list --source jellyfin
any-player list --source both

# Search for tracks
any-player search "artist name" --source spotify
any-player search "song" --source both

# Search for playlists
any-player search "workout" --source spotify --playlists

# Play a playlist or track
any-player play <id> --source spotify

# Create a playlist
any-player create-playlist "My Playlist" --source spotify --description "My custom mix"

# Add track to playlist
any-player add-track <playlist-id> <track-id> --source spotify

# Show status
any-player status

# Authentication
any-player auth spotify
any-player auth jellyfin
```

## Configuration

Configuration file location: `~/.config/any-player/config.toml`

### Example Config

```toml
[general]
logging_enabled = true
log_level = "info"
enable_images = true
theme = "default"

[spotify]
client_id = "your-client-id"
client_secret = "your-client-secret"
redirect_uri = "http://127.0.0.1:8989/login"
enable_streaming = true

[jellyfin]
server_url = "http://192.168.1.100:8096"
api_key = "your-api-key"
username = "your-username"
```

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `space` | Play/Pause |
| `n` | Next track |
| `p` | Previous track |
| `s` | Toggle shuffle |
| `r` | Cycle repeat mode |
| `+` / `-` | Volume up/down |
| `/` | Search |
| `u` `p` | View playlists |
| `q` | Quit |

## Architecture

See [ARCHITECTURE.md](./ARCHITECTURE.md) for detailed architecture documentation.

### High-Level Design

```
┌─────────────────────────────────────┐
│     CLI / TUI (ratatui)             │
│  - Pages, Components, Themes        │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│    PlaybackManager                   │
│  - Queue Management                 │
│  - Playback State (async)           │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│  ProviderRegistry                   │
│  - MusicProvider Trait              │
│  - Spotify Provider                 │
│  - Jellyfin Provider                │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│  External APIs                       │
│  - Spotify Web API (rspotify)       │
│  - Jellyfin API (reqwest)           │
│  - Audio Playback (rodio)           │
└─────────────────────────────────────┘
```

## Development

### Project Structure

```
src/
├── main.rs           # CLI entry point
├── lib.rs            # Library root
├── models/           # Data types
├── providers/        # Music sources
├── playback/         # Playback engine
├── config/           # Configuration
└── ui/               # Terminal UI
```

### Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Check code quality
cargo clippy

# Format code
cargo fmt
```

## Current Status

### ✅ Implemented
- Core architecture and trait system
- Project scaffolding and module layout
- Configuration management
- CLI command structure
- Playback queue and manager
- Basic UI components

### ⏳ In Development
- Spotify provider integration
- Jellyfin provider integration
- Audio playback implementation
- TUI event loop and rendering

## Contributing

Contributions welcome! Please:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit changes (`git commit -m 'Add amazing feature'`)
4. Push to branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## License

MIT License - see LICENSE file for details

## Acknowledgments

Inspired by:
- [spotify-player](https://github.com/aome510/spotify-player) - Excellent Spotify TUI reference
- [ncspot](https://github.com/hrkfdn/ncspot) - Spotify CLI player
- [ratatui](https://github.com/ratatui/ratatui) - Rust TUI framework

## Troubleshooting

### Authentication Issues

If you encounter authentication problems:

1. Clear cached credentials: `rm -rf ~/.cache/any-player/`
2. Re-authenticate: `any-player auth spotify` or `any-player auth jellyfin`
3. Check configuration: `cat ~/.config/any-player/config.toml`

### Audio Not Working

- Ensure your system has audio output configured
- Check logs: `RUST_LOG=debug any-player`
- On Linux, verify ALSA is installed

## Support

For issues and questions:
- GitHub Issues: [any-player/issues](https://github.com/yourusername/any-player/issues)
- Discussions: [any-player/discussions](https://github.com/yourusername/any-player/discussions)
