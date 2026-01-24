import { useMemo, useState } from "react";
import { usePlayback } from "../hooks";
import { NowPlayingControls } from "./NowPlayingControls";
import { ProgressBar } from "./ProgressBar";
import { VolumeControl } from "./VolumeControl";

export function NowPlaying() {
  const playback = usePlayback();
  const [isQueueOpen, setIsQueueOpen] = useState(false);

  const currentTrack = useMemo(() => {
    if (playback.playbackStatus?.current_track) {
      return {
        title: playback.playbackStatus.current_track.title,
        artist: playback.playbackStatus.current_track.artist,
        album: playback.playbackStatus.current_track.album,
      };
    }
    return {
      title: "No track playing",
      artist: "Select a track to play",
    };
  }, [playback.playbackStatus?.current_track]);

  return (
    <section id="now-playing" className="page active">
      <div className="now-playing-wrapper">
        <div className="now-playing-container">
          <div className="album-art">
            <div className="placeholder">🎵</div>
          </div>
          <div className="track-info">
            <h2 id="track-title">{currentTrack.title}</h2>
            <p id="track-artist">{currentTrack.artist}</p>
            <p id="track-album" className="album-name">
              {currentTrack.album}
            </p>
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
                  <li key={`${track.id}-${index}`}>
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
