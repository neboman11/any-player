# Any Player

A universal music player desktop application that unifies music streaming from multiple sources (Spotify and Jellyfin) into a single, elegant interface. Built with Tauri 2, React, and Rust for a native, high-performance experience.

## Overview

Any Player provides a seamless music listening experience by allowing users to search, browse, and play music from different streaming services through one unified application. The application features secure authentication, high-quality audio playback, and a modern React-based user interface.

## Getting Started

### Prerequisites

- **Node.js** 18+ and pnpm (or npm/yarn)
- **Rust** 1.70+ (Rust 2021 edition)
- **Operating System**: Linux, macOS, or Windows
- **Spotify Account** (for Spotify integration)
- **Jellyfin Server** (optional, for Jellyfin integration)

### Installation

1. Clone the repository:
```bash
git clone <repository-url>
cd any-player
```

2. Install frontend dependencies:
```bash
pnpm install
```

3. Install Rust dependencies (automatic when building):
```bash
cd src-tauri
cargo build
```

### Development

Run the application in development mode with hot reload:

```bash
pnpm tauri dev
```

This starts:
- Vite development server on port 1420
- HMR (Hot Module Replacement) on port 1421
- Tauri application window

### Building for Production

Build optimized binaries for your platform:

```bash
pnpm tauri build
```

The compiled application will be in `src-tauri/target/release/`.

## Project Structure

```
any-player/
├── src/                          # Frontend source (TypeScript/React)
│   ├── App.tsx                   # Main application component
│   ├── api.ts                    # Tauri API wrapper
│   ├── types.ts                  # TypeScript type definitions
│   ├── components/               # React UI components
│   │   ├── NowPlaying.tsx
│   │   ├── Search.tsx
│   │   ├── Playlists.tsx
│   │   └── ...
│   └── hooks/                    # Custom React hooks
│       ├── useSpotifyAuth.ts
│       ├── usePlayback.ts
│       └── ...
├── src-tauri/                    # Backend source (Rust)
│   ├── src/
│   │   ├── lib.rs                # Application entry point
│   │   ├── main.rs               # Binary entry point
│   │   ├── commands.rs           # Tauri command handlers
│   │   ├── models/               # Data models
│   │   ├── providers/            # Music provider implementations
│   │   ├── playback/             # Audio playback engine
│   │   └── config/               # Configuration management
│   ├── Cargo.toml                # Rust dependencies
│   └── tauri.conf.json           # Tauri configuration
├── public/                       # Static assets
├── package.json                  # Node.js dependencies
├── tsconfig.json                 # TypeScript configuration
├── vite.config.ts                # Vite configuration
└── .github/
    └── copilot/
        └── copilot-instructions.md  # Development guidelines
```

## Key Features

### Multi-Provider Support
- **Spotify Integration** - OAuth 2.0 authentication with full streaming support via librespot
- **Jellyfin Integration** - API key authentication for self-hosted media servers
- Unified interface for browsing and playing music from any source

### Authentication & Security
- OAuth 2.0 flow for Spotify with local callback server
- Secure token storage using OS-native keyring (no plaintext credentials)
- Automatic session restoration on app startup
- Token refresh for expired credentials

### Playback Features
- High-quality audio playback using rodio and symphonia
- Full playback controls (play, pause, stop, skip)
- Volume control
- Progress tracking and seeking
- Shuffle mode
- Repeat modes (Off, One, All)

### Music Discovery
- Search tracks across all connected providers
- Search playlists by name or description
- Browse user playlists from each provider
- View track metadata (title, artist, album, artwork)

### User Interface
- Clean, modern React-based UI
- Real-time playback status updates
- Now playing display with album artwork
- Responsive layout
- Settings panel for provider configuration


## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for detailed guidelines on coding standards, testing, and development workflow
