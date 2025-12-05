# Any Player - Multi-Source Music Client Architecture

A Rust-based CLI application for playing and managing music from multiple sources (Spotify and Jellyfin) with a unified interface.

## Project Overview

**Any Player** is a terminal-based music player that supports multiple streaming sources through a plugin-based provider architecture. Users can search, browse, manage playlists, and play music from both Spotify and Jellyfin simultaneously.

## Core Architecture

### 1. **Provider System** (`src/providers/`)

#### Trait-Based Design
All music providers implement the `MusicProvider` trait, enabling source-agnostic operations:

```rust
pub trait MusicProvider: Send + Sync {
    // Authentication
    async fn authenticate(&mut self) -> Result<(), ProviderError>;
    fn is_authenticated(&self) -> bool;

    // Playlist operations
    async fn get_playlists(&self) -> Result<Vec<Playlist>, ProviderError>;
    async fn get_playlist(&self, id: &str) -> Result<Playlist, ProviderError>;
    async fn create_playlist(&self, name: &str, description: Option<&str>) -> Result<Playlist, ProviderError>;

    // Search
    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>, ProviderError>;
    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>, ProviderError>;

    // Streaming
    async fn get_stream_url(&self, track_id: &str) -> Result<String, ProviderError>;

    // Track management
    async fn add_track_to_playlist(&self, playlist_id: &str, track: &Track) -> Result<(), ProviderError>;
    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<(), ProviderError>;

    // History
    async fn get_recently_played(&self, limit: usize) -> Result<Vec<Track>, ProviderError>;
}
```

#### Provider Registry
`ProviderRegistry` manages provider instances:
- Multiple providers can be registered simultaneously
- Lookup by source type
- Centralized provider lifecycle management

### 2. **Data Models** (`src/models/`)

**Core Types:**
- `Source`: Enum (Spotify, Jellyfin) identifying the provider
- `Track`: Individual song with metadata (title, artist, album, duration, source, URL)
- `Playlist`: Collection of tracks with owner information
- `PlaybackState`: Enum (Playing, Paused, Stopped)
- `RepeatMode`: Enum (Off, One, All)
- `PlaybackInfo`: Current playback status including position, volume, shuffle state

### 3. **Playback System** (`src/playback/`)

#### PlaybackQueue
- Manages ordered list of tracks
- Tracks current position
- Supports navigation (next, previous)
- Methods: `add_track()`, `add_tracks()`, `current_track()`, `clear()`

#### PlaybackManager
- Async-safe playback state management using `Arc<Mutex<T>>`
- Core operations:
  - `queue_track()` / `queue_tracks()`: Add to queue
  - `play()` / `pause()` / `toggle_play_pause()`: Control playback
  - `next_track()` / `previous_track()`: Navigate queue
  - `seek()`: Jump to position
  - `set_volume()`: Control volume (0-100)
  - `toggle_shuffle()` / `set_repeat_mode()`: Playback modes
- Thread-safe design for integration with async runtime

### 4. **Configuration System** (`src/config/`)

#### Config Struct
```rust
pub struct Config {
    general: GeneralConfig,      // Logging, UI, theme
    spotify: Option<SpotifyConfig>,    // Client ID/Secret
    jellyfin: Option<JellyfinConfig>,  // Server URL, API key
}
```

**Storage:**
- Default location: `~/.config/any-player/config.toml`
- Cache location: `~/.cache/any-player/`
- Auto-creates with defaults on first run
- TOML format for human readability

### 5. **UI Layer** (`src/ui/`)

#### Pages
- `HomePage`: Initial interface with source selection
- `SearchPage`: Search interface for tracks/playlists
- `PlaylistPage`: View playlist contents
- `NowPlayingPage`: Display current track and playback controls
- `QueuePage`: Show upcoming tracks

#### Components (Reusable)
- `PlaybackBar`: Progress indicator with time display
- `TrackInfo`: Display current track metadata
- `PlaybackControls`: Play/pause, shuffle, repeat controls
- `SourceSelector`: Source provider selection

#### Themes
- `Theme` struct with predefined palettes:
  - `default_dark()`: Dark theme
  - `default_light()`: Light theme
  - `spotify()`: Spotify green theme
- Colors: primary, secondary, accent, background, foreground, error, success

## Implementation Status

### ✅ Completed
- Project structure and module layout
- Core trait system for providers (`MusicProvider`)
- Data models (Track, Playlist, PlaybackInfo, etc.)
- Playback queue and manager system
- Configuration management
- CLI command structure (argument parsing)
- UI component scaffolding
- Theme system

