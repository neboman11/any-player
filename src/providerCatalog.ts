import { tauriAPI } from "./api";
import type { Playlist, TauriSource, Track } from "./types";

export type ServiceSource = Exclude<TauriSource, "all" | "custom">;

type ServiceProviderDefinition = {
  id: ServiceSource;
  label: string;
  isAuthenticated: () => Promise<boolean>;
  getPlaylists: () => Promise<Playlist[]>;
  searchTracks: (query: string) => Promise<Track[]>;
  searchPlaylists?: (query: string) => Promise<Playlist[]>;
};

export const SERVICE_PROVIDERS: readonly ServiceProviderDefinition[] = [
  {
    id: "spotify",
    label: "Spotify",
    isAuthenticated: () => tauriAPI.isSpotifyAuthenticated(),
    getPlaylists: () => tauriAPI.getSpotifyPlaylists(),
    searchTracks: (query) => tauriAPI.searchSpotifyTracks(query),
  },
  {
    id: "jellyfin",
    label: "Jellyfin",
    isAuthenticated: () => tauriAPI.isJellyfinAuthenticated(),
    getPlaylists: () => tauriAPI.getJellyfinPlaylists(),
    searchTracks: (query) => tauriAPI.searchJellyfinTracks(query),
    searchPlaylists: (query) => tauriAPI.searchJellyfinPlaylists(query),
  },
  {
    id: "plex",
    label: "Plex",
    isAuthenticated: () => tauriAPI.isPlexAuthenticated(),
    getPlaylists: () => tauriAPI.getPlexPlaylists(),
    searchTracks: (query) => tauriAPI.searchPlexTracks(query),
    searchPlaylists: (query) => tauriAPI.searchPlexPlaylists(query),
  },
];

export function includesSource(
  filter: TauriSource,
  source: ServiceSource,
): boolean {
  return filter === "all" || filter === source;
}
