import { useState, useEffect, useCallback } from "react";
import { tauriAPI } from "../api";
import type {
  CustomPlaylist,
  PlaylistTrack,
  Track,
  UnionPlaylistSource,
} from "../types";

const CACHE_VERSION = 1;

interface CustomPlaylistCacheData {
  version: number;
  timestamp: number;
  playlists: CustomPlaylist[];
}

// Singleton cache for custom playlists
let customPlaylistCache: CustomPlaylist[] = [];
let customCacheInitialized = false;

// Disk cache helpers for custom playlists using Rust backend
async function saveCustomToDiskCache(playlists: CustomPlaylist[]) {
  try {
    const cacheData: CustomPlaylistCacheData = {
      version: CACHE_VERSION,
      timestamp: Date.now(),
      playlists,
    };
    await tauriAPI.writeCustomPlaylistsCache(JSON.stringify(cacheData));
    console.log(`Saved ${playlists.length} custom playlists to disk cache`);
  } catch (err) {
    console.error("Failed to save custom playlists to disk cache:", err);
  }
}

async function loadCustomFromDiskCache(): Promise<CustomPlaylist[] | null> {
  try {
    const cached = await tauriAPI.readCustomPlaylistsCache();
    if (!cached) return null;

    const cacheData: CustomPlaylistCacheData = JSON.parse(cached);

    // Check version compatibility
    if (cacheData.version !== CACHE_VERSION) {
      console.log("Custom cache version mismatch, ignoring disk cache");
      await tauriAPI.clearCustomPlaylistsCache();
      return null;
    }

    console.log(
      `Loaded ${cacheData.playlists.length} custom playlists from disk cache`,
    );
    return cacheData.playlists;
  } catch (err) {
    console.error("Failed to load custom playlists from disk cache:", err);
    return null;
  }
}

async function clearCustomDiskCache() {
  try {
    await tauriAPI.clearCustomPlaylistsCache();
    console.log("Cleared custom playlists disk cache");
  } catch (err) {
    console.error("Failed to clear custom disk cache:", err);
  }
}

