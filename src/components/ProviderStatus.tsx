export interface ProviderStatusInfo {
  isConnected: boolean;
  isPremium?: boolean | null;
  sessionReady?: boolean;
  initializingSession?: boolean;
  error?: string | null;
  isLoading?: boolean;
}

export interface ProviderStatusProps {
  status: ProviderStatusInfo;
  onInitializeSession?: () => Promise<void>;
  providerColor?: string;
  providerName?: string;
}

/**
 * Generic provider status component that can be reused across different providers
 * Shows connection status, premium/subscription info, and session initialization
 */
export function ProviderStatus({
  status,
  onInitializeSession,
  providerColor = "#1DB954",
  providerName = "Provider",
}: ProviderStatusProps) {
  return (
    <div>
      <p className="status-text">
        {status.isConnected ? "✓ Connected" : "✗ Not connected"}
      </p>

      {status.isConnected && (
        <div style={{ marginTop: "10px", fontSize: "0.9em" }}>
          {/* Premium/Subscription Status */}
          {status.isPremium !== null && status.isPremium !== undefined && (
            <p
              style={{
                margin: "5px 0",
                color: status.isPremium ? providerColor : "#999",
              }}
            >
              {status.isPremium
                ? `✓ ${providerName} Premium`
                : `✗ ${providerName} Free Tier`}
            </p>
          )}

          {/* Session Status */}
          {status.isPremium && (
            <p
              style={{
                margin: "5px 0",
                color: status.sessionReady ? providerColor : "#ff9800",
              }}
            >
              {status.initializingSession
                ? "⏳ Initializing session..."
                : status.sessionReady
                ? "✓ Full playback ready"
                : "⚠ Initialize for full track playback"}
            </p>
          )}

          {/* Session Initialization Button */}
          {status.isPremium && !status.sessionReady && onInitializeSession && (
            <button
              onClick={onInitializeSession}
              disabled={status.initializingSession}
              style={{
                marginTop: "8px",
                padding: "8px 16px",
                background: providerColor,
                color: "white",
                border: "none",
                borderRadius: "4px",
                cursor: status.initializingSession ? "not-allowed" : "pointer",
                opacity: status.initializingSession ? 0.6 : 1,
              }}
            >
              {status.initializingSession
                ? "Initializing..."
                : "Initialize Session"}
            </button>
          )}
        </div>
      )}

      {/* Error Display */}
      {status.error && (
        <p style={{ color: "red", fontSize: "0.9em", marginTop: "8px" }}>
          Error: {status.error}
        </p>
      )}
    </div>
  );
}
