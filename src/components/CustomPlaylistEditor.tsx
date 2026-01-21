import { useState } from "react";
import { TrackTable } from "./TrackTable";
import { useCustomPlaylistTracks } from "../hooks";
import type { CustomPlaylist, PlaylistTrack } from "../types";
import "./CustomPlaylistEditor.css";

interface CustomPlaylistEditorProps {
  playlist: CustomPlaylist;
  onBack: () => void;
  onUpdate: (
    name: string | null,
    description: string | null,
    imageUrl: string | null,
  ) => Promise<void>;
  onDelete: () => Promise<void>;
}

export function CustomPlaylistEditor({
  playlist,
  onBack,
  onUpdate,
  onDelete,
}: CustomPlaylistEditorProps) {
  const { tracks, loading, removeTrack, reorderTrack } =
    useCustomPlaylistTracks(playlist.id);
  const [isEditing, setIsEditing] = useState(false);
  const [editName, setEditName] = useState(playlist.name);
  const [editDescription, setEditDescription] = useState(
    playlist.description || "",
  );
  const [showAddTrack, setShowAddTrack] = useState(false);

  const handleSaveEdit = async () => {
    try {
      await onUpdate(
        editName !== playlist.name ? editName : null,
        editDescription !== (playlist.description || "")
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

  const handlePlayTrack = (track: PlaylistTrack) => {
    // TODO: Implement track playback
    console.log("Play track:", track);
  };

  return (
    <div className="custom-playlist-editor">
      <div className="editor-header">
        <button className="back-btn" onClick={onBack}>
          ← Back
        </button>

        <div className="playlist-info">
          {isEditing ? (
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
                    setEditDescription(playlist.description || "");
                  }}
                >
                  Cancel
                </button>
              </div>
            </div>
          ) : (
            <>
              <h2>{playlist.name}</h2>
              {playlist.description && (
                <p className="playlist-description">{playlist.description}</p>
              )}
              <p className="playlist-meta">
                {playlist.track_count} tracks • Created{" "}
                {new Date(playlist.created_at * 1000).toLocaleDateString()}
              </p>
            </>
          )}
        </div>

        <div className="header-actions">
          {!isEditing && (
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
        {loading ? (
          <div className="loading">Loading tracks...</div>
        ) : (
          <TrackTable
            tracks={tracks}
            onRemoveTrack={removeTrack}
            onReorderTrack={reorderTrack}
            onPlayTrack={handlePlayTrack}
          />
        )}
      </div>
    </div>
  );
}
