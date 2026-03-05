import type {
  DuplicateGroup,
  DuplicateOccurrence,
  PlaylistTrack,
  Track,
} from "../types";

type DuplicateTrack = PlaylistTrack | Track;

function normalizeDuplicateKey(track: DuplicateTrack): string {
  return `${track.title.trim().toLowerCase()}|${track.artist.trim().toLowerCase()}`;
}

function getTrackId(track: DuplicateTrack): string {
  return "track_source" in track ? track.track_id : String(track.id);
}

export function buildDuplicateGroups(
  tracks: PlaylistTrack[] | Track[],
): DuplicateGroup[] {
  const firstOccurrenceIndex = new Map<string, number>();
  const duplicateOccurrences = new Map<string, DuplicateOccurrence[]>();

  tracks.forEach((track, index) => {
    const key = normalizeDuplicateKey(track);
    const firstIndex = firstOccurrenceIndex.get(key);

    if (firstIndex === undefined) {
      firstOccurrenceIndex.set(key, index);
      return;
    }

    const nextOccurrence: DuplicateOccurrence = {
      index,
      track_id: getTrackId(track),
    };
    const occurrences = duplicateOccurrences.get(key);
    if (occurrences) {
      occurrences.push(nextOccurrence);
    } else {
      duplicateOccurrences.set(key, [nextOccurrence]);
    }
  });

  return Array.from(firstOccurrenceIndex.entries())
    .filter(([key]) => duplicateOccurrences.has(key))
    .map(([key, firstIndex]) => ({
      key,
      first_occurrence_index: firstIndex,
      occurrences: duplicateOccurrences.get(key) ?? [],
    }));
}

/**
 * Returns a copy of `tracks` with duplicate title+artist entries removed,
 * keeping only the first occurrence of each key (mirrors backend dedup logic).
 */
export function buildDistinctTracks<T extends DuplicateTrack>(tracks: T[]): T[] {
  const groups = buildDuplicateGroups(tracks as PlaylistTrack[] | Track[]);
  const duplicateIndices = new Set<number>();
  for (const group of groups) {
    for (const occ of group.occurrences) {
      duplicateIndices.add(occ.index);
    }
  }
  return tracks.filter((_, index) => !duplicateIndices.has(index));
}
