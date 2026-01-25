import { useState, useCallback, useEffect } from "react";
import { tauriAPI } from "../api";
import type { PlaybackStatus, RepeatMode } from "../types";

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
      }
    } catch (error) {
      console.error("Error updating playback status:", error);
    }
  }, []);

  const togglePlayPause = useCallback(async () => {
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
  }, [isPlaying, updateStatus]);

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

  // Poll for status updates
  useEffect(() => {
    void updateStatus();
    const interval = setInterval(() => void updateStatus(), 500);
    return () => clearInterval(interval);
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
    updateStatus,
    togglePlayPause,
    nextTrack,
    previousTrack,
    toggleShuffle,
    cycleRepeatMode,
    setVolumeValue,
    seekTo,
    playTrack,
  };
}
