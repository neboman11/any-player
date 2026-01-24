import { useState, useEffect } from "react";
import toast from "react-hot-toast";
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
    refresh: refreshCustomTracks,
  } = useCustomPlaylistTracks(customPlaylistId);

  const [isEditing, setIsEditing] = useState(false);
  const [editName, setEditName] = useState(playlist.name);
  const [editDescription, setEditDescription] = useState(
    "description" in playlist ? playlist.description || "" : "",
  );
  const [showAddTrack, setShowAddTrack] = useState(false);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
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
      toast.success("Playlist updated successfully!");
    } catch (err) {
      console.error("Failed to update playlist:", err);
      toast.error("Failed to update playlist");
    }
  };

  const handleDelete = async () => {
    if (!onDelete) return;

    try {
      await onDelete();
      onBack();
      toast.success("Playlist deleted");
    } catch (err) {
      console.error("Failed to delete playlist:", err);
      toast.error("Failed to delete playlist");
    }
  };

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

  const allTracks = isCustom ? customTracks : regularTracks;
  const isLoading = isCustom ? customLoading : loading;
  const trackCount = "track_count" in playlist ? playlist.track_count : 0;

  // Filter tracks based on search query
  const tracks =
    searchQuery.trim() === ""
      ? allTracks
      : allTracks.filter((track) => {
          const query = searchQuery.toLowerCase();
          const title = track.title?.toLowerCase() || "";
          const artist = track.artist?.toLowerCase() || "";
          const album = track.album?.toLowerCase() || "";
          const source =
            ("track_source" in track
              ? track.track_source
              : (track as Track).source
            )?.toLowerCase() || "";

          return (
            title.includes(query) ||
            artist.includes(query) ||
            album.includes(query) ||
            source.includes(query)
          );
        });

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
          <button
            className="refresh-btn"
            onClick={handleRefresh}
            title="Refresh tracks"
          >
            ⟳ Refresh
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
              <button
                className="delete-btn"
                onClick={() => setShowDeleteConfirm(true)}
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
        <div
          className="search-container"
          style={{ padding: "10px", marginBottom: "10px" }}
        >
          <input
            type="text"
            placeholder="Search tracks..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            style={{
              width: "100%",
              padding: "8px 12px",
              fontSize: "14px",
              border: "1px solid #444",
              borderRadius: "4px",
              backgroundColor: "#1e1e1e",
              color: "#fff",
            }}
          />
        </div>
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

      {/* Delete confirmation modal */}
      {showDeleteConfirm && (
        <div
          className="modal-overlay"
          onClick={() => setShowDeleteConfirm(false)}
        >
          <div className="modal-content" onClick={(e) => e.stopPropagation()}>
            <h3>Delete Playlist</h3>
            <p>
              Are you sure you want to delete "{playlist.name}"? This cannot be
              undone.
            </p>
            <div className="modal-actions">
              <button
                className="confirm-btn"
                onClick={() => {
                  setShowDeleteConfirm(false);
                  handleDelete();
                }}
              >
                Delete
              </button>
              <button
                className="cancel-btn"
                onClick={() => setShowDeleteConfirm(false)}
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
