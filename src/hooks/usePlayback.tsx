import {
  useState,
  useCallback,
  useEffect,
  useRef,
  createContext,
  useContext,
} from "react";
import type { ReactNode } from "react";
import toast from "react-hot-toast";
import { tauriAPI } from "../api";
import { backendSocket } from "../websocket";
import type { PlaybackStatus, RepeatMode, Track } from "../types";

type TrackSource = Track["source"];

const sourceDisplayName: Record<TrackSource, string> = {
  spotify: "Spotify",
  jellyfin: "Jellyfin",
  plex: "Plex",
  custom: "Custom",
};

async function isSourceAuthenticated(source: TrackSource): Promise<boolean> {
  switch (source) {
    case "spotify":
      return tauriAPI.isSpotifyAuthenticated();
    case "jellyfin":
      return tauriAPI.isJellyfinAuthenticated();
    case "plex":
      return tauriAPI.isPlexAuthenticated();
    case "custom":
      return true;
    default: {
      const _exhaustiveCheck: never = source;
      throw new Error(`Unhandled track source: ${String(_exhaustiveCheck)}`);
    }
  }
}

// How long (ms) to wait before re-checking a previously-failed auth result.
// This allows the UI to recover if the user reconnects a provider in Settings.
const AUTH_RECHECK_INTERVAL_MS = 30_000;

function notAuthenticatedReason(source: TrackSource): string {
  return `Playback disabled: ${sourceDisplayName[source]} is not configured/authenticated for this app. Reconnect it in Settings. Next/Previous still works.`;
}

function notVerifiedReason(source: TrackSource): string {
  return `Playback disabled: ${sourceDisplayName[source]} connection could not be verified. Reconnect it in Settings. Next/Previous still works.`;
}

