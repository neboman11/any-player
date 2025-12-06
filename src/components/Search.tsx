import { useState, useCallback, useRef } from "react";
import { useSearch } from "../hooks";
import type { SearchType, TauriSource } from "../types";

export function Search() {
  const [searchType, setSearchType] = useState<SearchType>("tracks");
  const [searchSource, setSearchSource] = useState<TauriSource>("all");
  const searchInputRef = useRef<HTMLInputElement>(null);
  const { results, isLoading, error, search, clearResults: _ } = useSearch();

  const handleSearch = useCallback(async () => {
    const query = searchInputRef.current?.value;
    if (!query) return;

    await search(query, searchType, searchSource);
  }, [search, searchType, searchSource]);

  const handleKeyPress = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter") {
        void handleSearch();
      }
    },
    [handleSearch]
  );

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
        <div className="search-results" id="search-results">
          {!results.length && !isLoading && !error && (
            <p>Enter a search query to get started</p>
          )}
          {isLoading && <p>Searching...</p>}
          {error && !isLoading && <p>Error: {error}</p>}
          {results.map((result) => (
            <div
              key={`${result.source}-${result.id}`}
              className="search-result-card playlist-card"
            >
              <h4>{result.name}</h4>
              {result.type === "track" ? (
                <>
                  <p>{result.artist}</p>
                  <small>{result.source}</small>
                </>
              ) : (
                <>
                  <p>{result.owner}</p>
                  <small>{result.source}</small>
                </>
              )}
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
