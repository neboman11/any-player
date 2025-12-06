import { useState, useCallback } from "react";
import { tauriAPI } from "../api";
import type { Playlist, TauriSource } from "../types";

export function usePlaylists() {
  const [playlists, setPlaylists] = useState<Playlist[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadPlaylists = useCallback(async (source: TauriSource) => {
    try {
      setIsLoading(true);
      setError(null);
      const allPlaylists: Playlist[] = [];

      // Load Spotify playlists if authenticated
      if (source === "spotify" || source === "all") {
        try {
          const authenticated = await tauriAPI.isSpotifyAuthenticated();
          if (authenticated) {
            const spotifyPlaylists = await tauriAPI.getSpotifyPlaylists();
            allPlaylists.push(...spotifyPlaylists);
          }
        } catch (err) {
          console.error("Error loading Spotify playlists:", err);
        }
      }

      // Load Jellyfin playlists if authenticated
      if (source === "jellyfin" || source === "all") {
        try {
          const authenticated = await tauriAPI.isJellyfinAuthenticated();
          if (authenticated) {
            const jellyfinPlaylists = await tauriAPI.getJellyfinPlaylists();
            allPlaylists.push(...jellyfinPlaylists);
          }
        } catch (err) {
          console.error("Error loading Jellyfin playlists:", err);
        }
      }

      if (allPlaylists.length === 0) {
        setError("No playlists found. Connect a service in Settings.");
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
    []
  );

  return {
    playlists,
    isLoading,
    error,
    loadPlaylists,
    queuePlaylist,
  };
}
