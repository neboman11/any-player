import { useState, useCallback, useEffect } from "react";
import { tauriAPI } from "../api";

export function useSpotifyAuth() {
  const [isConnected, setIsConnected] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [authUrl, setAuthUrl] = useState<string | null>(null);

  // Check initial auth status and load saved tokens
  useEffect(() => {
    const checkStatus = async () => {
      try {
        const authenticated = await tauriAPI.isSpotifyAuthenticated();
        setIsConnected(authenticated);

        // If not authenticated, try to restore from saved tokens
        if (!authenticated) {
          try {
            const restored = await tauriAPI.restoreSpotifySession();
            if (restored) {
              setIsConnected(true);
            }
          } catch (err) {
            console.log("No saved Spotify session found:", err);
          }
        }
      } catch (err) {
        console.error("Error checking Spotify status:", err);
      }
    };

    void checkStatus();
  }, []);

  // Listen for OAuth callback messages
  useEffect(() => {
    const handleMessage = (event: MessageEvent) => {
      const data = event.data;
      if (data && data.type === "spotify-auth") {
        if (data.code) {
          void completeAuth(data.code);
        } else if (data.error) {
          setError(data.error);
        }
      }
    };

    window.addEventListener("message", handleMessage);
    return () => window.removeEventListener("message", handleMessage);
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

  const completeAuth = useCallback(async (code: string) => {
    try {
      setIsLoading(true);
      setError(null);
      await tauriAPI.authenticateSpotify(code);
      setIsConnected(true);
      setAuthUrl(null);

      // Save the session for persistence
      await tauriAPI.saveSpotifySession();
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Authentication failed";
      setError(message);
    } finally {
      setIsLoading(false);
    }
  }, []);

  const connect = useCallback(async () => {
    try {
      console.log("Running Spotify Connect hook");
      setIsLoading(true);
      setError(null);
      const url = await getAuthUrl();
      setAuthUrl(url);

      // Try to open in browser
      if (window.__TAURI__?.shell) {
        try {
          await window.__TAURI__.shell.open(url);
        } catch (err) {
          console.error("Failed to open browser:", err);
          // Fall back to showing the URL for manual entry
        }
      }

      // Start polling for auth completion
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
            setIsConnected(true);
            setAuthUrl(null);
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

      // Clear saved session
      await tauriAPI.clearSpotifySession();
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to disconnect";
      setError(message);
    }
  }, []);

  const clearAuthUrl = useCallback(() => {
    setAuthUrl(null);
  }, []);

  return {
    isConnected,
    isLoading,
    error,
    authUrl,
    connect,
    disconnect,
    clearAuthUrl,
  };
}
