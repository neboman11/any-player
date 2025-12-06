// Tauri API - stub for local development
// This will be replaced by the actual Tauri API when running in the Tauri app
if (!window.__TAURI__) {
    // Create mock Tauri API for development
    window.__TAURI__ = {
        invoke: async (command, args) => {
            console.log(`Mock invoke: ${command}`, args);
            // Return mock data for development
            switch (command) {
                case 'get_playback_status':
                    return {
                        state: 'stopped',
                        current_track: null,
                        position: 0,
                        volume: 100,
                        shuffle: false,
                        repeat_mode: 'off'
                    };
                case 'get_playlists':
                    return [
                        {
                            id: 'playlist1',
                            name: 'My Favorites',
                            description: 'My favorite tracks',
                            track_count: 25,
                            owner: 'Me',
                            source: 'spotify'
                        },
                        {
                            id: 'playlist2',
                            name: 'Chill Vibes',
                            description: 'Relaxing music',
                            track_count: 42,
                            owner: 'Me',
                            source: 'jellyfin'
                        }
                    ];
                default:
                    return null;
            }
        }
    };
} else if (!window.__TAURI__.invoke && window.__TAURI_INVOKE__) {
    // Tauri v2 API structure - invoke is at window.__TAURI_INVOKE__
    window.__TAURI__.invoke = window.__TAURI_INVOKE__;
}

