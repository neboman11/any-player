import { useState, useMemo, useEffect } from "react";
import { Toaster } from "react-hot-toast";
import "./App.css";
import {
  Sidebar,
  NowPlaying,
  Playlists,
  Search,
  Settings,
  BottomPlayBar,
} from "./components";
import { usePlaylists, useCustomPlaylists } from "./hooks";
import { tauriAPI } from "./api";
import type { BackendInitStatus, Page } from "./types";
import { listen } from "@tauri-apps/api/event";
import { LoadingSpinner } from "./components/shared/LoadingSpinner";
import { backendSocket } from "./websocket";

const STARTUP_PROVIDER_CHECK_TIMEOUT_MS = 2500;
const STARTUP_SERVICE_SYNC_TIMEOUT_MS = 8000;
const STARTUP_CUSTOM_PLAYLIST_TIMEOUT_MS = 2500;

async function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  fallbackValue: T,
): Promise<T> {
  let timeoutId: number | undefined;

  const timeoutPromise = new Promise<T>((resolve) => {
    timeoutId = window.setTimeout(() => resolve(fallbackValue), timeoutMs);
  });

  try {
    return await Promise.race([promise, timeoutPromise]);
  } finally {
    if (timeoutId !== undefined) {
      window.clearTimeout(timeoutId);
    }
  }
}

export default function App() {
  const [currentPage, setCurrentPage] = useState<Page>("now-playing");
  const [startupLoading, setStartupLoading] = useState(true);
  const [backendInitLoading, setBackendInitLoading] = useState(false);
  const [startupMessage, setStartupMessage] = useState(
    "Loading your library...",
  );
  const { loadPlaylists } = usePlaylists();
  const { refresh: refreshCustomPlaylists } = useCustomPlaylists();

  // Listen for track completion events and auto-advance
  useEffect(() => {
    const unlisten = listen("track-completed", () => {
      console.log("Track completed event received, calling next_track");
      tauriAPI.nextTrack().catch((err) => {
        console.error("Failed to auto-advance to next track:", err);
      });
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Warm caches on startup without blocking the UI
  useEffect(() => {
    let cancelled = false;

    const initializePlaylists = async () => {
      try {
        if (!cancelled) {
          setStartupMessage("Loading custom playlists...");
        }

        console.log("Warming custom playlist cache on startup...");
        const customPlaylistLoadPromise = withTimeout(
          refreshCustomPlaylists(),
          STARTUP_CUSTOM_PLAYLIST_TIMEOUT_MS,
          undefined,
        );

        if (!cancelled) {
          setStartupMessage("Checking connected services...");
        }

        const [spotifyAuth, jellyfinAuth] = await Promise.all([
          withTimeout(
            tauriAPI.isSpotifyAuthenticated().catch(() => false),
            STARTUP_PROVIDER_CHECK_TIMEOUT_MS,
            false,
          ),
          withTimeout(
            tauriAPI.isJellyfinAuthenticated().catch(() => false),
            STARTUP_PROVIDER_CHECK_TIMEOUT_MS,
            false,
          ),
        ]);

        // If at least one service is connected, load all playlists
        if (spotifyAuth || jellyfinAuth) {
          console.log(
            `Background-loading service playlists on startup (Spotify: ${spotifyAuth}, Jellyfin: ${jellyfinAuth})...`,
          );
          if (!cancelled) {
            setStartupMessage("Syncing service playlists...");
          }
          await withTimeout(
            loadPlaylists("all"),
            STARTUP_SERVICE_SYNC_TIMEOUT_MS,
            undefined,
          );
          console.log("Playlists loaded and cached");
        } else {
          console.log("No authenticated services found on startup");
        }

        await customPlaylistLoadPromise;
      } catch (err) {
        console.error("Error initializing playlists:", err);
      } finally {
        if (!cancelled) {
          setStartupLoading(false);
        }
      }
    };

    void initializePlaylists();

    return () => {
      cancelled = true;
    };
  }, [loadPlaylists, refreshCustomPlaylists]);

  useEffect(() => {
    const unsubscribe = backendSocket.on<BackendInitStatus>(
      "backend-init-status",
      (status) => {
        setStartupMessage(status.message);
        setBackendInitLoading(!status.done);
      },
    );

    return unsubscribe;
  }, []);

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
      <Toaster position="top-right" />
      <div className="container">
        <Sidebar currentPage={currentPage} setCurrentPage={setCurrentPage} />
        <main className="main-content">
          {(startupLoading || backendInitLoading) && (
            <div
              className="startup-loading-banner"
              role="status"
              aria-live="polite"
            >
              <LoadingSpinner size="small" />
              <span>{startupMessage}</span>
            </div>
          )}
          {pageContent}
          {currentPage !== "now-playing" && <BottomPlayBar />}
        </main>
      </div>
    </div>
  );
}