export function useCustomPlaylists() {
  const [playlists, setPlaylists] =
    useState<CustomPlaylist[]>(customPlaylistCache);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Load from disk cache on first mount
  useEffect(() => {
    if (!customCacheInitialized) {
      loadCustomFromDiskCache()
        .then((diskCache) => {
          if (diskCache && diskCache.length > 0) {
            customPlaylistCache = diskCache;
            customCacheInitialized = true;
            setPlaylists(diskCache);
            console.log("Initialized custom playlists from disk cache");
          }
        })
        .catch((err) => {
          console.error("Failed to load custom disk cache on mount:", err);
        });
    }
  }, []);

  const loadPlaylists = useCallback(async (forceReload = false) => {
    // Use cache if available and not forcing reload
    if (
      customCacheInitialized &&
      !forceReload &&
      customPlaylistCache.length > 0
    ) {
      setPlaylists(customPlaylistCache);
      setLoading(false);
      return;
    }

    try {
      setLoading(true);
      setError(null);
      const data = await tauriAPI.getCustomPlaylists();
      setPlaylists(data);

      // Update cache
      customPlaylistCache = data;
      customCacheInitialized = true;

      // Save to disk cache (async, don't await to avoid blocking)
      saveCustomToDiskCache(data).catch((err) => {
        console.error("Failed to save custom disk cache:", err);
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load playlists");
      console.error("Error loading custom playlists:", err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadPlaylists();
  }, [loadPlaylists]);

  const createPlaylist = useCallback(
    async (
      name: string,
      description: string | null = null,
      imageUrl: string | null = null,
    ) => {
      try {
        const newPlaylist = await tauriAPI.createCustomPlaylist(
          name,
          description,
          imageUrl,
        );
        await loadPlaylists(true);
        return newPlaylist;
      } catch (err) {
        console.error("Error creating playlist:", err);
        throw err;
      }
    },
    [loadPlaylists],
  );

  const updatePlaylist = useCallback(
    async (
      playlistId: string,
      name: string | null = null,
      description: string | null = null,
      imageUrl: string | null = null,
    ) => {
      try {
        await tauriAPI.updateCustomPlaylist(
          playlistId,
          name,
          description,
          imageUrl,
        );
        await loadPlaylists(true);
      } catch (err) {
        console.error("Error updating playlist:", err);
        throw err;
      }
    },
    [loadPlaylists],
  );

  const deletePlaylist = useCallback(
    async (playlistId: string) => {
      try {
        await tauriAPI.deleteCustomPlaylist(playlistId);
        await loadPlaylists(true);
      } catch (err) {
        console.error("Error deleting playlist:", err);
        throw err;
      }
    },
    [loadPlaylists],
  );

  const clearCache = useCallback(() => {
    customPlaylistCache = [];
    customCacheInitialized = false;
    setPlaylists([]);
    clearCustomDiskCache().catch((err) => {
      console.error("Failed to clear custom disk cache:", err);
    });
  }, []);

  return {
    playlists,
    loading,
    error,
    refresh: loadPlaylists,
    createPlaylist,
    updatePlaylist,
    deletePlaylist,
    clearCache,
    isCached: customCacheInitialized,
  };
}

export function useCustomPlaylistTracks(playlistId: string | null) {
  const [tracks, setTracks] = useState<PlaylistTrack[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadTracks = useCallback(async () => {
    if (!playlistId) {
      setTracks([]);
      setLoading(false);
      return;
    }

    try {
      setLoading(true);
      setError(null);
      const data = await tauriAPI.getCustomPlaylistTracks(playlistId);
      setTracks(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load tracks");
      console.error("Error loading playlist tracks:", err);
    } finally {
      setLoading(false);
    }
  }, [playlistId]);

  useEffect(() => {
    loadTracks();
  }, [loadTracks]);

  const addTrack = useCallback(
    async (track: Track) => {
      if (!playlistId) return;

      try {
        await tauriAPI.addTrackToCustomPlaylist(playlistId, track);
        await loadTracks();
      } catch (err) {
        console.error("Error adding track:", err);
        throw err;
      }
    },
    [playlistId, loadTracks],
  );

  const removeTrack = useCallback(
    async (trackId: number) => {
      try {
        await tauriAPI.removeTrackFromCustomPlaylist(trackId);
        await loadTracks();
      } catch (err) {
        console.error("Error removing track:", err);
        throw err;
      }
    },
    [loadTracks],
  );

  const reorderTrack = useCallback(
    async (trackId: number, newPosition: number) => {
      if (!playlistId) return;

      try {
        await tauriAPI.reorderCustomPlaylistTracks(
          playlistId,
          trackId,
          newPosition,
        );
        await loadTracks();
      } catch (err) {
        console.error("Error reordering track:", err);
        throw err;
      }
    },
    [playlistId, loadTracks],
  );

  return {
    tracks,
    loading,
    error,
    refresh: loadTracks,
    addTrack,
    removeTrack,
    reorderTrack,
  };
}

export function useUnionPlaylistSources(unionPlaylistId: string | null) {
  const [sources, setSources] = useState<UnionPlaylistSource[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadSources = useCallback(async () => {
    if (!unionPlaylistId) {
      setSources([]);
      setLoading(false);
      return;
    }

    try {
      setLoading(true);
      setError(null);
      const data = await tauriAPI.getUnionPlaylistSources(unionPlaylistId);
      setSources(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load sources");
      console.error("Error loading union playlist sources:", err);
    } finally {
      setLoading(false);
    }
  }, [unionPlaylistId]);

  useEffect(() => {
    loadSources();
  }, [loadSources]);

  const addSource = useCallback(
    async (sourceType: string, sourcePlaylistId: string) => {
      if (!unionPlaylistId) return;

      try {
        await tauriAPI.addSourceToUnionPlaylist(
          unionPlaylistId,
          sourceType,
          sourcePlaylistId,
        );
        await loadSources();
      } catch (err) {
        console.error("Error adding source:", err);
        throw err;
      }
    },
    [unionPlaylistId, loadSources],
  );

  const removeSource = useCallback(
    async (sourceId: number) => {
      try {
        await tauriAPI.removeSourceFromUnionPlaylist(sourceId);
        await loadSources();
      } catch (err) {
        console.error("Error removing source:", err);
        throw err;
      }
    },
    [loadSources],
  );

  const reorderSource = useCallback(
    async (sourceId: number, newPosition: number) => {
      if (!unionPlaylistId) return;

      try {
        await tauriAPI.reorderUnionPlaylistSources(
          unionPlaylistId,
          sourceId,
          newPosition,
        );
        await loadSources();
      } catch (err) {
        console.error("Error reordering source:", err);
        throw err;
      }
    },
    [unionPlaylistId, loadSources],
  );

  return {
    sources,
    loading,
    error,
    refresh: loadSources,
    addSource,
    removeSource,
    reorderSource,
  };
}

export function useUnionPlaylistTracks(unionPlaylistId: string | null) {
  const [tracks, setTracks] = useState<Track[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadTracks = useCallback(async () => {
    if (!unionPlaylistId) {
      setTracks([]);
      setLoading(false);
      return;
    }

    try {
      setLoading(true);
      setError(null);
      const data = await tauriAPI.getUnionPlaylistTracks(unionPlaylistId);
      setTracks(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load tracks");
      console.error("Error loading union playlist tracks:", err);
    } finally {
      setLoading(false);
    }
  }, [unionPlaylistId]);

  useEffect(() => {
    loadTracks();
  }, [loadTracks]);

  return {
    tracks,
    loading,
    error,
    refresh: loadTracks,
  };
}
