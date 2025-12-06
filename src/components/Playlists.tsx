import { useState, useCallback, useEffect } from "react";
import { usePlaylists } from "../hooks";
import type { TauriSource } from "../types";

export function Playlists() {
  const [activeSource, setActiveSource] = useState<TauriSource>("all");
  const { playlists, isLoading, error, loadPlaylists } = usePlaylists();
  const sources: TauriSource[] = ["all", "spotify", "jellyfin"];

  useEffect(() => {
    void loadPlaylists(activeSource);
  }, [activeSource, loadPlaylists]);

  const handlePlaylistClick = useCallback((playlistId: string) => {
    console.log("Clicked playlist:", playlistId);
    // TODO: Load playlist details and start playing
  }, []);

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
              onClick={() => handlePlaylistClick(playlist.id)}
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
