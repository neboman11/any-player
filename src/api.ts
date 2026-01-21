/**
 * Tauri API wrapper - handles communication with Rust backend
 */

import { invoke } from "@tauri-apps/api/core";
import type {
  PlaybackStatus,
  Playlist,
  Track,
  CustomPlaylist,
  PlaylistTrack,
  ColumnPreferences,
  UnionPlaylistSource,
} from "./types";

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

  async playPlaylist(playlistId: string, source: string): Promise<void> {
    return invoke<void>("play_playlist", { playlistId, source });
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

  async searchSpotifyTracks(query: string): Promise<Track[]> {
    return invoke<Track[]>("search_spotify_tracks", { query });
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

  async getJellyfinCredentials(): Promise<[string, string] | null> {
    return invoke<[string, string] | null>("get_jellyfin_credentials");
  }

  async restoreJellyfinSession(): Promise<boolean> {
    return invoke<boolean>("restore_jellyfin_session");
  }

  async getAudioFile(url: string): Promise<string> {
    return invoke<string>("get_audio_file", { url });
  }

  // Custom playlist commands
  async createCustomPlaylist(
    name: string,
    description: string | null,
    imageUrl: string | null,
  ): Promise<CustomPlaylist> {
    return invoke("create_custom_playlist", {
      name,
      description,
      imageUrl,
    });
  }

  async getCustomPlaylists(): Promise<CustomPlaylist[]> {
    return invoke("get_custom_playlists");
  }

  async getCustomPlaylist(playlistId: string): Promise<CustomPlaylist | null> {
    return invoke("get_custom_playlist", { playlistId });
  }

  async updateCustomPlaylist(
    playlistId: string,
    name: string | null,
    description: string | null,
    imageUrl: string | null,
  ): Promise<void> {
    return invoke("update_custom_playlist", {
      playlistId,
      name,
      description,
      imageUrl,
    });
  }

  async deleteCustomPlaylist(playlistId: string): Promise<void> {
    return invoke("delete_custom_playlist", { playlistId });
  }

  async addTrackToCustomPlaylist(
    playlistId: string,
    track: Track,
  ): Promise<PlaylistTrack> {
    return invoke("add_track_to_custom_playlist", { playlistId, track });
  }

  async getCustomPlaylistTracks(playlistId: string): Promise<PlaylistTrack[]> {
    return invoke("get_custom_playlist_tracks", { playlistId });
  }

  async removeTrackFromCustomPlaylist(trackId: number): Promise<void> {
    return invoke("remove_track_from_custom_playlist", { trackId });
  }

  async reorderCustomPlaylistTracks(
    playlistId: string,
    trackId: number,
    newPosition: number,
  ): Promise<void> {
    return invoke("reorder_custom_playlist_tracks", {
      playlistId,
      trackId,
      newPosition,
    });
  }

  async getColumnPreferences(): Promise<ColumnPreferences> {
    return invoke("get_column_preferences");
  }

  async saveColumnPreferences(preferences: ColumnPreferences): Promise<void> {
    return invoke("save_column_preferences", { preferences });
  }

  // Union playlist commands
  async createUnionPlaylist(
    name: string,
    description: string | null,
    imageUrl: string | null,
  ): Promise<CustomPlaylist> {
    return invoke("create_union_playlist", {
      name,
      description,
      imageUrl,
    });
  }

  async addSourceToUnionPlaylist(
    unionPlaylistId: string,
    sourceType: string,
    sourcePlaylistId: string,
  ): Promise<UnionPlaylistSource> {
    return invoke("add_source_to_union_playlist", {
      unionPlaylistId,
      sourceType,
      sourcePlaylistId,
    });
  }

  async getUnionPlaylistSources(
    unionPlaylistId: string,
  ): Promise<UnionPlaylistSource[]> {
    return invoke("get_union_playlist_sources", { unionPlaylistId });
  }

  async removeSourceFromUnionPlaylist(sourceId: number): Promise<void> {
    return invoke("remove_source_from_union_playlist", { sourceId });
  }

  async reorderUnionPlaylistSources(
    unionPlaylistId: string,
    sourceId: number,
    newPosition: number,
  ): Promise<void> {
    return invoke("reorder_union_playlist_sources", {
      unionPlaylistId,
      sourceId,
      newPosition,
    });
  }

  async getUnionPlaylistTracks(unionPlaylistId: string): Promise<Track[]> {
    return invoke("get_union_playlist_tracks", { unionPlaylistId });
  }

  // Cache commands
  async writePlaylistsCache(data: string): Promise<void> {
    return invoke("write_playlists_cache", { data });
  }

  async readPlaylistsCache(): Promise<string | null> {
    return invoke("read_playlists_cache");
  }

  async clearPlaylistsCache(): Promise<void> {
    return invoke("clear_playlists_cache");
  }

  async writeCustomPlaylistsCache(data: string): Promise<void> {
    return invoke("write_custom_playlists_cache", { data });
  }

  async readCustomPlaylistsCache(): Promise<string | null> {
    return invoke("read_custom_playlists_cache");
  }

  async clearCustomPlaylistsCache(): Promise<void> {
    return invoke("clear_custom_playlists_cache");
  }
}

// Create and export global instance
export const tauriAPI = new TauriAPI();
