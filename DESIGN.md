# Any Player - Design Summary for Implementation

## Executive Summary

**Any Player** is a Rust-based, multi-source music streaming client with a plugin architecture supporting Spotify and Jellyfin. The design prioritizes extensibility, async-first operations, and a polished terminal user interface.

**Current Status:** MVP architecture complete and compiling ✅

## Architectural Overview

```
┌─────────────────────────────────────────────────────────┐
│                  CLI / TUI Layer                         │
│  (ratatui framework with crossterm backend)             │
│  - Pages: Home, Search, Playlists, NowPlaying, Queue   │
│  - Components: Playback bar, Track info, Controls      │
│  - Themes: Dark, Light, Spotify-themed                 │
└──────────────────┬──────────────────────────────────────┘
                   │ Async channels & events
┌──────────────────▼──────────────────────────────────────┐
│                Playback Manager                          │
│  (Tokio async + Arc<Mutex<>> for thread safety)        │
│  - PlaybackQueue: Track management                     │
│  - PlaybackInfo: State tracking (play, volume, etc.)   │
│  - Commands: play(), pause(), next(), seek()           │
└──────────────────┬──────────────────────────────────────┘
                   │ Provider abstraction
┌──────────────────▼──────────────────────────────────────┐
│              Provider Registry                           │
│  (Dynamic dispatch with Arc<dyn MusicProvider>)         │
│  ┌─────────────────┬─────────────────────────────────┐ │
│  │ Spotify         │ Jellyfin                        │ │
│  │ Provider        │ Provider                        │ │
│  │ (rspotify,      │ (reqwest HTTP                   │ │
│  │ librespot)      │ client)                         │ │
│  └─────────────────┴─────────────────────────────────┘ │
└──────────────────┬──────────────────────────────────────┘
                   │ HTTP & Auth
┌──────────────────▼──────────────────────────────────────┐
│           External Services                              │
│  - Spotify Web API / OAuth                              │
│  - Spotify Direct Audio (via librespot)                 │
│  - Jellyfin HTTP API                                    │
│  - Audio Streaming Services (HTTP/HTTP+)                │
└─────────────────────────────────────────────────────────┘
```

## Core Components

### 1. Models Layer (`src/models/mod.rs`)

**Purpose:** Define domain entities that all components use

**Key Types:**
```rust
pub enum Source { Spotify, Jellyfin }
pub struct Track { id, title, artist, album, duration_ms, source, url, ... }
pub struct Playlist { id, name, description, owner, tracks, source, ... }
pub enum PlaybackState { Playing, Paused, Stopped }
pub enum RepeatMode { Off, One, All }
pub struct PlaybackInfo { current_track, state, position_ms, shuffle, repeat_mode, volume }
```

**Design Notes:**
- All types derive `Serialize`/`Deserialize` for config persistence
- `Source` enum ensures type safety across providers
- URL fields allow flexible streaming mechanisms

### 2. Provider Trait System (`src/providers/mod.rs`)

**Core Trait:**
```rust
pub trait MusicProvider: Send + Sync {
    fn source(&self) -> Source;
    async fn authenticate(&mut self) -> Result<(), ProviderError>;
    fn is_authenticated(&self) -> bool;
    async fn get_playlists(&self) -> Result<Vec<Playlist>, ProviderError>;
    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>, ProviderError>;
    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>, ProviderError>;
    async fn get_stream_url(&self, track_id: &str) -> Result<String, ProviderError>;
    async fn create_playlist(&self, name: &str, ...) -> Result<Playlist, ProviderError>;
    async fn add_track_to_playlist(&self, playlist_id: &str, track: &Track) -> Result<(), ProviderError>;
    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<(), ProviderError>;
    async fn get_recently_played(&self, limit: usize) -> Result<Vec<Track>, ProviderError>;
}
```

**ProviderRegistry:**
- Maps `Source` to `Arc<dyn MusicProvider>`
- Enables multiple providers simultaneously
- Thread-safe access pattern

