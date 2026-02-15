import { useMemo, useState } from "react";
import { usePlayback } from "../hooks";
import { NowPlayingControls } from "./NowPlayingControls";
import { ProgressBar } from "./ProgressBar";
import { VolumeControl } from "./VolumeControl";
import {
  getTrackQualityLabel,
  getTrackSourceLabel,
  isSpotifyQualityUnavailable,
} from "../utils/trackIndicators";

export function NowPlaying() {
  const playback = usePlayback();
  const [isQueueOpen, setIsQueueOpen] = useState(false);
  const [imageLoadError, setImageLoadError] = useState(false);

  const currentTrack = useMemo(() => {
    if (playback.playbackStatus?.current_track) {
      const track = playback.playbackStatus.current_track;
      console.log("Current track image_url:", track.image_url);
      return track;
    }
    return null;
  }, [playback.playbackStatus?.current_track]);

  return (
    <section id="now-playing" className="page active">
      <div className="now-playing-wrapper">
        <div className="now-playing-container">
          <div className="album-art">
            {currentTrack?.image_url && !imageLoadError ? (
              <img
                key={currentTrack.image_url}
                src={currentTrack.image_url}
                alt={`${currentTrack.album || currentTrack.title} cover`}
                className="album-art-image"
                onError={() => {
                  console.error(
                    "Failed to load album art:",
                    currentTrack?.image_url,
                  );
                  setImageLoadError(true);
                }}
                onLoad={() =>
                  console.log(
                    "Album art loaded successfully:",
                    currentTrack?.image_url,
                  )
                }
              />
            ) : (
              <div className="placeholder">🎵</div>
            )}
          </div>
          <div className="track-info">
            <h2 id="track-title">{currentTrack?.title || "No track playing"}</h2>
            <p id="track-artist">{currentTrack?.artist || "Select a track to play"}</p>
            <p id="track-album" className="album-name">
              {currentTrack?.album || ""}
            </p>
            {currentTrack && (
              <div className="now-playing-indicators">
                <span className="track-indicator">
                  Source: {getTrackSourceLabel(currentTrack.source)}
                </span>
                <span className="track-indicator">
                  Quality: {getTrackQualityLabel(currentTrack)}
                  {isSpotifyQualityUnavailable(currentTrack) && (
                    <span
                      className="quality-info-icon"
                      title="Spotify does not expose per-track playback bitrate/sample-rate to this app, so exact quality values cannot be shown."
                      aria-label="Spotify quality info"
                    >
                      ℹ
                    </span>
                  )}
                </span>
              </div>
            )}
          </div>
          <ProgressBar
            position={playback.position}
            duration={playback.duration}
            onSeek={playback.seekTo}
          />
          <NowPlayingControls {...playback} />
          <VolumeControl
            volume={playback.volume}
            setVolumeValue={playback.setVolumeValue}
          />
        </div>

        <button
          className="queue-toggle-btn"
          onClick={() => setIsQueueOpen(!isQueueOpen)}
          aria-label={isQueueOpen ? "Close queue" : "Open queue"}
        >
          {isQueueOpen ? "▶" : "◀"}
          <span className="queue-label">Queue</span>
        </button>

        <aside className={`queue-sidebar ${isQueueOpen ? "open" : ""}`}>
          <div className="queue-info">
            <h3>Queue</h3>
            <ul id="queue-list" className="queue-list">
              {playback.playbackStatus?.queue &&
              playback.playbackStatus.queue.length > 0 ? (
                playback.playbackStatus.queue.map((track, index) => (
                  <li
                    key={`${track.id}-${index}`}
                    onClick={() => playback.skipToQueueIndex(index)}
                    title="Click to play this track"
                  >
                    <div className="queue-track-title">{track.title}</div>
                    <div className="queue-track-artist">{track.artist}</div>
                  </li>
                ))
              ) : (
                <li>No tracks in queue</li>
              )}
            </ul>
          </div>
        </aside>
      </div>
    </section>
  );
}
