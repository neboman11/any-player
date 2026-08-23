import { useState, useCallback, useRef, useEffect } from "react";
import { tauriAPI } from "../api";
import type { Playlist, TauriSource } from "../types";
import { withTimeout } from "../utils/timeout";
import { SERVICE_PROVIDERS, includesSource } from "../providerCatalog";

const CACHE_VERSION = 1;
const PROVIDER_CHECK_TIMEOUT_MS = 2500;
const PLAYLIST_FETCH_TIMEOUT_MS = 8000;

interface PlaylistCacheData {
  version: number;
  timestamp: number;
  playlists: Playlist[];
}

// Singleton cache to persist playlist data across component re-renders and unmounts
// LIMITATION: This cache is module-scoped and will be shared across all hook instances.
// During hot module reloading in development, the cache may not be cleared as expected.
// If multiple instances of the app were to run in the same process (unlikely in Tauri),
// they would share this cache. For production single-instance desktop apps, this is acceptable.
let playlistCache: Playlist[] = [];
let cacheInitialized = false;

// Disk cache helpers using Rust backend
async function saveToDiskCache(playlists: Playlist[]) {
  try {
    const cacheData: PlaylistCacheData = {
      version: CACHE_VERSION,
      timestamp: Date.now(),
      playlists,
    };
    await tauriAPI.writePlaylistsCache(JSON.stringify(cacheData));
    console.log(`Saved ${playlists.length} playlists to disk cache`);
  } catch (err) {
    console.error("Failed to save playlists to disk cache:", err);
  }
}

async function loadFromDiskCache(): Promise<Playlist[] | null> {
  try {
    const cached = await tauriAPI.readPlaylistsCache();
    if (!cached) return null;

    const cacheData: PlaylistCacheData = JSON.parse(cached);

    // Check version compatibility
    if (cacheData.version !== CACHE_VERSION) {
      console.log("Cache version mismatch, clearing disk cache");
      try {
        await tauriAPI.clearPlaylistsCache();
      } catch (clearErr) {
        console.error(
          "Failed to clear outdated cache - this may indicate a permissions issue or disk problem:",
          clearErr,
        );
      }
      return null;
    }

    // No max-age check: cached playlists are kept indefinitely until the
    // user explicitly refreshes or the cache version changes.
    console.log(
      `Loaded ${cacheData.playlists.length} playlists from disk cache`,
    );
    return cacheData.playlists;
  } catch (err) {
    console.error("Failed to load playlists from disk cache:", err);
    return null;
  }
}

async function clearDiskCache() {
  try {
    await tauriAPI.clearPlaylistsCache();
    console.log("Cleared playlists disk cache");
  } catch (err) {
    console.error("Failed to clear disk cache:", err);
  }
}

export function usePlaylists() {
  const [playlists, setPlaylists] = useState<Playlist[]>(playlistCache);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const isLoadingRef = useRef(false);

  // Load from disk cache on first mount
  useEffect(() => {
    if (!cacheInitialized) {
      loadFromDiskCache()
        .then((diskCache) => {
          if (diskCache && diskCache.length > 0) {
            playlistCache = diskCache;
            cacheInitialized = true;
            setPlaylists(diskCache);
            console.log("Initialized playlists from disk cache");
          }
        })
        .catch((err) => {
          console.error("Failed to load disk cache on mount:", err);
        });
    }
  }, []);

  const loadPlaylists = useCallback(
    async (source: TauriSource, forceReload = false) => {
      // Prevent concurrent loads
      if (isLoadingRef.current) {
        return;
      }

      // Use cache when initialized unless force reload is explicitly requested
      if (cacheInitialized && !forceReload) {
        setPlaylists(playlistCache);
        setError(null);
        setIsLoading(false);
        return;
      }

      try {
        isLoadingRef.current = true;
        setIsLoading(true);
        setError(null);
        const allPlaylists: Playlist[] = [];
        const authResults = await Promise.all(
          SERVICE_PROVIDERS.map(async (provider) => {
            if (!includesSource(source, provider.id)) {
              return {
                provider,
                authenticated: false,
                timedOut: false,
              };
            }

            const authResult = await withTimeout(
              provider.isAuthenticated().catch((err) => {
                console.error(
                  `Error checking ${provider.label} authentication:`,
                  err,
                );
                return false;
              }),
              PROVIDER_CHECK_TIMEOUT_MS,
              false,
            );

            if (authResult.timedOut) {
              console.warn(`${provider.label} authentication check timed out`);
            }

            return {
              provider,
              authenticated: authResult.value,
              timedOut: authResult.timedOut,
            };
          }),
        );

        const playlistResults = await Promise.all(
          authResults.map(async ({ provider, authenticated }) => {
            if (!authenticated) {
              return {
                provider,
                playlists: [] as Playlist[],
                timedOut: false,
              };
            }

            const playlistsResult = await withTimeout(
              provider.getPlaylists().catch((err) => {
                console.error(
                  `Error fetching ${provider.label} playlists:`,
                  err,
                );
                return [];
              }),
              PLAYLIST_FETCH_TIMEOUT_MS,
              [] as Playlist[],
            );

            if (playlistsResult.timedOut) {
              console.warn(
                `${provider.label} playlist fetch timed out, may have incomplete data`,
              );
            }

            return {
              provider,
              playlists: playlistsResult.value,
              timedOut: playlistsResult.timedOut,
            };
          }),
        );

        playlistResults.forEach(({ provider, playlists }) => {
          if (playlists.length > 0) {
            allPlaylists.push(...playlists);
            console.log(
              `Loaded ${playlists.length} ${provider.label} playlists`,
            );
          }
        });

        if (allPlaylists.length === 0) {
          const hasAnyConnectedService = authResults.some(
            ({ authenticated }) => authenticated,
          );

          if (!hasAnyConnectedService) {
            setError(
              "No services connected. Connect Spotify, Jellyfin, or Plex in Settings.",
            );
          } else {
            setError(
              "No playlists found. Create some playlists in your music service.",
            );
          }
        }

        setPlaylists(allPlaylists);
        playlistCache = allPlaylists;
        cacheInitialized = true;

        // Save to disk cache (async, don't await to avoid blocking)
        saveToDiskCache(allPlaylists).catch((err) => {
          console.error("Failed to save disk cache:", err);
        });
      } catch (err) {
        const message =
          err instanceof Error ? err.message : "Failed to load playlists";
        setError(message);
        setPlaylists([]);
        playlistCache = [];
      } finally {
        setIsLoading(false);
        isLoadingRef.current = false;
      }
    },
    [],
  );

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

  const playPlaylist = useCallback(
    async (playlistId: string, source: TauriSource) => {
      try {
        if (source === "all") return;
        await tauriAPI.playPlaylist(playlistId, source);
      } catch (err) {
        const message =
          err instanceof Error ? err.message : "Failed to play playlist";
        setError(message);
        console.error("Error playing playlist:", err);
      }
    },
    [],
  );

  const refresh = useCallback(() => {
    setRefreshKey((prev) => prev + 1);
  }, []);

  const clearCache = useCallback(() => {
    playlistCache = [];
    cacheInitialized = false;
    setPlaylists([]);
    clearDiskCache().catch((err) => {
      console.error("Failed to clear disk cache:", err);
    });
  }, []);

  return {
    playlists,
    isLoading,
    error,
    loadPlaylists,
    queuePlaylist,
    playPlaylist,
    refresh,
    refreshKey,
    clearCache,
    isCached: cacheInitialized,
  };
}