**Design Decisions:**
- All I/O is `async` for non-blocking operations
- Error type is `ProviderError` for abstraction
- `Arc<dyn MusicProvider>` enables polymorphism
- `Send + Sync` ensures thread safety

### 3. Implementations (`src/providers/{spotify,jellyfin}.rs`)

**Spotify Provider:**
- Uses `rspotify` crate for API access
- OAuth authentication with device flow or auth code flow
- Streaming via:
  - **Option A:** librespot (full-track playback)
  - **Option B:** Preview URLs (30-second clips)
- Playlist management through Spotify Web API

**Jellyfin Provider:**
- HTTP client (`reqwest`) for API calls
- API key authentication
- Direct HTTP streaming of audio files
- Playlist management through Jellyfin API

### 4. Playback System (`src/playback/mod.rs`)

**PlaybackQueue:**
- Ordered list of tracks
- Current position tracking
- Navigation methods: `next()`, `previous()`
- Queue operations: `add_track()`, `add_tracks()`, `clear()`

**PlaybackManager:**
- Thread-safe state via `Arc<Mutex<T>>`
- Core operations:
  - `play()`, `pause()`, `toggle_play_pause()`
  - `next_track()`, `previous_track()`
  - `seek(position_ms)`, `set_volume(0-100)`
  - `toggle_shuffle()`, `set_repeat_mode()`
- Integrates with audio backend (rodio/symphonia)

**Design Pattern:**
```rust
pub struct PlaybackManager {
    queue: Arc<Mutex<PlaybackQueue>>,
    info: Arc<Mutex<PlaybackInfo>>,
    // audio_sink: Arc<rodio::Sink>, // TODO
}
```

### 5. Configuration (`src/config/mod.rs`)

**Structure:**
```rust
pub struct Config {
    general: GeneralConfig,           // Logging, UI, theme
    spotify: Option<SpotifyConfig>,   // OAuth credentials
    jellyfin: Option<JellyfinConfig>, // Server URL, API key
}
```

**Storage:**
- Location: `~/.config/any-player/config.toml`
- TOML format for human readability
- Auto-creates with defaults on first run
- Methods: `Config::load()`, `Config::save()`

**Features:**
- Per-source configuration (disable sources)
- Logging level control
- Theme selection
- Optional custom data directory

### 6. UI Framework (`src/ui/`)

**Architecture:**
```
src/ui/
├── mod.rs          # AppState, AppPage enum, event handling
├── pages.rs        # Page implementations (Home, Search, NowPlaying, etc.)
├── components.rs   # Reusable widgets (PlaybackBar, TrackInfo, SourceSelector)
└── theme.rs        # Color themes and styling
```

**Pages (to be implemented):**
1. **HomePage**: Source selection, quick navigation
2. **SearchPage**: Search input and results display
3. **PlaylistPage**: Browse playlist tracks
4. **NowPlayingPage**: Current track with progress and controls
5. **QueuePage**: Upcoming tracks

**Components:**
- `PlaybackBar`: Progress indicator with time display
- `TrackInfo`: Display current track metadata
- `PlaybackControls`: Play/pause/shuffle/repeat controls
- `SourceSelector`: Select active provider(s)

**Themes:**
- `default_dark()`: Dark background, bright text
- `default_light()`: Light background, dark text
- `spotify()`: Spotify green signature color

### 7. CLI Layer (`src/main.rs`)

**Command Structure:**
```
any-player [OPTIONS] [COMMAND]

Commands:
  tui                          # Start interactive TUI
  list --source <SOURCE>       # List playlists
  search <QUERY> --source <S>  # Search playlists/tracks
  play <ID> --source <SOURCE>  # Play playlist/track
  create-playlist <NAME>       # Create new playlist
  add-track <PL-ID> <TR-ID>   # Add track to playlist
  auth <PROVIDER>              # Authenticate with provider
  status                       # Show playback status
```

**Handlers (to be completed):**
- Parse arguments with `clap` derive macro
- Route to appropriate command handler
- Integrate with providers and playback manager
- Display results (JSON, tables, etc.)

## Data Flow Examples

### Example 1: User Searches for a Playlist