function usePlaybackState() {
  const [playbackStatus, setPlaybackStatus] = useState<PlaybackStatus | null>(
    null,
  );
  const [isPlaying, setIsPlaying] = useState(false);
  const [shuffle, setShuffle] = useState(false);
  const [repeatMode, setRepeatMode] = useState<RepeatMode>("off");
  const [volume, setVolume] = useState(100);
  const [position, setPosition] = useState(0);
  const [duration, setDuration] = useState(0);
  const [isLoading, setIsLoading] = useState(false);
  const [playbackDisabledReason, setPlaybackDisabledReason] = useState<
    string | null
  >(null);

  const lastCheckedSource = useRef<TrackSource | null>(null);
  const lastAuthResult = useRef<boolean | null>(null);
  const lastAuthCheckTime = useRef<number>(0);
  const hasIssuedPause = useRef(false);

  const updatePlaybackAvailability = useCallback(
    async (status: PlaybackStatus) => {
      const source = status.current_track?.source;
      if (!source || source === "custom") {
        setPlaybackDisabledReason(null);
        lastCheckedSource.current = null;
        lastAuthResult.current = null;
        lastAuthCheckTime.current = 0;
        hasIssuedPause.current = false;
        return;
      }

      // Re-check auth when the source changes or when a previous failure has
      // exceeded the recheck interval (so the UI recovers after reconnecting).
      const shouldRecheck =
        source !== lastCheckedSource.current ||
        (lastAuthResult.current === false &&
          Date.now() - lastAuthCheckTime.current > AUTH_RECHECK_INTERVAL_MS);

      if (shouldRecheck) {
        // Capture the source being checked so stale results can be discarded.
        const checkSource = source;
        lastCheckedSource.current = source;
        lastAuthCheckTime.current = Date.now();
        hasIssuedPause.current = false;
        try {
          const authenticated = await isSourceAuthenticated(checkSource);
          // Discard result if the source changed while we were awaiting.
          if (lastCheckedSource.current !== checkSource) {
            return;
          }
          lastAuthResult.current = authenticated;
          if (!authenticated) {
            setPlaybackDisabledReason(notAuthenticatedReason(checkSource));
          } else {
            setPlaybackDisabledReason(null);
          }
        } catch (error) {
          // Discard error if the source changed while we were awaiting.
          if (lastCheckedSource.current !== checkSource) {
            return;
          }
          console.error("Error checking playback source availability:", error);
          lastAuthResult.current = false;
          setPlaybackDisabledReason(notVerifiedReason(checkSource));
        }
      }

      // Apply cached auth result: pause once if playing while source is disabled,
      // and reset the gate when playback reaches a paused state.
      if (lastAuthResult.current === false) {
        if (status.state === "paused") {
          hasIssuedPause.current = false;
        } else if (status.state === "playing" && !hasIssuedPause.current) {
          hasIssuedPause.current = true;
          try {
            await tauriAPI.pause();
          } catch (pauseError) {
            console.error(
              "Error pausing playback after availability failure:",
              pauseError,
            );
          }
        }
      }
    },
    [],
  );

  // Fetch current playback status
  const updateStatus = useCallback(async () => {
    try {
      const status = await tauriAPI.getPlaybackStatus();
      if (status) {
        setPlaybackStatus(status);
        setIsPlaying(status.state === "playing");
        setShuffle(status.shuffle);
        setRepeatMode(status.repeat_mode);
        setVolume(status.volume);
        if (status.position !== undefined) setPosition(status.position);
        if (status.duration !== undefined) setDuration(status.duration);
        await updatePlaybackAvailability(status);
      }
    } catch (error) {
      console.error("Error updating playback status:", error);
    }
  }, [updatePlaybackAvailability]);

  const togglePlayPause = useCallback(async () => {
    if (playbackDisabledReason) {
      return;
    }

    try {
      setIsLoading(true);
      await tauriAPI.togglePlayPause();
      setIsPlaying(!isPlaying);
      await updateStatus();
    } catch (error) {
      console.error("Error toggling play/pause:", error);
      toast.error(typeof error === "string" ? error : "Failed to play/pause");
      await updateStatus();
    } finally {
      setIsLoading(false);
    }
  }, [isPlaying, playbackDisabledReason, updateStatus]);

  const nextTrack = useCallback(async () => {
    try {
      setIsLoading(true);
      await tauriAPI.nextTrack();
      await updateStatus();
    } catch (error) {
      console.error("Error playing next track:", error);
    } finally {
      setIsLoading(false);
    }
  }, [updateStatus]);

  const previousTrack = useCallback(async () => {
    try {
      setIsLoading(true);
      await tauriAPI.previousTrack();
      await updateStatus();
    } catch (error) {
      console.error("Error playing previous track:", error);
    } finally {
      setIsLoading(false);
    }
  }, [updateStatus]);

  const skipToQueueIndex = useCallback(
    async (index: number) => {
      try {
        setIsLoading(true);
        await tauriAPI.skipToQueueIndex(index);
        await updateStatus();
      } catch (error) {
        console.error("Error skipping to queue index:", error);
      } finally {
        setIsLoading(false);
      }
    },
    [updateStatus],
  );

  const toggleShuffle = useCallback(async () => {
    try {
      await tauriAPI.toggleShuffle();
      setShuffle(!shuffle);
    } catch (error) {
      console.error("Error toggling shuffle:", error);
    }
  }, [shuffle]);

  const cycleRepeatMode = useCallback(async () => {
    const modes: RepeatMode[] = ["off", "one", "all"];
    const currentIndex = modes.indexOf(repeatMode);
    const nextMode = modes[(currentIndex + 1) % modes.length];

    try {
      await tauriAPI.setRepeatMode(nextMode);
      setRepeatMode(nextMode);
    } catch (error) {
      console.error("Error setting repeat mode:", error);
    }
  }, [repeatMode]);

  const setVolumeValue = useCallback(async (value: number) => {
    try {
      await tauriAPI.setVolume(value);
      setVolume(value);
    } catch (error) {
      console.error("Error setting volume:", error);
      toast.error(typeof error === "string" ? error : "Failed to set volume");
    }
  }, []);

  const seekTo = useCallback(async (positionMs: number) => {
    try {
      await tauriAPI.seek(positionMs);
      setPosition(positionMs);
    } catch (error) {
      console.error("Error seeking:", error);
      toast.error(typeof error === "string" ? error : "Failed to seek");
      await updateStatus();
    }
  }, [updateStatus]);

  const playTrack = useCallback(
    async (trackId: string, source: TrackSource) => {
      // Check provider auth before attempting playback, mirroring the
      // togglePlayPause guard so all playback entry points are consistent.
      if (source !== "custom") {
        try {
          const authenticated = await isSourceAuthenticated(source);
          if (!authenticated) {
            setPlaybackDisabledReason(notAuthenticatedReason(source));
            return;
          }
          // Auth confirmed – clear any stale disabled reason.
          setPlaybackDisabledReason(null);
        } catch (error) {
          console.error("Error checking playback source availability:", error);
          setPlaybackDisabledReason(notVerifiedReason(source));
          return;
        }
      }

      try {
        setIsLoading(true);
        await tauriAPI.playTrack(trackId, source);
        await updateStatus();
      } catch (error) {
        console.error("Error playing track:", error);
      } finally {
        setIsLoading(false);
      }
    },
    [updateStatus],
  );

  useEffect(() => {
    const unsubscribe = backendSocket.on<PlaybackStatus>(
      "playback-status",
      (status) => {
        if (!status) {
          return;
        }
        setPlaybackStatus(status);
        setIsPlaying(status.state === "playing");
        setShuffle(status.shuffle);
        setRepeatMode(status.repeat_mode as RepeatMode);
        setVolume(status.volume);
        setPosition(status.position ?? 0);
        setDuration(status.duration ?? 0);
        void updatePlaybackAvailability(status);
      },
    );

    void updateStatus();

    // Fallback polling in case websocket is unavailable (5-second interval is infrequent
    // enough to avoid performance impact while ensuring UI doesn't freeze if websocket fails)
    const fallbackInterval = setInterval(() => {
      void updateStatus();
    }, 5000);

    return () => {
      unsubscribe();
      clearInterval(fallbackInterval);
    };
  }, [updateStatus, updatePlaybackAvailability]);

  return {
    playbackStatus,
    isPlaying,
    shuffle,
    repeatMode,
    volume,
    position,
    duration,
    isLoading,
    playbackDisabledReason,
    updateStatus,
    togglePlayPause,
    nextTrack,
    previousTrack,
    skipToQueueIndex,
    toggleShuffle,
    cycleRepeatMode,
    setVolumeValue,
    seekTo,
    playTrack,
  };
}

type PlaybackContextType = ReturnType<typeof usePlaybackState>;

const PlaybackContext = createContext<PlaybackContextType | null>(null);

export function PlaybackProvider({ children }: { children: ReactNode }) {
  const state = usePlaybackState();
  return (
    <PlaybackContext.Provider value={state}>{children}</PlaybackContext.Provider>
  );
}

// eslint-disable-next-line react-refresh/only-export-components
export function usePlayback(): PlaybackContextType {
  const context = useContext(PlaybackContext);
  if (!context) {
    throw new Error("usePlayback must be used within a PlaybackProvider");
  }
  return context;
}
