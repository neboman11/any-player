import { tauriAPI } from "./api";
import { normalizeServerTarget, toWebSocketUrl } from "./syncHelpers";
import type {
  ExportConfigPayload,
  ExportCustomPlaylist,
  PlaybackStatus,
  PlaylistTrack,
  Track,
} from "./types";

const SYNC_SETTINGS_KEY = "any-player.sync.settings.v1";
const SYNC_CLIENT_ID_KEY = "any-player.sync.client-id.v1";

type SyncDomain =
  | "app_state"
  | "playlists"
  | "provider_configuration"
  | "settings";

export interface SyncSettings {
  serverTarget: string;
  syncAppState: boolean;
  syncPlaylists: boolean;
  syncProviderConfiguration: boolean;
  syncSettings: boolean;
}

interface SyncSnapshotResponse {
  version: number;
  app_state?: unknown;
  playlists?: unknown;
  provider_configuration?: unknown;
  settings?: unknown;
}

interface SyncUpdateEvent {
  event_type?: string;
  namespace?: string;
  version?: number;
  source_client_id?: string | null;
}

interface RemoteAppStatePayload {
  state?: "playing" | "paused" | "stopped";
  shuffle?: boolean;
  repeat_mode?: "off" | "one" | "all";
  volume?: number;
  position?: number;
  duration?: number;
  current_track?: Track | null;
  queue?: Track[];
}

const DEFAULT_SYNC_SETTINGS: SyncSettings = {
  serverTarget: "",
  syncAppState: true,
  syncPlaylists: true,
  syncProviderConfiguration: true,
  syncSettings: true,
};

export function getSyncSettings(): SyncSettings {
  try {
    const raw = localStorage.getItem(SYNC_SETTINGS_KEY);
    if (!raw) {
      return DEFAULT_SYNC_SETTINGS;
    }
    const parsed = JSON.parse(raw) as Partial<SyncSettings>;
    return {
      ...DEFAULT_SYNC_SETTINGS,
      ...parsed,
    };
  } catch {
    return DEFAULT_SYNC_SETTINGS;
  }
}

export function saveSyncSettings(settings: SyncSettings): void {
  localStorage.setItem(SYNC_SETTINGS_KEY, JSON.stringify(settings));
}

