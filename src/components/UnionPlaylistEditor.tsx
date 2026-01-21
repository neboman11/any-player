import { useState, useEffect } from "react";
import { TrackTable } from "./TrackTable";
import {
  useUnionPlaylistSources,
  useUnionPlaylistTracks,
  useCustomPlaylists,
} from "../hooks";
import { tauriAPI } from "../api";
import type {
  CustomPlaylist,
  Track,
  UnionPlaylistSource,
  Playlist,
  PlaylistTrack,
} from "../types";
import "./CustomPlaylistEditor.css";

interface UnionPlaylistEditorProps {
  playlist: CustomPlaylist;
  onBack: () => void;
  onUpdate: (
    name: string | null,
    description: string | null,
    imageUrl: string | null,
  ) => Promise<void>;
  onDelete: () => Promise<void>;
}

export function UnionPlaylistEditor({
  playlist,
  onBack,
  onUpdate,
  onDelete,
}: UnionPlaylistEditorProps) {
  const {
    sources,
    loading: sourcesLoading,
    addSource,
    removeSource,
  } = useUnionPlaylistSources(playlist.id);
  const { tracks, loading: tracksLoading } = useUnionPlaylistTracks(
    playlist.id,
  );
  const { playlists: customPlaylists } = useCustomPlaylists();
  const [isEditing, setIsEditing] = useState(false);
  const [editName, setEditName] = useState(playlist.name);
  const [editDescription, setEditDescription] = useState(
    playlist.description || "",
  );
  const [showAddSource, setShowAddSource] = useState(false);
  const [selectedSourceType, setSelectedSourceType] = useState<string>("all");
  const [availablePlaylists, setAvailablePlaylists] = useState<Playlist[]>([]);

  // Load playlists when needed
  useEffect(() => {
    if (showAddSource) {
      loadExternalPlaylists();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showAddSource, selectedSourceType]);

  const loadExternalPlaylists = async () => {
    const playlists: Playlist[] = [];

    try {
      // Add custom playlists (excluding current and union types)
      if (selectedSourceType === "all" || selectedSourceType === "custom") {
        const filtered = customPlaylists.filter(
          (p) => p.id !== playlist.id && p.playlist_type !== "union",
        );
        playlists.push(
          ...filtered.map((p) => ({
            id: p.id,
            name: p.name,
            owner: "You",
            track_count: p.track_count,
            source: "custom" as const,
            description: p.description || undefined,
            image_url: p.image_url || undefined,
          })),
        );
      }

      // Add external playlists
      if (selectedSourceType === "all" || selectedSourceType === "spotify") {
        const spotifyAuth = await tauriAPI.isSpotifyAuthenticated();
        if (spotifyAuth) {
          const spotifyPlaylists = await tauriAPI.getSpotifyPlaylists();
          playlists.push(...spotifyPlaylists);
        }
      }

      if (selectedSourceType === "all" || selectedSourceType === "jellyfin") {
        const jellyfinAuth = await tauriAPI.isJellyfinAuthenticated();
        if (jellyfinAuth) {
          const jellyfinPlaylists = await tauriAPI.getJellyfinPlaylists();
          playlists.push(...jellyfinPlaylists);
        }
      }

      setAvailablePlaylists(playlists);
    } catch (err) {
      console.error("Failed to load playlists:", err);
    }
  };

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

  const handleAddSource = async (playlistId: string, sourceType: string) => {
    try {
      await addSource(sourceType, playlistId);
      setShowAddSource(false);
    } catch (err) {
      console.error("Failed to add source:", err);
      alert("Failed to add source playlist");
    }
  };

  const handleRemoveSource = async (sourceId: number) => {
    if (!confirm("Remove this playlist from the union?")) {
      return;
    }

    try {
      await removeSource(sourceId);
    } catch (err) {
      console.error("Failed to remove source:", err);
      alert("Failed to remove source playlist");
    }
  };

  const handlePlayTrack = (track: Track | PlaylistTrack) => {
    // TODO: Implement track playback
    console.log("Play track:", track);
  };

  const getSourcePlaylistName = (source: UnionPlaylistSource): string => {
    // Try to find in available playlists or custom playlists
    const allPlaylists = [...availablePlaylists];

    // Add custom playlists
    customPlaylists.forEach((p) => {
      allPlaylists.push({
        id: p.id,
        name: p.name,
        owner: "You",
        track_count: p.track_count,
        source: "custom" as const,
      });
    });

    const found = allPlaylists.find((p) => p.id === source.source_playlist_id);
    return found ? found.name : source.source_playlist_id;
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
              <h2>
                {playlist.name} <span className="union-badge">UNION</span>
              </h2>
              {playlist.description && (
                <p className="playlist-description">{playlist.description}</p>
              )}
              <p className="playlist-meta">
                {sources.length} source playlists • {tracks.length} total tracks
                • Created{" "}
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
                onClick={() => setShowAddSource(!showAddSource)}
              >
                + Add Playlist
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

      {showAddSource && (
        <div className="add-track-panel">
          <h3>Add Playlist to Union</h3>
          <div className="source-filter">
            <label>Filter by source:</label>
            <select
              value={selectedSourceType}
              onChange={(e) => setSelectedSourceType(e.target.value)}
            >
              <option value="all">All Sources</option>
              <option value="spotify">Spotify</option>
              <option value="jellyfin">Jellyfin</option>
              <option value="custom">Custom</option>
            </select>
          </div>
          <div className="available-playlists">
            {availablePlaylists.length === 0 ? (
              <p>No playlists available</p>
            ) : (
              <ul>
                {availablePlaylists.map((pl) => (
                  <li key={`${pl.source}-${pl.id}`}>
                    <span>
                      {pl.name} ({pl.source})
                    </span>
                    <button
                      onClick={() => handleAddSource(pl.id, pl.source)}
                      className="add-btn"
                    >
                      Add
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <button onClick={() => setShowAddSource(false)}>Close</button>
        </div>
      )}

      {/* Source playlists section */}
      <div className="sources-section">
        <h3>Source Playlists</h3>
        {sourcesLoading ? (
          <div className="loading">Loading sources...</div>
        ) : sources.length === 0 ? (
          <div className="empty-state">
            No source playlists added yet. Click "Add Playlist" to start
            building your union.
          </div>
        ) : (
          <div className="source-list">
            {sources.map((source) => (
              <div key={source.id} className="source-item">
                <div className="source-info">
                  <span className="source-name">
                    {getSourcePlaylistName(source)}
                  </span>
                  <span className="source-type-badge">
                    {source.source_type}
                  </span>
                </div>
                <button
                  className="remove-btn"
                  onClick={() => handleRemoveSource(source.id)}
                >
                  Remove
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Combined tracks section */}
      <div className="tracks-section">
        <h3>Combined Tracks ({tracks.length})</h3>
        {tracksLoading ? (
          <div className="loading">Loading tracks...</div>
        ) : tracks.length === 0 ? (
          <div className="empty-state">
            No tracks available. Add source playlists to see their tracks here.
          </div>
        ) : (
          <TrackTable tracks={tracks} onPlayTrack={handlePlayTrack} />
        )}
      </div>
    </div>
  );
}
