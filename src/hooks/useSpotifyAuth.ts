import { useState, useCallback, useEffect } from "react";
import { tauriAPI } from "../api";

// Time to wait for backend to finish processing OAuth authentication (in milliseconds).
// NOTE: 2000ms was chosen based on observed worst-case latency for the backend to
//       exchange the OAuth code for tokens and persist session state. Reducing this
//       delay may reintroduce race conditions where `checkAuthStatus` runs before the
//       session is fully ready. If backend performance improves, consider:
//       - Lowering this value, or
//       - Replacing the fixed delay with short-interval polling of auth status
//         until it reports as authenticated or a timeout is reached.
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
      console.log("=== Checking Spotify authentication status ===");
      const authenticated = await tauriAPI.isSpotifyAuthenticated();
      console.log("Backend auth status:", authenticated);

      setIsConnected(authenticated);
      console.log("Updated isConnected state to:", authenticated);

      if (authenticated) {
        // Check premium status and session readiness
        try {
          const premium = await tauriAPI.checkSpotifyPremium();
          setIsPremium(premium);
          console.log("Premium status:", premium);
        } catch (err) {
          console.error("Error checking premium status:", err);
          setIsPremium(null);
        }

        try {
          const ready = await tauriAPI.isSpotifySessionReady();
          setSessionReady(ready);
          console.log("Session ready:", ready);
        } catch (err) {
          console.error("Error checking session status:", err);
          setSessionReady(false);
        }
      } else {
        setIsPremium(null);
        setSessionReady(false);
      }

      console.log("=== Auth status check complete ===");
      return authenticated;
    } catch (err) {
      console.error("Error checking Spotify status:", err);
      return false;
    }
  }, []);

  // Check initial auth status and load saved tokens
  useEffect(() => {
    void checkAuthStatus();
  }, [checkAuthStatus]);

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

  const pollForAuth = useCallback(async () => {
    console.log(">>> pollForAuth started, waiting for OAuth callback...");
    let pollCount = 0;
    const maxPolls = 600; // 10 minutes at 1 second intervals

    return new Promise<void>((resolve) => {
      const checkInterval = setInterval(async () => {
        pollCount++;
        console.log(`Polling attempt ${pollCount}...`);
        try {
          const hasCode = await tauriAPI.checkOAuthCode();
          console.log("checkOAuthCode result:", hasCode);
          if (hasCode) {
            console.log("OAuth code processed by backend");
            clearInterval(checkInterval);

            // Wait for backend to complete authentication
            console.log(
              `Waiting ${AUTH_PROCESSING_DELAY_MS}ms for backend to complete auth...`,
            );
            await new Promise((r) => setTimeout(r, AUTH_PROCESSING_DELAY_MS));

            // Re-check auth status from backend
            console.log("Re-checking authentication status from backend...");
            await checkAuthStatus();
            setAuthUrl(null);
            setIsLoading(false);

            resolve();
          }
        } catch (err) {
          console.error("Auth polling error:", err);
        }

        if (pollCount >= maxPolls) {
          clearInterval(checkInterval);
          setError("Authentication timeout");
          setIsLoading(false);
          resolve();
        }
      }, 1000);
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

      // Start polling for auth completion
      console.log("Starting polling for OAuth callback completion");
      await pollForAuth();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Connection failed";
      setError(message);
      setIsLoading(false);
    }
  }, [getAuthUrl, pollForAuth]);

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
