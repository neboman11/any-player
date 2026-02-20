import { useState, useCallback, useEffect } from "react";
import { tauriAPI } from "../api";
import { backendSocket } from "../websocket";
import { retryWithDelay } from "../utils/retryHelper";
import type { PlexAuthStatus } from "../types";

function extractErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }

  if (typeof error === "string" && error.trim()) {
    return error;
  }

  if (error && typeof error === "object") {
    const record = error as Record<string, unknown>;
    if (typeof record.message === "string" && record.message.trim()) {
      return record.message;
    }
    if (typeof record.error === "string" && record.error.trim()) {
      return record.error;
    }
  }

  return fallback;
}

export function usePlexAuth() {
  const [isConnected, setIsConnected] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const checkAuthStatus = useCallback(async () => {
    try {
      const authenticated = await tauriAPI.isPlexAuthenticated();
      setIsConnected(authenticated);
      return authenticated;
    } catch (err) {
      console.error("Error checking Plex status:", err);
      return false;
    }
  }, []);

  useEffect(() => {
    const checkStatus = async () => {
      await retryWithDelay(async () => {
        return await checkAuthStatus();
      });
    };

    void checkStatus();
  }, [checkAuthStatus]);

  useEffect(() => {
    const unsubscribe = backendSocket.on<PlexAuthStatus>(
      "plex-auth-status",
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
    async (url: string, token: string) => {
      if (!url || !token) {
        setError("Please enter both URL and token");
        return;
      }

      try {
        setIsLoading(true);
        setError(null);
        await tauriAPI.authenticatePlex(url, token);

        const authenticated = await checkAuthStatus();
        if (!authenticated) {
          setError("Authentication failed");
        }
      } catch (err) {
        const message = extractErrorMessage(err, "Connection failed");
        setError(message);
      } finally {
        setIsLoading(false);
      }
    },
    [checkAuthStatus],
  );

  const disconnect = useCallback(async () => {
    try {
      await tauriAPI.disconnectPlex();
      setIsConnected(false);
      setError(null);
    } catch (err) {
      const message = extractErrorMessage(err, "Failed to disconnect");
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
