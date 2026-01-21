/**
 * Type definitions for Any Player Tauri API
 */

export type Page = "now-playing" | "playlists" | "search" | "settings";

export interface PlaybackStatus {
  state: "playing" | "paused" | "stopped";
  shuffle: boolean;
  repeat_mode: "off" | "one" | "all";
  volume: number;
  current_track: Track | null;
  position: number;
  duration: number;
}

export interface Track {
  id: string;
  title: string;
  artist: string;
  album?: string;
  duration?: number;
  source: "spotify" | "jellyfin" | "custom";
  url?: string;
}

export interface Playlist {
  id: string;
  name: string;
  owner: string;
  track_count: number;
  source: "spotify" | "jellyfin" | "custom";
  image_url?: string;
  tracks?: Track[];
  description?: string;
}

export interface CustomPlaylist {
  id: string;
  name: string;
  description: string | null;
  image_url: string | null;
  created_at: number;
  updated_at: number;
  track_count: number;
}

export interface PlaylistTrack {
  id: number;
  playlist_id: string;
  track_source: string;
  track_id: string;
  position: number;
  added_at: number;
  title: string;
  artist: string;
  album: string | null;
  duration_ms: number | null;
  image_url: string | null;
}

export interface ColumnPreferences {
  columns: string[];
  column_order: number[];
  column_widths: Record<string, number>;
}

export interface SearchResult {
  id: string;
  name: string;
  artist?: string;
  owner?: string;
  type: "track" | "playlist";
  source: "spotify" | "jellyfin" | "custom";
}

export interface OAuthCallbackData {
  type: "spotify-auth";
  code?: string;
  error?: string;
}

export type TauriSource = "spotify" | "jellyfin" | "custom" | "all";
export type SearchType = "tracks" | "playlists";
export type RepeatMode = "off" | "one" | "all";
