/**
 * Bridges Spotify's Web Playback SDK (https://sdk.scdn.co/spotify-player.js) into the
 * app. This SDK streams audio directly in this webview via public Web API + browser
 * EME/DRM, replacing the in-process librespot engine, which stopped working once
 * Spotify started rejecting non-approved client IDs on its private `spclient`
 * endpoints. Requires a Spotify Premium account.
 */
import { tauriAPI } from "./api";

const SDK_SCRIPT_URL = "https://sdk.scdn.co/spotify-player.js";
const PLAYER_NAME = "Any Player";
// Reporting throttle while position advances continuously during normal playback.
// State changes (paused toggled) are always reported immediately regardless of this.
const STATE_REPORT_THROTTLE_MS = 900;
// After the true end of a track, the SDK reports paused=true at position=0. Only
// treat that as "track finished" if we were close to the end just before it happened.
const END_OF_TRACK_POSITION_EPSILON_MS = 1500;

interface SpotifyPlayerTrack {
  uri: string;
}

interface SpotifyPlaybackStateData {
  paused: boolean;
  position: number;
  duration: number;
  track_window: {
    current_track: SpotifyPlayerTrack;
  };
}

interface SpotifyPlayerErrorEvent {
  message: string;
}

interface SpotifyPlayerInstance {
  connect(): Promise<boolean>;
  disconnect(): void;
  addListener(event: string, callback: (payload: never) => void): void;
  getCurrentState(): Promise<SpotifyPlaybackStateData | null>;
  setVolume(volume: number): Promise<void>;
  pause(): Promise<void>;
  resume(): Promise<void>;
  seek(positionMs: number): Promise<void>;
  togglePlay(): Promise<void>;
}

interface SpotifyPlayerConstructorOptions {
  name: string;
  getOAuthToken: (callback: (token: string) => void) => void;
  volume?: number;
}

declare global {
  interface Window {
    onSpotifyWebPlaybackSDKReady?: () => void;
    Spotify?: {
      Player: new (
        options: SpotifyPlayerConstructorOptions,
      ) => SpotifyPlayerInstance;
    };
  }
}

export type SpotifyWebPlaybackErrorType =
  | "initialization_error"
  | "authentication_error"
  | "account_error"
  | "playback_error";
export type SpotifyWebPlaybackErrorListener = (
  type: SpotifyWebPlaybackErrorType,
  message: string,
) => void;

let player: SpotifyPlayerInstance | null = null;
let deviceId: string | null = null;
let sdkReadyPromise: Promise<void> | null = null;
// True once we've asked the SDK to start playing the current backend track. Cleared
// whenever the backend's current track is no longer a Spotify track, so `isActive()`
// accurately reflects "is the SDK currently driving playback for the active track".
let activeForCurrentTrack = false;

let lastReportedAt = 0;
let lastReportedPaused: boolean | null = null;
let lastKnownPositionMs = 0;
let lastKnownDurationMs = 0;

const errorListeners = new Set<SpotifyWebPlaybackErrorListener>();

function notifyError(type: SpotifyWebPlaybackErrorType, message: string) {
  console.error(`[SpotifyWebPlayback] ${type}: ${message}`);
  for (const listener of errorListeners) {
    listener(type, message);
  }
}

function loadSdkScript(): Promise<void> {
  if (sdkReadyPromise) {
    return sdkReadyPromise;
  }

  sdkReadyPromise = new Promise((resolve) => {
    if (window.Spotify) {
      resolve();
      return;
    }
    window.onSpotifyWebPlaybackSDKReady = () => resolve();
    if (!document.querySelector(`script[src="${SDK_SCRIPT_URL}"]`)) {
      const script = document.createElement("script");
      script.src = SDK_SCRIPT_URL;
      script.async = true;
      document.head.appendChild(script);
    }
  });

  return sdkReadyPromise;
}

function reportState(state: SpotifyPlaybackStateData | null) {
  if (!state) {
    return;
  }

  const now = Date.now();
  const pausedChanged = state.paused !== lastReportedPaused;
  if (!pausedChanged && now - lastReportedAt < STATE_REPORT_THROTTLE_MS) {
    return;
  }
  lastReportedAt = now;
  lastReportedPaused = state.paused;

  void tauriAPI
    .reportSpotifyPlaybackState(state.position, state.duration, !state.paused)
    .catch((err) => {
      console.error("Failed to report Spotify playback state to backend:", err);
    });
}

