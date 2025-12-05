# Implementation Guide for Any Player

This document provides a detailed implementation guide for autonomous AI agents to complete the Any Player application.

## Overview

Any Player is a multi-source music player with Spotify and Jellyfin support. The architecture is trait-based and async-first, allowing easy addition of new providers and concurrent operations.

## Current State

✅ **Completed:**
- Project structure and module organization
- Core trait definitions (`MusicProvider`)
- Data models (Track, Playlist, PlaybackInfo, etc.)
- Playback queue and manager with async support
- Configuration system with TOML persistence
- CLI command structure and argument parsing
- UI component scaffolding with ratatui

❌ **Not Yet Implemented:**
- Provider implementations (Spotify, Jellyfin)
- TUI event loop and rendering
- Actual audio playback
- Stream URL acquisition and playback

## Priority Implementation Tasks

### Task 1: Spotify Provider Implementation

**Location:** `src/providers/spotify.rs`

**Objective:** Implement all methods of `MusicProvider` trait for Spotify

**Dependencies:**
- `rspotify` crate for API access
- OAuth authentication with Spotify
- `librespot` for direct audio playback (or use Web API preview URLs)

**Key Implementation Steps:**

1. **Authentication**
   - Use OAuth Authorization Code flow
   - Store refresh token securely
   - Handle token refresh automatically

2. **Playlist Operations**
   - `get_playlists()`: Fetch user's playlists via `SpotifyBuilder::get_current_user_playlists()`
   - `get_playlist()`: Fetch specific playlist with tracks
   - `create_playlist()`: Create new playlist for authenticated user
   - `add_track_to_playlist()`: Add track to playlist
   - `remove_track_from_playlist()`: Remove track from playlist

3. **Search**
   - `search_tracks()`: Use Spotify search API
   - `search_playlists()`: Search for public playlists
   - Parse results and convert to Track/Playlist models

4. **Streaming**
   - `get_stream_url()`: Provide URL for playback
   - Options:
     - Option A: Use librespot with Spotify credentials for DRM-free playback
     - Option B: Use 30-second preview URLs from Web API (limited)
     - Recommended: Librespot for full-track support

5. **Recent History**
   - `get_recently_played()`: Fetch user's recently played tracks

**Pseudo-code Structure:**

```rust
pub struct SpotifyProvider {
    client: SpotifyBuilder,
    user_id: Option<String>,
    authenticated: bool,
}

impl MusicProvider for SpotifyProvider {
    async fn authenticate(&mut self) -> Result<(), ProviderError> {
        // 1. Start local HTTP server for OAuth redirect
        // 2. Open browser to Spotify auth URL
        // 3. Get authorization code from redirect
        // 4. Exchange code for access token
        // 5. Store token and user info
        self.authenticated = true;
        Ok(())
    }

    async fn get_playlists(&self) -> Result<Vec<Playlist>, ProviderError> {
        // 1. Call SpotifyBuilder API for user playlists
        // 2. For each playlist, fetch full details including tracks
        // 3. Convert to Playlist model
        // 4. Return Vec<Playlist>
    }

    async fn search_tracks(&self, query: &str) -> Result<Vec<Track>, ProviderError> {
        // 1. Call Spotify search API with query
        // 2. Parse results
        // 3. Convert to Track models
        // 4. Return results
    }

    async fn get_stream_url(&self, track_id: &str) -> Result<String, ProviderError> {
        // Option 1: Use librespot
        // - Get track URI from track_id
        // - Return URI for librespot playback
        
        // Option 2: Use preview URL
        // - Fetch track details from API
        // - Return preview_url if available
    }
}
```

### Task 2: Jellyfin Provider Implementation

**Location:** `src/providers/jellyfin.rs`

**Objective:** Implement all methods of `MusicProvider` trait for Jellyfin

**Dependencies:**
- `reqwest` HTTP client for API calls
- `serde_json` for JSON parsing
- Jellyfin API documentation

**Key Implementation Steps:**

1. **Authentication**
   - HTTP POST to `/Users/AuthenticateByName`
   - Validate API key in headers
   - Store access token

2. **Playlist Operations**
   - `get_playlists()`: GET `/Users/{userId}/Items` with `Filters=IsPlaylist`
   - `get_playlist()`: GET `/Users/{userId}/Items/{id}`
   - `create_playlist()`: POST `/Playlists` with playlist data
   - `add_track_to_playlist()`: POST `/Playlists/{playlistId}/Items?ids={trackId}`
   - `remove_track_from_playlist()`: DELETE `/Playlists/{playlistId}/Items?ids={trackId}`

