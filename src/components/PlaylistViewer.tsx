import { useState, useEffect } from "react";
import toast from "react-hot-toast";
import { TrackTable } from "./TrackTable";
import {
  PlaylistHeader,
  DeleteConfirmModal,
  SearchBar,
  DuplicatesSection,
} from "./shared";
import {
  useCustomPlaylistTracks,
  usePlayback,
  usePlaylistEditor,
} from "../hooks";
import { tauriAPI } from "../api";
import { buildDuplicateGroups, buildDistinctTracks } from "../utils/duplicates";
import { filterTracks } from "../utils/trackFilters";
import type { CustomPlaylist, PlaylistTrack, Playlist, Track } from "../types";
import type { ServiceSource } from "../providerCatalog";
import "./PlaylistEditor.css";

const CACHE_VERSION = 1;

interface ProviderPlaylistCacheData {
  version: number;
  timestamp: number;
  playlist: Playlist;
}

interface PlaylistViewerProps {
  playlist: CustomPlaylist | Playlist;
  isCustom: boolean;
  onBack: () => void;
  onUpdate?: (
    name: string | null,
    description: string | null,
    imageUrl: string | null,
  ) => Promise<void>;
  onDelete?: () => Promise<void>;
}

const SENSITIVE_QUERY_PARAMS = [
  "api_key",
  "X-Plex-Token",
  "access_token",
  "token",
];

function sanitizeUrlForCache(rawUrl?: string | null): string | undefined {
  if (!rawUrl) return rawUrl ?? undefined;
  try {
    const url = new URL(rawUrl);
    let modified = false;
    for (const param of SENSITIVE_QUERY_PARAMS) {
      if (url.searchParams.has(param)) {
        url.searchParams.delete(param);
        modified = true;
      }
    }
    return modified ? url.toString() : rawUrl;
  } catch {
    return rawUrl;
  }
}

function normalizeToTrackSource(source: string): Track["source"] {
  if (source === "spotify" || source === "jellyfin" || source === "plex") {
    return source;
  }
  return "custom";
}

function sanitizeTrackForCache(track: Track): Track {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  const { auth_headers: _auth_headers, ...rest } = track as Track & {
    auth_headers?: unknown;
  };
  const sanitized: Track = { ...rest };
  const sanitizedImageUrl = sanitizeUrlForCache(track.image_url);
  if (sanitizedImageUrl !== undefined) sanitized.image_url = sanitizedImageUrl;
  const sanitizedUrl = sanitizeUrlForCache(track.url);
  if (sanitizedUrl !== undefined) sanitized.url = sanitizedUrl;
  return sanitized;
}

function getPlaylistById(source: ServiceSource, id: string): Promise<Playlist> {
  switch (source) {
    case "spotify":
      return tauriAPI.getSpotifyPlaylist(id);
    case "jellyfin":
      return tauriAPI.getJellyfinPlaylist(id);
    case "plex":
      return tauriAPI.getPlexPlaylist(id);
    default: {
      const _exhaustiveCheck: never = source;
      throw new Error(`Unknown source: ${_exhaustiveCheck}`);
    }
  }
}

