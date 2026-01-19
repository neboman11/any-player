import { useState, useCallback } from "react";
import { tauriAPI } from "../api";
import type { Playlist, TauriSource } from "../types";

export function usePlaylists() {
  const [playlists, setPlaylists] = useState<Playlist[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);

  const loadPlaylists = useCallback(async (source: TauriSource) => {
    try {
      setIsLoading(true);
      setError(null);
      const allPlaylists: Playlist[] = [];
      let spotifyAuth = false;
      let jellyfinAuth = false;

      // Load Spotify playlists if authenticated
      if (source === "spotify" || source === "all") {
        try {
          spotifyAuth = await tauriAPI.isSpotifyAuthenticated();
          if (spotifyAuth) {
            const spotifyPlaylists = await tauriAPI.getSpotifyPlaylists();
            allPlaylists.push(...spotifyPlaylists);
            console.log(`Loaded ${spotifyPlaylists.length} Spotify playlists`);
          }
        } catch (err) {
          console.error("Error loading Spotify playlists:", err);
        }
      }

      // Load Jellyfin playlists if authenticated
      if (source === "jellyfin" || source === "all") {
        try {
          jellyfinAuth = await tauriAPI.isJellyfinAuthenticated();
          if (jellyfinAuth) {
            const jellyfinPlaylists = await tauriAPI.getJellyfinPlaylists();
            allPlaylists.push(...jellyfinPlaylists);
            console.log(
              `Loaded ${jellyfinPlaylists.length} Jellyfin playlists`,
            );
          }
        } catch (err) {
          console.error("Error loading Jellyfin playlists:", err);
        }
      }

      if (allPlaylists.length === 0) {
        if (!spotifyAuth && !jellyfinAuth) {
          setError(
            "No services connected. Connect Spotify or Jellyfin in Settings.",
          );
        } else {
          setError(
            "No playlists found. Create some playlists in your music service.",
          );
        }
      }

      setPlaylists(allPlaylists);
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to load playlists";
      setError(message);
      setPlaylists([]);
    } finally {
      setIsLoading(false);
    }
  }, []);

  const queuePlaylist = useCallback(
    async (playlistId: string, source: TauriSource) => {
      try {
        if (source === "all") return;
        await tauriAPI.queueTrack(playlistId, source);
      } catch (err) {
        console.error("Error queueing playlist:", err);
      }
    },
    [],
  );

  const refresh = useCallback(() => {
    setRefreshKey((prev) => prev + 1);
  }, []);

  return {
    playlists,
    isLoading,
    error,
    loadPlaylists,
    queuePlaylist,
    refresh,
    refreshKey,
  };
}