```
User Input: "any-player search 'workout' --source both"
    ↓
CLI Parser (clap) → Command::Search { query, source, playlists }
    ↓
handle_search_command(query, source, playlists)
    ↓
Get providers from registry (Spotify + Jellyfin)
    ↓
Call provider.search_playlists(query) [async, concurrent]
    ↓
Await both futures
    ↓
Combine results from both sources
    ↓
Format and display as table/JSON
    ↓
User selects playlist
```

### Example 2: User Plays a Playlist

```
User Input: "any-player play <playlist-id> --source spotify"
    ↓
CLI Parser → Command::Play { id, source }
    ↓
Get Spotify provider
    ↓
provider.get_playlist(id) → Playlist with tracks[]
    ↓
Add all tracks to PlaybackManager.queue_tracks()
    ↓
Call playback_manager.play()
    ↓
Audio backend starts streaming first track URL
    ↓
UI shows "Now Playing" page
    ↓
Background task monitors playback, auto-advances on completion
```

### Example 3: Cross-Source Playlist Creation

```
TUI: User creates playlist "My Mix"
    ↓
Get active source (e.g., Spotify)
    ↓
provider.create_playlist("My Mix", None)
    ↓
Playlist created, show ID
    ↓
User searches and adds Spotify tracks
    ↓
Search different source (Jellyfin)
    ↓
Convert Jellyfin track to generic Track model
    ↓
Call spotify_provider.add_track_to_playlist() with converted track
    ↓
Handle metadata mismatch (Jellyfin track added to Spotify)
```

## Thread Safety & Async Design

**Key Principles:**

1. **Arc<Mutex<T>> Pattern:**
   - Playback state shared across threads via `Arc`
   - Protected by `Mutex` for safe mutation
   - `tokio::sync::Mutex` for async-aware locking

2. **Async/Await:**
   - All I/O operations are async
   - Tokio runtime handles scheduling
   - No blocking calls in async functions

3. **Provider Trait Bounds:**
   - `Send + Sync` ensures thread-safe implementations
   - Allows safe `Arc<dyn MusicProvider>` usage

4. **Event Loop:**
   - TUI event loop runs on main thread
   - Spawns async tasks for provider operations
   - Results communicated via channels

## Error Handling Strategy

**Error Type Hierarchy:**
```
ProviderError
├── Authentication failed: "{reason}"
├── Playlist not found: "{id}"
├── Track not found: "{id}"
├── Stream unavailable: "{id}"
├── API error: "{response}"
└── Network error: "{details}"
```

**Handling Patterns:**
1. Provider-specific errors caught and converted to `ProviderError`
2. Propagated through trait as `Result<T, ProviderError>`
3. CLI/TUI formats error for user display
4. Logging includes context (action, resource, error details)

## Performance Considerations

1. **Caching:**
   - Cache recently fetched playlists (5-minute TTL)
   - Store search results during session
   - Avoid duplicate API calls

2. **Pagination:**
   - Load playlists in pages (20-50 items)
   - Lazy-load tracks on playlist selection
   - Show "Loading..." during fetch

3. **Concurrency:**
   - Fetch from both providers simultaneously
   - Merge results client-side
   - Parallel search across sources

4. **Memory:**
   - Stream audio instead of buffering entire tracks
   - Limit queue size (prevent memory explosion)
   - Clear cache periodically

## Integration Points

### Spotify Integration
- **Library:** rspotify (Web API) + librespot (direct playback)
- **Auth:** OAuth with redirect to http://127.0.0.1:8989/login
- **Data:** Track URIs like "spotify:track:123abc"
- **Streaming:** Via librespot or preview URLs

### Jellyfin Integration
- **Library:** reqwest HTTP client
- **Auth:** API key in header: `X-MediaBrowser-Token`
- **Data:** Internal IDs from Jellyfin database
- **Streaming:** Direct HTTP URL with format `/Audio/{id}/universal`

### Audio Playback
- **Backend:** rodio with selectable audio sink
- **Format Support:** MP3, FLAC, OGG via symphonia
- **Stream Source:** HTTP URL from provider

