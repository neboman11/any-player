import { useState, useEffect } from "react";
import type { PlaylistTrack, ColumnPreferences, Track } from "../types";
import { tauriAPI } from "../api";
import "./TrackTable.css";

interface TrackTableProps {
  tracks: PlaylistTrack[] | Track[];
  onRemoveTrack?: (trackId: number) => void;
  onReorderTrack?: (trackId: number, newPosition: number) => void;
  onPlayTrack?: (track: PlaylistTrack | Track) => void;
  onPlayFromTrack?: (index: number) => void;
}

const DEFAULT_COLUMNS: ColumnPreferences = {
  columns: ["title", "artist", "album", "duration", "source"],
  column_order: [0, 1, 2, 3, 4],
  column_widths: {
    title: 300,
    artist: 200,
    album: 200,
    duration_ms: 100,
    source: 100,
  },
};

export function TrackTable({
  tracks,
  onRemoveTrack,
  onReorderTrack,
  onPlayTrack,
  onPlayFromTrack,
}: TrackTableProps) {
  const [columnPrefs, setColumnPrefs] =
    useState<ColumnPreferences>(DEFAULT_COLUMNS);
  const [draggedTrack, setDraggedTrack] = useState<number | null>(null);

  useEffect(() => {
    const loadPreferences = async () => {
      try {
        const prefs = await tauriAPI.getColumnPreferences();
        setColumnPrefs(prefs);
      } catch (err) {
        console.error("Failed to load column preferences:", err);
      }
    };
    loadPreferences();
  }, []);

  const formatDuration = (ms: number | null) => {
    if (!ms) return "--:--";
    const totalSeconds = Math.floor(ms / 1000);
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return `${minutes}:${seconds.toString().padStart(2, "0")}`;
  };

  const handleDragStart = (trackId: number | string) => {
    setDraggedTrack(Number(trackId));
  };

  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault();
  };

  const handleDrop = (targetPosition: number) => {
    if (draggedTrack === null || !onReorderTrack) return;

    const draggedIndex = tracks.findIndex((t) => t.id === draggedTrack);
    if (draggedIndex !== -1 && draggedIndex !== targetPosition) {
      onReorderTrack(draggedTrack, targetPosition);
    }

    setDraggedTrack(null);
  };

  const visibleColumns = columnPrefs.column_order
    .map((index) => columnPrefs.columns[index])
    .filter((col) => col !== undefined);

  const getColumnValue = (track: PlaylistTrack | Track, column: string) => {
    // Check if it's a PlaylistTrack (has track_source) or Track (has source)
    const isPlaylistTrack = "track_source" in track;

    switch (column) {
      case "title":
        return track.title;
      case "artist":
        return track.artist;
      case "album":
        return track.album || "--";
      case "duration":
        return formatDuration(track.duration_ms || 0);
      case "source":
        return isPlaylistTrack
          ? (track as PlaylistTrack).track_source
          : (track as Track).source;
      default:
        return "";
    }
  };

  const getColumnLabel = (column: string) => {
    switch (column) {
      case "title":
        return "Title";
      case "artist":
        return "Artist";
      case "album":
        return "Album";
      case "duration":
        return "Duration";
      case "source":
        return "Source";
      default:
        return column;
    }
  };

  return (
    <div className="track-table">
      <table>
        <thead>
          <tr>
            {onPlayFromTrack && <th className="play-column"></th>}
            <th className="position-column">#</th>
            {visibleColumns.map((column) => (
              <th
                key={column}
                style={{
                  width: columnPrefs.column_widths[column] || "auto",
                }}
              >
                {getColumnLabel(column)}
              </th>
            ))}
            {onRemoveTrack && <th className="actions-column">Actions</th>}
          </tr>
        </thead>
        <tbody>
          {tracks.map((track, index) => (
            <tr
              key={track.id}
              draggable={!!onReorderTrack}
              onDragStart={() => handleDragStart(track.id)}
              onDragOver={handleDragOver}
              onDrop={() => handleDrop(index)}
              className={draggedTrack === track.id ? "dragging" : ""}
              onClick={() => onPlayTrack?.(track)}
            >
              {onPlayFromTrack && (
                <td className="play-column">
                  <button
                    className="play-track-btn"
                    onClick={(e) => {
                      e.stopPropagation();
                      onPlayFromTrack(index);
                    }}
                    aria-label="Play from here"
                    title="Play from here"
                  >
                    ▶
                  </button>
                </td>
              )}
              <td className="position-column">{index + 1}</td>
              {visibleColumns.map((column) => (
                <td
                  key={column}
                  style={{
                    width: columnPrefs.column_widths[column] || "auto",
                  }}
                >
                  {getColumnValue(track, column)}
                </td>
              ))}
              {onRemoveTrack && (
                <td className="actions-column">
                  <button
                    className="remove-btn"
                    onClick={(e) => {
                      e.stopPropagation();
                      onRemoveTrack(Number(track.id));
                    }}
                    aria-label="Remove track"
                  >
                    ✕
                  </button>
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
      {tracks.length === 0 && (
        <div className="empty-state">No tracks in this playlist</div>
      )}
    </div>
  );
}
