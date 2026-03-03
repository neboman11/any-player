import { useState, useCallback, useEffect, useRef } from "react";
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
  }
}

export function usePlayback() {
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
  const hasIssuedPause = useRef(false);

  const updatePlaybackAvailability = useCallback(
    async (status: PlaybackStatus) => {
      const source = status.current_track?.source;
      if (!source || source === "custom") {
        setPlaybackDisabledReason(null);
        lastCheckedSource.current = null;
        lastAuthResult.current = null;
        hasIssuedPause.current = false;
        return;
      }

      if (source !== lastCheckedSource.current) {
        // Source changed – reset cached state and re-check authentication.
        lastCheckedSource.current = source;
        hasIssuedPause.current = false;
        try {
          const authenticated = await isSourceAuthenticated(source);
          lastAuthResult.current = authenticated;
          if (!authenticated) {
            setPlaybackDisabledReason(
              `Playback disabled: ${sourceDisplayName[source]} is not configured/authenticated for this app. Reconnect it in Settings. Next/Previous still works.`,
            );
          } else {
            setPlaybackDisabledReason(null);
          }
        } catch (error) {
          console.error("Error checking playback source availability:", error);
          lastAuthResult.current = false;
          setPlaybackDisabledReason(
            `Playback disabled: ${sourceDisplayName[source]} connection could not be verified. Reconnect it in Settings. Next/Previous still works.`,
          );
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
    }
  }, []);

  const seekTo = useCallback(async (positionMs: number) => {
    try {
      await tauriAPI.seek(positionMs);
      setPosition(positionMs);
    } catch (error) {
      console.error("Error seeking:", error);
    }
  }, []);

  const playTrack = useCallback(
    async (trackId: string, source: string) => {
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
  }, [updateStatus]);

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
