import { useCallback } from "react";
import { tauriAPI } from "../api";

// Create a singleton audio element that's shared across all hook instances
let globalAudioElement: HTMLAudioElement | null = null;

function getAudioElement(): HTMLAudioElement {
  if (!globalAudioElement) {
    globalAudioElement = new Audio();
  }
  return globalAudioElement;
}

/**
 * Hook for managing HTML5 audio playback
 * Handles actual audio playing via the Web Audio API
 */
export function useAudioPlayback() {
  const playAudio = useCallback(async (url: string) => {
    try {
      // Spotify premium tracks (spotify:track: URIs) are handled by the backend's
      // librespot integration, not by the frontend audio player
      if (url.startsWith("spotify:track:")) {
        console.log(
          "Skipping frontend playback for Spotify URI - handled by backend librespot:",
          url,
        );
        return;
      }

      const audio = getAudioElement();

      console.log("Getting audio file for URL:", url);

      // Download audio through backend to bypass CORS
      const fileUrl = await tauriAPI.getAudioFile(url);
      console.log("Audio file URL:", fileUrl);

      audio.src = fileUrl;
      audio
        .play()
        .then(() => {
          console.log("Audio playback started for:", url);
        })
        .catch((error) => {
          console.error("Failed to play audio:", error);
        });
    } catch (error) {
      console.error("Error playing audio:", error);
    }
  }, []);

  const pauseAudio = useCallback(() => {
    const audio = getAudioElement();
    audio.pause();
    console.log("Audio paused");
  }, []);

  const resumeAudio = useCallback(() => {
    const audio = getAudioElement();
    audio
      .play()
      .catch((error) => console.error("Failed to resume audio:", error));
    console.log("Audio resumed");
  }, []);

  const seekAudio = useCallback((positionMs: number) => {
    const audio = getAudioElement();
    audio.currentTime = positionMs / 1000;
    console.log("Seek to:", positionMs, "ms");
  }, []);

  const setVolume = useCallback((volume: number) => {
    const audio = getAudioElement();
    // Volume is 0-100, convert to 0-1
    const normalizedVolume = volume / 100;
    audio.volume = normalizedVolume;
    console.log(
      `Volume set to ${volume}% (normalized: ${normalizedVolume}, actual: ${audio.volume})`,
    );
  }, []);

  const getCurrentPosition = useCallback(() => {
    const audio = getAudioElement();
    return Math.floor(audio.currentTime * 1000);
  }, []);

  const getDuration = useCallback(() => {
    const audio = getAudioElement();
    const durationSeconds = audio.duration;
    if (!Number.isFinite(durationSeconds)) {
      return 0;
    }
    return Math.floor(durationSeconds * 1000);
  }, []);

  const cleanup = useCallback(() => {
    const audio = getAudioElement();
    audio.pause();
    audio.src = "";
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
