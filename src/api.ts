/**
 * Tauri API wrapper - handles communication with Rust backend
 */

import { invoke } from "@tauri-apps/api/core";
import type { PlaybackStatus, Playlist, Track } from "./types";

declare global {
  interface Window {
    __TAURI__: {
      invoke: (
        command: string,
        args?: Record<string, unknown>,
      ) => Promise<unknown>;
      shell?: {
        open: (url: string) => Promise<void>;
      };
    };
  }
}

export class TauriAPI {
  // Playback commands
  async getPlaybackStatus(): Promise<PlaybackStatus> {
    return invoke<PlaybackStatus>("get_playback_status");
  }

  async play(): Promise<void> {
    return invoke<void>("play");
  }

  async pause(): Promise<void> {
    return invoke<void>("pause");
  }

  async togglePlayPause(): Promise<void> {
    return invoke<void>("toggle_play_pause");
  }

  async nextTrack(): Promise<void> {
    return invoke<void>("next_track");
  }

  async previousTrack(): Promise<void> {
    return invoke<void>("previous_track");
  }

  async seek(position: number): Promise<void> {
    return invoke<void>("seek", { position });
  }

  async setVolume(volume: number): Promise<void> {
    return invoke<void>("set_volume", { volume });
  }

  async toggleShuffle(): Promise<void> {
    return invoke<void>("toggle_shuffle");
  }

  async setRepeatMode(mode: "off" | "one" | "all"): Promise<void> {
    return invoke<void>("set_repeat_mode", { mode });
  }

  // Playlist commands
  async getPlaylists(source: string): Promise<Playlist[]> {
    return invoke<Playlist[]>("get_playlists", { source });
  }

  async playTrack(trackId: string, source: string): Promise<void> {
    return invoke<void>("play_track", { trackId, source });
  }

  async queueTrack(trackId: string, source: string): Promise<void> {
    return invoke<void>("queue_track", { trackId, source });
  }

  async clearQueue(): Promise<void> {
    return invoke<void>("clear_queue");
  }

  // Spotify commands
  async getSpotifyAuthUrl(): Promise<string> {
    return invoke<string>("get_spotify_auth_url");
  }

  async authenticateSpotify(code: string): Promise<void> {
    return invoke<void>("authenticate_spotify", { code });
  }

  async isSpotifyAuthenticated(): Promise<boolean> {
    return invoke<boolean>("is_spotify_authenticated");
  }

  async checkSpotifyPremium(): Promise<boolean> {
    return invoke<boolean>("check_spotify_premium");
  }

  async initializeSpotifySession(accessToken: string): Promise<void> {
    return invoke<void>("initialize_spotify_session", { accessToken });
  }

  async initializeSpotifySessionFromProvider(): Promise<void> {
    return invoke<void>("initialize_spotify_session_from_provider");
  }

  async isSpotifySessionReady(): Promise<boolean> {
    return invoke<boolean>("is_spotify_session_ready");
  }

  async refreshSpotifyToken(): Promise<void> {
    return invoke<void>("refresh_spotify_token");
  }

  async getSpotifyPlaylists(): Promise<Playlist[]> {
    return invoke<Playlist[]>("get_spotify_playlists");
  }

  async getSpotifyPlaylist(id: string): Promise<Playlist> {
    return invoke<Playlist>("get_spotify_playlist", { id });
  }

  async checkOAuthCode(): Promise<boolean> {
    return invoke<boolean>("check_oauth_code");
  }

  async disconnectSpotify(): Promise<void> {
    return invoke<void>("disconnect_spotify");
  }

  async saveSpotifySession(): Promise<void> {
    return invoke<void>("save_spotify_session");
  }

  async restoreSpotifySession(): Promise<boolean> {
    return invoke<boolean>("restore_spotify_session");
  }

  async clearSpotifySession(): Promise<void> {
    return invoke<void>("clear_spotify_session");
  }

  // Jellyfin commands
  async authenticateJellyfin(url: string, apiKey: string): Promise<void> {
    return invoke<void>("authenticate_jellyfin", {
      url,
      apiKey: apiKey,
    });
  }

  async isJellyfinAuthenticated(): Promise<boolean> {
    return invoke<boolean>("is_jellyfin_authenticated");
  }

  async getJellyfinPlaylists(): Promise<Playlist[]> {
    return invoke<Playlist[]>("get_jellyfin_playlists");
  }

  async getJellyfinPlaylist(id: string): Promise<Playlist> {
    return invoke<Playlist>("get_jellyfin_playlist", { id });
  }

  async searchJellyfinTracks(query: string): Promise<Track[]> {
    return invoke<Track[]>("search_jellyfin_tracks", { query });
  }

  async searchJellyfinPlaylists(query: string): Promise<Playlist[]> {
    return invoke<Playlist[]>("search_jellyfin_playlists", { query });
  }

  async getJellyfinRecentlyPlayed(limit: number): Promise<Track[]> {
    return invoke<Track[]>("get_jellyfin_recently_played", { limit });
  }

  async disconnectJellyfin(): Promise<void> {
    return invoke<void>("disconnect_jellyfin");
  }

  async getAudioFile(url: string): Promise<string> {
    return invoke<string>("get_audio_file", { url });
  }
}

// Create and export global instance
export const tauriAPI = new TauriAPI();
