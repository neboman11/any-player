import { useCallback } from "react";
import { usePlayback } from "../hooks";

export function NowPlayingControls({
  playbackStatus,
  togglePlayPause,
  nextTrack,
  previousTrack,
  toggleShuffle,
  cycleRepeatMode,
  shuffle,
  repeatMode,
  isLoading,
  playbackDisabledReason,
}: ReturnType<typeof usePlayback>) {
  const getRepeatIcon = useCallback(() => {
    const icons = {
      off: "🔁",
      one: "🔂",
      all: "🔁",
    };
    return icons[repeatMode];
  }, [repeatMode]);

  return (
    <div className="playback-controls">
      <button
        id="btn-shuffle"
        className="control-btn"
        title="Shuffle"
        onClick={toggleShuffle}
        style={{ opacity: shuffle ? "1" : "0.5" }}
        disabled={isLoading}
      >
        <span>🔀</span>
      </button>
      <button
        id="btn-previous"
        className="control-btn"
        title="Previous"
        onClick={previousTrack}
        disabled={isLoading}
      >
        <span>⏮</span>
      </button>
      <button
        id="btn-play-pause"
        className="control-btn play-pause"
        title={
          playbackDisabledReason ||
          (playbackStatus?.state === "playing" ? "Pause" : "Play")
        }
        aria-label={
          playbackDisabledReason ||
          (playbackStatus?.state === "playing" ? "Pause" : "Play")
        }
        onClick={togglePlayPause}
        disabled={isLoading || Boolean(playbackDisabledReason)}
      >
        <span>{playbackStatus?.state === "playing" ? "⏸" : "▶"}</span>
      </button>
      <button
        id="btn-next"
        className="control-btn"
        title="Next"
        onClick={nextTrack}
        disabled={isLoading}
      >
        <span>⏭</span>
      </button>
      <button
        id="btn-repeat"
        className="control-btn"
        title="Repeat"
        onClick={cycleRepeatMode}
        style={{ opacity: repeatMode !== "off" ? "1" : "0.5" }}
        disabled={isLoading}
      >
        <span>{getRepeatIcon()}</span>
      </button>
    </div>
  );
}
