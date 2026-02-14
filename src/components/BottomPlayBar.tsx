import { useCallback, useMemo, useState } from "react";
import { usePlayback } from "../hooks";
import {
  getTrackQualityLabel,
  getTrackSourceLabel,
  isSpotifyQualityUnavailable,
} from "../utils/trackIndicators";

export function BottomPlayBar() {
  const playback = usePlayback();
  const [imageLoadError, setImageLoadError] = useState(false);

  const currentTrack = playback.playbackStatus?.current_track;

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
    [playback],
  );

  const handleVolumeChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const value = Number(e.target.value);
      void playback.setVolumeValue(value);
    },
    [playback],
  );

  const formatTime = useCallback((ms: number): string => {
    if (!ms || Number.isNaN(ms)) return "0:00";
    const totalSeconds = Math.floor(ms / 1000);
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return `${minutes}:${seconds.toString().padStart(2, "0")}`;
  }, []);

  const repeatIcon = useMemo(() => {
    const icons = {
      off: "🔁",
      one: "🔂",
      all: "🔁",
    };
    return icons[playback.repeatMode];
  }, [playback.repeatMode]);

  // Don't show the bar if there's no current track
  if (!currentTrack) {
    return null;
  }

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
            {currentTrack.image_url && !imageLoadError ? (
              <img
                key={currentTrack.image_url}
                src={currentTrack.image_url}
                alt={`${currentTrack.album || currentTrack.title} cover`}
                className="bottom-bar-album-art-image"
                onError={() => {
                  console.error(
                    "Failed to load bottom bar album art:",
                    currentTrack.image_url,
                  );
                  setImageLoadError(true);
                }}
              />
            ) : (
              "🎵"
            )}
          </div>
          <div className="bottom-bar-text">
            <div className="bottom-bar-title">{currentTrack.title}</div>
            <div className="bottom-bar-artist">{currentTrack.artist}</div>
            <div className="bottom-bar-indicators">
              <span className="bottom-bar-indicator">
                {getTrackSourceLabel(currentTrack.source)}
              </span>
              <span className="bottom-bar-indicator">
                {getTrackQualityLabel(currentTrack)}
                {isSpotifyQualityUnavailable(currentTrack) && (
                  <span
                    className="bottom-bar-quality-info-icon"
                    title="Spotify does not expose per-track playback bitrate/sample-rate to this app, so exact quality values cannot be shown."
                    aria-label="Spotify quality info"
                  >
                    ℹ
                  </span>
                )}
              </span>
            </div>
          </div>
          <div className="bottom-bar-time">
            <span className="bottom-bar-time-current">
              {formatTime(playback.position)}
            </span>
            <span className="bottom-bar-time-separator">/</span>
            <span className="bottom-bar-time-total">
              {formatTime(playback.duration)}
            </span>
          </div>
        </div>

        <div className="bottom-bar-controls">
          <button
            className="bottom-bar-control-btn"
            title="Shuffle"
            onClick={playback.toggleShuffle}
            style={{ opacity: playback.shuffle ? "1" : "0.5" }}
            disabled={playback.isLoading}
          >
            <span>🔀</span>
          </button>
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
              playback.playbackStatus?.state === "playing" ? "Pause" : "Play"
            }
            onClick={playback.togglePlayPause}
            disabled={playback.isLoading}
          >
            <span>
              {playback.playbackStatus?.state === "playing" ? "⏸" : "▶"}
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
          <button
            className="bottom-bar-control-btn"
            title="Repeat"
            onClick={playback.cycleRepeatMode}
            style={{ opacity: playback.repeatMode !== "off" ? "1" : "0.5" }}
            disabled={playback.isLoading}
          >
            <span>{repeatIcon}</span>
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
