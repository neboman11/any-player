import { useState, useCallback, useEffect } from "react";
import { tauriAPI } from "../api";

export function useJellyfinAuth() {
  const [isConnected, setIsConnected] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Check initial auth status
  useEffect(() => {
    const checkStatus = async () => {
      try {
        const authenticated = await tauriAPI.isJellyfinAuthenticated();
        setIsConnected(authenticated);
      } catch (err) {
        console.error("Error checking Jellyfin status:", err);
      }
    };

    void checkStatus();
  }, []);

  const connect = useCallback(async (url: string, apiKey: string) => {
    if (!url || !apiKey) {
      setError("Please enter both URL and API key");
      return;
    }

    try {
      setIsLoading(true);
      setError(null);
      await tauriAPI.authenticateJellyfin(url, apiKey);

      const authenticated = await tauriAPI.isJellyfinAuthenticated();
      if (authenticated) {
        setIsConnected(true);
      } else {
        setError("Authentication failed");
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : "Connection failed";
      setError(message);
    } finally {
      setIsLoading(false);
    }
  }, []);

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
  };
}
