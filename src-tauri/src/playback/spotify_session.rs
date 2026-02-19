/// Spotify librespot session management.
use std::sync::Arc;

use any_player_spotify_engine::{
    SpotifyEngineError, SpotifySessionBackend, SpotifySessionConfig, SpotifySessionEngine,
    SpotifyToken,
};
use async_trait::async_trait;
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::config::SessionConfig;
use librespot_core::session::Session;

#[derive(Clone, Default)]
struct DesktopLibrespotBackend;

#[async_trait]
impl SpotifySessionBackend for DesktopLibrespotBackend {
    type SessionHandle = Session;

    async fn connect(
        &self,
        config: &SpotifySessionConfig,
        access_token: &str,
    ) -> Result<Self::SessionHandle, SpotifyEngineError> {
        tracing::debug!(
            "Initializing librespot session using injected Spotify client_id (len={})",
            config.client_id.len()
        );

        let session_config = SessionConfig::default();
        let credentials = Credentials::with_access_token(access_token.to_string());
        let cache = Cache::new::<&std::path::Path>(None, None, None, None).map_err(|error| {
            SpotifyEngineError::new(
                "spotify_cache_create_failed",
                format!("Failed to create librespot cache: {}", error),
            )
        })?;
        let session = Session::new(session_config, Some(cache));

        Session::connect(&session, credentials, true)
            .await
            .map_err(|error| {
                SpotifyEngineError::new(
                    "spotify_session_connect_failed",
                    format!("Failed to initialize librespot session: {:?}", error),
                )
            })?;

        Ok(session)
    }

    async fn disconnect(&self, session: &Self::SessionHandle) -> Result<(), SpotifyEngineError> {
        session.shutdown();
        Ok(())
    }
}

/// Manages librespot session for Spotify track streaming.
pub struct SpotifySessionManager {
    engine: Arc<SpotifySessionEngine<DesktopLibrespotBackend>>,
}

impl SpotifySessionManager {
    /// Create a new Spotify session manager.
    pub fn new(client_id: String) -> Self {
        let config = SpotifySessionConfig::new(client_id);
        Self {
            engine: Arc::new(SpotifySessionEngine::new(config, DesktopLibrespotBackend)),
        }
    }

    /// Check if session is initialized.
    pub async fn is_initialized(&self) -> bool {
        self.engine.is_ready().await
    }

    /// Initialize session with OAuth access token.
    pub async fn initialize_with_oauth_token(&self, access_token: &str) -> Result<(), String> {
        tracing::info!("SpotifySessionManager: Starting session initialization with OAuth token");
        self.engine
            .initialize_with_token(SpotifyToken::with_access_token(access_token))
            .await
            .map_err(|error| error.to_string())
    }

    /// Get the current access token.
    pub async fn get_access_token(&self) -> Option<String> {
        self.engine.access_token().await
    }

    /// Retrieve a clone of the connected librespot session, if available.
    pub async fn get_session(&self) -> Option<Session> {
        self.engine.session_handle().await
    }

    /// Close and cleanup the session.
    pub async fn close_session(&self) -> Result<(), String> {
        self.engine.close().await.map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_manager_creation() {
        let manager = SpotifySessionManager::new("test_client_id".to_string());
        assert!(!manager.is_initialized().await);
    }

    #[tokio::test]
    async fn test_session_closure() {
        let manager = SpotifySessionManager::new("test_client_id".to_string());
        let result = manager.close_session().await;
        assert!(result.is_ok());
        assert!(!manager.is_initialized().await);
    }
}
