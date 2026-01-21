import { useState, useCallback } from "react";
import { tauriAPI } from "../api";
import type { SearchResult, TauriSource, SearchType } from "../types";

export function useSearch() {
  const [results, setResults] = useState<SearchResult[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const search = useCallback(
    async (query: string, searchType: SearchType, source: TauriSource) => {
      if (!query.trim()) {
        setResults([]);
        return;
      }

      try {
        setIsLoading(true);
        setError(null);
        const searchResults: SearchResult[] = [];

        if (searchType === "tracks") {
          if (source === "spotify" || source === "all") {
            try {
              const spotifyTracks = await tauriAPI.searchSpotifyTracks(query);
              searchResults.push(
                ...spotifyTracks.map((track) => ({
                  id: track.id,
                  name: track.title,
                  artist: track.artist,
                  type: "track" as const,
                  source: track.source,
                })),
              );
            } catch (err) {
              console.error("Spotify search error:", err);
            }
          }

          if (source === "jellyfin" || source === "all") {
            try {
              const jellyfinTracks = await tauriAPI.searchJellyfinTracks(query);
              searchResults.push(
                ...jellyfinTracks.map((track) => ({
                  id: track.id,
                  name: track.title,
                  artist: track.artist,
                  type: "track" as const,
                  source: track.source,
                })),
              );
            } catch (err) {
              console.error("Jellyfin search error:", err);
            }
          }
        } else {
          // Playlists
          if (source === "jellyfin" || source === "all") {
            try {
              const jellyfinPlaylists =
                await tauriAPI.searchJellyfinPlaylists(query);
              searchResults.push(
                ...jellyfinPlaylists.map((playlist) => ({
                  id: playlist.id,
                  name: playlist.name,
                  owner: playlist.owner,
                  type: "playlist" as const,
                  source: playlist.source,
                })),
              );
            } catch (err) {
              console.error("Jellyfin search error:", err);
            }
          }
        }

        if (searchResults.length === 0) {
          setError("No results found");
        }

        setResults(searchResults);
      } catch (err) {
        const message = err instanceof Error ? err.message : "Search failed";
        setError(message);
        setResults([]);
      } finally {
        setIsLoading(false);
      }
    },
    [],
  );

  const clearResults = useCallback(() => {
    setResults([]);
    setError(null);
  }, []);

  return {
    results,
    isLoading,
    error,
    search,
    clearResults,
  };
}
