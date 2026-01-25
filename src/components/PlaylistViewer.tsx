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
import "./CustomPlaylistEditor.css";

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

  // Load tracks for regular playlists
  useEffect(() => {
    if (!isCustom) {
      const regularPlaylist = playlist as Playlist;
      const loadTracks = async () => {
        setLoading(true);
        try {
          const fullPlaylist =
            regularPlaylist.source === "spotify"
              ? await tauriAPI.getSpotifyPlaylist(regularPlaylist.id)
              : await tauriAPI.getJellyfinPlaylist(regularPlaylist.id);

          setRegularTracks(fullPlaylist.tracks || []);
        } catch (err) {
          console.error("Failed to load playlist tracks:", err);
        } finally {
          setLoading(false);
        }
      };
      void loadTracks();
    }
  }, [isCustom, playlist.id, playlist]);

  const handleRefresh = async () => {
    if (isCustom && refreshCustomTracks) {
      await refreshCustomTracks(true);
    } else if (!isCustom) {
      // For regular playlists, reload tracks
      const regularPlaylist = playlist as Playlist;
      setLoading(true);
      try {
        const fullPlaylist =
          regularPlaylist.source === "spotify"
            ? await tauriAPI.getSpotifyPlaylist(regularPlaylist.id)
            : await tauriAPI.getJellyfinPlaylist(regularPlaylist.id);
        setRegularTracks(fullPlaylist.tracks || []);
      } catch (err) {
        console.error("Failed to reload playlist tracks:", err);
      } finally {
        setLoading(false);
      }
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
        // Custom playlist track has source in track_source
        source = (track as PlaylistTrack).track_source;
      } else {
        // Regular playlist track has source property
        source = (track as Track).source || (playlist as Playlist).source;
      }

      // Normalize source to lowercase for backend
      const normalizedSource = source.toLowerCase();

      await playback.playTrack(trackId, normalizedSource);
      await playback.updateStatus();
    } catch (err) {
      console.error("Failed to play track:", err);
    }
  };

  const handlePlayFromTrack = async (index: number) => {
    try {
      await tauriAPI.playPlaylistFromTrack(tracks, index);
      await playback.updateStatus();
    } catch (err) {
      console.error("Failed to play from track:", err);
    }
  };

  const isLoading = isCustom ? customLoading : loading;
  const trackCount = "track_count" in playlist ? playlist.track_count : 0;

  // Filter tracks based on search query - handle types separately
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
