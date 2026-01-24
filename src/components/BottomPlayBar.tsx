import { usePlayback } from "../hooks";

export function BottomPlayBar() {
  const playback = usePlayback();

  // Don't show the bar if there's no current track
  if (!playback.playbackStatus?.current_track) {
    return null;
  }

  const currentTrack = playback.playbackStatus.current_track;

  return (
    <div className="bottom-play-bar">
      <div className="bottom-bar-track-info">
        <div className="bottom-bar-album-art">🎵</div>
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
          title={playback.playbackStatus.state === "playing" ? "Pause" : "Play"}
          onClick={playback.togglePlayPause}
          disabled={playback.isLoading}
        >
          <span>{playback.playbackStatus.state === "playing" ? "⏸" : "▶"}</span>
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

      <div className="bottom-bar-spacer"></div>
    </div>
  );
}
