import type { Track } from "../types";

const SOURCE_LABELS: Record<Track["source"], string> = {
  spotify: "Spotify",
  jellyfin: "Jellyfin",
  plex: "Plex",
  custom: "Custom",
};

export function getTrackSourceLabel(source: Track["source"]): string {
  return SOURCE_LABELS[source] ?? "Unknown";
}

export function getTrackQualityLabel(
  track: Pick<Track, "bitrate_kbps" | "sample_rate_hz">,
): string {
  const parts: string[] = [];

  if (typeof track.bitrate_kbps === "number" && track.bitrate_kbps > 0) {
    parts.push(`${track.bitrate_kbps} kbps`);
  }

  if (typeof track.sample_rate_hz === "number" && track.sample_rate_hz > 0) {
    const sampleRateKhz = track.sample_rate_hz / 1000;
    const sampleRateLabel = Number.isInteger(sampleRateKhz)
      ? `${sampleRateKhz.toFixed(0)} kHz`
      : `${sampleRateKhz.toFixed(1)} kHz`;
    parts.push(sampleRateLabel);
  }

  if (parts.length === 0) {
    return "Unknown";
  }

  return parts.join(" • ");
}

export function isSpotifyQualityUnavailable(
  track: Pick<Track, "source" | "bitrate_kbps" | "sample_rate_hz">,
): boolean {
  return (
    track.source === "spotify" &&
    !(typeof track.bitrate_kbps === "number" && track.bitrate_kbps > 0) &&
    !(typeof track.sample_rate_hz === "number" && track.sample_rate_hz > 0)
  );
}
