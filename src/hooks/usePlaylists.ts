import { useState, useCallback, useRef, useEffect } from "react";
import { tauriAPI } from "../api";
import type { Playlist, TauriSource } from "../types";

const CACHE_VERSION = 1;

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
        console.error("Failed to clear outdated cache:", clearErr);
      }
      return null;
    }

    // Optional: Check if cache is too old (e.g., older than 24 hours)
    const MAX_CACHE_AGE = 24 * 60 * 60 * 1000; // 24 hours
    if (Date.now() - cacheData.timestamp > MAX_CACHE_AGE) {
      console.log("Disk cache expired, will refresh");
      // Don't remove it yet, we can still use it while loading fresh data
    }

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

      // Use cache if available and not forcing reload
      if (cacheInitialized && !forceReload) {
        setPlaylists(playlistCache);
        setIsLoading(false);
        return;
      }

      try {
        isLoadingRef.current = true;
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
              console.log(
                `Loaded ${spotifyPlaylists.length} Spotify playlists`,
              );
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