function getStableClientId(): string {
  const existing = localStorage.getItem(SYNC_CLIENT_ID_KEY);
  if (existing && existing.trim()) {
    return existing;
  }

  const next =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `desktop-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  localStorage.setItem(SYNC_CLIENT_ID_KEY, next);
  return next;
}

function readBoolean(obj: unknown, ...keys: string[]): boolean | null {
  if (!obj || typeof obj !== "object") {
    return null;
  }
  const map = obj as Record<string, unknown>;
  for (const key of keys) {
    const value = map[key];
    if (typeof value === "boolean") {
      return value;
    }
  }
  return null;
}

function readNumber(obj: unknown, ...keys: string[]): number | null {
  if (!obj || typeof obj !== "object") {
    return null;
  }
  const map = obj as Record<string, unknown>;
  for (const key of keys) {
    const value = map[key];
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
  }
  return null;
}

async function fetchSnapshot(
  serverTarget: string,
): Promise<SyncSnapshotResponse> {
  const base = normalizeServerTarget(serverTarget);
  const response = await fetch(`${base}/v1/snapshot`, {
    method: "GET",
    headers: {
      "x-client-id": getStableClientId(),
    },
  });

  if (!response.ok) {
    throw new Error(`Sync snapshot request failed (${response.status})`);
  }

  return (await response.json()) as SyncSnapshotResponse;
}

async function fetchSnapshotSince(
  serverTarget: string,
  sinceVersion: number,
): Promise<SyncSnapshotResponse | null> {
  const base = normalizeServerTarget(serverTarget);
  const response = await fetch(
    `${base}/v1/snapshot?since_version=${sinceVersion}`,
    {
      method: "GET",
      headers: {
        "x-client-id": getStableClientId(),
      },
    },
  );

  if (response.status === 304) {
    return null;
  }

  if (!response.ok) {
    throw new Error(`Sync snapshot request failed (${response.status})`);
  }

  return (await response.json()) as SyncSnapshotResponse;
}

async function putAppState(
  serverTarget: string,
  payload: RemoteAppStatePayload,
): Promise<void> {
  const base = normalizeServerTarget(serverTarget);
  const response = await fetch(`${base}/v1/state/app-state`, {
    method: "PUT",
    headers: {
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      client_id: getStableClientId(),
      data: payload,
    }),
  });

  if (!response.ok) {
    throw new Error(`Sync put app state failed (${response.status})`);
  }
}

function normalizeTrackSource(value: string): Track["source"] {
  if (value === "spotify" || value === "jellyfin" || value === "plex") {
    return value;
  }
  return "custom";
}

function toTrack(track: PlaylistTrack): Track {
  return {
    id: track.track_id,
    title: track.title,
    artist: track.artist,
    album: track.album ?? undefined,
    duration_ms: track.duration_ms ?? 0,
    source: normalizeTrackSource(track.track_source),
    url: track.url,
    image_url: track.image_url ?? undefined,
  };
}

function readRemotePlaylists(value: unknown): ExportCustomPlaylist[] {
  if (Array.isArray(value)) {
    return value as ExportCustomPlaylist[];
  }
  if (!value || typeof value !== "object") {
    return [];
  }

  const wrapped = value as Partial<ExportConfigPayload> & {
    custom_playlists?: unknown;
  };
  if (Array.isArray(wrapped.custom_playlists)) {
    return wrapped.custom_playlists as ExportCustomPlaylist[];
  }

  return [];
}

async function applyRemotePlaylists(playlistsValue: unknown): Promise<boolean> {
  const remotePlaylists = readRemotePlaylists(playlistsValue);
  const localPlaylists = await tauriAPI.getCustomPlaylists();

  if (
    localPlaylists.length > remotePlaylists.length &&
    !window.confirm(
      `Sync playlists will delete local playlists not present on the server (local: ${localPlaylists.length}, remote: ${remotePlaylists.length}). Continue?`,
    )
  ) {
    return false;
  }

  for (const playlist of localPlaylists) {
    await tauriAPI.deleteCustomPlaylist(playlist.id);
  }

  const idMap = new Map<string, string>();

  for (const item of remotePlaylists) {
    const playlist = item.playlist;
    const created =
      playlist.playlist_type === "union"
        ? await tauriAPI.createUnionPlaylist(
            playlist.name,
            playlist.description,
            playlist.image_url,
          )
        : await tauriAPI.createCustomPlaylist(
            playlist.name,
            playlist.description,
            playlist.image_url,
          );

    idMap.set(playlist.id, created.id);

    if (playlist.playlist_type !== "union") {
      const sortedTracks = [...item.tracks].sort(
        (left, right) => left.position - right.position,
      );
      for (const track of sortedTracks) {
        await tauriAPI.addTrackToCustomPlaylist(created.id, toTrack(track));
      }
    }
  }

  for (const item of remotePlaylists) {
    if (item.playlist.playlist_type !== "union") {
      continue;
    }

    const newUnionId = idMap.get(item.playlist.id);
    if (!newUnionId) {
      continue;
    }

    const sortedSources = [...item.union_sources].sort(
      (left, right) => left.position - right.position,
    );

    for (const source of sortedSources) {
      const mappedSourcePlaylistId =
        idMap.get(source.source_playlist_id) ?? source.source_playlist_id;
      await tauriAPI.addSourceToUnionPlaylist(
        newUnionId,
        source.source_type,
        mappedSourcePlaylistId,
      );
    }
  }

  return true;
}

async function applyProviderConfiguration(value: unknown): Promise<void> {
  if (!value || typeof value !== "object") {
    return;
  }

  const wrapped = value as Record<string, unknown>;
  const providerConfigs =
    wrapped.provider_configs && typeof wrapped.provider_configs === "object"
      ? (wrapped.provider_configs as Record<string, unknown>)
      : wrapped;

  const jellyfin = providerConfigs.jellyfin as
    | Record<string, unknown>
    | undefined;
  const plex = providerConfigs.plex as Record<string, unknown> | undefined;

  const jellyfinUrl =
    typeof jellyfin?.base_url === "string" ? jellyfin.base_url.trim() : "";
  const jellyfinKey =
    typeof jellyfin?.api_key === "string" ? jellyfin.api_key.trim() : "";

  const plexUrl =
    typeof plex?.base_url === "string" ? plex.base_url.trim() : "";
  const plexToken = typeof plex?.token === "string" ? plex.token.trim() : "";

  if (jellyfinUrl && jellyfinKey) {
    await tauriAPI.authenticateJellyfin(jellyfinUrl, jellyfinKey);
  }

  if (plexUrl && plexToken) {
    await tauriAPI.authenticatePlex(plexUrl, plexToken);
  }
}

async function applySettings(value: unknown): Promise<void> {
  const enabled = readBoolean(
    value,
    "audio_normalization_enabled",
    "audioNormalizationEnabled",
  );
  const strictMode = readBoolean(
    value,
    "audio_normalization_strict_mode",
    "audioNormalizationStrictMode",
    "strict_mode",
  );

  if (enabled === null && strictMode === null) {
    return;
  }

  const current = await tauriAPI.getAudioNormalizationSettings();
  await tauriAPI.setAudioNormalizationSettings(
    enabled ?? current.enabled,
    strictMode ?? current.strict_mode,
  );
}

async function applyAppState(value: unknown): Promise<void> {
  const volume = readNumber(value, "volume");
  const shuffle = readBoolean(value, "shuffle");
  const repeat =
    value && typeof value === "object"
      ? (value as Record<string, unknown>).repeat_mode
      : undefined;

  if (typeof volume === "number") {
    await tauriAPI.setVolume(Math.min(100, Math.max(0, Math.round(volume))));
  }

  const playback = await tauriAPI.getPlaybackStatus();
  if (typeof shuffle === "boolean" && playback.shuffle !== shuffle) {
    await tauriAPI.toggleShuffle();
  }

  if (repeat === "off" || repeat === "one" || repeat === "all") {
    await tauriAPI.setRepeatMode(repeat);
  }

  if (!value || typeof value !== "object") {
    return;
  }

  const state = value as RemoteAppStatePayload;
  const remoteCurrent = state.current_track ?? null;
  const remoteQueue = Array.isArray(state.queue) ? state.queue : [];

  if (remoteCurrent) {
    const remoteTracks = [remoteCurrent, ...remoteQueue];
    const sameCurrentTrack =
      playback.current_track?.id === remoteCurrent.id &&
      playback.current_track?.source === remoteCurrent.source;

    if (!sameCurrentTrack) {
      await tauriAPI.playTracksImmediate(remoteTracks);
    }

    if (typeof state.position === "number") {
      const localPosition = playback.position ?? 0;
      const remotePosition = Math.max(0, Math.floor(state.position));
      if (Math.abs(localPosition - remotePosition) > 2500) {
        await tauriAPI.seek(remotePosition);
      }
    }

    if (state.state === "paused") {
      await tauriAPI.pause();
    } else if (state.state === "playing") {
      await tauriAPI.play();
    }
  } else if (state.state === "stopped") {
    await tauriAPI.clearQueue();
  }
}

function buildAppStateFromPlayback(
  playback: PlaybackStatus,
): RemoteAppStatePayload {
  return {
    state: playback.state,
    shuffle: playback.shuffle,
    repeat_mode: playback.repeat_mode,
    volume: playback.volume,
    position: playback.position,
    duration: playback.duration,
    current_track: playback.current_track,
    queue: playback.queue,
  };
}

export interface SyncPullResult {
  appliedDomains: SyncDomain[];
  usedFallback: boolean;
  skippedPlaylistsByConfirmation: boolean;
}

export async function pullSyncSnapshot(
  settings: SyncSettings,
): Promise<SyncPullResult> {
  const serverTarget = normalizeServerTarget(settings.serverTarget);
  if (!serverTarget) {
    return {
      appliedDomains: [],
      usedFallback: true,
      skippedPlaylistsByConfirmation: false,
    };
  }

  let snapshot: SyncSnapshotResponse;
  try {
    snapshot = await fetchSnapshot(serverTarget);
  } catch {
    return {
      appliedDomains: [],
      usedFallback: true,
      skippedPlaylistsByConfirmation: false,
    };
  }

  const appliedDomains: SyncDomain[] = [];
  let skippedPlaylistsByConfirmation = false;

  if (settings.syncSettings && snapshot.settings !== undefined) {
    await applySettings(snapshot.settings);
    appliedDomains.push("settings");
  }

  if (settings.syncAppState && snapshot.app_state !== undefined) {
    await applyAppState(snapshot.app_state);
    appliedDomains.push("app_state");
  }

  if (
    settings.syncProviderConfiguration &&
    snapshot.provider_configuration !== undefined
  ) {
    await applyProviderConfiguration(snapshot.provider_configuration);
    appliedDomains.push("provider_configuration");
  }

  if (settings.syncPlaylists && snapshot.playlists !== undefined) {
    const playlistApplied = await applyRemotePlaylists(snapshot.playlists);
    if (playlistApplied) {
      appliedDomains.push("playlists");
    } else {
      skippedPlaylistsByConfirmation = true;
    }
  }

  return {
    appliedDomains,
    usedFallback: false,
    skippedPlaylistsByConfirmation,
  };
}

export function startRealtimeAppStateSync(): () => void {
  let stopped = false;
  let websocket: WebSocket | null = null;
  let connectedTarget = "";
  let lastSyncedVersion = 0;
  let remoteApplySuppressUntil = 0;
  let lastPushedSignature = "";
  let lastPushAt = 0;

  const closeSocket = () => {
    if (websocket) {
      websocket.close();
      websocket = null;
    }
    connectedTarget = "";
  };

  const connectSocket = (serverTarget: string) => {
    const wsUrl = toWebSocketUrl(serverTarget);
    websocket = new WebSocket(wsUrl);
    connectedTarget = serverTarget;

    websocket.onmessage = async (event) => {
      if (stopped) {
        return;
      }

      try {
        const message = JSON.parse(String(event.data)) as SyncUpdateEvent;
        if (message.event_type !== "state_updated") {
          return;
        }
        if (
          message.namespace !== "app_state" &&
          message.namespace !== "snapshot"
        ) {
          return;
        }
        if (message.source_client_id === getStableClientId()) {
          return;
        }

        const incomingVersion =
          typeof message.version === "number" ? message.version : null;
        if (incomingVersion !== null && incomingVersion <= lastSyncedVersion) {
          return;
        }

        const snapshot = await fetchSnapshotSince(
          serverTarget,
          Math.max(0, lastSyncedVersion),
        );
        if (!snapshot || snapshot.app_state === undefined) {
          return;
        }

        remoteApplySuppressUntil = Date.now() + 3500;
        await applyAppState(snapshot.app_state);
        lastSyncedVersion = Math.max(lastSyncedVersion, snapshot.version ?? 0);
      } catch (err) {
        console.debug("Ignoring sync websocket message", err);
      }
    };

    websocket.onerror = (event) => {
      console.error("Sync websocket error", event);
      if (!stopped && connectedTarget === serverTarget) {
        closeSocket();
      }
    };

    websocket.onclose = () => {
      if (!stopped && connectedTarget === serverTarget) {
        connectedTarget = "";
        websocket = null;
      }
    };
  };

  const transportInterval = window.setInterval(() => {
    if (stopped) {
      return;
    }

    const settings = getSyncSettings();
    const target = normalizeServerTarget(settings.serverTarget);
    if (!target || !settings.syncAppState) {
      closeSocket();
      return;
    }

    if (!websocket || websocket.readyState === WebSocket.CLOSED) {
      connectSocket(target);
    } else if (connectedTarget !== target) {
      closeSocket();
      connectSocket(target);
    }
  }, 5000);

  const pushInterval = window.setInterval(async () => {
    if (stopped) {
      return;
    }

    const settings = getSyncSettings();
    const target = normalizeServerTarget(settings.serverTarget);
    if (!target || !settings.syncAppState) {
      return;
    }
    if (Date.now() < remoteApplySuppressUntil) {
      return;
    }

    try {
      const playback = await tauriAPI.getPlaybackStatus();
      const payload = buildAppStateFromPlayback(playback);

      const positionBucket = Math.floor((payload.position ?? 0) / 5000);
      const signature = JSON.stringify({
        state: payload.state,
        current_track: payload.current_track
          ? `${payload.current_track.source}:${payload.current_track.id}`
          : null,
        queue: (payload.queue ?? []).map(
          (track) => `${track.source}:${track.id}`,
        ),
        shuffle: payload.shuffle,
        repeat_mode: payload.repeat_mode,
        volume: payload.volume,
        position_bucket: positionBucket,
      });

      if (
        signature === lastPushedSignature &&
        Date.now() - lastPushAt < 15000
      ) {
        return;
      }

      await putAppState(target, payload);
      lastPushedSignature = signature;
      lastPushAt = Date.now();
    } catch {
      console.debug(
        "Sync server unavailable; continuing with local playback state",
      );
    }
  }, 2000);

  return () => {
    stopped = true;
    closeSocket();
    clearInterval(transportInterval);
    clearInterval(pushInterval);
  };
}
