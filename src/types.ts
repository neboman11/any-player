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
  queue: Track[];
}

export interface Track {
  id: string;
  title: string;
  artist: string;
  album?: string;
  duration_ms?: number;
  source: "spotify" | "jellyfin" | "plex" | "custom";
  url?: string;
  image_url?: string;
  bitrate_kbps?: number;
  sample_rate_hz?: number;
  enriched?: boolean;
}

export interface Playlist {
  id: string;
  name: string;
  owner: string;
  track_count: number;
  source: "spotify" | "jellyfin" | "plex" | "custom";
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
  playlist_type: "standard" | "union";
}

export interface UnionPlaylistSource {
  id: number;
  union_playlist_id: string;
  source_type: string; // "spotify", "jellyfin", "custom"
  source_playlist_id: string;
  position: number;
  added_at: number;
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
  url?: string;
}

export interface ColumnPreferences {
  columns: string[];
  column_order: number[];
  column_widths: Record<string, number>;
}

export interface ExportServerConfig {
  base_url: string | null;
}

export interface ExportSpotifyConfig {
  client_id: string | null;
  redirect_uri: string | null;
}

export interface ExportProviderConfigs {
  spotify: ExportSpotifyConfig;
  jellyfin: ExportServerConfig;
  plex: ExportServerConfig;
}

export interface ExportCustomPlaylist {
  playlist: CustomPlaylist;
  tracks: PlaylistTrack[];
  union_sources: UnionPlaylistSource[];
}

export interface ExportConfigPayload {
  export_version: number;
  provider_configs: ExportProviderConfigs;
  custom_playlists: ExportCustomPlaylist[];
}

export interface SearchResult {
  id: string;
  name: string;
  artist?: string;
  owner?: string;
  type: "track" | "playlist";
  source: "spotify" | "jellyfin" | "plex" | "custom";
}

export interface OAuthCallbackData {
  type: "spotify-auth";
  code?: string;
  error?: string;
}

export interface SpotifyAuthStatus {
  authenticated: boolean;
  premium: boolean | null;
  session_ready: boolean;
}

export interface JellyfinAuthStatus {
  authenticated: boolean;
}

export interface PlexAuthStatus {
  authenticated: boolean;
}

export interface BackendInitStatus {
  stage: string;
  message: string;
  done: boolean;
  success: boolean;
}

export interface OAuthCodeReceived {
  source: "spotify";
}

export type TauriSource = "spotify" | "jellyfin" | "plex" | "custom" | "all";
export type SearchType = "tracks" | "playlists";
export type RepeatMode = "off" | "one" | "all";
