import { useState, useEffect } from "react";
import { TrackTable } from "./TrackTable";
import { useCustomPlaylistTracks } from "../hooks";
import { usePlayback } from "../hooks";
import { tauriAPI } from "../api";
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
  } = useCustomPlaylistTracks(customPlaylistId);

  const [isEditing, setIsEditing] = useState(false);
  const [editName, setEditName] = useState(playlist.name);
  const [editDescription, setEditDescription] = useState(
    "description" in playlist ? playlist.description || "" : "",
  );
  const [showAddTrack, setShowAddTrack] = useState(false);
  const [regularTracks, setRegularTracks] = useState<Track[]>([]);
  const [loading, setLoading] = useState(false);

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
  }, [isCustom, playlist.id]);

  const handleSaveEdit = async () => {
    if (!onUpdate) return;
    try {
      await onUpdate(
        editName !== playlist.name ? editName : null,
        editDescription !==
          ("description" in playlist ? playlist.description || "" : "")
          ? editDescription
          : null,
        null,
      );
      setIsEditing(false);
    } catch (err) {
      console.error("Failed to update playlist:", err);
      alert("Failed to update playlist");
    }
  };

  const handleDelete = async () => {
    if (!onDelete) return;
    if (
      !confirm(
        `Are you sure you want to delete "${playlist.name}"? This cannot be undone.`,
      )
    ) {
      return;
    }

    try {
      await onDelete();
      onBack();
    } catch (err) {
      console.error("Failed to delete playlist:", err);
      alert("Failed to delete playlist");
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

      // Capitalize source for backend
      const capitalizedSource =
        source.charAt(0).toUpperCase() + source.slice(1);

      await playback.playTrack(trackId, capitalizedSource);
      await playback.updateStatus();
    } catch (err) {
      console.error("Failed to play track:", err);
    }
  };

  const tracks = isCustom ? customTracks : regularTracks;
  const isLoading = isCustom ? customLoading : loading;
  const trackCount = "track_count" in playlist ? playlist.track_count : 0;

  return (
    <div className="custom-playlist-editor">
      <div className="editor-header">
        <button className="back-btn" onClick={onBack}>
          ← Back
        </button>

        <div className="playlist-info">
          {isEditing && isCustom ? (
            <div className="edit-form">
              <input
                type="text"
                value={editName}
                onChange={(e) => setEditName(e.target.value)}
                placeholder="Playlist name"
                className="edit-name-input"
              />
              <textarea
                value={editDescription}
                onChange={(e) => setEditDescription(e.target.value)}
                placeholder="Description (optional)"
                className="edit-description-input"
                rows={3}
              />
              <div className="edit-actions">
                <button className="save-btn" onClick={handleSaveEdit}>
                  Save
                </button>
                <button
                  className="cancel-btn"
                  onClick={() => {
                    setIsEditing(false);
                    setEditName(playlist.name);
                    setEditDescription(
                      "description" in playlist
                        ? playlist.description || ""
                        : "",
                    );
                  }}
                >
                  Cancel
                </button>
              </div>
            </div>
          ) : (
            <>
              <h2>{playlist.name}</h2>
              {"description" in playlist && playlist.description && (
                <p className="playlist-description">{playlist.description}</p>
              )}
              <p className="playlist-meta">
                {trackCount} tracks •{" "}
                {isCustom ? "You" : (playlist as Playlist).owner}
                {"created_at" in playlist &&
                  ` • Created ${new Date((playlist as CustomPlaylist).created_at * 1000).toLocaleDateString()}`}
              </p>
            </>
          )}
        </div>

        <div className="header-actions">
          <button className="play-btn" onClick={handlePlayPlaylist}>
            ▶ Play All
          </button>
          {isCustom && !isEditing && (
            <>
              <button
                className="add-track-btn"
                onClick={() => setShowAddTrack(!showAddTrack)}
              >
                + Add Track
              </button>
              <button className="edit-btn" onClick={() => setIsEditing(true)}>
                Edit
              </button>
              <button className="delete-btn" onClick={handleDelete}>
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
        {isLoading ? (
          <div className="loading">Loading tracks...</div>
        ) : (
          <TrackTable
            tracks={tracks}
            onRemoveTrack={isCustom ? removeTrack : undefined}
            onReorderTrack={isCustom ? reorderTrack : undefined}
            onPlayTrack={handlePlayTrack}
          />
        )}
      </div>
    </div>
  );
}
