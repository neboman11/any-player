import { useState, useCallback, useEffect } from "react";
import { tauriAPI } from "../api";

export function useSpotifyAuth() {
  const [isConnected, setIsConnected] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [authUrl, setAuthUrl] = useState<string | null>(null);
  const [isPremium, setIsPremium] = useState<boolean | null>(null);
  const [sessionReady, setSessionReady] = useState(false);

  // Check initial auth status
  useEffect(() => {
    const checkStatus = async () => {
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
          }

          try {
            const ready = await tauriAPI.isSpotifySessionReady();
            setSessionReady(ready);
          } catch (err) {
            console.error("Error checking session status:", err);
          }
        }
      } catch (err) {
        console.error("Error checking Spotify status:", err);
      }
    };

    void checkStatus();
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
    } finally {
      setIsLoading(false);
    }
  }, [getAuthUrl]);

  const pollForAuth = useCallback(async () => {
    let pollCount = 0;
    const maxPolls = 600; // 10 minutes at 1 second intervals

    return new Promise<void>((resolve) => {
      const checkInterval = setInterval(async () => {
        pollCount++;
        try {
          const hasCode = await tauriAPI.checkOAuthCode();
          if (hasCode) {
            console.log(
              "OAuth code received by backend - session initialization should be in progress",
            );
            setIsConnected(true);
            setAuthUrl(null);

            // Wait a moment for the backend to process the auth
            await new Promise((r) => setTimeout(r, 1000));

            // Check if premium and session is ready
            try {
              const premium = await tauriAPI.checkSpotifyPremium();
              setIsPremium(premium);
              console.log("Premium status:", premium);

              if (premium) {
                const ready = await tauriAPI.isSpotifySessionReady();
                setSessionReady(ready);
                console.log("Session ready:", ready);

                if (ready) {
                  console.log("✓ Spotify session is initialized and ready");
                } else {
                  console.warn("⚠ Premium user but session not ready yet");
                }
              }
            } catch (err) {
              console.error("Error checking status after auth:", err);
            }

            clearInterval(checkInterval);
            resolve();
          }
        } catch (err) {
          console.error("Auth polling error:", err);
        }

        if (pollCount >= maxPolls) {
          clearInterval(checkInterval);
          setError("Authentication timeout");
          resolve();
        }
      }, 1000);
    });
  }, []);

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
    isPremium,
    sessionReady,
  };
}
