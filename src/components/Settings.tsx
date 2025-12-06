import { useState, useCallback } from "react";
import { useSpotifyAuth, useJellyfinAuth } from "../hooks";

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
  const [autoplay, setAutoplay] = useState<boolean>(false);

  const spotify = useSpotifyAuth();
  const jellyfin = useJellyfinAuth();

  const handleSpotifyConnect = useCallback(async () => {
    if (spotify.isConnected) {
      await spotify.disconnect();
    } else {
      try {
        await spotify.connect();
      } catch (err) {
        console.error("Spotify connection error:", err);
      }
    }
  }, [spotify]);

  const handleJellyfinConnect = useCallback(async () => {
    if (jellyfin.isConnected) {
      await jellyfin.disconnect();
    } else {
      await jellyfin.connect(jellyfinUrl, jellyfinApiKey);
    }
  }, [jellyfin, jellyfinUrl, jellyfinApiKey]);

  return (
    <section id="settings" className="page">
      <div className="settings-container">
        <h2>Settings</h2>
        <div className="settings-section">
          <h3>Providers</h3>
          <div className="provider-settings">
            <div className="provider-item">
              <h4>Spotify</h4>
              <button
                id="spotify-connect-btn"
                className="btn-primary"
                onClick={handleSpotifyConnect}
                disabled={spotify.isLoading}
              >
                {spotify.isLoading
                  ? "Connecting..."
                  : spotify.isConnected
                  ? "Disconnect Spotify"
                  : "Connect Spotify"}
              </button>
              <p id="spotify-status" className="status-text">
                {spotify.isConnected ? "✓ Connected" : "✗ Not connected"}
              </p>
              {spotify.error && (
                <p style={{ color: "red", fontSize: "0.9em" }}>
                  Error: {spotify.error}
                </p>
              )}
            </div>
            <div className="provider-item">
              <h4>Jellyfin</h4>
              <input
                type="text"
                id="jellyfin-url"
                placeholder="Server URL"
                className="setting-input"
                value={jellyfinUrl}
                onChange={(e) => setJellyfinUrl(e.target.value)}
              />
              <input
                type="text"
                id="jellyfin-api-key"
                placeholder="API Key"
                className="setting-input"
                value={jellyfinApiKey}
                onChange={(e) => setJellyfinApiKey(e.target.value)}
              />
              <button
                id="jellyfin-connect-btn"
                className="btn-primary"
                onClick={handleJellyfinConnect}
                disabled={jellyfin.isLoading}
              >
                {jellyfin.isLoading ? "Connecting..." : "Connect Jellyfin"}
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
          </div>
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
        </div>
      </div>
      {spotify.authUrl && (
        <AuthModal authUrl={spotify.authUrl} onClose={spotify.clearAuthUrl} />
      )}
    </section>
  );
}
