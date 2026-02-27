import { useState, useEffect } from "react";
import { TrackTable } from "./TrackTable";
import { PlaylistHeader, DeleteConfirmModal, SearchBar } from "./shared";
import {
  useCustomPlaylistTracks,
  usePlayback,
  usePlaylistEditor,
} from "../hooks";
import { tauriAPI } from "../api";
import { filterTracks } from "../utils/trackFilters";
import type { CustomPlaylist, PlaylistTrack, Playlist, Track } from "../types";
import type { ServiceSource } from "../providerCatalog";
import "./CustomPlaylistEditor.css";

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

function sanitizeTrackForCache(track: Track): Track {
  if (!track.image_url) return track;
  try {
    const url = new URL(track.image_url);
    if (!url.searchParams.has("api_key")) return track;
    url.searchParams.delete("api_key");
    return { ...track, image_url: url.toString() };
  } catch {
    return track;
  }
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

  const playback = usePlayback();

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
  }, [
    isCustom,
    playlist.id,
    isCustom ? "custom" : (playlist as Playlist).source,
  ]);

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
      let source: string;

      if (isCustom) {
        source = (track as PlaylistTrack).track_source;
      } else {
        source = (track as Track).source || (playlist as Playlist).source;
      }

      const normalizedSource = source.toLowerCase();

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
      const originalIndex = unfilteredTracks.findIndex(
        (t) => t.id === trackToPlay.id,
      );
      await tauriAPI.playPlaylistFromTrack(unfilteredTracks, originalIndex);
      await playback.updateStatus();
    } catch (err) {
      console.error("Failed to play from track:", err);
    }
  };

  const isLoading = isCustom ? customLoading : loading;
  const trackCount = "track_count" in playlist ? playlist.track_count : 0;

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
    </div>
  );
}
