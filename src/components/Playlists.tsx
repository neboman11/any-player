import { useState, useCallback, useEffect } from "react";
import { usePlaylists, useCustomPlaylists } from "../hooks";
import { PlaylistViewer } from "./PlaylistViewer";
import { UnionPlaylistEditor } from "./UnionPlaylistEditor";
import { tauriAPI } from "../api";
import type { TauriSource, CustomPlaylist, Playlist } from "../types";

export function Playlists() {
  const [activeSource, setActiveSource] = useState<TauriSource>("all");
  const [selectedCustomPlaylist, setSelectedCustomPlaylist] =
    useState<CustomPlaylist | null>(null);
  const [selectedRegularPlaylist, setSelectedRegularPlaylist] =
    useState<Playlist | null>(null);
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [showCreateTypeDialog, setShowCreateTypeDialog] = useState(false);
  const [newPlaylistName, setNewPlaylistName] = useState("");

  const {
    playlists,
    isLoading,
    error,
    loadPlaylists,
    refreshKey,
    isCached,
    refresh,
  } = usePlaylists();

  const {
    playlists: customPlaylists,
    loading: customLoading,
    error: customError,
    createPlaylist,
    updatePlaylist,
    deletePlaylist,
    refresh: refreshCustomPlaylists,
  } = useCustomPlaylists();

  const sources: TauriSource[] = ["all", "custom", "spotify", "jellyfin"];

  // Load playlists from cache on mount, or reload if source changes or refresh is requested
  useEffect(() => {
    // If cache is available and we're on the initial load, use it
    // Only reload when activeSource changes or when refresh is explicitly requested
    if (!isCached || refreshKey > 0) {
      void loadPlaylists(activeSource, refreshKey > 0);
    }
  }, [activeSource, loadPlaylists, refreshKey, isCached]);

  const handlePlaylistClick = useCallback(
    async (playlistId: string, source: string) => {
      // If it's a custom playlist, open it in the viewer
      if (source === "custom") {
        const playlist = customPlaylists.find((p) => p.id === playlistId);
        if (playlist) {
          setSelectedCustomPlaylist(playlist);
        }
        return;
      }

      // For regular playlists, find and open in viewer
      const playlist = playlists.find((p) => p.id === playlistId);
      if (playlist) {
        setSelectedRegularPlaylist(playlist);
      }
    },
    [playlists, customPlaylists],
  );

  const handleCreatePlaylist = async () => {
    if (!newPlaylistName.trim()) return;

    try {
      await createPlaylist(newPlaylistName.trim());
      setNewPlaylistName("");
      setShowCreateDialog(false);
    } catch (err) {
      console.error("Failed to create playlist:", err);
      alert("Failed to create playlist");
    }
  };

  const handleCreateUnionPlaylist = async () => {
    if (!newPlaylistName.trim()) return;

    try {
      const newPlaylist = await tauriAPI.createUnionPlaylist(
        newPlaylistName.trim(),
        null,
        null,
      );
      setNewPlaylistName("");
      setShowCreateTypeDialog(false);
      // Open the newly created union playlist for editing
      setSelectedCustomPlaylist(newPlaylist);
    } catch (err) {
      console.error("Failed to create union playlist:", err);
      alert("Failed to create union playlist");
    }
  };

  const handleUpdatePlaylist = async (
    name: string | null,
    description: string | null,
    imageUrl: string | null,
  ) => {
    if (!selectedCustomPlaylist) return;
    await updatePlaylist(
      selectedCustomPlaylist.id,
      name,
      description,
      imageUrl,
    );
  };

  const handleDeletePlaylist = async () => {
    if (!selectedCustomPlaylist) return;
    await deletePlaylist(selectedCustomPlaylist.id);
    setSelectedCustomPlaylist(null);
  };

  // If viewing a custom playlist, check if it's a union playlist
  if (selectedCustomPlaylist) {
    if (selectedCustomPlaylist.playlist_type === "union") {
      return (
        <UnionPlaylistEditor
          playlist={selectedCustomPlaylist}
          onBack={() => setSelectedCustomPlaylist(null)}
          onUpdate={handleUpdatePlaylist}
          onDelete={handleDeletePlaylist}
        />
      );
    }

    return (
      <PlaylistViewer
        playlist={selectedCustomPlaylist}
        isCustom={true}
        onBack={() => setSelectedCustomPlaylist(null)}
        onUpdate={handleUpdatePlaylist}
        onDelete={handleDeletePlaylist}
      />
    );
  }

  // If viewing a regular playlist, show the viewer
  if (selectedRegularPlaylist) {
    return (
      <PlaylistViewer
        playlist={selectedRegularPlaylist}
        isCustom={false}
        onBack={() => setSelectedRegularPlaylist(null)}
      />
    );
  }

  // Get playlists to display based on active source
  const getDisplayPlaylists = () => {
    if (activeSource === "custom") {
      return customPlaylists.map((p) => ({
        id: p.id,
        name: p.name,
        owner: "You",
        track_count: p.track_count,
        source: "custom" as const,
        description: p.description || undefined,
      }));
    }

    if (activeSource === "all") {
      const customAsList = customPlaylists.map((p) => ({
        id: p.id,
        name: p.name,
        owner: "You",
        track_count: p.track_count,
        source: "custom" as const,
        description: p.description || undefined,
      }));
      return [...customAsList, ...playlists];
    }

    return playlists;
  };

  const displayPlaylists = getDisplayPlaylists();
  const isAnyLoading = isLoading || customLoading;
  const anyError = error || customError;

  return (
    <section id="playlists" className="page">
      <div className="playlists-container">
        <div className="playlists-header">
          <h2>Your Playlists</h2>
          <div style={{ display: "flex", gap: "8px" }}>
            <button
              className="refresh-btn"
              onClick={async () => {
                refresh();
                await refreshCustomPlaylists(true);
              }}
              title="Refresh playlists"
              style={{
                padding: "8px 12px",
                backgroundColor: "#333",
                border: "1px solid #555",
                borderRadius: "4px",
                cursor: "pointer",
                color: "#fff",
              }}
            >
              🔄 Refresh
            </button>
            <button
              className="create-playlist-btn"
              onClick={() => setShowCreateTypeDialog(true)}
            >
              + Create Playlist
            </button>
          </div>
        </div>

        {showCreateTypeDialog && (
          <div className="create-dialog">
            <h3>Create New Playlist</h3>
            <div className="playlist-type-buttons">
              <button
                className="type-btn"
                onClick={() => {
                  setShowCreateTypeDialog(false);
                  setShowCreateDialog(true);
                }}
              >
                <strong>Standard Playlist</strong>
                <small>Store tracks directly in this playlist</small>
              </button>
              <button
                className="type-btn"
                onClick={() => {
                  setShowCreateTypeDialog(false);
                  setShowCreateDialog(true);
                }}
              >
                <strong>Union Playlist</strong>
                <small>Combine tracks from multiple playlists</small>
              </button>
            </div>
            <div className="dialog-actions">
              <button onClick={() => setShowCreateTypeDialog(false)}>
                Cancel
              </button>
            </div>
          </div>
        )}

        {showCreateDialog && (
          <div className="create-dialog">
            <h3>Create New Playlist</h3>
            <input
              type="text"
              placeholder="Playlist name"
              value={newPlaylistName}
              onChange={(e) => setNewPlaylistName(e.target.value)}
              onKeyPress={(e) => e.key === "Enter" && handleCreatePlaylist()}
            />
            <div className="dialog-actions">
              <button onClick={handleCreatePlaylist}>Create Standard</button>
              <button onClick={handleCreateUnionPlaylist}>Create Union</button>
              <button
                onClick={() => {
                  setShowCreateDialog(false);
                  setNewPlaylistName("");
                }}
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        <div className="playlist-tabs">
          {sources.map((source) => (
            <button
              key={source}
              className={`tab-btn ${activeSource === source ? "active" : ""}`}
              data-source={source}
              onClick={() => setActiveSource(source)}
            >
              {source.charAt(0).toUpperCase() + source.slice(1)}
            </button>
          ))}
        </div>
        <div className="playlists-grid" id="playlists-grid">
          {isAnyLoading && (
            <div className="playlist-card loading">Loading playlists...</div>
          )}
          {anyError && !isAnyLoading && (
            <div className="playlist-card">{anyError}</div>
          )}
          {!isAnyLoading && !anyError && displayPlaylists.length === 0 && (
            <div className="playlist-card">
              {activeSource === "custom"
                ? "No custom playlists. Create one to get started!"
                : "No playlists found. Connect a service in Settings."}
            </div>
          )}
          {displayPlaylists.map((playlist) => (
            <div
              key={`${playlist.source}-${playlist.id}`}
              className="playlist-card"
              onClick={() => handlePlaylistClick(playlist.id, playlist.source)}
              style={{ cursor: "pointer" }}
            >
              <h4>{playlist.name}</h4>
              <p>{playlist.owner}</p>
              <p>{playlist.track_count} tracks</p>
              <small>{playlist.source}</small>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
