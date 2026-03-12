import { useState, useCallback, useEffect } from "react";
import { tauriAPI } from "../api";
import { backendSocket } from "../websocket";
import { retryWithDelay } from "../utils/retryHelper";
import type { JellyfinAuthStatus } from "../types";

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

  useEffect(() => {
    const unsubscribe = backendSocket.on<JellyfinAuthStatus>(
      "jellyfin-auth-status",
      (status) => {
        if (!status) {
          return;
        }
        setIsConnected(status.authenticated);
      },
    );

    return unsubscribe;
  }, []);

  const connect = useCallback(
    async (url: string, apiKey: string, pageSize?: number) => {
      if (!url || !apiKey) {
        setError("Please enter both URL and API key");
        return;
      }

      try {
        setIsLoading(true);
        setError(null);
        await tauriAPI.authenticateJellyfin(url, apiKey, pageSize);

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
