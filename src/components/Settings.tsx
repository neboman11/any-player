import { useState, useCallback, useEffect, useMemo } from "react";
import {
  useSpotifyAuth,
  useJellyfinAuth,
  usePlexAuth,
  usePlaylists,
} from "../hooks";
import { save } from "@tauri-apps/plugin-dialog";
import { tauriAPI } from "../api";
import { LoadingSpinner } from "./shared/LoadingSpinner";
import {
  getSyncSettings,
  pullSyncSnapshot,
  saveSyncSettings,
  type SyncSettings,
} from "../sync";

type SettingsTab = "general" | "spotify" | "jellyfin" | "plex";

const SETTINGS_TABS: SettingsTab[] = ["general", "spotify", "jellyfin", "plex"];

interface AuthModalProps {
  authUrl: string;
  onClose: () => void;
}

function AuthModal({ authUrl, onClose }: AuthModalProps) {
  const handleCopyLink = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(authUrl);
      alert("Link copied to clipboard!");
    } catch (err) {
      console.error("Failed to copy:", err);
    }
  }, [authUrl]);

  return (
    <div
      id="auth-fallback"
      style={{
        position: "fixed",
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        background: "rgba(0, 0, 0, 0.8)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 10000,
      }}
    >
      <div
        style={{
          background: "white",
          padding: "30px",
          borderRadius: "10px",
          maxWidth: "500px",
          textAlign: "center",
          boxShadow: "0 4px 20px rgba(0, 0, 0, 0.3)",
        }}
      >
        <h2>Complete Spotify Login</h2>
        <p style={{ margin: "20px 0", color: "#666" }}>
          Click the button below to log in to Spotify, or copy the link:
        </p>
        <div style={{ margin: "20px 0" }}>
          <a
            href={authUrl}
            target="_blank"
            rel="noopener noreferrer"
            style={{
              display: "inline-block",
              background: "#1DB954",
              color: "white",
              padding: "12px 30px",
              borderRadius: "25px",
              textDecoration: "none",
              fontWeight: "bold",
              marginBottom: "15px",
            }}
          >
            Open Spotify Login
          </a>
        </div>
        <p style={{ fontSize: "12px", color: "#999", margin: "15px 0" }}>
          Or copy this link:
        </p>
        <div
          style={{
            background: "#f5f5f5",
            padding: "10px",
            borderRadius: "5px",
            margin: "10px 0",
            wordBreak: "break-all",
            fontFamily: "monospace",
            fontSize: "12px",
          }}
        >
          <input
            type="text"
            value={authUrl}
            readOnly
            style={{
              width: "100%",
              border: "none",
              background: "transparent",
              fontFamily: "monospace",
              fontSize: "12px",
              padding: "5px",
            }}
          />
        </div>
        <button
          onClick={handleCopyLink}
          style={{
            background: "#ddd",
            border: "none",
            padding: "8px 16px",
            borderRadius: "5px",
            cursor: "pointer",
            fontSize: "12px",
          }}
        >
          Copy Link
        </button>
        <p
          style={{
            marginTop: "20px",
            color: "#666",
            fontSize: "14px",
          }}
        >
          After logging in, this window will automatically close.
        </p>
        <button
          onClick={onClose}
          style={{
            marginTop: "15px",
            background: "#f0f0f0",
            border: "none",
            padding: "10px 20px",
            borderRadius: "5px",
            cursor: "pointer",
          }}
        >
          Close
        </button>
      </div>
    </div>
  );
}

