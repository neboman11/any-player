import { useState, useCallback, useEffect } from "react";
import { tauriAPI } from "../api";

// Retry configuration for authentication checks
// Initial delay allows backend time to start session restoration
const AUTH_CHECK_INITIAL_DELAY_MS = 500;
// Delay between retry attempts
const AUTH_CHECK_RETRY_DELAY_MS = 300;
// Maximum number of retry attempts
const AUTH_CHECK_MAX_RETRIES = 3;

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
      // Initial delay to allow backend to start session restoration
      await new Promise((resolve) => setTimeout(resolve, AUTH_CHECK_INITIAL_DELAY_MS));

      // Try up to AUTH_CHECK_MAX_RETRIES times
      for (let i = 0; i < AUTH_CHECK_MAX_RETRIES; i++) {
        await checkAuthStatus();

        // Check if we're now connected
        const authenticated = await tauriAPI
          .isSpotifyAuthenticated()
          .catch(() => false);
        if (authenticated) {
          break; // Success, stop retrying
        }

        // Wait before next retry
        if (i < AUTH_CHECK_MAX_RETRIES - 1) {
          await new Promise((resolve) => setTimeout(resolve, AUTH_CHECK_RETRY_DELAY_MS));
        }
      }
    };

    void checkWithRetry();
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
    let pollCount = 0;
    const maxPolls = 600; // 10 minutes at 1 second intervals

    return new Promise<void>((resolve) => {
      const checkInterval = setInterval(async () => {
        pollCount++;
        try {
          const hasCode = await tauriAPI.checkOAuthCode();
          if (hasCode) {
            clearInterval(checkInterval);

            // Wait for backend to complete authentication
            await new Promise((r) => setTimeout(r, AUTH_PROCESSING_DELAY_MS));

            // Re-check auth status from backend
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
