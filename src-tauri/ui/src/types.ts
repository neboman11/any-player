/**
 * Type definitions for Any Player Tauri API
 */

export interface PlaybackStatus {
  state: "playing" | "paused" | "stopped";
  shuffle: boolean;
  repeat_mode: "off" | "one" | "all";
  volume: number;
  current_track?: Track;
  position?: number;
  duration?: number;
}

export interface Track {
  id: string;
  title: string;
  artist: string;
  album?: string;
  duration?: number;
  source: "spotify" | "jellyfin";
}

export interface Playlist {
  id: string;
  name: string;
  owner: string;
  track_count: number;
  source: "spotify" | "jellyfin";
  image_url?: string;
}

export interface SearchResult {
  id: string;
  name: string;
  artist?: string;
  owner?: string;
  type: "track" | "playlist";
  source: "spotify" | "jellyfin";
}

export interface OAuthCallbackData {
  type: "spotify-auth";
  code?: string;
  error?: string;
}

export type TauriSource = "spotify" | "jellyfin" | "all";
export type SearchType = "tracks" | "playlists";
export type RepeatMode = "off" | "one" | "all";
