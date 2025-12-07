# Spotify Audio Playback Implementation Status

## Current Implementation

### ✅ Completed Features

1. **Spotify OAuth Authentication**
   - PKCE-based OAuth flow for public clients
   - Secure credential handling
   - Token management via rspotify

2. **Playlist & Track Fetching**
   - Fetch user's Spotify playlists
   - Retrieve tracks from playlists
   - Full track metadata (title, artist, album, duration, cover art)

3. **Audio Playback Infrastructure**
   - HTTP audio URL playback via rodio
   - Playback state management (playing, paused, stopped)
   - Real-time position tracking
   - Pause/resume control
   - Progress bar support

4. **Preview URL Support**
   - Automatic fallback to preview URLs for playable tracks
   - 30-second preview clips for supported tracks
   - Proper error handling when preview URLs unavailable

## Current Limitations

### Preview URLs Only
- About 30-second preview clips available for many (but not all) Spotify tracks
- Some tracks have no preview URL available at all
- Full track streaming requires Spotify Premium + librespot

### Example Issue
The "Go-Getters" track in some playlists doesn't have a preview URL:
- Log shows: `WARN Track 'Go-Getters' has no preview URL available`
- Tracks without preview URLs cannot be played without full librespot implementation

## What's Needed for Full Spotify Audio Support

### Option 1: Full Librespot Integration (Recommended for Premium Users)

This requires implementing librespot-based streaming which:

1. **Session Management**
   ```rust
   // Initialize a librespot session with OAuth token
   let session = Session::new(
       SessionConfig::default(),
       oauth_token.access_token
   ).await?;
   ```

2. **Track Streaming**
   ```rust
   // Stream full tracks using librespot
   let player = Player::new(
       PlayerConfig::default(),
       session.clone(),
       audio_backend::find(None)?
   );
   
   player.load(spotify_track_id);
   player.play();
   ```

3. **Audio Pipeline**
   ```
   Spotify Track URI
        ↓
   Librespot Session (authentication)
        ↓
   Encrypted Audio Fetch (from Spotify CDN)
        ↓
   Decryption & Decompression (Ogg Vorbis)
        ↓
   PCM Audio Samples
        ↓
   Rodio Audio Backend
        ↓
   System Audio Output
   ```

### Implementation Steps:

1. **Store OAuth Credentials**
   - After authentication, store the access token securely
   - Make it available to the playback module

2. **Create Librespot Session Manager**
   ```rust
   pub struct LibrespotSessionManager {
       session: Arc<Mutex<Option<Session>>>,
   }
   ```

3. **Implement Track Streaming**
   - Detect when a track has no preview URL
   - Use librespot session to stream full track
   - Handle playback state (play, pause, seek)

4. **Integration Points**
   - Modify `AudioPlayer::play_url()` to detect spotify: URIs
   - Route to librespot for URI playback
   - Maintain compatible interface with existing rodio playback

### Code Changes Required:

**In `src-tauri/src/playback/mod.rs`:**
```rust
// Add librespot support
pub async fn play_spotify_track(
    track_id: &str,
    session: Arc<Session>,
    handle: &PlaybackHandle
) -> Result<(), String> {
    let player = Player::new(
        PlayerConfig::default(),
        session,
        audio_backend::find(None)?
    );
    
    let spotify_id = SpotifyId::from_base62(track_id)?;
    player.load(spotify_id);
    player.play();
    
    // Track progress and state...
}
```

**In `src-tauri/src/providers/spotify.rs`:**
```rust
// Store session in provider
pub struct SpotifyProvider {
    client: Option<AuthCodePkceSpotify>,
    session: Arc<Mutex<Option<Session>>>,  // Add this
    // ...
}

// Provide access to session for playback
pub fn get_session(&self) -> Arc<Mutex<Option<Session>>> {
    self.session.clone()
}
```

**In `src-tauri/src/playback/mod.rs`:**
```rust
pub struct PlaybackManager {
    queue: Arc<Mutex<PlaybackQueue>>,
    info: Arc<Mutex<PlaybackInfo>>,
    audio_player: Arc<AudioPlayer>,
    spotify_session: Arc<Mutex<Option<Session>>>,  // Add this
}

pub async fn play_track(&self, track: Track) {
    // ... existing code ...
    
    // For spotify: URIs, use librespot
    if let Some(url) = &track.url {
        if url.starts_with("spotify:") {
            let session = self.spotify_session.lock().await;
            if let Some(session) = session.as_ref() {
                // Use librespot
                self.audio_player.play_spotify_track(...).await?;
            }
        } else {
            // Use HTTP preview URL
            self.audio_player.play_url(url).await?;
        }
    }
}
```

### Option 2: Web Playback API (Requires Premium + Web Player)

Spotify provides a Web Playback SDK but requires:
- Spotify Premium account
- Premium account validation
- Running in a web context
- Less suitable for desktop app

## Testing & Validation

### To Test Current Preview URL Playback:

1. Select a popular track from a Spotify playlist
2. Most mainstream tracks have 30-second previews
3. Verify:
   - Track appears in Now Playing
   - Progress bar shows 0:00 → 0:30
   - Audio plays through speakers
   - Pause/resume controls work
   - Position updates in real-time

### To Enable Full Spotify Streaming:

1. Implement librespot session management
2. Add the integration code above
3. Test with tracks that have no preview URLs
4. Verify full-track playback (not just 30-second previews)

## Dependencies

Current:
```toml
librespot = { version = "0.4", features = ["rodio-backend"] }
librespot-core = "0.4"
rodio = "0.17"
rspotify = "0.12"
```

For full implementation, may need:
```toml
librespot-playback = "0.4"
librespot-metadata = "0.4"
```

## References

- [Librespot Documentation](https://github.com/librespot-org/librespot)
- [Spotify Auth Scopes](https://developer.spotify.com/documentation/web-api/concepts/scopes)
- [Spotify Web API](https://developer.spotify.com/documentation/web-api)
- [Audio Backend Options](https://github.com/librespot-org/librespot/wiki/Audio-Backends)

## Summary

The application now has a solid foundation for Spotify integration with:
- ✅ OAuth authentication
- ✅ Playlist/track fetching
- ✅ Preview URL playback
- ✅ Proper playback state management

To support full-track streaming for all Spotify songs, implement the librespot integration as outlined above. This is a moderate engineering effort (~500-1000 LOC) but enables unlimited Spotify Premium playback within the app.