## Testing Strategy

### Unit Tests (Per Component)
- Playback queue logic
- Config serialization/deserialization
- Provider error handling
- Model conversions

### Integration Tests
- Mock provider with fake data
- End-to-end CLI command flow
- Async operation correctness
- Config persistence

### Manual Testing Checklist
- [ ] Spotify authentication flow
- [ ] Jellyfin server connection
- [ ] Playlist display (both sources)
- [ ] Track search
- [ ] Playlist creation
- [ ] Cross-source playlist management
- [ ] Playback start/stop/seek
- [ ] Volume control
- [ ] TUI navigation
- [ ] Error recovery

## Known Limitations & Future Enhancements

### MVP Limitations
- Single-user (no multi-user authentication)
- No offline caching
- No lyrics support
- No desktop notifications
- No advanced filtering

### Planned Features (v0.2+)
- User authentication caching
- Offline playlist support
- Synced lyrics display
- Desktop notifications (notify-rust)
- Advanced search filters
- Batch track operations
- Daemon mode support
- Cross-source playlist export

### Platform-Specific
- **Audio:** Platform audio devices via rodio
- **Authentication:** Browser-based for OAuth
- **UI:** Works on any terminal (tested on Kitty, iTerm2, GNOME Terminal)

## Success Metrics

✅ **MVP Release:**
- Both providers fully functional
- All CLI commands working
- TUI with basic navigation
- Playback working for both sources
- Configuration system working
- Error handling graceful

⏰ **Future Releases:**
- Cross-source playlist creation
- Advanced search and filtering
- Daemon mode
- Image rendering
- Lyrics display

## Development Roadmap

**Phase 1 (Current):** Architecture & Scaffolding ✅

**Phase 2 (Next):** Provider Implementations
- Spotify provider (full implementation)
- Jellyfin provider (full implementation)
- Integration testing

**Phase 3:** Playback Engine
- Audio streaming and decoding
- Playback state synchronization
- Queue management

**Phase 4:** TUI Completion
- Event loop and rendering
- All page implementations
- Keyboard controls
- Real-time updates

**Phase 5:** CLI Commands
- Full command implementation
- JSON output support
- Scripting support

## Quick Reference: Key Files

| File | Purpose |
|------|---------|
| `src/lib.rs` | Library root, exports |
| `src/main.rs` | CLI entry point, command handlers |
| `src/models/mod.rs` | Data types (Track, Playlist, etc.) |
| `src/providers/mod.rs` | MusicProvider trait, registry |
| `src/providers/spotify.rs` | Spotify implementation |
| `src/providers/jellyfin.rs` | Jellyfin implementation |
| `src/playback/mod.rs` | PlaybackManager, queue |
| `src/config/mod.rs` | Config struct, file I/O |
| `src/ui/mod.rs` | UI state, event handling |
| `src/ui/pages.rs` | Page implementations |
| `src/ui/components.rs` | Reusable UI widgets |
| `src/ui/theme.rs` | Color themes |
| `Cargo.toml` | Dependencies and metadata |
| `ARCHITECTURE.md` | Detailed architecture doc |
| `IMPLEMENTATION.md` | Implementation guide for agents |

## Dependencies Summary

| Crate | Version | Purpose |
|-------|---------|---------|
| `clap` | 4.4 | CLI argument parsing |
| `ratatui` | 0.26 | Terminal UI framework |
| `crossterm` | 0.27 | Terminal backend |
| `tokio` | 1.0 | Async runtime |
| `rspotify` | 0.12 | Spotify Web API |
| `librespot` | 0.4 | Spotify audio streaming |
| `reqwest` | 0.11 | HTTP client |
| `serde` | 1.0 | Serialization |
| `toml` | 0.8 | Config format |
| `tracing` | 0.1 | Logging |
| `rodio` | 0.17 | Audio playback |
| `async-trait` | 0.1 | Async trait support |

---

**This design is ready for implementation by autonomous AI agents. See IMPLEMENTATION.md for detailed task breakdowns.**
