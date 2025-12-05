# Implementation Checklist: Tauri Desktop UI

This checklist tracks the current implementation status and future work needed to complete the Any Player desktop application.

## ✅ Completed Tasks

### Project Setup
- [x] Workspace configuration in Cargo.toml
- [x] Feature flags for CLI and GUI modes
- [x] Tauri application structure
- [x] Build configuration

### Tauri Backend
- [x] Tauri app initialization
- [x] Command handler system
- [x] IPC bridge implementation
- [x] State management with Arc<Mutex<T>>
- [x] Error handling

### Frontend UI
- [x] HTML structure with 4 pages
- [x] Professional dark theme (Spotify-inspired)
- [x] Responsive layout
- [x] Page navigation system
- [x] UI controller with event handling
- [x] Tauri API wrapper

### Playback Controls
- [x] Play/pause buttons
- [x] Next/previous buttons
- [x] Volume slider
- [x] Seek bar
- [x] Shuffle toggle
- [x] Repeat mode cycle

### Documentation
- [x] TAURI_SETUP.md - Complete setup guide
- [x] TAURI_QUICKSTART.md - Quick start
- [x] TAURI_IMPLEMENTATION.md - Implementation details

## 🚧 In Progress / TODO

### Backend Implementation (High Priority)

#### Playback Manager Completion
- [ ] Implement `clear_queue()` method in PlaybackManager
- [ ] Implement actual audio playback (currently stubbed)
- [ ] Implement seek functionality
- [ ] Implement queue management (next/previous logic)
- [ ] Handle stream URL from providers

#### Provider Integration
- [ ] Complete Spotify provider implementation
  - [ ] OAuth authentication flow
  - [ ] Search implementation
  - [ ] Playlist fetching
  - [ ] Stream URL retrieval
- [ ] Complete Jellyfin provider implementation
  - [ ] Server connection
  - [ ] API key authentication
  - [ ] Browse functionality
  - [ ] Stream URL retrieval

#### Command Completion
- [ ] Implement `get_playlists()` command fully
- [ ] Implement `queue_track()` command fully
- [ ] Add `search_tracks()` command
- [ ] Add `search_playlists()` command
- [ ] Add provider authentication commands
- [ ] Add provider status commands

### Frontend Enhancement (High Priority)

#### Playlists Page
- [ ] Load actual playlists from backend
- [ ] Click to view playlist details
- [ ] Display track list
- [ ] Play/queue tracks from playlist

#### Search Page
- [ ] Connect search input to backend
- [ ] Display search results
- [ ] Differentiate between track and playlist results
- [ ] Click to queue or play results

#### Settings Page
- [ ] Spotify OAuth flow UI
- [ ] Display Spotify connection status
- [ ] Jellyfin connection testing
- [ ] Display provider status
- [ ] Save provider credentials

#### Now Playing Page
- [ ] Display actual current track name/artist
- [ ] Display album artwork
- [ ] Update progress in real-time
- [ ] Display queue contents

#### General UI
- [ ] Add loading indicators
- [ ] Add error messages/toasts
- [ ] Add confirmation dialogs
- [ ] Add context menus (right-click)
- [ ] Dark/light theme toggle

### Features (Medium Priority)

#### Audio Playback
- [ ] Implement actual audio playback using Rodio
- [ ] Handle different audio formats (MP3, FLAC, OGG, etc.)
- [ ] Implement gapless playback (if needed)
- [ ] Audio visualization (optional)

#### System Integration
- [ ] Media key support (play/pause, next, previous)
- [ ] System media controls (Windows/macOS/Linux)
- [ ] Desktop notifications
- [ ] Taskbar integration
- [ ] Tray icon

#### User Experience
- [ ] Keyboard shortcuts (Ctrl+Space = play/pause, etc.)
- [ ] Drag-and-drop to reorder queue
- [ ] Recently played history
- [ ] Favorites/bookmarks
- [ ] Persistent playback state
- [ ] Application remember window size/position