export function Settings() {
  const [jellyfinUrl, setJellyfinUrl] = useState<string>("");
  const [jellyfinApiKey, setJellyfinApiKey] = useState<string>("");
  const [jellyfinPlaylistPageSize, setJellyfinPlaylistPageSize] =
    useState<string>("300");
  const [plexUrl, setPlexUrl] = useState<string>("");
  const [plexToken, setPlexToken] = useState<string>("");
  const [plexPlaylistPageSize, setPlexPlaylistPageSize] =
    useState<string>("300");
  const [showApiKey, setShowApiKey] = useState<boolean>(false);
  const [showPlexToken, setShowPlexToken] = useState<boolean>(false);
  const [autoplay, setAutoplay] = useState<boolean>(false);
  const [audioNormalizationEnabled, setAudioNormalizationEnabled] =
    useState<boolean>(false);
  const [audioNormalizationStrictMode, setAudioNormalizationStrictMode] =
    useState<boolean>(false);
  const [isExportingConfig, setIsExportingConfig] = useState<boolean>(false);
  const [isClearingProviderCache, setIsClearingProviderCache] =
    useState<boolean>(false);
  const [exportError, setExportError] = useState<string>("");
  const [exportSuccessPath, setExportSuccessPath] = useState<string>("");
  const [activeTab, setActiveTab] = useState<SettingsTab>("general");
  const [syncSettings, setSyncSettings] =
    useState<SyncSettings>(getSyncSettings());
  const [syncStatus, setSyncStatus] = useState<string>("");
  const [syncLoading, setSyncLoading] = useState<boolean>(false);

  const spotify = useSpotifyAuth();
  const jellyfin = useJellyfinAuth();
  const plex = usePlexAuth();
  const { clearCache, loadPlaylists } = usePlaylists();

  // Refresh authentication status when Settings page is mounted/visible
  useEffect(() => {
    const refreshAuthStatus = async () => {
      // Refresh Spotify auth status
      if (spotify.checkAuthStatus) {
        await spotify.checkAuthStatus();
      }

      // Refresh Jellyfin auth status
      if (jellyfin.checkAuthStatus) {
        await jellyfin.checkAuthStatus();
      }

      // Refresh Plex auth status
      if (plex.checkAuthStatus) {
        await plex.checkAuthStatus();
      }
    };

    void refreshAuthStatus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // Only run on mount - we want to refresh status when Settings page loads

  // Load stored Jellyfin credentials when component mounts or connection state changes
  useEffect(() => {
    const loadCredentials = async () => {
      try {
        const credentials = await tauriAPI.getJellyfinCredentials();
        if (credentials) {
          const [url, apiKey] = credentials;
          setJellyfinUrl(url);
          setJellyfinApiKey(apiKey);
        }
      } catch (err) {
        console.error("Failed to load Jellyfin credentials:", err);
      }
    };

    if (jellyfin.isConnected) {
      void loadCredentials();
    }
  }, [jellyfin.isConnected]);

  // Load stored Plex credentials when component mounts or connection state changes
  useEffect(() => {
    const loadCredentials = async () => {
      try {
        const credentials = await tauriAPI.getPlexCredentials();
        if (credentials) {
          const [url, token] = credentials;
          setPlexUrl(url);
          setPlexToken(token);
        }
      } catch (err) {
        console.error("Failed to load Plex credentials:", err);
      }
    };

    if (plex.isConnected) {
      void loadCredentials();
    }
  }, [plex.isConnected]);

  useEffect(() => {
    const loadAudioNormalizationSettings = async () => {
      try {
        const settings = await tauriAPI.getAudioNormalizationSettings();
        setAudioNormalizationEnabled(settings.enabled);
        setAudioNormalizationStrictMode(settings.strict_mode);
      } catch (err) {
        console.error("Failed to load audio normalization settings:", err);
      }
    };

    void loadAudioNormalizationSettings();
  }, []);

  const handleSpotifyConnect = useCallback(async () => {
    if (spotify.isConnected) {
      await spotify.disconnect();
      // Clear playlist cache when disconnecting
      clearCache();
    } else {
      try {
        await spotify.connect();
        // Reload playlists after successfully connecting
        // Wait a bit for the connection to be fully established
        setTimeout(async () => {
          try {
            const isAuth = await tauriAPI.isSpotifyAuthenticated();
            if (isAuth) {
              await loadPlaylists("all", true);
            }
          } catch (err) {
            console.error("Failed to reload playlists:", err);
          }
        }, 1000);
      } catch (err) {
        console.error("Spotify connection error:", err);
      }
    }
  }, [spotify, clearCache, loadPlaylists]);

  const handleJellyfinConnect = useCallback(async () => {
    if (jellyfin.isConnected) {
      await jellyfin.disconnect();
      // Clear playlist cache when disconnecting
      clearCache();
      // Clear fields after disconnecting
      setJellyfinUrl("");
      setJellyfinApiKey("");
      setShowApiKey(false);
    } else {
      const parsedJellyfinPageSize = parseInt(jellyfinPlaylistPageSize, 10);
      const pageSize = Number.isFinite(parsedJellyfinPageSize)
        ? Math.min(Math.max(parsedJellyfinPageSize, 1), 1000)
        : 300;
      await jellyfin.connect(jellyfinUrl, jellyfinApiKey, pageSize);
      // Reload playlists after successfully connecting
      setTimeout(async () => {
        try {
          const isAuth = await tauriAPI.isJellyfinAuthenticated();
          if (isAuth) {
            await loadPlaylists("all", true);
          }
        } catch (err) {
          console.error("Failed to reload playlists:", err);
        }
      }, 1000);
    }
  }, [
    jellyfin,
    jellyfinUrl,
    jellyfinApiKey,
    jellyfinPlaylistPageSize,
    clearCache,
    loadPlaylists,
  ]);

  const handlePlexConnect = useCallback(async () => {
    if (plex.isConnected) {
      await plex.disconnect();
      clearCache();
      setPlexUrl("");
      setPlexToken("");
      setShowPlexToken(false);
    } else {
      const parsedPlexPageSize = parseInt(plexPlaylistPageSize, 10);
      const pageSize = Number.isFinite(parsedPlexPageSize)
        ? Math.min(Math.max(parsedPlexPageSize, 1), 1000)
        : 300;
      await plex.connect(plexUrl, plexToken, pageSize);
      setTimeout(async () => {
        try {
          const isAuth = await tauriAPI.isPlexAuthenticated();
          if (isAuth) {
            await loadPlaylists("all", true);
          }
        } catch (err) {
          console.error("Failed to reload playlists:", err);
        }
      }, 1000);
    }
  }, [
    plex,
    plexUrl,
    plexToken,
    plexPlaylistPageSize,
    clearCache,
    loadPlaylists,
  ]);

  const handleExportConfig = useCallback(async () => {
    setIsExportingConfig(true);
    setExportError("");
    setExportSuccessPath("");

    try {
      const timestamp = new Date().toISOString().slice(0, 10);
      const selectedPath = await save({
        title: "Export Config",
        defaultPath: `any-player-config-${timestamp}.json`,
        filters: [
          {
            name: "JSON",
            extensions: ["json"],
          },
        ],
      });

      if (!selectedPath) {
        return;
      }

      const outputPath = await tauriAPI.exportAppConfigToPath(selectedPath);
      setExportSuccessPath(outputPath);
    } catch (err) {
      console.error("Failed to export app config:", err);
      setExportError("Failed to export config");
    } finally {
      setIsExportingConfig(false);
    }
  }, []);

  const handleClearProviderCache = useCallback(async () => {
    setIsClearingProviderCache(true);

    try {
      const clearedProviderCacheFiles =
        await tauriAPI.clearProviderPlaylistsCache();
      clearCache();
      alert(
        `Cleared provider cache (${clearedProviderCacheFiles} playlist detail cache files removed). Next playlist load will fetch fresh provider data.`,
      );
    } catch (err) {
      console.error("Failed to clear provider cache:", err);
      alert("Failed to clear provider cache");
    } finally {
      setIsClearingProviderCache(false);
    }
  }, [clearCache]);

  const handleAudioNormalizationChange = useCallback(
    async (enabled: boolean, strictMode: boolean) => {
      setAudioNormalizationEnabled(enabled);
      setAudioNormalizationStrictMode(strictMode);

      try {
        await tauriAPI.setAudioNormalizationSettings(enabled, strictMode);
      } catch (err) {
        console.error("Failed to save audio normalization settings:", err);
      }
    },
    [],
  );

  const updateSyncSettings = useCallback((next: SyncSettings) => {
    setSyncSettings(next);
    saveSyncSettings(next);
  }, []);

  const handleTabKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLButtonElement>) => {
      const currentIndex = SETTINGS_TABS.indexOf(activeTab);
      if (e.key === "ArrowRight") {
        e.preventDefault();
        setActiveTab(SETTINGS_TABS[(currentIndex + 1) % SETTINGS_TABS.length]);
      } else if (e.key === "ArrowLeft") {
        e.preventDefault();
        setActiveTab(
          SETTINGS_TABS[
            (currentIndex - 1 + SETTINGS_TABS.length) % SETTINGS_TABS.length
          ],
        );
      }
    },
    [activeTab],
  );

  const handlePullSyncSnapshot = useCallback(async () => {
    setSyncLoading(true);
    setSyncStatus("");

    try {
      const result = await pullSyncSnapshot(syncSettings);
      if (result.usedFallback) {
        setSyncStatus(
          "Sync server unavailable; continuing with local app state.",
        );
      } else if (result.appliedDomains.length === 0) {
        setSyncStatus("Sync completed with no applicable remote domains.");
      } else {
        const suffix = result.skippedPlaylistsByConfirmation
          ? " (playlist overwrite skipped by confirmation)"
          : "";
        setSyncStatus(
          `Applied sync domains: ${result.appliedDomains.join(", ")}${suffix}.`,
        );
        await loadPlaylists("all", true);
      }
    } catch (err) {
      console.error("Failed to pull sync snapshot:", err);
      setSyncStatus("Failed to sync from server.");
    } finally {
      setSyncLoading(false);
    }
  }, [syncSettings, loadPlaylists]);

  const spotifyTabTooltip = useMemo(() => {
    if (!spotify.isConnected) {
      return "Spotify not connected";
    }

    const tier =
      spotify.isPremium === true
        ? "Premium"
        : spotify.isPremium === false
          ? "Free"
          : "Unknown tier";
    const playback = spotify.sessionReady
      ? "Playback ready"
      : "Playback session not initialized";

    return `Spotify connected • ${tier} • ${playback}`;
  }, [spotify.isConnected, spotify.isPremium, spotify.sessionReady]);

  const jellyfinTabTooltip = jellyfin.isConnected
    ? `Jellyfin connected${jellyfinUrl ? ` • ${jellyfinUrl}` : ""}`
    : "Jellyfin not connected";

  const plexTabTooltip = plex.isConnected
    ? `Plex connected${plexUrl ? ` • ${plexUrl}` : ""}`
    : "Plex not connected";

  return (
    <section id="settings" className="page">
      <div className="settings-container">
        <h2>Settings</h2>
        <div
          className="settings-tabs"
          role="tablist"
          aria-label="Settings Tabs"
        >
          <button
            type="button"
            role="tab"
            id="settings-tab-general"
            aria-controls="settings-panel-general"
            aria-selected={activeTab === "general"}
            tabIndex={activeTab === "general" ? 0 : -1}
            className={`settings-tab-btn ${activeTab === "general" ? "active" : ""}`}
            onClick={() => setActiveTab("general")}
            onKeyDown={handleTabKeyDown}
          >
            General
          </button>
          <button
            type="button"
            role="tab"
            id="settings-tab-spotify"
            aria-controls="settings-panel-spotify"
            aria-selected={activeTab === "spotify"}
            tabIndex={activeTab === "spotify" ? 0 : -1}
            className={`settings-tab-btn ${activeTab === "spotify" ? "active" : ""}`}
            onClick={() => setActiveTab("spotify")}
            onKeyDown={handleTabKeyDown}
          >
            Spotify
            {spotify.isConnected && (
              <span className="settings-tab-check" title={spotifyTabTooltip}>
                ✓
              </span>
            )}
          </button>
          <button
            type="button"
            role="tab"
            id="settings-tab-jellyfin"
            aria-controls="settings-panel-jellyfin"
            aria-selected={activeTab === "jellyfin"}
            tabIndex={activeTab === "jellyfin" ? 0 : -1}
            className={`settings-tab-btn ${activeTab === "jellyfin" ? "active" : ""}`}
            onClick={() => setActiveTab("jellyfin")}
            onKeyDown={handleTabKeyDown}
          >
            Jellyfin
            {jellyfin.isConnected && (
              <span className="settings-tab-check" title={jellyfinTabTooltip}>
                ✓
              </span>
            )}
          </button>
          <button
            type="button"
            role="tab"
            id="settings-tab-plex"
            aria-controls="settings-panel-plex"
            aria-selected={activeTab === "plex"}
            tabIndex={activeTab === "plex" ? 0 : -1}
            className={`settings-tab-btn ${activeTab === "plex" ? "active" : ""}`}
            onClick={() => setActiveTab("plex")}
            onKeyDown={handleTabKeyDown}
          >
            Plex
            {plex.isConnected && (
              <span className="settings-tab-check" title={plexTabTooltip}>
                ✓
              </span>
            )}
          </button>
        </div>

        {activeTab === "general" && (
          <div
            role="tabpanel"
            id="settings-panel-general"
            aria-labelledby="settings-tab-general"
          >
            <div className="settings-section">
              <h3>Sync Server</h3>
              <p className="section-description">
                Pull state from your Any Player sync server. Local state is used
                automatically if the server is unavailable.
              </p>
              <input
                type="text"
                className="setting-input"
                placeholder="http://localhost:8080"
                value={syncSettings.serverTarget}
                onChange={(e) =>
                  updateSyncSettings({
                    ...syncSettings,
                    serverTarget: e.target.value,
                  })
                }
              />
              <input
                type="password"
                className="setting-input"
                placeholder="Sync bearer token"
                aria-label="Sync bearer token"
                value={syncSettings.authToken}
                onChange={(e) =>
                  updateSyncSettings({
                    ...syncSettings,
                    authToken: e.target.value,
                  })
                }
              />
              <div className="setting-item">
                <label>
                  <input
                    type="checkbox"
                    checked={syncSettings.syncAppState}
                    onChange={(e) =>
                      updateSyncSettings({
                        ...syncSettings,
                        syncAppState: e.target.checked,
                      })
                    }
                  />
                  Sync app_state
                </label>
              </div>
              <div className="setting-item">
                <label>
                  <input
                    type="checkbox"
                    checked={syncSettings.syncPlaylists}
                    onChange={(e) =>
                      updateSyncSettings({
                        ...syncSettings,
                        syncPlaylists: e.target.checked,
                      })
                    }
                  />
                  Sync playlists (confirms before destructive overwrite)
                </label>
              </div>
              <div className="setting-item">
                <label>
                  <input
                    type="checkbox"
                    checked={syncSettings.syncProviderConfiguration}
                    onChange={(e) =>
                      updateSyncSettings({
                        ...syncSettings,
                        syncProviderConfiguration: e.target.checked,
                      })
                    }
                  />
                  Sync provider_configuration
                </label>
              </div>
              <div className="setting-item">
                <label>
                  <input
                    type="checkbox"
                    checked={syncSettings.syncSettings}
                    onChange={(e) =>
                      updateSyncSettings({
                        ...syncSettings,
                        syncSettings: e.target.checked,
                      })
                    }
                  />
                  Sync settings
                </label>
              </div>
              <button
                className="btn-primary"
                onClick={handlePullSyncSnapshot}
                disabled={syncLoading}
              >
                {syncLoading ? "Syncing..." : "Pull Sync Snapshot"}
              </button>
              {syncStatus && (
                <p className="section-description" style={{ marginTop: "8px" }}>
                  {syncStatus}
                </p>
              )}
            </div>

            <div className="settings-section">
              <h3>Playback</h3>
              <div className="setting-item">
                <label>
                  <input
                    type="checkbox"
                    id="autoplay-checkbox"
                    checked={autoplay}
                    onChange={(e) => setAutoplay(e.target.checked)}
                  />
                  Enable Autoplay
                </label>
              </div>
              <div className="setting-item">
                <label>
                  <input
                    type="checkbox"
                    id="audio-normalization-checkbox"
                    checked={audioNormalizationEnabled}
                    onChange={(e) =>
                      void handleAudioNormalizationChange(
                        e.target.checked,
                        audioNormalizationStrictMode,
                      )
                    }
                  />
                  Normalize Audio Across Providers
                </label>
              </div>
              <div className="setting-item">
                <label>
                  <input
                    type="checkbox"
                    id="audio-normalization-strict-checkbox"
                    checked={audioNormalizationStrictMode}
                    onChange={(e) =>
                      void handleAudioNormalizationChange(
                        audioNormalizationEnabled,
                        e.target.checked,
                      )
                    }
                  />
                  Strict Normalization (Adaptive Cross-Track + Spotify Offset)
                </label>
              </div>
            </div>

            <div className="settings-section">
              <h3>Configuration</h3>
              <p className="section-description">
                Export custom playlists and non-secret provider settings
              </p>
              <button
                className="btn-primary"
                onClick={handleExportConfig}
                disabled={isExportingConfig}
              >
                {isExportingConfig ? "Exporting..." : "Export Config"}
              </button>
              {exportSuccessPath && (
                <p
                  style={{
                    color: "green",
                    marginTop: "8px",
                    wordBreak: "break-all",
                  }}
                >
                  Saved to: {exportSuccessPath}
                </p>
              )}
              {exportError && (
                <p style={{ color: "red", marginTop: "8px" }}>{exportError}</p>
              )}

              <div style={{ marginTop: "16px" }}>
                <button
                  className="btn-primary"
                  onClick={handleClearProviderCache}
                  disabled={isClearingProviderCache}
                >
                  {isClearingProviderCache
                    ? "Clearing Provider Cache..."
                    : "Clear Provider Cache"}
                </button>
                <p className="section-description" style={{ marginTop: "8px" }}>
                  Removes cached provider playlist metadata and track lists so
                  the next load performs a clean resync.
                </p>
              </div>
            </div>

            <ColumnPreferencesSection />
          </div>
        )}

        {activeTab === "spotify" && (
          <div
            className="settings-section"
            role="tabpanel"
            id="settings-panel-spotify"
            aria-labelledby="settings-tab-spotify"
          >
            <h3>Spotify</h3>
            <button
              id="spotify-connect-btn"
              className="btn-primary"
              onClick={handleSpotifyConnect}
              disabled={spotify.isLoading && !spotify.isConnected}
            >
              {spotify.isLoading && !spotify.isConnected && (
                <LoadingSpinner size="small" />
              )}
              {spotify.isConnected
                ? "Disconnect Spotify"
                : spotify.isLoading
                  ? "Connecting to Spotify..."
                  : "Connect Spotify"}
            </button>
            <p className="status-text">
              {spotify.isConnected ? "✓ Connected" : "✗ Not connected"}
            </p>
          </div>
        )}

        {activeTab === "jellyfin" && (
          <div
            className="settings-section"
            role="tabpanel"
            id="settings-panel-jellyfin"
            aria-labelledby="settings-tab-jellyfin"
          >
            <h3>Jellyfin</h3>
            <input
              type="text"
              id="jellyfin-url"
              placeholder="Server URL"
              className="setting-input"
              value={jellyfinUrl}
              onChange={(e) => setJellyfinUrl(e.target.value)}
              disabled={jellyfin.isConnected}
            />
            <div
              style={{
                position: "relative",
                display: "flex",
                alignItems: "center",
              }}
            >
              <input
                type={showApiKey ? "text" : "password"}
                id="jellyfin-api-key"
                placeholder="API Key"
                className="setting-input"
                style={{ paddingRight: "40px" }}
                value={jellyfinApiKey}
                onChange={(e) => setJellyfinApiKey(e.target.value)}
                disabled={jellyfin.isConnected}
              />
              {jellyfinApiKey && (
                <button
                  type="button"
                  onClick={() => setShowApiKey(!showApiKey)}
                  style={{
                    position: "absolute",
                    right: "8px",
                    background: "none",
                    border: "none",
                    cursor: "pointer",
                    padding: "4px 8px",
                    fontSize: "16px",
                    color: "#666",
                  }}
                  aria-label={showApiKey ? "Hide API key" : "Show API key"}
                >
                  {showApiKey ? "👁️" : "👁️‍🗨️"}
                </button>
              )}
            </div>
            <input
              type="number"
              id="jellyfin-page-size"
              placeholder="Playlist Page Size (default: 300)"
              className="setting-input"
              value={jellyfinPlaylistPageSize}
              onChange={(e) => setJellyfinPlaylistPageSize(e.target.value)}
              min="1"
              max="1000"
            />
            <button
              id="jellyfin-connect-btn"
              className="btn-primary"
              onClick={handleJellyfinConnect}
              disabled={jellyfin.isLoading}
            >
              {jellyfin.isLoading && !jellyfin.isConnected && (
                <LoadingSpinner size="small" />
              )}
              {jellyfin.isLoading
                ? "Connecting to Jellyfin..."
                : jellyfin.isConnected
                  ? "Disconnect Jellyfin"
                  : "Connect Jellyfin"}
            </button>
            <p id="jellyfin-status" className="status-text">
              {jellyfin.isConnected ? "✓ Connected" : "✗ Not connected"}
            </p>
            {jellyfin.error && (
              <p style={{ color: "red", fontSize: "0.9em" }}>
                Error: {jellyfin.error}
              </p>
            )}
          </div>
        )}

        {activeTab === "plex" && (
          <div
            className="settings-section"
            role="tabpanel"
            id="settings-panel-plex"
            aria-labelledby="settings-tab-plex"
          >
            <h3>Plex</h3>
            <input
              type="text"
              id="plex-url"
              placeholder="Server URL"
              className="setting-input"
              value={plexUrl}
              onChange={(e) => setPlexUrl(e.target.value)}
              disabled={plex.isConnected}
            />
            <div
              style={{
                position: "relative",
                display: "flex",
                alignItems: "center",
              }}
            >
              <input
                type={showPlexToken ? "text" : "password"}
                id="plex-token"
                placeholder="Token"
                className="setting-input"
                style={{ paddingRight: "40px" }}
                value={plexToken}
                onChange={(e) => setPlexToken(e.target.value)}
                disabled={plex.isConnected}
              />
              {plexToken && (
                <button
                  type="button"
                  onClick={() => setShowPlexToken(!showPlexToken)}
                  style={{
                    position: "absolute",
                    right: "8px",
                    background: "none",
                    border: "none",
                    cursor: "pointer",
                    padding: "4px 8px",
                    fontSize: "16px",
                    color: "#666",
                  }}
                  aria-label={showPlexToken ? "Hide token" : "Show token"}
                >
                  {showPlexToken ? "👁️" : "👁️‍🗨️"}
                </button>
              )}
            </div>
            <input
              type="number"
              id="plex-page-size"
              placeholder="Playlist Page Size (default: 300)"
              className="setting-input"
              value={plexPlaylistPageSize}
              onChange={(e) => setPlexPlaylistPageSize(e.target.value)}
              min="1"
              max="1000"
            />
            <button
              id="plex-connect-btn"
              className="btn-primary"
              onClick={handlePlexConnect}
              disabled={plex.isLoading}
            >
              {plex.isLoading && !plex.isConnected && (
                <LoadingSpinner size="small" />
              )}
              {plex.isLoading
                ? "Connecting to Plex..."
                : plex.isConnected
                  ? "Disconnect Plex"
                  : "Connect Plex"}
            </button>
            <p id="plex-status" className="status-text">
              {plex.isConnected ? "✓ Connected" : "✗ Not connected"}
            </p>
            {plex.error && (
              <p style={{ color: "red", fontSize: "0.9em" }}>
                Error: {plex.error}
              </p>
            )}
          </div>
        )}
      </div>
      {spotify.authUrl && !spotify.isConnected && (
        <AuthModal authUrl={spotify.authUrl} onClose={spotify.clearAuthUrl} />
      )}
    </section>
  );
}

