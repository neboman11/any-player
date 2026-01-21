import { useState, useCallback, useEffect } from "react";
import { usePlaylists, useCustomPlaylists } from "../hooks";
import { usePlayback } from "../hooks";
import { CustomPlaylistEditor } from "./CustomPlaylistEditor";
import type { TauriSource, CustomPlaylist } from "../types";

export function Playlists() {
  const [activeSource, setActiveSource] = useState<TauriSource>("all");
  const [selectedCustomPlaylist, setSelectedCustomPlaylist] =
    useState<CustomPlaylist | null>(null);
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [newPlaylistName, setNewPlaylistName] = useState("");

  const {
    playlists,
    isLoading,
    error,
    loadPlaylists,
    playPlaylist,
    refreshKey,
  } = usePlaylists();

  const {
    playlists: customPlaylists,
    loading: customLoading,
    error: customError,
    createPlaylist,
    updatePlaylist,
    deletePlaylist,
  } = useCustomPlaylists();

  const playback = usePlayback();
  const sources: TauriSource[] = ["all", "custom", "spotify", "jellyfin"];

  // Reload playlists when activeSource or refreshKey changes
  useEffect(() => {
    void loadPlaylists(activeSource);
  }, [activeSource, loadPlaylists, refreshKey]);

  const handlePlaylistClick = useCallback(
    async (playlistId: string, source: string) => {
      // If it's a custom playlist, open the editor
      if (source === "custom") {
        const playlist = customPlaylists.find((p) => p.id === playlistId);
        if (playlist) {
          setSelectedCustomPlaylist(playlist);
        }
        return;
      }

      try {
        console.log("Playing playlist:", playlistId, "from", source);

        // Use the new playPlaylist method which handles everything
        await playPlaylist(playlistId, source as TauriSource);

        // Update playback status
        await playback.updateStatus();

        console.log("Playlist playback started");
      } catch (err) {
        console.error("Error playing playlist:", err);
      }
    },
    [playPlaylist, playback, customPlaylists],
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

  // If viewing a custom playlist, show the editor
  if (selectedCustomPlaylist) {
    return (
      <CustomPlaylistEditor
        playlist={selectedCustomPlaylist}
        onBack={() => setSelectedCustomPlaylist(null)}
        onUpdate={handleUpdatePlaylist}
        onDelete={handleDeletePlaylist}
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
          <button
            className="create-playlist-btn"
            onClick={() => setShowCreateDialog(true)}
          >
            + Create Playlist
          </button>
        </div>

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
              <button onClick={handleCreatePlaylist}>Create</button>
              <button onClick={() => setShowCreateDialog(false)}>Cancel</button>
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
