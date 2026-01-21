import { useState, useCallback, useRef } from "react";
import {
  useSearch,
  usePlayback,
  useAudioPlayback,
  useCustomPlaylists,
} from "../hooks";
import type { SearchType, TauriSource, Track } from "../types";
import { tauriAPI } from "../api";

export function Search() {
  const [searchType, setSearchType] = useState<SearchType>("tracks");
  const [searchSource, setSearchSource] = useState<TauriSource>("all");
  const [page, setPage] = useState(1);
  const [selectedTrack, setSelectedTrack] = useState<Track | null>(null);
  const [showPlaylistSelector, setShowPlaylistSelector] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const { results, isLoading, error, search } = useSearch();
  const { playlists } = useCustomPlaylists();
  const playback = usePlayback();
  const audio = useAudioPlayback();

  const RESULTS_PER_PAGE = 20;

  const handleSearch = useCallback(async () => {
    const query = searchInputRef.current?.value;
    if (!query) return;

    setPage(1);
    await search(query, searchType, searchSource);
  }, [search, searchType, searchSource]);

  const handleKeyPress = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter") {
        void handleSearch();
      }
    },
    [handleSearch],
  );

  const handlePlayTrack = useCallback(
    async (result: (typeof results)[0]) => {
      if (result.type === "track") {
        await playback.playTrack(result.id, result.source);
        await playback.updateStatus();

        if (
          "url" in result &&
          typeof (result as { url?: unknown }).url === "string"
        ) {
          console.log("Playing audio from URL:", result.url);
          audio.playAudio((result as { url: string }).url);
        }
      }
    },
    [playback, audio],
  );

  // Helper to capitalize source for Rust enum serialization
  const capitalizeSource = (
    source: string,
  ): "Spotify" | "Jellyfin" | "Custom" => {
    return (source.charAt(0).toUpperCase() + source.slice(1)) as
      | "Spotify"
      | "Jellyfin"
      | "Custom";
  };

  const handleAddToPlaylist = useCallback(
    (result: (typeof results)[0], e: React.MouseEvent) => {
      e.stopPropagation();
      if (result.type === "track") {
        const track: Track = {
          id: result.id,
          title: result.name,
          artist: result.artist || "Unknown Artist",
          album: "",
          duration_ms: 0,
          source: capitalizeSource(result.source),
        };
        setSelectedTrack(track);
        setShowPlaylistSelector(true);
      }
    },
    [],
  );

  const handleSelectPlaylist = useCallback(
    async (playlistId: string) => {
      if (!selectedTrack) return;

      try {
        await tauriAPI.addTrackToCustomPlaylist(playlistId, selectedTrack);
        setShowPlaylistSelector(false);
        setSelectedTrack(null);
        alert("Track added to playlist!");
      } catch (err) {
        console.error("Failed to add track:", err);
        alert("Failed to add track to playlist");
      }
    },
    [selectedTrack],
  );

  const paginatedResults = results.slice(
    (page - 1) * RESULTS_PER_PAGE,
    page * RESULTS_PER_PAGE,
  );
  const totalPages = Math.ceil(results.length / RESULTS_PER_PAGE);

  return (
    <section id="search" className="page">
      <div className="search-container">
        <h2>Search Music</h2>
        <div className="search-bar">
          <input
            ref={searchInputRef}
            type="text"
            id="search-input"
            placeholder="Search for tracks or playlists..."
            onKeyPress={handleKeyPress}
          />
          <button id="search-btn" onClick={handleSearch} disabled={isLoading}>
            {isLoading ? "Searching..." : "🔍 Search"}
          </button>
        </div>
        <div className="search-tabs">
          {(["tracks", "playlists"] as SearchType[]).map((type) => (
            <button
              key={type}
              className={`tab-btn ${searchType === type ? "active" : ""}`}
              data-type={type}
              onClick={() => setSearchType(type)}
            >
              {type.charAt(0).toUpperCase() + type.slice(1)}
            </button>
          ))}
        </div>
        <div className="search-source-tabs">
          {(["all", "spotify", "jellyfin"] as TauriSource[]).map((source) => (
            <button
              key={source}
              className={`tab-btn ${searchSource === source ? "active" : ""}`}
              data-source={source}
              onClick={() => setSearchSource(source)}
            >
              {source.charAt(0).toUpperCase() + source.slice(1)}
            </button>
          ))}
        </div>

        {!results.length && !isLoading && !error ? (
          <div className="search-empty">
            <p>Enter a search query to get started</p>
          </div>
        ) : isLoading ? (
          <div className="search-empty">
            <p>Searching...</p>
          </div>
        ) : error && !isLoading ? (
          <div className="search-empty">
            <p>Error: {error}</p>
          </div>
        ) : (
          <>
            <div className="track-table-container">
              <table className="track-table">
                <thead>
                  <tr>
                    <th>Title</th>
                    {searchType === "tracks" ? <th>Artist</th> : <th>Owner</th>}
                    <th>Source</th>
                    {searchType === "tracks" && playlists.length > 0 && (
                      <th>Actions</th>
                    )}
                  </tr>
                </thead>
                <tbody>
                  {paginatedResults.map((result) => (
                    <tr
                      key={`${result.source}-${result.id}`}
                      onClick={() =>
                        result.type === "track" && handlePlayTrack(result)
                      }
                      style={{
                        cursor: result.type === "track" ? "pointer" : "default",
                      }}
                    >
                      <td>{result.name}</td>
                      {result.type === "track" ? (
                        <td>{result.artist || "Unknown Artist"}</td>
                      ) : (
                        <td>{result.owner || "—"}</td>
                      )}
                      <td>
                        <span className="source-badge">{result.source}</span>
                      </td>
                      {result.type === "track" && playlists.length > 0 && (
                        <td>
                          <button
                            className="add-to-playlist-btn"
                            onClick={(e) => handleAddToPlaylist(result, e)}
                            title="Add to playlist"
                          >
                            + Playlist
                          </button>
                        </td>
                      )}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>

            {totalPages > 1 && (
              <div className="pagination">
                <button
                  onClick={() => setPage((p) => Math.max(1, p - 1))}
                  disabled={page === 1}
                  className="pagination-btn"
                >
                  ← Previous
                </button>
                <span className="pagination-info">
                  Page {page} of {totalPages} ({results.length} results)
                </span>
                <button
                  onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                  disabled={page === totalPages}
                  className="pagination-btn"
                >
                  Next →
                </button>
              </div>
            )}
          </>
        )}

        {showPlaylistSelector && (
          <div
            className="modal-overlay"
            onClick={() => setShowPlaylistSelector(false)}
          >
            <div className="modal-content" onClick={(e) => e.stopPropagation()}>
              <h3>Add to Playlist</h3>
              <p className="modal-subtitle">
                Adding: <strong>{selectedTrack?.title}</strong>
              </p>
              <div className="playlist-list">
                {playlists.map((playlist) => (
                  <button
                    key={playlist.id}
                    className="playlist-option"
                    onClick={() => handleSelectPlaylist(playlist.id)}
                  >
                    <span className="playlist-option-name">
                      {playlist.name}
                    </span>
                    <span className="playlist-option-count">
                      {playlist.track_count} tracks
                    </span>
                  </button>
                ))}
              </div>
              <button
                className="modal-close-btn"
                onClick={() => setShowPlaylistSelector(false)}
              >
                Cancel
              </button>
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
