import type { Track, PlaylistTrack } from "../types";

export function filterTracks(
  tracks: (Track | PlaylistTrack)[],
  query: string,
): (Track | PlaylistTrack)[] {
  if (query.trim() === "") return tracks;

  const lowerQuery = query.toLowerCase();

  return tracks.filter((track) => {
    const title = track.title?.toLowerCase() || "";
    const artist = track.artist?.toLowerCase() || "";
    const album = track.album?.toLowerCase() || "";
    const source =
      ("track_source" in track
        ? track.track_source
        : (track as Track).source
      )?.toLowerCase() || "";

    return (
      title.includes(lowerQuery) ||
      artist.includes(lowerQuery) ||
      album.includes(lowerQuery) ||
      source.includes(lowerQuery)
    );
  });
}
