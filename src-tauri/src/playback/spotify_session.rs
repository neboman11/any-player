/// Spotify playback warm-up session management.
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

pub(crate) async fn connect_librespot_session(
    access_token: &str,
    reconnect_credentials: bool,
) -> Result<Session, SpotifyEngineError> {
    let session_config = SessionConfig::default();
    let credentials = Credentials::with_access_token(access_token.to_string());
    let cache = Cache::new::<&std::path::Path>(None, None, None, None).map_err(|error| {
        SpotifyEngineError::new(
            "spotify_cache_create_failed",
            format!("Failed to create librespot cache: {}", error),
        )
    })?;
    let session = Session::new(session_config, Some(cache));

    Session::connect(&session, credentials, reconnect_credentials)
        .await
        .map_err(|error| {
            SpotifyEngineError::new(
                "spotify_session_connect_failed",
                format!("Failed to initialize librespot session: {:?}", error),
            )
        })?;

    session.login5().inject_bearer_token(access_token, 0);

    Ok(session)
}

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

        connect_librespot_session(access_token, true).await
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

    /// Check if warm-up state is initialized.
    pub async fn is_initialized(&self) -> bool {
        self.engine.is_ready().await
    }

    /// Initialize warm-up state with OAuth access token.
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

    /// Retrieve a clone of the connected session handle, if available.
    pub async fn get_session(&self) -> Option<Session> {
        self.engine.session_handle().await
    }

    /// Close and cleanup warm-up session state.
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

    #[tokio::test]
    async fn injected_bearer_token_skips_login5_exchange() {
        let session = Session::new(SessionConfig::default(), None);

        session.login5().inject_bearer_token("test-access-token", 0);

        let token = session
            .login5()
            .auth_token()
            .await
            .expect("injected bearer token should be available without Login5");
        assert_eq!(token.access_token, "test-access-token");
    }
}