function handlePlayerStateChanged(state: SpotifyPlaybackStateData | null) {
  reportState(state);

  if (state) {
    const isEndOfTrack =
      state.paused &&
      state.position === 0 &&
      lastKnownPositionMs > 0 &&
      lastKnownDurationMs > 0 &&
      lastKnownPositionMs >= lastKnownDurationMs - END_OF_TRACK_POSITION_EPSILON_MS;

    lastKnownPositionMs = state.position;
    lastKnownDurationMs = state.duration;

    if (isEndOfTrack) {
      advanceToNextTrack();
    }
  }
}

function advanceToNextTrack() {
  void tauriAPI.nextTrack().catch((err) => {
    console.error("Failed to advance to next track after Spotify track ended:", err);
  });
}

async function ensurePlayer(): Promise<SpotifyPlayerInstance> {
  if (player) {
    return player;
  }

  await loadSdkScript();
  if (!window.Spotify) {
    throw new Error("Spotify Web Playback SDK failed to load");
  }

  const instance = new window.Spotify.Player({
    name: PLAYER_NAME,
    getOAuthToken: (callback) => {
      tauriAPI
        .getSpotifyAccessToken()
        .then(callback)
        .catch((err) => {
          notifyError(
            "authentication_error",
            err instanceof Error ? err.message : String(err),
          );
        });
    },
    volume: 1,
  });

  instance.addListener("ready", (payload: { device_id: string }) => {
    deviceId = payload.device_id;
  });
  instance.addListener("not_ready", () => {
    deviceId = null;
  });
  instance.addListener(
    "player_state_changed",
    handlePlayerStateChanged as (payload: never) => void,
  );
  instance.addListener("initialization_error", (payload: SpotifyPlayerErrorEvent) => {
    notifyError("initialization_error", payload.message);
  });
  instance.addListener("authentication_error", (payload: SpotifyPlayerErrorEvent) => {
    notifyError("authentication_error", payload.message);
  });
  instance.addListener("account_error", (payload: SpotifyPlayerErrorEvent) => {
    notifyError("account_error", payload.message);
  });
  instance.addListener("playback_error", (payload: SpotifyPlayerErrorEvent) => {
    notifyError("playback_error", payload.message);
  });

  await instance.connect();
  player = instance;
  return instance;
}

async function ensureReady(): Promise<boolean> {
  try {
    await ensurePlayer();
    // `connect()` resolves once the transport is up, but `device_id` only arrives
    // via the subsequent `ready` event - give it a brief moment to arrive.
    for (let attempt = 0; !deviceId && attempt < 20; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return deviceId !== null;
  } catch (err) {
    notifyError(
      "initialization_error",
      err instanceof Error ? err.message : String(err),
    );
    return false;
  }
}

export const spotifyWebPlayback = {
  /** Whether the SDK is currently the active audio path for the backend's current track. */
  isActive(): boolean {
    return activeForCurrentTrack && player !== null && deviceId !== null;
  },

  getDeviceId(): string | null {
    return deviceId;
  },

  onError(listener: SpotifyWebPlaybackErrorListener): () => void {
    errorListeners.add(listener);
    return () => errorListeners.delete(listener);
  },

  /** Call when the backend's current track is no longer a Spotify track. */
  deactivate(): void {
    activeForCurrentTrack = false;
  },

  /** Start playback of a single `spotify:track:...` URI on this device. */
  async playUris(uris: string[]): Promise<void> {
    const ready = await ensureReady();
    if (!ready || !deviceId) {
      throw new Error("Spotify Web Playback SDK device not ready");
    }

    lastKnownPositionMs = 0;
    lastKnownDurationMs = 0;
    lastReportedPaused = null;

    const token = await tauriAPI.getSpotifyAccessToken();
    const response = await fetch(
      `https://api.spotify.com/v1/me/player/play?device_id=${encodeURIComponent(deviceId)}`,
      {
        method: "PUT",
        headers: {
          Authorization: `Bearer ${token}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ uris }),
      },
    );

    if (!response.ok && response.status !== 204) {
      const body = await response.text().catch(() => "");
      throw new Error(`Spotify play request failed: ${response.status} ${body}`);
    }

    activeForCurrentTrack = true;
  },

  async pause(): Promise<void> {
    if (player) {
      await player.pause();
    }
  },

  async resume(): Promise<void> {
    if (player) {
      await player.resume();
    }
  },

  async togglePlay(): Promise<void> {
    if (player) {
      await player.togglePlay();
    }
  },

  async seek(positionMs: number): Promise<void> {
    if (player) {
      await player.seek(positionMs);
    }
  },

  async setVolume(volume0to100: number): Promise<void> {
    if (player) {
      await player.setVolume(Math.max(0, Math.min(100, volume0to100)) / 100);
    }
  },
};
