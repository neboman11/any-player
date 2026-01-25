import { useState } from "react";
import { TrackTable } from "./TrackTable";
import { PlaylistHeader, DeleteConfirmModal } from "./shared";
import { useCustomPlaylistTracks, usePlaylistEditor } from "../hooks";
import type { CustomPlaylist, PlaylistTrack, Track } from "../types";
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
  const { tracks, loading, removeTrack, reorderTrack, refresh } =
    useCustomPlaylistTracks(playlist.id);

  const editorState = usePlaylistEditor({
    playlistName: playlist.name,
    playlistDescription: playlist.description,
    onUpdate,
    onDelete,
    onBack,
  });

  const [showAddTrack, setShowAddTrack] = useState(false);

  const handlePlayTrack = (track: PlaylistTrack | Track) => {
    // TODO: Implement track playback
    console.log("Play track:", track);
  };

  const handleRefresh = async () => {
    await refresh(true);
  };

  const metaInfo = `${playlist.track_count} tracks • Created ${new Date(playlist.created_at * 1000).toLocaleDateString()}`;

  return (
    <div className="custom-playlist-editor">
      <PlaylistHeader
        isEditing={editorState.isEditing}
        editName={editorState.editName}
        editDescription={editorState.editDescription}
        playlistName={playlist.name}
        playlistDescription={playlist.description}
        metaInfo={metaInfo}
        onEditNameChange={editorState.setEditName}
        onEditDescriptionChange={editorState.setEditDescription}
        onSave={editorState.handleSaveEdit}
        onCancelEdit={editorState.handleCancelEdit}
        onBack={onBack}
      />

      <div className="editor-header">
        <div className="header-actions">
          {!editorState.isEditing && (
            <>
              <button
                className="add-track-btn"
                onClick={() => setShowAddTrack(!showAddTrack)}
              >
                + Add Track
              </button>
              <button
                className="refresh-btn"
                onClick={handleRefresh}
                title="Refresh tracks"
              >
                ⟳ Refresh
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