function ColumnPreferencesSection() {
  const [columns, setColumns] = useState<string[]>([
    "title",
    "artist",
    "album",
    "duration",
    "source",
  ]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const loadPreferences = async () => {
      try {
        const prefs = await tauriAPI.getColumnPreferences();
        setColumns(prefs.columns);
      } catch (err) {
        console.error("Failed to load column preferences:", err);
      } finally {
        setLoading(false);
      }
    };
    loadPreferences();
  }, []);

  const toggleColumn = async (column: string) => {
    const newColumns = columns.includes(column)
      ? columns.filter((c) => c !== column)
      : [...columns, column];

    setColumns(newColumns);

    try {
      // Get current preferences
      const currentPrefs = await tauriAPI.getColumnPreferences();

      // Update with new columns
      await tauriAPI.saveColumnPreferences({
        ...currentPrefs,
        columns: newColumns,
        column_order: newColumns.map((_, i) => i),
      });
    } catch (err) {
      console.error("Failed to save column preferences:", err);
    }
  };

  const allColumns = ["title", "artist", "album", "duration", "source"];

  if (loading) {
    return (
      <div className="settings-section">
        <h3>Track Table Columns</h3>
        <p className="loading-inline">
          <LoadingSpinner size="small" />
          <span>Loading preferences...</span>
        </p>
      </div>
    );
  }

  return (
    <div className="settings-section">
      <h3>Track Table Columns</h3>
      <p className="section-description">
        Choose which columns to display in custom playlist track tables
      </p>
      <div className="column-preferences">
        {allColumns.map((column) => (
          <div key={column} className="setting-item">
            <label>
              <input
                type="checkbox"
                checked={columns.includes(column)}
                onChange={() => toggleColumn(column)}
              />
              {column.charAt(0).toUpperCase() + column.slice(1)}
            </label>
          </div>
        ))}
      </div>
    </div>
  );
}
