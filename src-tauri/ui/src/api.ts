/**
 * Tauri API wrapper - handles communication with Rust backend
 */

import type { PlaybackStatus, Playlist, Track } from "./types";

declare global {
  interface Window {
    __TAURI__: {
      invoke: (
        command: string,
        args?: Record<string, unknown>
      ) => Promise<unknown>;
      shell?: {
        open: (url: string) => Promise<void>;
      };
    };
  }
}

export class TauriAPI {
  private ready: boolean = false;

  async init(): Promise<void> {
    // Wait for Tauri to be ready
    return new Promise((resolve) => {
      const checkTauri = () => {
        // Check if window.__TAURI__.invoke exists (normalized by tauri.js)
        if (window.__TAURI__ && typeof window.__TAURI__.invoke === "function") {
          this.ready = true;
          console.log("Tauri API initialized");
          resolve();
        } else {
          setTimeout(checkTauri, 50);
        }
      };
      checkTauri();
    });
  }

  private async invoke<T = unknown>(
    command: string,
    args: Record<string, unknown> = {}
  ): Promise<T> {
    if (!this.ready) {
      console.warn("Tauri not ready");
      return null as unknown as T;
    }
    try {
      return (await window.__TAURI__.invoke(command, args)) as T;
    } catch (error) {
      console.error(`Error invoking ${command}:`, error);
      throw error;
    }
  }

  // Playback commands
  async getPlaybackStatus(): Promise<PlaybackStatus> {
    return this.invoke<PlaybackStatus>("get_playback_status");
  }

  async play(): Promise<void> {
    return this.invoke<void>("play");
  }

  async pause(): Promise<void> {
    return this.invoke<void>("pause");
  }

  async togglePlayPause(): Promise<void> {
    return this.invoke<void>("toggle_play_pause");
  }

  async nextTrack(): Promise<void> {
    return this.invoke<void>("next_track");
  }

  async previousTrack(): Promise<void> {
    return this.invoke<void>("previous_track");
  }

  async seek(position: number): Promise<void> {
    return this.invoke<void>("seek", { position });
  }

  async setVolume(volume: number): Promise<void> {
    return this.invoke<void>("set_volume", { volume });
  }

  async toggleShuffle(): Promise<void> {
    return this.invoke<void>("toggle_shuffle");
  }

  async setRepeatMode(mode: "off" | "one" | "all"): Promise<void> {
    return this.invoke<void>("set_repeat_mode", { mode });
  }

  // Playlist commands
  async getPlaylists(source: string): Promise<Playlist[]> {
    return this.invoke<Playlist[]>("get_playlists", { source });
  }

  async queueTrack(trackId: string, source: string): Promise<void> {
    return this.invoke<void>("queue_track", { track_id: trackId, source });
  }

  async clearQueue(): Promise<void> {
    return this.invoke<void>("clear_queue");
  }

  // Spotify commands
  async getSpotifyAuthUrl(): Promise<string> {
    return this.invoke<string>("get_spotify_auth_url");
  }

  async authenticateSpotify(code: string): Promise<void> {
    return this.invoke<void>("authenticate_spotify", { code });
  }

  async isSpotifyAuthenticated(): Promise<boolean> {
    return this.invoke<boolean>("is_spotify_authenticated");
  }

  async getSpotifyPlaylists(): Promise<Playlist[]> {
    return this.invoke<Playlist[]>("get_spotify_playlists");
  }

  async checkOAuthCode(): Promise<boolean> {
    return this.invoke<boolean>("check_oauth_code");
  }

  // Jellyfin commands
  async authenticateJellyfin(url: string, apiKey: string): Promise<void> {
    return this.invoke<void>("authenticate_jellyfin", {
      url,
      api_key: apiKey,
    });
  }

  async isJellyfinAuthenticated(): Promise<boolean> {
    return this.invoke<boolean>("is_jellyfin_authenticated");
  }

  async getJellyfinPlaylists(): Promise<Playlist[]> {
    return this.invoke<Playlist[]>("get_jellyfin_playlists");
  }

  async getJellyfinPlaylist(id: string): Promise<Playlist> {
    return this.invoke<Playlist>("get_jellyfin_playlist", { id });
  }

  async searchJellyfinTracks(query: string): Promise<Track[]> {
    return this.invoke<Track[]>("search_jellyfin_tracks", { query });
  }

  async searchJellyfinPlaylists(query: string): Promise<Playlist[]> {
    return this.invoke<Playlist[]>("search_jellyfin_playlists", { query });
  }

  async getJellyfinRecentlyPlayed(limit: number): Promise<Track[]> {
    return this.invoke<Track[]>("get_jellyfin_recently_played", { limit });
  }
}

// Create and export global instance
export const tauriAPI = new TauriAPI();
