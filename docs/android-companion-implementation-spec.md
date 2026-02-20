# Any Player Android Companion App — Implementation Specification

## 0) Goal
Build a **new Android app** that reaches feature parity with the current desktop app and adds robust **state import/export** between desktop and Android.

This document is written for an AI coding agent to execute directly.

---

## 1) Multi-Pass Analysis Summary (Completed)

### Pass 1 — Product/UX capabilities (frontend)
Reviewed app flows in React/Tauri frontend (`App.tsx`, hooks, components):
- Pages: **Now Playing**, **Playlists**, **Search**, **Settings**.
- Playback controls: play/pause, next/previous, seek, volume, shuffle, repeat, queue jump.
- Providers: Spotify, Jellyfin, Plex auth and status.
- Playlist workflows: provider playlists + local custom playlists + union playlists.
- Track table preferences (column visibility/order).
- Startup orchestration with retry/cancel and status banners.
- Realtime updates via WebSocket events.

### Pass 2 — Backend command/state capabilities (Rust)
Reviewed Tauri command registry + command modules + persistence:
- Auth/session restore commands for Spotify/Jellyfin/Plex.
- Provider playlist/search/fetch commands.
- Playback engine commands and queue behavior.
- Custom/union playlist storage in SQLite.
- Disk cache files for playlist materialization.
- Persistent playback state save/restore with sensitive field sanitization.
- OAuth callback server for Spotify.

### Pass 3 — Parity/edge-case audit
Cross-checked frontend API wrappers vs backend command registration; identified practical constraints:
- `get_playlists` exists but is currently stubbed backend-side; app mostly uses provider-specific calls.
- Playback state commands exist and are used mainly by backend startup/shutdown lifecycle.
- Legacy `src/ui.ts` is not current app entry path (entrypoint is `src/main.tsx` → `App.tsx`).
- WebSocket event names and startup statuses are part of functional UX and should be mirrored in Android state model.

---

## 2) Required Feature Parity (Android must implement all)

## 2.1 Provider Authentication
Implement full connect/disconnect/status for:
1. **Spotify**
   - OAuth flow in external browser/Custom Tabs.
   - Premium status check.
   - Session readiness concept (for direct playback capability).
2. **Jellyfin**
   - URL + API key auth.
   - Saved credentials restore.
3. **Plex**
   - URL + token auth.
   - Saved credentials restore.

Behavior requirements:
- Persist credentials securely (Android Keystore encrypted storage).
- Restore sessions on app startup.
- Expose per-provider status in UI and in app state.

## 2.2 Playback
Implement:
- play, pause, toggle play/pause
- next, previous
- seek to position (ms)
- set volume (0–100)
- shuffle toggle
- repeat modes: off / one / all
- queue display and skip-to-queue-index
- progress + duration display
- now-playing metadata and artwork

Behavior requirements:
- Queue behavior must support “play from selected row” semantics used in desktop.
- Shuffle + repeat interactions must match desktop logic.
- Background playback + notification controls required.

## 2.3 Search
Implement search for:
- Track search (Spotify/Jellyfin/Plex)
- Playlist search (Jellyfin/Plex; Spotify playlist search is currently not implemented in desktop backend)
- Source filter: all / spotify / jellyfin / plex
- Type filter: tracks / playlists
- Paginated rendering in UI (or lazy list with equivalent UX clarity)

## 2.4 Playlists
Implement all playlist categories:
1. Provider playlists (Spotify/Jellyfin/Plex)
2. Local custom playlists (standard)
3. Local union playlists (composition of source playlists)

Custom playlist features:
- create/update/delete
- add/remove/reorder tracks
- play entire playlist
- play from specific track

Union playlist features:
- create/update/delete
- add/remove/reorder source playlists
- materialize union track list
- play entire union playlist
- play from specific track

## 2.5 Settings
Implement:
- Provider connection forms/status/actions
- Track table column preferences equivalent (at least same visible options)
- Playback setting parity where meaningful

## 2.6 Startup/Resilience UX
Implement equivalent startup behavior:
- Session restoration + initial data warmup
- Non-blocking startup with progress messages
- Timeout/retry handling
- Continue-without-provider fallback

## 2.7 Persistence & Caching
Implement:
- Local DB for custom + union playlists
- Playlist/materialized cache strategy for faster startup
- Playback state persistence and restore
- Sensitive field sanitization in exported persisted state where required

---

## 3) Android Technical Architecture (Required)

