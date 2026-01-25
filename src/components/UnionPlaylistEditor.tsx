import { useState, useEffect } from "react";
import toast from "react-hot-toast";
import { TrackTable } from "./TrackTable";
import { PlaylistHeader, DeleteConfirmModal, SearchBar } from "./shared";
import {
  useUnionPlaylistSources,
  useUnionPlaylistTracks,
  useCustomPlaylists,
  usePlayback,
  usePlaylistEditor,
} from "../hooks";
import { tauriAPI } from "../api";
import { filterTracks } from "../utils/trackFilters";
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
  const {
    tracks,
    loading: tracksLoading,
    refresh: refreshTracks,
  } = useUnionPlaylistTracks(playlist.id);
  const { playlists: customPlaylists } = useCustomPlaylists();
  const playback = usePlayback();

  const editorState = usePlaylistEditor({
    playlistName: playlist.name,
    playlistDescription: playlist.description,
    onUpdate,
    onDelete,
    onBack,
  });

  const [showAddSource, setShowAddSource] = useState(false);
  const [selectedSourceType, setSelectedSourceType] = useState<string>("all");
  const [availablePlaylists, setAvailablePlaylists] = useState<Playlist[]>([]);
  const [showRemoveSourceConfirm, setShowRemoveSourceConfirm] = useState<
    number | null
  >(null);
  const [searchQuery, setSearchQuery] = useState("");

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

  const handleAddSource = async (playlistId: string, sourceType: string) => {
    try {
      await addSource(sourceType, playlistId);
      setShowAddSource(false);
      toast.success("Playlist added to union");
    } catch (err) {
      console.error("Failed to add source:", err);
      toast.error("Failed to add source playlist");
    }
  };

  const handleRefresh = async () => {
    await refreshTracks(true);
  };

  const handleRemoveSource = async (sourceId: number) => {
    try {
      await removeSource(sourceId);
      setShowRemoveSourceConfirm(null);
      toast.success("Playlist removed from union");
    } catch (err) {
      console.error("Failed to remove source:", err);
      toast.error("Failed to remove source playlist");
    }
  };

  const handlePlayPlaylist = async () => {
    try {
      // For union playlists, send the cached tracks directly to start playback immediately
      // The backend will start playing right away and enrich track details in the background
      if (tracks.length === 0) {
        toast.error("No tracks in this union playlist");
        return;
      }

      // Use the optimized immediate playback API
      await tauriAPI.playTracksImmediate(tracks);
      await playback.updateStatus();
    } catch (err) {
      console.error("Failed to play union playlist:", err);
      toast.error("Failed to play playlist");
    }
  };

  const handlePlayTrack = async (track: Track | PlaylistTrack) => {
    try {
      const trackId = String(track.id);
      const source = (track as Track).source || "custom";
      // Normalize source to lowercase
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

  // Filter tracks based on search query
  const filteredTracks = filterTracks(tracks, searchQuery);

  const metaInfo = `${sources.length} source playlists • ${tracks.length} total tracks • Created ${new Date(playlist.created_at * 1000).toLocaleDateString()}`;

  return (
    <div className="custom-playlist-editor">
      <PlaylistHeader
        isEditing={editorState.isEditing}
        editName={editorState.editName}
        editDescription={editorState.editDescription}
        playlistName={playlist.name}
        playlistDescription={playlist.description}
        metaInfo={metaInfo}
        badge={<span className="union-badge">UNION</span>}
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
              <button className="play-btn" onClick={handlePlayPlaylist}>
                ▶ Play All
              </button>
              <button
                className="add-track-btn"
                onClick={() => setShowAddSource(!showAddSource)}
              >
                + Add Playlist
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
                  onClick={() => setShowRemoveSourceConfirm(source.id)}
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
        <SearchBar value={searchQuery} onChange={setSearchQuery} />
        {tracksLoading ? (
          <div className="loading">Loading tracks...</div>
        ) : tracks.length === 0 ? (
          <div className="empty-state">
            No tracks available. Add source playlists to see their tracks here.
          </div>
        ) : (
          <TrackTable
            tracks={filteredTracks}
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

      <DeleteConfirmModal
        show={showRemoveSourceConfirm !== null}
        title="Remove Playlist"
        message="Remove this playlist from the union?"
        confirmLabel="Remove"
        onConfirm={() =>
          showRemoveSourceConfirm !== null &&
          handleRemoveSource(showRemoveSourceConfirm)
        }
        onCancel={() => setShowRemoveSourceConfirm(null)}
      />
    </div>
  );
}
