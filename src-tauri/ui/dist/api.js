// Tauri API wrapper - handles communication with Rust backend
class TauriAPI {
    constructor() {
        this.ready = false;
    }

    async init() {
        // Wait for Tauri to be ready
        return new Promise((resolve) => {
            const checkTauri = () => {
                // Check if window.__TAURI__.invoke exists (normalized by tauri.js)
                if (window.__TAURI__ && typeof window.__TAURI__.invoke === 'function') {
                    this.ready = true;
                    console.log('Tauri API initialized');
                    resolve();
                } else {
                    setTimeout(checkTauri, 50);
                }
            };
            checkTauri();
        });
    }

    async invoke(command, args = {}) {
        if (!this.ready) {
            console.warn('Tauri not ready');
            return null;
        }
        try {
            return await window.__TAURI__.invoke(command, args);
        } catch (error) {
            console.error(`Error invoking ${command}:`, error);
            throw error;
        }
    }

    // Playback commands
    async getPlaybackStatus() {
        return this.invoke('get_playback_status');
    }

    async play() {
        return this.invoke('play');
    }

    async pause() {
        return this.invoke('pause');
    }

    async togglePlayPause() {
        return this.invoke('toggle_play_pause');
    }

    async nextTrack() {
        return this.invoke('next_track');
    }

    async previousTrack() {
        return this.invoke('previous_track');
    }

    async seek(position) {
        return this.invoke('seek', { position });
    }

    async setVolume(volume) {
        return this.invoke('set_volume', { volume });
    }

    async toggleShuffle() {
        return this.invoke('toggle_shuffle');
    }

    async setRepeatMode(mode) {
        return this.invoke('set_repeat_mode', { mode });
    }

    // Playlist commands
    async getPlaylists(source) {
        return this.invoke('get_playlists', { source });
    }

    async queueTrack(trackId, source) {
        return this.invoke('queue_track', { track_id: trackId, source });
    }

    async clearQueue() {
        return this.invoke('clear_queue');
    }

    // Spotify commands
    async getSpotifyAuthUrl() {
        return this.invoke('get_spotify_auth_url');
    }

    async authenticateSpotify(code) {
        return this.invoke('authenticate_spotify', { code });
    }

    async isSpotifyAuthenticated() {
        return this.invoke('is_spotify_authenticated');
    }

    async getSpotifyPlaylists() {
        return this.invoke('get_spotify_playlists');
    }

    async checkOAuthCode() {
        return this.invoke('check_oauth_code');
    }
}

// Create global instance
const tauriAPI = new TauriAPI();
tauriAPI.init();