Use these defaults unless blocked:

- Language: **Kotlin**
- UI: **Jetpack Compose**
- Architecture: **Clean-ish modular MVVM + Repository + UseCase + DataSource**
- DI: **Hilt**
- Async: **Coroutines + Flow**
- Local DB: **Room**
- Secure storage: **EncryptedSharedPreferences + Android Keystore**
- Networking: **Ktor or Retrofit/OkHttp** (choose one and keep consistent)
- Playback: **Media3 (ExoPlayer + MediaSessionService)**
- Serialization: **kotlinx.serialization**

Recommended module layout:
- `app` (UI shell/navigation)
- `core-model`
- `core-network`
- `core-storage`
- `feature-auth`
- `feature-playback`
- `feature-search`
- `feature-playlists`
- `feature-settings`
- `feature-state-transfer` (import/export)

---

## 4) Data Model Contract for Android

Create Kotlin models aligned with desktop types:
- `Track`:
  - `id`, `title`, `artist`, `album?`, `durationMs?`, `source`, `url?`, `imageUrl?`, `bitrateKbps?`, `sampleRateHz?`, `enriched?`
- `Playlist`:
  - `id`, `name`, `owner`, `trackCount`, `source`, `imageUrl?`, `tracks?`, `description?`
- `CustomPlaylist`:
  - `id`, `name`, `description?`, `imageUrl?`, `createdAt`, `updatedAt`, `trackCount`, `playlistType` (`standard|union`)
- `UnionPlaylistSource`:
  - `id`, `unionPlaylistId`, `sourceType`, `sourcePlaylistId`, `position`, `addedAt`
- `PlaylistTrack`:
  - `id`, `playlistId`, `trackSource`, `trackId`, `position`, `addedAt`, `title`, `artist`, `album?`, `durationMs?`, `imageUrl?`, `url?`
- `PlaybackStatus`:
  - `state`, `shuffle`, `repeatMode`, `volume`, `currentTrack?`, `position`, `duration`, `queue`

Use a shared enum for source values: `spotify | jellyfin | plex | custom | all`.

---

## 5) Provider Integration Requirements

## 5.1 Spotify
- OAuth authorization code flow.
- Store and refresh token.
- Query premium status.
- Playback capability:
  - If direct stream not viable on Android for Spotify API constraints, define fallback behavior clearly (e.g., app-control only vs playable preview).
  - Must not silently fail; expose status to UI.

## 5.2 Jellyfin
- Auth with URL/API key.
- Playlist and track search.
- Playlist track retrieval and stream URL handling.

## 5.3 Plex
- Auth with URL/token.
- Playlist and track search.
- Playlist track retrieval and stream URL handling.

---

## 6) Local Database Schema (Android)

Create Room tables equivalent to desktop database intent:
- `custom_playlists`
- `playlist_tracks`
- `union_playlist_sources`
- `column_preferences`
- optional: `app_cache_entries` for structured cache metadata/versioning

Constraints:
- Stable ordering fields (`position`) for reorder operations.
- Cascade delete for playlist-related rows.
- Migration strategy with schema versioning from day one.

---

## 7) State Import/Export (New Capability)

This is mandatory and cross-platform. Implement in Android **and** define format compatible with desktop implementation.

## 7.1 Export scope
Include:
- Custom playlists (standard + union)
- Playlist tracks
- Union source definitions
- Column preferences
- Provider connection profiles (non-ephemeral):
  - Jellyfin URL (+ API key only in encrypted exports)
  - Plex URL (+ token only in encrypted exports)
  - Spotify connection metadata (never raw access token in plain export)
- Optional playback state snapshot (queue/current track/position settings)

Exclude by default:
- Raw provider access tokens in plain-text export
- Disk cache blobs (regenerable)

## 7.2 File format
Define canonical JSON envelope:

```json
{
  "format": "any-player-state",
  "version": 1,
  "createdAt": "2026-02-16T00:00:00Z",
  "sourceApp": {
    "platform": "android|desktop",
    "appVersion": "x.y.z"
  },
  "data": {
    "customPlaylists": [],
    "playlistTracks": [],
    "unionPlaylistSources": [],
    "columnPreferences": {},
    "connections": {},
    "playbackState": {}
  },
  "integrity": {
    "sha256": "..."
  }
}
```

## 7.3 Encryption mode
Support two export modes:
1. **Portable (plain JSON)** — no secrets, safe for sharing.
2. **Private (encrypted)** — includes credentials/tokens where applicable.