3. **Search**
   - `search_tracks()`: GET `/Items` with `SearchTerm` parameter
   - `search_playlists()`: GET `/Items` filtered by collection type
   - Parse results and convert to models

4. **Streaming**
   - `get_stream_url()`: Format direct stream URL
   - URL pattern: `{base_url}/Audio/{itemId}/universal?api_key={apiKey}`
   - Supports direct HTTP streaming of audio files

5. **Recent History**
   - `get_recently_played()`: GET `/Users/{userId}/Items/Latest` or similar

**Pseudo-code Structure:**

```rust
pub struct JellyfinProvider {
    base_url: String,
    api_key: String,
    user_id: Option<String>,
    client: reqwest::Client,
    authenticated: bool,
}

impl MusicProvider for JellyfinProvider {
    async fn authenticate(&mut self) -> Result<(), ProviderError> {
        // 1. Test connection with GET /System/Info
        // 2. If auth required, use api_key header
        // 3. Fetch current user info
        // 4. Store user_id
        self.authenticated = true;
        Ok(())
    }

    async fn get_playlists(&self) -> Result<Vec<Playlist>, ProviderError> {
        // 1. GET /Users/{userId}/Items?Filters=IsPlaylist
        // 2. Deserialize JSON response
        // 3. For each item, fetch playlist details
        // 4. Convert to Playlist model
        // 5. Return Vec<Playlist>
    }

    async fn get_stream_url(&self, track_id: &str) -> Result<String, ProviderError> {
        // Return direct stream URL:
        // format!("{}/Audio/{}/universal?api_key={}", 
        //         self.base_url, track_id, self.api_key)
    }
}
```

### Task 3: Audio Playback Implementation

**Location:** `src/playback/mod.rs` (extend existing)

**Objective:** Implement actual audio streaming and playback

**Dependencies:**
- `rodio` audio sink
- `reqwest` for HTTP streaming
- Stream handling and error recovery

**Key Components:**

1. **Audio Sink Setup**
   - Initialize default audio device
   - Create rodio Sink for playback
   - Handle device selection/switching

2. **Stream Decoder**
   - Accept stream URL from provider
   - Download/stream audio data
   - Decode format (MP3, FLAC, OGG, etc.)

3. **Playback State Sync**
   - Track current position
   - Monitor playback completion
   - Update `PlaybackInfo` in real-time
   - Handle errors and reconnections

4. **Queue Processing**
   - Auto-play next track when current finishes
   - Handle repeat modes
   - Shuffle implementation

**Integration Points:**
- `PlaybackManager::play()` should start audio stream
- `PlaybackManager::seek()` should update stream position
- Background task to monitor playback and update queue

### Task 4: TUI Implementation

**Location:** `src/ui/` (complete implementation)

**Objective:** Build full interactive terminal interface

**File Structure:**
- `mod.rs`: App state and event handling
- `pages.rs`: Page rendering (already scaffolded)
- `components.rs`: Reusable widgets (already scaffolded)
- `theme.rs`: Color management (already implemented)
- *New file*: `event.rs`: Input handling

**Key Tasks:**

1. **Event Loop** (in `mod.rs`)
   ```rust
   async fn run_event_loop() {
       // 1. Setup terminal (crossterm)
       // 2. Create event receiver (keyboard, mouse)
       // 3. Main loop:
       //    - Receive events
       //    - Update app state
       //    - Render frame
       //    - Handle async provider operations
   }
   ```

2. **Keyboard Handling** (new `event.rs`)
   - Parse keyboard input
   - Map to commands (play, pause, next, search, etc.)
   - Handle modifier keys (Shift, Ctrl, etc.)

3. **Page Navigation**
   - Update `AppPage` on user input
   - Render correct page based on current state
   - Show/hide pages with transitions

4. **Real-time Updates**
   - Display current playback time
   - Update progress bar
   - Show track info
   - Refresh playlist when changes occur

5. **Input Fields**
   - Search input
   - Playlist name creation
   - Authentication prompts

### Task 5: CLI Command Handlers

**Location:** `src/main.rs` (implement command handlers)

**Commands to Complete:**