export function PlaylistViewer({
  playlist,
  isCustom,
  onBack,
  onUpdate,
  onDelete,
}: PlaylistViewerProps) {
  const customPlaylistId = isCustom ? playlist.id : null;
  const {
    tracks: customTracks,
    loading: customLoading,
    removeTrack,
    reorderTrack,
    refresh: refreshCustomTracks,
  } = useCustomPlaylistTracks(customPlaylistId);

  const editorState = usePlaylistEditor({
    playlistName: playlist.name,
    playlistDescription:
      "description" in playlist ? playlist.description : null,
    onUpdate,
    onDelete,
    onBack,
  });

  const [showAddTrack, setShowAddTrack] = useState(false);
  const [regularTracks, setRegularTracks] = useState<Track[]>([]);
  const [loading, setLoading] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [isDistinct, setIsDistinct] = useState(
    isCustom ? (playlist as CustomPlaylist).is_distinct : false,
  );
  const [isDistinctSaving, setIsDistinctSaving] = useState(false);
  const [duplicateIndexToRemove, setDuplicateIndexToRemove] = useState<
    number | null
  >(null);

  const playback = usePlayback();
  const playlistSource = isCustom ? "custom" : (playlist as Playlist).source;

  async function loadRegularPlaylistTracks(forceReload = false) {
    const regularPlaylist = playlist as Playlist;
    const source = regularPlaylist.source as ServiceSource;

    if (!forceReload) {
      try {
        const cached = await tauriAPI.readProviderPlaylistCache(
          source,
          regularPlaylist.id,
        );

        if (cached) {
          const cacheData: ProviderPlaylistCacheData = JSON.parse(cached);
          if (
            cacheData.version === CACHE_VERSION &&
            Array.isArray(cacheData.playlist?.tracks)
          ) {
            setRegularTracks(cacheData.playlist.tracks || []);
            setLoading(false);
            return;
          }
        }
      } catch (err) {
        console.error("Failed to read provider playlist cache:", err);
      }
    }

    setLoading(true);
    try {
      const fullPlaylist = await getPlaylistById(source, regularPlaylist.id);
      setRegularTracks(fullPlaylist.tracks || []);

      const sanitizedPlaylist = {
        ...fullPlaylist,
        tracks: fullPlaylist.tracks?.map(sanitizeTrackForCache),
      };
      const cacheData: ProviderPlaylistCacheData = {
        version: CACHE_VERSION,
        timestamp: Date.now(),
        playlist: sanitizedPlaylist,
      };
      tauriAPI
        .writeProviderPlaylistCache(
          source,
          regularPlaylist.id,
          JSON.stringify(cacheData),
        )
        .catch((err) => {
          console.error("Failed to write provider playlist cache:", err);
        });
    } catch (err) {
      console.error("Failed to load playlist tracks:", err);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    if (!isCustom) {
      void loadRegularPlaylistTracks(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isCustom, playlist.id, playlistSource]);

  useEffect(() => {
    let isActive = true;

    const loadDistinctPreference = async () => {
      if (isCustom) {
        setIsDistinct((playlist as CustomPlaylist).is_distinct);
        return;
      }

      try {
        const providerPlaylist = playlist as Playlist;
        const preference = await tauriAPI.getProviderPlaylistPreference(
          providerPlaylist.source,
          providerPlaylist.id,
        );
        if (isActive) {
          setIsDistinct(preference?.is_distinct ?? false);
        }
      } catch (err) {
        console.error("Failed to load provider distinct preference:", err);
        if (isActive) {
          setIsDistinct(false);
        }
      }
    };

    void loadDistinctPreference();
    return () => {
      isActive = false;
    };
  }, [isCustom, playlist]);

  const handleRefresh = async () => {
    if (isCustom && refreshCustomTracks) {
      await refreshCustomTracks(true);
    } else if (!isCustom) {
      await loadRegularPlaylistTracks(true);
    }
  };

  const handlePlayPlaylist = async () => {
    try {
      const source = isCustom ? "custom" : (playlist as Playlist).source;
      await tauriAPI.playPlaylist(playlist.id, source);
      await playback.updateStatus();
    } catch (err) {
      console.error("Failed to play playlist:", err);
      alert("Failed to play playlist");
    }
  };

  const handlePlayTrack = async (track: PlaylistTrack | Track) => {
    try {
      const trackId = String(track.id);
      let rawSource: string;

      if (isCustom) {
        rawSource = (track as PlaylistTrack).track_source;
      } else {
        rawSource = (track as Track).source || (playlist as Playlist).source;
      }

      const normalizedSource = normalizeToTrackSource(rawSource);

      await playback.playTrack(trackId, normalizedSource);
      await playback.updateStatus();
    } catch (err) {
      console.error("Failed to play track:", err);
    }
  };

  const handlePlayFromTrack = async (index: number) => {
    try {
      const unfilteredTracks = isCustom ? customTracks : regularTracks;
      const filteredTracks = isCustom
        ? filterTracks(customTracks, searchQuery)
        : filterTracks(regularTracks, searchQuery);
      const trackToPlay = filteredTracks[index];
      if (isCustom && isDistinct) {
        const playlistTrack = trackToPlay as PlaylistTrack;
        await tauriAPI.playCustomPlaylistFromTrack(
          playlist.id,
          playlistTrack.track_id,
        );
      } else if (!isCustom && isDistinct) {
        // For provider playlists in distinct mode, apply frontend dedup to match
        // the queue that play_playlist builds, then start from the selected track.
        const distinctTracks = buildDistinctTracks(regularTracks);
        const distinctIndex = distinctTracks.findIndex(
          (t) => t.id === trackToPlay.id,
        );
        await tauriAPI.playPlaylistFromTrack(
          distinctTracks,
          distinctIndex >= 0 ? distinctIndex : 0,
        );
      } else {
        const originalIndex = unfilteredTracks.findIndex(
          (t) => t.id === trackToPlay.id,
        );
        await tauriAPI.playPlaylistFromTrack(unfilteredTracks, originalIndex);
      }
      await playback.updateStatus();
    } catch (err) {
      console.error("Failed to play from track:", err);
    }
  };

  const handleDistinctToggle = async (nextValue: boolean) => {
    const previousValue = isDistinct;
    setIsDistinct(nextValue);
    setIsDistinctSaving(true);

    try {
      if (isCustom) {
        await tauriAPI.setCustomPlaylistDistinct(playlist.id, nextValue);
      } else {
        const providerPlaylist = playlist as Playlist;
        await tauriAPI.setProviderPlaylistPreference(
          providerPlaylist.source,
          providerPlaylist.id,
          nextValue,
        );
      }
    } catch (err) {
      console.error("Failed to persist distinct toggle:", err);
      setIsDistinct(previousValue);
      toast.error("Failed to save distinct preference");
    } finally {
      setIsDistinctSaving(false);
    }
  };

  const handleRemoveDuplicateRequest = (occurrenceIndex: number) => {
    setDuplicateIndexToRemove(occurrenceIndex);
  };

  const confirmDuplicateRemoval = async () => {
    if (duplicateIndexToRemove === null) {
      return;
    }

    const occurrenceTrack = customTracks[duplicateIndexToRemove];
    if (!occurrenceTrack) {
      setDuplicateIndexToRemove(null);
      toast.error("Duplicate track could not be resolved");
      return;
    }

    try {
      await removeTrack(Number(occurrenceTrack.id));
      setDuplicateIndexToRemove(null);
      toast.success("Duplicate removed");
    } catch (err) {
      console.error("Failed to remove duplicate track:", err);
      toast.error("Failed to remove duplicate track");
    }
  };

  const isLoading = isCustom ? customLoading : loading;
  const trackCount = "track_count" in playlist ? playlist.track_count : 0;
  const unfilteredTracks = isCustom ? customTracks : regularTracks;
  const duplicateGroups = isDistinct ? buildDuplicateGroups(unfilteredTracks) : [];
  const isStandardCustomPlaylist =
    isCustom && (playlist as CustomPlaylist).playlist_type === "standard";
  const duplicateTrack =
    duplicateIndexToRemove !== null ? customTracks[duplicateIndexToRemove] : null;

  const tracks = isCustom
    ? filterTracks(customTracks, searchQuery)
    : filterTracks(regularTracks, searchQuery);

  const playlistDescription =
    "description" in playlist ? playlist.description : null;
  const metaInfo = `${trackCount} tracks • ${
    isCustom ? "You" : (playlist as Playlist).owner
  }${
    "created_at" in playlist
      ? ` • Created ${new Date((playlist as CustomPlaylist).created_at * 1000).toLocaleDateString()}`
      : ""
  }`;

  return (
    <div className="custom-playlist-editor">
      <PlaylistHeader
        isEditing={editorState.isEditing}
        editName={editorState.editName}
        editDescription={editorState.editDescription}
        playlistName={playlist.name}
        playlistDescription={playlistDescription}
        metaInfo={metaInfo}
        onEditNameChange={editorState.setEditName}
        onEditDescriptionChange={editorState.setEditDescription}
        onSave={editorState.handleSaveEdit}
        onCancelEdit={editorState.handleCancelEdit}
        onBack={onBack}
      />

      <div className="editor-header">
        <div className="header-actions">
          <label className="distinct-toggle-control">
            <input
              type="checkbox"
              checked={isDistinct}
              onChange={(event) =>
                void handleDistinctToggle(event.target.checked)
              }
              disabled={isDistinctSaving}
            />
            Distinct
          </label>
          <button className="play-btn" onClick={handlePlayPlaylist}>
            ▶ Play All
          </button>
          <button
            className="refresh-btn"
            onClick={handleRefresh}
            title="Refresh tracks"
          >
            ⟳ Refresh
          </button>
          {isCustom && !editorState.isEditing && (
            <>
              <button
                className="add-track-btn"
                onClick={() => setShowAddTrack(!showAddTrack)}
              >
                + Add Track
              </button>
              <button
                className="edit-btn"
                onClick={() => editorState.setIsEditing(true)}
              >
                Edit
              </button>
              <button
                className="delete-btn"
                onClick={() => editorState.setShowDeleteConfirm(true)}
              >
                Delete
              </button>
            </>
          )}
        </div>
      </div>

      {showAddTrack && (
        <div className="add-track-panel">
          <p>Search for tracks to add to this playlist:</p>
          <p className="help-text">
            Go to the Search page, find a track, and click "Add to Playlist"
          </p>
          <button onClick={() => setShowAddTrack(false)}>Close</button>
        </div>
      )}

      <div className="tracks-section">
        <SearchBar value={searchQuery} onChange={setSearchQuery} />
        {isDistinct && duplicateGroups.length > 0 && (
          <DuplicatesSection
            groups={duplicateGroups}
            tracks={unfilteredTracks}
            isReadOnly={!isStandardCustomPlaylist}
            onRemoveDuplicate={
              isStandardCustomPlaylist ? handleRemoveDuplicateRequest : undefined
            }
          />
        )}
        {isLoading ? (
          <div className="loading">Loading tracks...</div>
        ) : (
          <TrackTable
            tracks={tracks}
            onRemoveTrack={isCustom ? removeTrack : undefined}
            onReorderTrack={isCustom ? reorderTrack : undefined}
            onPlayTrack={handlePlayTrack}
            onPlayFromTrack={handlePlayFromTrack}
            sortStorageKey={`playlist-viewer:${isCustom ? "custom" : "provider"}:${playlist.id}`}
          />
        )}
      </div>

      <DeleteConfirmModal
        show={editorState.showDeleteConfirm}
        title="Delete Playlist"
        message={`Are you sure you want to delete "${playlist.name}"? This cannot be undone.`}
        onConfirm={editorState.handleDelete}
        onCancel={() => editorState.setShowDeleteConfirm(false)}
      />

      <DeleteConfirmModal
        show={duplicateIndexToRemove !== null}
        title="Remove Duplicate"
        message={
          duplicateTrack
            ? `Remove duplicate track "${duplicateTrack.title}" by ${duplicateTrack.artist}?`
            : "Remove this duplicate track?"
        }
        confirmLabel="Remove"
        onConfirm={() => {
          void confirmDuplicateRemoval();
        }}
        onCancel={() => setDuplicateIndexToRemove(null)}
      />
    </div>
  );
}
