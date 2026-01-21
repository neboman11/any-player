import { useState, useMemo, useEffect } from "react";
import "./App.css";
import { Sidebar, NowPlaying, Playlists, Search, Settings } from "./components";
import { usePlaylists, useCustomPlaylists } from "./hooks";
import { tauriAPI } from "./api";
import type { Page } from "./types";

export default function App() {
  const [currentPage, setCurrentPage] = useState<Page>("now-playing");
  const { loadPlaylists } = usePlaylists();
  const { refresh: refreshCustomPlaylists } = useCustomPlaylists();

  // Auto-load playlists on startup after validating connections
  useEffect(() => {
    const initializePlaylists = async () => {
      try {
        // Always load custom playlists (they're local)
        console.log("Auto-loading custom playlists on app startup...");
        await refreshCustomPlaylists();

        // Wait a moment for backend session restoration to complete
        // The backend restores sessions asynchronously on startup
        await new Promise((resolve) => setTimeout(resolve, 500));

        // Check which services are authenticated
        // Retry a few times in case backend is still initializing
        let spotifyAuth = false;
        let jellyfinAuth = false;

        for (let i = 0; i < 3; i++) {
          [spotifyAuth, jellyfinAuth] = await Promise.all([
            tauriAPI.isSpotifyAuthenticated().catch(() => false),
            tauriAPI.isJellyfinAuthenticated().catch(() => false),
          ]);

          // If we found at least one authenticated service, stop retrying
          if (spotifyAuth || jellyfinAuth) {
            break;
          }

          // Wait before retrying
          if (i < 2) {
            await new Promise((resolve) => setTimeout(resolve, 300));
          }
        }

        // If at least one service is connected, load all playlists
        if (spotifyAuth || jellyfinAuth) {
          console.log(
            `Auto-loading service playlists on app startup (Spotify: ${spotifyAuth}, Jellyfin: ${jellyfinAuth})...`,
          );
          await loadPlaylists("all");
          console.log("Playlists loaded and cached");
        } else {
          console.log("No authenticated services found on startup");
        }
      } catch (err) {
        console.error("Error initializing playlists:", err);
      }
    };

    void initializePlaylists();
  }, [loadPlaylists, refreshCustomPlaylists]);

  // Memoize the page content to avoid unnecessary re-renders
  const pageContent = useMemo(() => {
    switch (currentPage) {
      case "now-playing":
        return <NowPlaying />;
      case "playlists":
        return <Playlists />;
      case "search":
        return <Search />;
      case "settings":
        return <Settings />;
      default:
        return <NowPlaying />;
    }
  }, [currentPage]);

  return (
    <div className="app">
      <div className="container">
        <Sidebar currentPage={currentPage} setCurrentPage={setCurrentPage} />
        <main className="main-content">{pageContent}</main>
      </div>
    </div>
  );
}
