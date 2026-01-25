import { useState } from "react";
import toast from "react-hot-toast";

interface UsePlaylistEditorOptions {
  playlistName: string;
  playlistDescription: string | null | undefined;
  onUpdate?: (
    name: string | null,
    description: string | null,
    imageUrl: string | null,
  ) => Promise<void>;
  onDelete?: () => Promise<void>;
  onBack?: () => void;
}

export function usePlaylistEditor({
  playlistName,
  playlistDescription,
  onUpdate,
  onDelete,
  onBack,
}: UsePlaylistEditorOptions) {
  const [isEditing, setIsEditing] = useState(false);
  const [editName, setEditName] = useState(playlistName);
  const [editDescription, setEditDescription] = useState(
    playlistDescription || "",
  );
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);

  const handleSaveEdit = async () => {
    if (!onUpdate) return;
    try {
      await onUpdate(
        editName !== playlistName ? editName : null,
        editDescription !== (playlistDescription || "")
          ? editDescription
          : null,
        null,
      );
      setIsEditing(false);
      toast.success("Playlist updated successfully!");
    } catch (err) {
      console.error("Failed to update playlist:", err);
      toast.error("Failed to update playlist");
    }
  };

  const handleCancelEdit = () => {
    setIsEditing(false);
    setEditName(playlistName);
    setEditDescription(playlistDescription || "");
  };

  const handleDelete = async () => {
    if (!onDelete) return;

    try {
      await onDelete();
      setShowDeleteConfirm(false);
      if (onBack) onBack();
      toast.success("Playlist deleted");
    } catch (err) {
      console.error("Failed to delete playlist:", err);
      toast.error("Failed to delete playlist");
    }
  };

  return {
    isEditing,
    setIsEditing,
    editName,
    setEditName,
    editDescription,
    setEditDescription,
    showDeleteConfirm,
    setShowDeleteConfirm,
    handleSaveEdit,
    handleCancelEdit,
    handleDelete,
  };
}
