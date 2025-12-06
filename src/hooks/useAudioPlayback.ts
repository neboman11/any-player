import { useRef, useCallback } from "react";
import { tauriAPI } from "../api";

/**
 * Hook for managing HTML5 audio playback
 * Handles actual audio playing via the Web Audio API
 */
export function useAudioPlayback() {
  const audioRef = useRef<HTMLAudioElement | null>(null);

  const playAudio = useCallback(async (url: string) => {
    try {
      // Create or reuse audio element
      if (!audioRef.current) {
        audioRef.current = new Audio();
      }

      console.log("Getting audio file for URL:", url);

      // Download audio through backend to bypass CORS
      const fileUrl = await tauriAPI.getAudioFile(url);
      console.log("Audio file URL:", fileUrl);

      if (audioRef.current) {
        audioRef.current.src = fileUrl;
        audioRef.current
          .play()
          .then(() => {
            console.log("Audio playback started for:", url);
          })
          .catch((error) => {
            console.error("Failed to play audio:", error);
          });
      }
    } catch (error) {
      console.error("Error playing audio:", error);
    }
  }, []);

  const pauseAudio = useCallback(() => {
    if (audioRef.current) {
      audioRef.current.pause();
      console.log("Audio paused");
    }
  }, []);

  const resumeAudio = useCallback(() => {
    if (audioRef.current) {
      audioRef.current
        .play()
        .catch((error) => console.error("Failed to resume audio:", error));
      console.log("Audio resumed");
    }
  }, []);

  const seekAudio = useCallback((positionMs: number) => {
    if (audioRef.current) {
      audioRef.current.currentTime = positionMs / 1000;
      console.log("Seek to:", positionMs, "ms");
    }
  }, []);

  const setVolume = useCallback((volume: number) => {
    if (audioRef.current) {
      // Volume is 0-100, convert to 0-1
      audioRef.current.volume = volume / 100;
    }
  }, []);

  const getCurrentPosition = useCallback(() => {
    if (audioRef.current) {
      return Math.floor(audioRef.current.currentTime * 1000);
    }
    return 0;
  }, []);

  const getDuration = useCallback(() => {
    if (audioRef.current) {
      return Math.floor(audioRef.current.duration * 1000);
    }
    return 0;
  }, []);

  const cleanup = useCallback(() => {
    if (audioRef.current) {
      audioRef.current.pause();
      audioRef.current.src = "";
    }
  }, []);

  return {
    playAudio,
    pauseAudio,
    resumeAudio,
    seekAudio,
    setVolume,
    getCurrentPosition,
    getDuration,
    cleanup,
  };
}
