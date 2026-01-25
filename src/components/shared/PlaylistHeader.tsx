import { ReactNode } from "react";

interface PlaylistHeaderProps {
  isEditing: boolean;
  editName: string;
  editDescription: string;
  playlistName: string;
  playlistDescription?: string | null;
  metaInfo: string;
  badge?: ReactNode;
  onEditNameChange: (value: string) => void;
  onEditDescriptionChange: (value: string) => void;
  onSave: () => void;
  onCancelEdit: () => void;
  onBack: () => void;
}

export function PlaylistHeader({
  isEditing,
  editName,
  editDescription,
  playlistName,
  playlistDescription,
  metaInfo,
  badge,
  onEditNameChange,
  onEditDescriptionChange,
  onSave,
  onCancelEdit,
  onBack,
}: PlaylistHeaderProps) {
  return (
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
              onChange={(e) => onEditNameChange(e.target.value)}
              placeholder="Playlist name"
              className="edit-name-input"
            />
            <textarea
              value={editDescription}
              onChange={(e) => onEditDescriptionChange(e.target.value)}
              placeholder="Description (optional)"
              className="edit-description-input"
              rows={3}
            />
            <div className="edit-actions">
              <button className="save-btn" onClick={onSave}>
                Save
              </button>
              <button className="cancel-btn" onClick={onCancelEdit}>
                Cancel
              </button>
            </div>
          </div>
        ) : (
          <>
            <h2>
              {playlistName} {badge}
            </h2>
            {playlistDescription && (
              <p className="playlist-description">{playlistDescription}</p>
            )}
            <p className="playlist-meta">{metaInfo}</p>
          </>
        )}
      </div>
    </div>
  );
}