### ⏳ In Progress (Next Steps)
1. **Spotify Provider** (`src/providers/spotify.rs`)
   - OAuth authentication flow
   - Playlist fetching via rspotify
   - Track search
   - Stream URL generation (using preview URLs or librespot)

2. **Jellyfin Provider** (`src/providers/jellyfin.rs`)
   - HTTP client for Jellyfin API
   - Authentication with API key
   - Playlist/track API calls
   - Direct stream URL generation

3. **Playback Engine**
   - Audio sink setup (rodio backend)
   - Stream decoding (symphonia)
   - Playback state synchronization

4. **TUI Implementation**
   - Event loop with crossterm
   - Key bindings and navigation
   - Real-time rendering updates
   - Async command handling

5. **CLI Commands**
   - Playlist listing and searching
   - Track playback
   - Playlist creation and management
   - User authentication flows

## Technology Stack

| Component | Library | Version |
|-----------|---------|---------|
| CLI | `clap` | 4.4 |
| TUI | `ratatui` | 0.26 |
| Terminal | `crossterm` | 0.27 |
| Async Runtime | `tokio` | 1.0 |
| Spotify | `rspotify` | 0.12 |
| Direct Playback | `librespot` | 0.4 |
| Audio | `rodio` | 0.17 |
| Logging | `tracing` | 0.1 |
| Config | `toml` | 0.8 |
| HTTP | `reqwest` | 0.11 |

## API Design

### Provider Interface
All providers expose a consistent interface, allowing:
- Swappable implementations
- Source-agnostic UI code
- Easy testing with mock providers

### Async-First Design
- All I/O operations are async (`async fn`)
- Playback manager uses `Arc<Mutex<T>>` for thread safety
- Tokio runtime for concurrent operations

### Error Handling
- `ProviderError` wraps provider-specific errors
- `anyhow::Result<T>` for fallible operations
- Detailed error messages for debugging

## Key Design Decisions

1. **Provider Registry Pattern**: Central location for all providers enables dynamic addition/removal
2. **Async/Await**: Tokio runtime allows concurrent provider requests and responsive UI
3. **Arc<Mutex<T>>**: Thread-safe, non-blocking access to shared playback state
4. **Trait Objects**: `Arc<dyn MusicProvider>` allows dynamic dispatch while maintaining type safety
5. **Config on Disk**: TOML format readable by humans and parseable by Rust
6. **Ratatui TUI**: Cross-platform terminal rendering with rich widget support

## Directory Structure

```
src/
├── main.rs              # CLI entry point and command handlers
├── lib.rs               # Library exports
├── models/              # Data types
│   └── mod.rs
├── providers/           # Music source implementations
│   ├── mod.rs           # Trait definition and registry
│   ├── spotify.rs       # Spotify provider
│   └── jellyfin.rs      # Jellyfin provider
├── playback/            # Playback management
│   └── mod.rs           # Queue and PlaybackManager
├── config/              # Configuration system
│   └── mod.rs           # Config struct and I/O
└── ui/                  # Terminal user interface
    ├── mod.rs           # UI state management
    ├── pages.rs         # Page implementations
    ├── components.rs    # Reusable components
    └── theme.rs         # Color themes
```

## Usage Examples (Future)

```bash
# Start interactive TUI
any-player

# List playlists from both sources
any-player list --source both

# Search for playlists
any-player search "workout" --source spotify --playlists

# Play a playlist
any-player play <playlist-id> --source spotify

# Create new playlist
any-player create-playlist "My Mix" --source spotify

# Authenticate
any-player auth spotify
any-player auth jellyfin
```

## Roadmap

### MVP (Current)
- [x] Core architecture and traits
- [ ] Spotify provider implementation
- [ ] Jellyfin provider implementation
- [ ] Basic playback functionality
- [ ] TUI with essential controls

### Version 0.2
- [ ] Advanced search and filtering
- [ ] Playlist editing (add/remove/reorder tracks)
- [ ] Playback queue management
- [ ] Shuffle and repeat modes

### Version 1.0
- [ ] User authentication caching
- [ ] Cross-source playlist creation (mix Spotify + Jellyfin)
- [ ] Lyrics display
- [ ] Desktop notifications
- [ ] Daemon mode
- [ ] Image rendering in terminal

### Future
- [ ] Apple Music support
- [ ] YouTube Music support
- [ ] Local file support
- [ ] Batch operations (add multiple tracks)
- [ ] Playlist export/import
