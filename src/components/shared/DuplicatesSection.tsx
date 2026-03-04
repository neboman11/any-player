import type { DuplicateGroup, PlaylistTrack, Track } from "../../types";

type DuplicateTrack = PlaylistTrack | Track;

interface DuplicatesSectionProps {
  groups: DuplicateGroup[];
  tracks: DuplicateTrack[];
  isReadOnly?: boolean;
  onRemoveDuplicate?: (occurrenceIndex: number) => void;
}

function trackLabel(track: DuplicateTrack | undefined, fallbackKey: string): string {
  if (!track) {
    return fallbackKey;
  }

  const title = track.title?.trim() || "Unknown title";
  const artist = track.artist?.trim() || "Unknown artist";
  return `${title} — ${artist}`;
}

export function DuplicatesSection({
  groups,
  tracks,
  isReadOnly = true,
  onRemoveDuplicate,
}: DuplicatesSectionProps) {
  if (groups.length === 0) {
    return null;
  }

  const canRemove = !isReadOnly && typeof onRemoveDuplicate === "function";

  return (
    <section className="duplicates-section" aria-label="Duplicate tracks">
      <h3>Duplicates</h3>
      <p className="duplicates-section-subtitle">
        Distinct playback keeps the first occurrence and skips the rest.
      </p>

      <div className="duplicates-groups">
        {groups.map((group) => {
          const keeperTrack = tracks[group.first_occurrence_index];
          const keeperLabel = trackLabel(keeperTrack, group.key);

          return (
            <div className="duplicate-group" key={group.key}>
              <div className="duplicate-group-title">{keeperLabel}</div>
              <ul>
                <li>
                  <span>{keeperLabel}</span>
                  <span className="duplicate-keeper-badge">Kept for playback</span>
                </li>

                {group.occurrences.map((occurrence) => {
                  const duplicateTrack = tracks[occurrence.index];
                  const duplicateLabel = trackLabel(duplicateTrack, group.key);

                  return (
                    <li key={`${group.key}:${occurrence.index}`}>
                      <span>{duplicateLabel}</span>
                      {canRemove && (
                        <button
                          type="button"
                          className="remove-btn"
                          onClick={() => onRemoveDuplicate(occurrence.index)}
                        >
                          Remove
                        </button>
                      )}
                    </li>
                  );
                })}
              </ul>
            </div>
          );
        })}
      </div>
    </section>
  );
}
