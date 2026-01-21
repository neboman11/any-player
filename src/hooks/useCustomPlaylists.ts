import { useState, useEffect, useCallback } from "react";
import { tauriAPI } from "../api";
import type {
  CustomPlaylist,
  PlaylistTrack,
  Track,
  UnionPlaylistSource,
} from "../types";

export function useCustomPlaylists() {
  const [playlists, setPlaylists] = useState<CustomPlaylist[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadPlaylists = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const data = await tauriAPI.getCustomPlaylists();
      setPlaylists(data);
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
        await loadPlaylists();
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
        await loadPlaylists();
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
        await loadPlaylists();
      } catch (err) {
        console.error("Error deleting playlist:", err);
        throw err;
      }
    },
    [loadPlaylists],
  );

  return {
    playlists,
    loading,
    error,
    refresh: loadPlaylists,
    createPlaylist,
    updatePlaylist,
    deletePlaylist,
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
