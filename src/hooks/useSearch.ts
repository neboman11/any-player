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
          for (const provider of activeProviders) {
            try {
              const providerTracks = await provider.searchTracks(query);
              searchResults.push(
                ...providerTracks.map((track) => ({
                  id: track.id,
                  name: track.title,
                  artist: track.artist,
                  type: "track" as const,
                  source: track.source,
                })),
              );
            } catch (err) {
              console.error(`${provider.label} search error:`, err);
            }
          }
        } else {
          // Playlists
          for (const provider of activeProviders) {
            if (!provider.searchPlaylists) {
              continue;
            }

            try {
              const providerPlaylists = await provider.searchPlaylists(query);
              searchResults.push(
                ...providerPlaylists.map((playlist) => ({
                  id: playlist.id,
                  name: playlist.name,
                  owner: playlist.owner,
                  type: "playlist" as const,
                  source: playlist.source,
                })),
              );
            } catch (err) {
              console.error(`${provider.label} search error:`, err);
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
