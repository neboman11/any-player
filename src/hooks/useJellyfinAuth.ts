import { useState, useCallback, useEffect } from "react";
import { tauriAPI } from "../api";
import { retryWithDelay } from "../utils/retryHelper";

export function useJellyfinAuth() {
  const [isConnected, setIsConnected] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const checkAuthStatus = useCallback(async () => {
    try {
      const authenticated = await tauriAPI.isJellyfinAuthenticated();
      setIsConnected(authenticated);
      return authenticated;
    } catch (err) {
      console.error("Error checking Jellyfin status:", err);
      return false;
    }
  }, []);

  // Check initial auth status
  // Retry a few times to account for backend session restoration delay
  useEffect(() => {
    const checkStatus = async () => {
      await retryWithDelay(async () => {
        return await checkAuthStatus();
      });
    };

    void checkStatus();
  }, [checkAuthStatus]);

  const connect = useCallback(
    async (url: string, apiKey: string) => {
      if (!url || !apiKey) {
        setError("Please enter both URL and API key");
        return;
      }

      try {
        setIsLoading(true);
        setError(null);
        await tauriAPI.authenticateJellyfin(url, apiKey);

        // Check authentication status after connecting
        const authenticated = await checkAuthStatus();
        if (!authenticated) {
          setError("Authentication failed");
        }
      } catch (err) {
        const message =
          err instanceof Error ? err.message : "Connection failed";
        setError(message);
      } finally {
        setIsLoading(false);
      }
    },
    [checkAuthStatus],
  );

  const disconnect = useCallback(async () => {
    try {
      await tauriAPI.disconnectJellyfin();
      setIsConnected(false);
      setError(null);
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to disconnect";
      setError(message);
    }
  }, []);

  return {
    isConnected,
    isLoading,
    error,
    connect,
    disconnect,
    checkAuthStatus,
  };
}