### Testing (Medium Priority)

#### Unit Tests
- [ ] Test Tauri commands
- [ ] Test backend state management
- [ ] Test provider integration

#### Integration Tests
- [ ] Test UI↔Backend communication
- [ ] Test provider authentication flows
- [ ] Test end-to-end playback

#### Manual Testing
- [ ] Test on Windows
- [ ] Test on macOS
- [ ] Test on Linux

### Distribution (Low Priority)

#### Packaging
- [ ] Code signing setup
- [ ] Installer customization
- [ ] Auto-update configuration
- [ ] Release process documentation

#### Marketing
- [ ] Screenshots for each page
- [ ] Feature video/GIF
- [ ] GitHub releases

## Implementation Priority Matrix

### P0 (Critical - Do First)
1. Spotify provider authentication
2. Jellyfin provider connection
3. Get playlists working
4. Actual audio playback

### P1 (Important - Do Next)
1. Search implementation
2. Queue management UI
3. Track details display
4. Provider status in settings

### P2 (Nice to Have)
1. System media keys
2. Keyboard shortcuts
3. Desktop notifications
4. Theme customization

### P3 (Future)
1. Mobile companion app
2. Web interface
3. Library sync
4. Collaborative playlists

## Development Notes

### Known Limitations

1. **Playback**: Currently stubbed - audio actually plays through Rodio backend, but UI doesn't reflect real audio
2. **Search**: Not connected to providers yet
3. **Queue**: Basic structure, needs full implementation
4. **Authentication**: OAuth flows not yet implemented
5. **Jellyfin**: Connection logic not fully implemented

### Architecture Decisions

- **Vanilla JavaScript**: Chosen for simplicity and no build step needed
- **Single Page App**: All UI in one HTML file with page switching
- **Arc<Mutex<T>>**: Used for thread-safe state sharing
- **Tauri v1**: Stable version, v2 available for future upgrade

### Building/Testing Locally

```bash
# Build development
cargo tauri dev

# Build release
cargo tauri build

# Run CLI (original TUI)
cargo build --features cli --bin any-player-cli
./target/debug/any-player-cli tui

# Run tests
cargo test
```

### Common Issues & Solutions

**Issue**: App won't start
**Solution**: 
```bash
cargo clean
cargo tauri dev
```

**Issue**: Frontend not updating
**Solution**: Clear browser cache (DevTools → Network → Disable cache)

**Issue**: Commands timing out
**Solution**: Check if Rust backend thread is blocking (use .await for async operations)

## Testing Checklist

### Before Release
- [ ] App launches without errors
- [ ] All UI buttons responsive
- [ ] Playback controls work
- [ ] Settings page loads
- [ ] Search interface works (even if no results)
- [ ] Provider configuration UI works
- [ ] No console errors
- [ ] App closes cleanly

### Cross-Platform (Before Distribution)
- [ ] Windows: Installer runs
- [ ] Windows: App launches
- [ ] macOS: App launches
- [ ] macOS: Code signing works
- [ ] Linux: .deb installs and runs
- [ ] Linux: AppImage runs

## Future Architecture Improvements

1. **Plugin System**: Allow third-party music sources
2. **Database**: Local SQLite for caching
3. **Sync**: Cloud sync of playlists/favorites
4. **Mobile**: Flutter companion app
5. **API**: REST API for other clients
6. **CLI**: Enhanced CLI with more features

## Resources for Contributors

- **Tauri Docs**: https://tauri.app/docs/
- **Rust Async**: https://tokio.rs/
- **Provider APIs**:
  - Spotify: https://developer.spotify.com/
  - Jellyfin: https://jellyfin.org/docs/
- **Music Formats**: https://hydrogenaudio.org/

---

**Last Updated**: December 5, 2025
**Version**: 1.0
**Status**: Foundation Complete, Implementation In Progress
