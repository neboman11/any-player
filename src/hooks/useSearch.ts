import { useState, useCallback } from "react";
import type { SearchResult, TauriSource, SearchType } from "../types";
import { SERVICE_PROVIDERS, includesSource } from "../providerCatalog";

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
        const activeProviders = SERVICE_PROVIDERS.filter((provider) =>
          includesSource(source, provider.id),
        );

        if (searchType === "tracks") {
          const trackResults = await Promise.allSettled(
            activeProviders.map((provider) => provider.searchTracks(query)),
          );

          for (let i = 0; i < trackResults.length; i++) {
            const result = trackResults[i];
            if (result.status === "fulfilled") {
              searchResults.push(
                ...result.value.map((track) => ({
                  id: track.id,
                  name: track.title,
                  artist: track.artist,
                  type: "track" as const,
                  source: track.source,
                })),
              );
            } else {
              console.error(`${activeProviders[i].label} search error:`, result.reason);
            }
          }
        } else {
          const playlistProviders = activeProviders.filter(
            (provider) => provider.searchPlaylists,
          );
          const playlistResults = await Promise.allSettled(
            playlistProviders.map((provider) => provider.searchPlaylists!(query)),
          );

          for (let i = 0; i < playlistResults.length; i++) {
            const result = playlistResults[i];
            if (result.status === "fulfilled") {
              searchResults.push(
                ...result.value.map((playlist) => ({
                  id: playlist.id,
                  name: playlist.name,
                  owner: playlist.owner,
                  type: "playlist" as const,
                  source: playlist.source,
                })),
              );
            } else {
              console.error(
                `${playlistProviders[i].label} search error:`,
                result.reason,
              );
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
