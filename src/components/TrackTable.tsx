import { useState, useEffect, useMemo } from "react";
import type { PlaylistTrack, ColumnPreferences, Track } from "../types";
import { tauriAPI } from "../api";
import "./TrackTable.css";

type TrackSortColumn = "title" | "artist" | "album" | "duration" | "source";

interface IndexedTrackRow {
  track: PlaylistTrack | Track;
  originalIndex: number;
}

interface TrackTableProps {
  tracks: PlaylistTrack[] | Track[];
  onRemoveTrack?: (trackId: number) => void;
  onReorderTrack?: (trackId: number, newPosition: number) => void;
  onPlayTrack?: (track: PlaylistTrack | Track) => void;
  onPlayFromTrack?: (index: number) => void;
  sortStorageKey?: string;
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
  sortStorageKey = "track-table-default",
}: TrackTableProps) {
  const [columnPrefs, setColumnPrefs] =
    useState<ColumnPreferences>(DEFAULT_COLUMNS);
  const [draggedTrack, setDraggedTrack] = useState<number | null>(null);
  const [sortColumn, setSortColumn] = useState<TrackSortColumn | null>(null);
  const [sortAscending, setSortAscending] = useState<boolean>(true);

  useEffect(() => {
    try {
      const rawValue = window.localStorage.getItem(
        `any-player.track-table.sort.${sortStorageKey}`,
      );
      if (!rawValue) {
        return;
      }

      const parsed = JSON.parse(rawValue) as {
        column?: TrackSortColumn | null;
        ascending?: boolean;
      };

      setSortColumn(parsed.column ?? null);
      setSortAscending(parsed.ascending ?? true);
    } catch (err) {
      console.warn("Failed to restore track table sort state", err);
    }
  }, [sortStorageKey]);

  useEffect(() => {
    try {
      window.localStorage.setItem(
        `any-player.track-table.sort.${sortStorageKey}`,
        JSON.stringify({
          column: sortColumn,
          ascending: sortAscending,
        }),
      );
    } catch (err) {
      console.warn("Failed to persist track table sort state", err);
    }
  }, [sortStorageKey, sortColumn, sortAscending]);

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

  const rows = useMemo<IndexedTrackRow[]>(() => {
    const indexedRows = tracks.map((track, originalIndex) => ({
      track,
      originalIndex,
    }));

    if (!sortColumn) {
      return indexedRows;
    }

    const sorted = [...indexedRows].sort((left, right) => {
      const leftValue = getSortValue(left.track, sortColumn);
      const rightValue = getSortValue(right.track, sortColumn);

      if (typeof leftValue === "number" && typeof rightValue === "number") {
        return leftValue - rightValue;
      }

      return String(leftValue).localeCompare(String(rightValue));
    });

    return sortAscending ? sorted : sorted.reverse();
  }, [tracks, sortColumn, sortAscending]);

  const isSorted = sortColumn !== null;

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

  function getSortValue(
    track: PlaylistTrack | Track,
    column: TrackSortColumn,
  ): string | number {
    const isPlaylistTrack = "track_source" in track;

    switch (column) {
      case "title":
        return track.title.toLowerCase();
      case "artist":
        return track.artist.toLowerCase();
      case "album":
        return (track.album || "").toLowerCase();
      case "duration":
        return track.duration_ms ?? 0;
      case "source":
        return String(
          isPlaylistTrack
            ? (track as PlaylistTrack).track_source
            : (track as Track).source,
        ).toLowerCase();
      default:
        return "";
    }
  }

  const handleColumnSort = (column: string) => {
    const typedColumn = column as TrackSortColumn;
    if (sortColumn !== typedColumn) {
      setSortColumn(typedColumn);
      setSortAscending(true);
      return;
    }

    if (sortAscending) {
      setSortAscending(false);
      return;
    }

    setSortColumn(null);
    setSortAscending(true);
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
                <button
                  type="button"
                  className="column-sort-btn"
                  onClick={() => handleColumnSort(column)}
                >
                  {sortColumn === column
                    ? `${getColumnLabel(column)} ${sortAscending ? "↑" : "↓"}`
                    : getColumnLabel(column)}
                </button>
              </th>
            ))}
            {onRemoveTrack && <th className="actions-column">Actions</th>}
          </tr>
        </thead>
        <tbody>
          {rows.map(({ track, originalIndex }, index) => (
            <tr
              key={track.id}
              draggable={!!onReorderTrack && !isSorted}
              onDragStart={() => handleDragStart(track.id)}
              onDragOver={handleDragOver}
              onDrop={() => {
                if (!isSorted) {
                  handleDrop(index);
                }
              }}
              className={draggedTrack === track.id ? "dragging" : ""}
              onClick={() => onPlayTrack?.(track)}
            >
              {onPlayFromTrack && (
                <td className="play-column">
                  <button
                    className="play-track-btn"
                    onClick={(e) => {
                      e.stopPropagation();
                      onPlayFromTrack(originalIndex);
                    }}
                    aria-label="Play from here"
                    title="Play from here"
                  >
                    ▶
                  </button>
                </td>
              )}
              <td className="position-column">{originalIndex + 1}</td>
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
