import { useState, useCallback, useEffect } from "react";
import { tauriAPI } from "../api";
import { backendSocket } from "../websocket";
import { retryWithDelay } from "../utils/retryHelper";
import type { OAuthCodeReceived, SpotifyAuthStatus } from "../types";

// Time to wait for backend to finish processing OAuth authentication (in milliseconds).
// NOTE: 2000ms was chosen based on observed worst-case latency for the backend to
//       exchange the OAuth code for tokens and persist session state. Reducing this
//       delay may reintroduce race conditions where `checkAuthStatus` runs before the
//       session is fully ready.
const AUTH_PROCESSING_DELAY_MS = 2000;

export function useSpotifyAuth() {
  const [isConnected, setIsConnected] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [authUrl, setAuthUrl] = useState<string | null>(null);
  const [isPremium, setIsPremium] = useState<boolean | null>(null);
  const [sessionReady, setSessionReady] = useState(false);

  const checkAuthStatus = useCallback(async () => {
    try {
      const authenticated = await tauriAPI.isSpotifyAuthenticated();

      setIsConnected(authenticated);

      if (authenticated) {
        // Check premium status and session readiness
        try {
          const premium = await tauriAPI.checkSpotifyPremium();
          setIsPremium(premium);
        } catch (err) {
          console.error("Error checking premium status:", err);
          setIsPremium(null);
        }

        try {
          const ready = await tauriAPI.isSpotifySessionReady();
          setSessionReady(ready);
        } catch (err) {
          console.error("Error checking session status:", err);
          setSessionReady(false);
        }
      } else {
        setIsPremium(null);
        setSessionReady(false);
      }

      return authenticated;
    } catch (err) {
      console.error("Error checking Spotify status:", err);
      return false;
    }
  }, []);

  // Check initial auth status and load saved tokens
  // Retry a few times to account for backend session restoration delay
  useEffect(() => {
    const checkWithRetry = async () => {
      await retryWithDelay(async () => {
        await checkAuthStatus();
        return await tauriAPI.isSpotifyAuthenticated().catch(() => false);
      });
    };

    void checkWithRetry();
  }, [checkAuthStatus]);

  useEffect(() => {
    const unsubscribe = backendSocket.on<SpotifyAuthStatus>(
      "spotify-auth-status",
      (status) => {
        if (!status) {
          return;
        }
        setIsConnected(status.authenticated);
        setIsPremium(status.premium ?? null);
        setSessionReady(status.session_ready ?? false);
      },
    );

    return unsubscribe;
  }, []);

  const getAuthUrl = useCallback(async (): Promise<string> => {
    try {
      return await tauriAPI.getSpotifyAuthUrl();
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to get auth URL";
      setError(message);
      throw err;
    }
  }, []);

  const waitForAuthCallback = useCallback(async () => {
    return new Promise<void>(async (resolve) => {
      let resolved = false;
      let unsubscribe: (() => void) | null = null;

      // Immediately check if auth already completed (in case websocket missed the event)
      try {
        const hasCode = await tauriAPI.checkOAuthCode();
        if (hasCode) {
          resolved = true;
          await new Promise((r) => setTimeout(r, AUTH_PROCESSING_DELAY_MS));
          await checkAuthStatus();
          setAuthUrl(null);
          setIsLoading(false);
          resolve();
          return;
        }
      } catch (err) {
        console.warn("Initial auth check failed, will wait for websocket event:", err);
      }

      const timeoutId = window.setTimeout(
        () => {
          if (resolved) {
            return;
          }
          resolved = true;
          if (unsubscribe) {
            unsubscribe();
          }
          setError("Authentication timeout");
          setIsLoading(false);
          resolve();
        },
        10 * 60 * 1000,
      );

      unsubscribe = backendSocket.on<OAuthCodeReceived>(
        "oauth-code-received",
        async (payload) => {
          if (resolved || payload?.source !== "spotify") {
            return;
          }

          resolved = true;
          window.clearTimeout(timeoutId);
          if (unsubscribe) {
            unsubscribe();
          }

          try {
            const hasCode = await tauriAPI.checkOAuthCode();
            if (hasCode) {
              await new Promise((r) => setTimeout(r, AUTH_PROCESSING_DELAY_MS));
              await checkAuthStatus();
            } else {
              setError("Authentication failed");
            }
          } catch (err) {
            console.error("Auth completion error:", err);
            setError("Authentication failed");
          } finally {
            setAuthUrl(null);
            setIsLoading(false);
            resolve();
          }
        },
      );
    });
  }, [checkAuthStatus]);

  const connect = useCallback(async () => {
    try {
      console.log("Starting Spotify authentication flow");
      setIsLoading(true);
      setError(null);
      const url = await getAuthUrl();
      setAuthUrl(url);

      // Try to open in browser
      if (window.__TAURI__?.shell) {
        try {
          console.log("Opening Spotify auth URL in browser");
          await window.__TAURI__.shell.open(url);
        } catch (err) {
          console.error("Failed to open browser:", err);
          // Fall back to showing the URL for manual entry
        }
      }

      // Wait for websocket notification of OAuth completion
      console.log("Waiting for OAuth callback completion");
      await waitForAuthCallback();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Connection failed";
      setError(message);
      setIsLoading(false);
    }
  }, [getAuthUrl, waitForAuthCallback]);

  const disconnect = useCallback(async () => {
    try {
      await tauriAPI.disconnectSpotify();
      setIsConnected(false);
      setError(null);
      setAuthUrl(null);
      setIsPremium(null);
      setSessionReady(false);
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to disconnect";
      setError(message);
    }
  }, []);

  const clearAuthUrl = useCallback(() => {
    setAuthUrl(null);
  }, []);

  const initializeSession = useCallback(async () => {
    if (!isConnected || !isPremium) {
      console.warn("Cannot initialize session: not connected or not premium");
      return;
    }

    try {
      setIsLoading(true);
      setError(null);
      console.log("Manually initializing Spotify session...");

      await tauriAPI.initializeSpotifySessionFromProvider();

      // Verify initialization
      const ready = await tauriAPI.isSpotifySessionReady();
      setSessionReady(ready);

      if (ready) {
        console.log("✓ Session initialized successfully");
      } else {
        setError("Session initialization completed but session not ready");
      }
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to initialize session";
      setError(message);
      console.error("Session initialization error:", err);
    } finally {
      setIsLoading(false);
    }
  }, [isConnected, isPremium]);

  return {
    isConnected,
    isLoading,
    error,
    authUrl,
    connect,
    disconnect,
    clearAuthUrl,
    initializeSession,
    checkAuthStatus,
    isPremium,
    sessionReady,
  };
}
