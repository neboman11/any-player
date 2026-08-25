# Any Player

Any Player is a desktop music app built with Tauri 2, React, and Rust. It unifies playback across Spotify, Jellyfin, and Plex while supporting local custom playlists and union playlists.

## What works today

### Providers
- Spotify auth/session flow (including premium/session checks)
- Jellyfin auth and media access
- Plex auth and media access
- Session restore on startup for all providers

### Playback
- Play/pause/toggle, previous/next, seek, volume
- Shuffle and repeat modes (`off`, `one`, `all`)
- Queue management (including jump-to-queue-index)
- Play from provider playlists, custom playlists, and union playlists
- WebSocket-driven playback status updates in the UI

### Library & discovery
- Provider playlists (Spotify/Jellyfin/Plex)
- Search tracks across Spotify/Jellyfin/Plex
- Search playlists for Jellyfin and Plex
- Local custom playlists (create/update/delete, add/remove/reorder tracks)
- Local union playlists (compose from provider/custom playlists)

### Persistence & export
- SQLite-backed local storage for custom/union playlists and table preferences
- Disk cache for playlists/custom playlist tracks/union tracks
- Playback state save/restore
- Export app config and playlist state to JSON

## Current UI layout

Main pages in the desktop app:
- Now Playing
- Playlists
- Search
- Settings

Startup includes background session restoration, service warmup, timeout/retry behavior, and a continue-without-provider path.

## Tech stack

- Frontend: React 18 + TypeScript + Vite
- Desktop shell: Tauri 2
- Backend: Rust (Tokio async runtime)
- Audio: rodio + symphonia for local playback; Spotify Connect for Spotify playback
- Storage: SQLite (`rusqlite`) + keyring-backed credential storage

## Prerequisites

- Node.js 18+
- pnpm
- Rust toolchain (edition 2021 compatible)
- Tauri system prerequisites for your OS: https://tauri.app/start/prerequisites/

## Development

Install dependencies:

```bash
pnpm install
```

Run desktop app in development mode:

```bash
pnpm tauri dev
```

Optional frontend-only dev server:

```bash
pnpm dev
```

Useful checks:

```bash
pnpm lint
cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings
```

## Build

```bash
pnpm tauri build
```

Build artifacts are generated under `src-tauri/target/release/`.

## Repository layout

```text
any-player/
├── src/                         # React frontend
│   ├── App.tsx                  # App shell + startup orchestration
│   ├── api.ts                   # Typed Tauri invoke wrapper
│   ├── providerCatalog.ts       # Provider capability catalog
│   ├── websocket.ts             # Backend websocket client bridge
│   ├── components/              # UI pages and shared widgets
│   ├── hooks/                   # Data/auth/playback hooks
│   └── utils/                   # Retry/timeout/filter helpers
├── src-tauri/
│   ├── src/
│   │   ├── lib.rs               # Tauri app wiring + command registration
│   │   ├── websocket.rs         # WS server + event broadcasting
│   │   ├── commands/            # Tauri command modules
│   │   ├── providers/           # Spotify/Jellyfin/Plex provider impls
│   │   ├── playback/            # Playback manager and Spotify session
│   │   ├── database/            # SQLite schema and queries
│   │   ├── cache/               # On-disk cache helpers
│   │   ├── state/               # Persistent playback state
│   │   ├── models/              # Domain models
│   │   └── config/              # App/provider config loading
│   ├── Cargo.toml
│   └── tauri.conf.json
├── docs/
│   └── android-companion-implementation-spec.md
└── CONTRIBUTING.md
```

## Notes and known limitations

- Generic `get_playlists` command exists but is currently a stub; provider-specific playlist commands are used in practice.
- Spotify playlist search is not currently implemented backend-side.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for coding standards, workflow, and testing guidance.
