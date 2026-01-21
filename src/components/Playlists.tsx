import { useState, useCallback, useEffect } from "react";
import { usePlaylists } from "../hooks";
import { usePlayback } from "../hooks";
import type { TauriSource } from "../types";

export function Playlists() {
  const [activeSource, setActiveSource] = useState<TauriSource>("all");
  const {
    playlists,
    isLoading,
    error,
    loadPlaylists,
    playPlaylist,
    refreshKey,
  } = usePlaylists();
  const playback = usePlayback();
  const sources: TauriSource[] = ["all", "spotify", "jellyfin"];

  // Reload playlists when activeSource or refreshKey changes
  useEffect(() => {
    void loadPlaylists(activeSource);
  }, [activeSource, loadPlaylists, refreshKey]);

  const handlePlaylistClick = useCallback(
    async (playlistId: string, source: string) => {
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
    [playPlaylist, playback],
  );

  return (
    <section id="playlists" className="page">
      <div className="playlists-container">
        <h2>Your Playlists</h2>
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
          {isLoading && (
            <div className="playlist-card loading">Loading playlists...</div>
          )}
          {error && !isLoading && <div className="playlist-card">{error}</div>}
          {!isLoading && !error && playlists.length === 0 && (
            <div className="playlist-card">
              No playlists found. Connect a service in Settings.
            </div>
          )}
          {playlists.map((playlist) => (
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
