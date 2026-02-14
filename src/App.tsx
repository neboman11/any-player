import { useState, useMemo, useEffect, useRef } from "react";
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
import { withTimeout } from "./utils/timeout";
import { withTimeoutAndRetry } from "./utils/timeoutWithRetry";

const STARTUP_PROVIDER_CHECK_TIMEOUT_MS = 2500;
const STARTUP_SERVICE_SYNC_TIMEOUT_MS = 8000;
const STARTUP_CUSTOM_PLAYLIST_TIMEOUT_MS = 2500;
const MAX_AUTH_RETRIES = 3;

export default function App() {
  const [currentPage, setCurrentPage] = useState<Page>("now-playing");
  const [startupLoading, setStartupLoading] = useState(true);
  const [backendInitLoading, setBackendInitLoading] = useState(false);
  const [backendInitFailed, setBackendInitFailed] = useState(false);
  const [startupMessage, setStartupMessage] = useState(
    "Loading your library...",
  );
  const [showRetryButton, setShowRetryButton] = useState(false);
  const [showCancelButton, setShowCancelButton] = useState(false);
  const { loadPlaylists } = usePlaylists();
  const { refresh: refreshCustomPlaylists } = useCustomPlaylists();
  const mountedRef = useRef(true);
  const abortControllerRef = useRef<AbortController | null>(null);

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
    mountedRef.current = true;

    const initializePlaylists = async () => {
      try {
        if (!mountedRef.current) return;
        setStartupMessage("Loading custom playlists...");
        setShowRetryButton(false);
        setShowCancelButton(false);

        // Create abort controller for cancellation
        abortControllerRef.current = new AbortController();

        console.log("Warming custom playlist cache on startup...");
        // Start custom playlist loading early but await it later to allow parallel loading
        // with the service playlist checks. This optimizes startup time by running
        // independent operations concurrently.
        const customPlaylistLoadPromise = withTimeout(
          refreshCustomPlaylists(),
          STARTUP_CUSTOM_PLAYLIST_TIMEOUT_MS,
          undefined,
        );

        if (!mountedRef.current) return;
        setStartupMessage("Checking connected services...");
        setShowCancelButton(true); // Enable cancel from the start of auth checks

        const [spotifyAuth, jellyfinAuth] = await Promise.all([
          withTimeoutAndRetry({
            promiseFactory: () => tauriAPI.isSpotifyAuthenticated().catch((err) => {
              console.error("Error checking Spotify authentication on startup:", err);
              return false;
            }),
            timeoutMs: STARTUP_PROVIDER_CHECK_TIMEOUT_MS,
            fallbackValue: false,
            maxRetries: MAX_AUTH_RETRIES,
            onRetry: (retryNumber) => {
              if (!mountedRef.current) return;
              setStartupMessage(`Retrying authentication check (${retryNumber} of ${MAX_AUTH_RETRIES} retries)...`);
            },
            signal: abortControllerRef.current.signal,
          }),
          withTimeoutAndRetry({
            promiseFactory: () => tauriAPI.isJellyfinAuthenticated().catch((err) => {
              console.error("Error checking Jellyfin authentication on startup:", err);
              return false;
            }),
            timeoutMs: STARTUP_PROVIDER_CHECK_TIMEOUT_MS,
            fallbackValue: false,
            maxRetries: MAX_AUTH_RETRIES,
            onRetry: (retryNumber) => {
              if (!mountedRef.current) return;
              setStartupMessage(`Retrying authentication check (${retryNumber} of ${MAX_AUTH_RETRIES} retries)...`);
            },
            signal: abortControllerRef.current.signal,
          }),
        ]);

        setShowCancelButton(false); // Auth checks complete, hide cancel button

        // Check if operation was cancelled
        if (abortControllerRef.current.signal.aborted) {
          console.log("Startup initialization cancelled by user");
          return;
        }

        // If at least one service is connected, load all playlists
        if (spotifyAuth || jellyfinAuth) {
          console.log(
            `Background-loading service playlists on startup (Spotify: ${spotifyAuth}, Jellyfin: ${jellyfinAuth})...`,
          );
          if (!mountedRef.current) return;
          setStartupMessage("Syncing service playlists...");
          const syncResult = await withTimeout(
            loadPlaylists("all"),
            STARTUP_SERVICE_SYNC_TIMEOUT_MS,
            undefined,
          );
          if (syncResult.timedOut) {
            console.log("Service playlist sync timed out but continuing...");
          } else {
            console.log("Playlists loaded and cached");
          }
        } else {
          console.log("Unable to authenticate with Spotify or Jellyfin after retries");
          // Show retry button if all automatic retries failed
          if (!abortControllerRef.current.signal.aborted) {
            setShowRetryButton(true);
            setStartupMessage("Unable to connect to Spotify or Jellyfin. You can retry or continue without them.");
          }
        }

        const customResult = await customPlaylistLoadPromise;
        if (customResult.timedOut) {
          console.log("Custom playlist loading timed out");
        }
      } catch (err) {
        console.error("Error initializing playlists:", err);
      } finally {
        if (mountedRef.current) {
          setStartupLoading(false);
        }
        setShowCancelButton(false);
        abortControllerRef.current = null;
      }
    };

    void initializePlaylists();

    return () => {
      mountedRef.current = false;
      // Cancel any ongoing retries when component unmounts
      if (abortControllerRef.current) {
        abortControllerRef.current.abort();
      }
    };
  }, [loadPlaylists, refreshCustomPlaylists]);

  useEffect(() => {
    const unsubscribe = backendSocket.on<BackendInitStatus>(
      "backend-init-status",
      (status) => {
        setStartupMessage(status.message);
        if (status.done) {
          setBackendInitLoading(false);
          if (!status.success) {
            setBackendInitFailed(true);
          }
        } else {
          setBackendInitLoading(true);
          setBackendInitFailed(false);
        }
      },
    );

    return unsubscribe;
  }, []);

  // Manual retry handler
  const handleManualRetry = () => {
    setStartupLoading(true);
    setShowRetryButton(false);
    // Trigger re-initialization by updating a dependency
    // Since we can't directly call the effect, we'll use a state update
    // that will cause the effect to re-run via its dependencies
    window.location.reload();
  };

  // Cancel handler
  const handleCancel = () => {
    if (abortControllerRef.current) {
      abortControllerRef.current.abort();
      setStartupMessage("Startup cancelled. Continuing without service authentication.");
      setStartupLoading(false);
      setShowRetryButton(false);
    }
  };

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

  const shouldShowBanner = startupLoading || backendInitLoading || backendInitFailed || showRetryButton;

  return (
    <div className="app">
      <Toaster position="top-right" />
      <div className="container">
        <Sidebar currentPage={currentPage} setCurrentPage={setCurrentPage} />
        <main className="main-content">
          {shouldShowBanner && (
            <div
              className={`startup-loading-banner ${backendInitFailed ? "error" : ""}`}
              role="status"
              aria-live="polite"
            >
              {!backendInitFailed && <LoadingSpinner size="small" />}
              <span>{startupMessage}</span>
              {showCancelButton && (
                <button
                  className="startup-banner-button startup-cancel-button"
                  onClick={handleCancel}
                  type="button"
                >
                  Cancel
                </button>
              )}
              {showRetryButton && (
                <button
                  className="startup-banner-button startup-retry-button"
                  onClick={handleManualRetry}
                  type="button"
                >
                  Retry
                </button>
              )}
            </div>
          )}
          {pageContent}
          {currentPage !== "now-playing" && <BottomPlayBar />}
        </main>
      </div>
    </div>
  );
}
