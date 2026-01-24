import { useCallback, useMemo } from "react";
import { usePlayback } from "../hooks";

export function BottomPlayBar() {
  const playback = usePlayback();

  const progressPercentage = useMemo(() => {
    if (!playback.duration || playback.duration === 0) return 0;
    return (playback.position / playback.duration) * 100;
  }, [playback.position, playback.duration]);

  const handleProgressChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const percentage = Number(e.target.value);
      const positionMs = (percentage / 100) * (playback.duration || 1);
      void playback.seekTo(positionMs);
    },
    [playback.duration, playback.seekTo],
  );

  const handleVolumeChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const value = Number(e.target.value);
      void playback.setVolumeValue(value);
    },
    [playback.setVolumeValue],
  );

  // Don't show the bar if there's no current track
  if (!playback.playbackStatus?.current_track) {
    return null;
  }

  const currentTrack = playback.playbackStatus.current_track;

  return (
    <div className="bottom-play-bar">
      <div className="bottom-bar-progress">
        <input
          type="range"
          className="bottom-bar-progress-slider"
          min="0"
          max="100"
          value={progressPercentage}
          onChange={handleProgressChange}
          style={{ "--progress": progressPercentage } as React.CSSProperties}
        />
      </div>

      <div className="bottom-bar-content">
        <div className="bottom-bar-track-info">
          <div className="bottom-bar-album-art">
            {currentTrack.image_url ? (
              <img
                src={currentTrack.image_url}
                alt={`${currentTrack.album || currentTrack.title} cover`}
                className="bottom-bar-album-art-image"
                onError={(e) => {
                  console.error(
                    "Failed to load bottom bar album art:",
                    currentTrack.image_url,
                  );
                  e.currentTarget.style.display = "none";
                }}
              />
            ) : (
              "🎵"
            )}
          </div>
          <div className="bottom-bar-text">
            <div className="bottom-bar-title">{currentTrack.title}</div>
            <div className="bottom-bar-artist">{currentTrack.artist}</div>
          </div>
        </div>

        <div className="bottom-bar-controls">
          <button
            className="bottom-bar-control-btn"
            title="Previous"
            onClick={playback.previousTrack}
            disabled={playback.isLoading}
          >
            <span>⏮</span>
          </button>
          <button
            className="bottom-bar-control-btn bottom-bar-play-pause"
            title={
              playback.playbackStatus.state === "playing" ? "Pause" : "Play"
            }
            onClick={playback.togglePlayPause}
            disabled={playback.isLoading}
          >
            <span>
              {playback.playbackStatus.state === "playing" ? "⏸" : "▶"}
            </span>
          </button>
          <button
            className="bottom-bar-control-btn"
            title="Next"
            onClick={playback.nextTrack}
            disabled={playback.isLoading}
          >
            <span>⏭</span>
          </button>
        </div>

        <div className="bottom-bar-volume">
          <span className="bottom-bar-volume-icon">🔊</span>
          <input
            type="range"
            className="bottom-bar-volume-slider"
            min="0"
            max="100"
            value={playback.volume}
            onChange={handleVolumeChange}
          />
          <span className="bottom-bar-volume-value">{playback.volume}%</span>
        </div>
      </div>
    </div>
  );
}