1. **`list` Command**
   ```rust
   async fn handle_list_command(source: &str) -> Result<(), Box<dyn Error>> {
       // 1. Get provider from registry
       // 2. Call provider.get_playlists()
       // 3. Format and display results
       // 4. Show as JSON or table
   }
   ```

2. **`search` Command**
   ```rust
   async fn handle_search_command(query: &str, source: &str, playlists: bool) {
       // 1. Get provider
       // 2. Call search_tracks() or search_playlists()
       // 3. Display results in table format
   }
   ```

3. **`play` Command**
   ```rust
   async fn handle_play_command(id: &str, source: &str) {
       // 1. Get provider and fetch track/playlist
       // 2. Queue tracks
       // 3. Start playback via PlaybackManager
       // 4. Show progress
   }
   ```

4. **`auth` Command**
   ```rust
   async fn handle_auth_command(provider: &str) {
       // 1. Get provider from registry
       // 2. Call provider.authenticate()
       // 3. Handle OAuth flow with user guidance
       // 4. Save config
   }
   ```

## Testing Strategy

### Unit Tests
- Create provider mocks for testing
- Test trait implementations
- Test config loading/saving
- Test PlaybackQueue logic

### Integration Tests
- Test provider with real API credentials
- Test playback state transitions
- Test CLI commands end-to-end

## Configuration for Providers

### Spotify Config Example
```toml
[spotify]
client_id = "YOUR_CLIENT_ID"
client_secret = "YOUR_CLIENT_SECRET"
redirect_uri = "http://127.0.0.1:8989/login"
enable_streaming = true
```

### Jellyfin Config Example
```toml
[jellyfin]
server_url = "http://192.168.1.100:8096"
api_key = "YOUR_API_KEY"
username = "your_username"
```

## Error Handling Strategy

All errors should propagate through the provider trait as `ProviderError`. Each implementation should:

1. Catch provider-specific errors
2. Convert to human-readable messages
3. Include context (track/playlist name, action, etc.)
4. Allow retry logic where appropriate

Example:
```rust
async fn get_playlists(&self) -> Result<Vec<Playlist>, ProviderError> {
    self.client
        .get_current_user_playlists()
        .await
        .map_err(|e| ProviderError(format!("Failed to fetch playlists: {}", e)))?
        .into()
}
```

## Performance Considerations

1. **Caching**: Store recently fetched playlists/tracks
2. **Pagination**: Implement for large playlist lists
3. **Async Concurrency**: Fetch from multiple providers simultaneously
4. **UI Responsiveness**: Long operations should show progress
5. **Memory**: Stream audio instead of buffering entire tracks

## Dependencies and External Services

### Required API Keys
- **Spotify**: Client ID/Secret from https://developer.spotify.com/
- **Jellyfin**: API key from Jellyfin server admin panel

### External Services
- Spotify Web API: https://api.spotify.com/
- Jellyfin: Self-hosted or private server
- OAuth: Spotify device flow or custom implementation

## Success Criteria for Each Task

**Task 1 (Spotify):** 
- [ ] All trait methods implemented
- [ ] OAuth authentication works
- [ ] Can fetch and display playlists
- [ ] Can search tracks
- [ ] Stream URLs are valid

**Task 2 (Jellyfin):**
- [ ] All trait methods implemented
- [ ] API authentication works
- [ ] Can fetch and display playlists
- [ ] Can search tracks/playlists
- [ ] Direct stream URLs work

**Task 3 (Playback):**
- [ ] Audio plays from stream URLs
- [ ] Play/pause/seek works
- [ ] Progress tracking accurate
- [ ] Queue auto-advances
- [ ] Handles errors gracefully

**Task 4 (TUI):**
- [ ] Terminal renders without errors
- [ ] Navigation works (pages)
- [ ] Keyboard input handled
- [ ] Playlists displayed
- [ ] Real-time playback display

**Task 5 (CLI Commands):**
- [ ] List command works
- [ ] Search command works
- [ ] Play command works
- [ ] Auth command works
- [ ] Status command works

## Next Steps for Agent

1. Start with **Task 1 or 2** (provider implementations) - these are independent
2. Implement comprehensive error handling
3. Add logging throughout
4. Create integration tests
5. Move to playback implementation
6. Finally, implement TUI

Choose whichever provider (Spotify or Jellyfin) seems most straightforward based on API complexity.