Encrypted mode requirements:
- AES-256-GCM
- Key from passphrase using PBKDF2-HMAC-SHA256 or Argon2id
- Store KDF params + nonce in file header
- Validate integrity before import

## 7.4 Import merge behavior
On import, present merge policy:
- `replace_all`
- `merge_keep_local`
- `merge_prefer_import`

ID collision rules:
- Preserve IDs if unique.
- On collision with different payloads, create deterministic remap and rewrite references (`playlistId`, `unionPlaylistId`, source links).

Validation rules:
- Reject unknown major `version`.
- Reject malformed references (e.g., union source points to missing playlist and missing provider id).
- Import transaction must be atomic (rollback on failure).

---

## 8) UX Requirements for Import/Export

Add in Settings:
- `Export State`
  - Choose mode: Portable / Private
  - File destination picker
  - Optional include playback state toggle
- `Import State`
  - File picker
  - Detect format/version/encryption
  - Prompt for passphrase if encrypted
  - Merge policy selector
  - Dry-run summary before applying

Post-import summary must show:
- playlists added/updated
- tracks added/updated
- union links added/updated
- connections imported/skipped
- errors/warnings

---

## 9) Implementation Plan for AI Agent

Execute in order:

1. **Scaffold project**
   - Create Android app with Compose, Hilt, Room, Media3.
   - Add CI build + lint + unit test workflow.

2. **Core models + storage**
   - Implement canonical models and Room entities/DAOs.
   - Add migration test baseline.

3. **Provider clients + auth**
   - Add Spotify/Jellyfin/Plex clients.
   - Implement secure credential storage and restore.

4. **Playback engine**
   - Build queue/state manager + Media3 service.
   - Wire controls and notifications.

5. **Feature screens**
   - Now Playing, Playlists, Search, Settings.
   - Implement all parity actions.

6. **Custom/union playlist engine**
   - Add all CRUD and ordering flows.
   - Add union materialization and play-from-index.

7. **Import/export subsystem**
   - Implement format v1 + encryption mode + merge policies.
   - Add dry-run validation and atomic import transaction.

8. **Hardening**
   - Startup resilience (timeouts/retries/fallback UX).
   - Cache + restore state.
   - Error telemetry/logging.

9. **Parity verification**
   - Run acceptance matrix in section 10.

---

## 10) Acceptance Matrix (must pass)

## 10.1 Auth
- Connect/disconnect/status works for Spotify/Jellyfin/Plex.
- Sessions restore after app restart.

## 10.2 Playback
- All controls behave correctly.
- Queue reflects ordering and supports jump.
- Shuffle/repeat semantics validated.

## 10.3 Search
- Source/type filters work.
- Track playback from result rows works.
- Add-to-custom-playlist works.

## 10.4 Playlists
- Standard custom playlist full CRUD + reorder + play-from-track.
- Union playlist full CRUD + source management + combined playback.

## 10.5 State transfer
- Export portable and private files succeed.
- Import supports replace/merge modes.
- Round-trip Android→Desktop→Android preserves structure and references.
- Invalid file/version/corrupt ciphertext handled safely.

## 10.6 Persistence
- Playback state resumes after restart.
- Column preferences persist.
- Cached data reload improves startup without stale corruption.

---

## 11) Known Desktop Constraints to Account For

- Desktop backend `get_playlists` is currently placeholder/stub; rely on provider-specific playlist fetches for parity behavior.
- Spotify playlist search may not be fully implemented backend-side; Android should gracefully degrade if API access limits apply.
- Legacy frontend file(s) not in active path should not be treated as source-of-truth behavior.

---

## 12) Definition of Done

Project is done when:
1. Android app has practical feature parity for all core workflows in this document.
2. State import/export works reliably between Android and desktop with versioned schema and conflict handling.
3. Tests cover core logic (playlist transforms, import validation, merge/remap, playback queue semantics).
4. Build/lint/test pass in CI.


## 13) Direct Instructions for Coding Agent

- Implement exactly the feature set above; do not defer import/export.
- Prefer simple, maintainable architecture over over-engineering.
- Add tests whenever new business logic is introduced (especially import/export and merge logic).
- Keep provider integrations isolated behind repository interfaces.
- Document any unavoidable platform/API limitations in `docs/android-known-limitations.md`.
- If a feature cannot be fully matched due to provider restrictions, implement the nearest explicit fallback and surface it in UI.
