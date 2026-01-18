/// Spotify librespot session management
use std::sync::Arc;
use tokio::sync::Mutex;

use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::config::SessionConfig;
use librespot_core::session::Session;
use librespot_oauth::{OAuthClientBuilder, OAuthToken};

const SPOTIFY_CLIENT_ID: &str = "243bb6667db04143b6586d8598aed48b";
const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8989/callback";
// based on https://developer.spotify.com/documentation/web-api/concepts/scopes#list-of-scopes
pub const OAUTH_SCOPES: &[&str] = &[
    // Spotify Connect
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-read-currently-playing",
    // Playback
    "app-remote-control",
    "streaming",
    // Playlists
    "playlist-read-private",
    "playlist-read-collaborative",
    "playlist-modify-private",
    "playlist-modify-public",
    // Follow
    "user-follow-modify",
    "user-follow-read",
    // Listening History
    "user-read-playback-position",
    "user-top-read",
    "user-read-recently-played",
    // Library
    "user-library-modify",
    "user-library-read",
    // Users
    "user-personalized",
];

/// Manages librespot session for Spotify track streaming
pub struct SpotifySessionManager {
    /// OAuth access token for authentication
    access_token: Arc<Mutex<Option<String>>>,
    /// Client ID for librespot session
    #[allow(dead_code)]
    client_id: String,
    /// Flag indicating session is ready for playback
    session_ready: Arc<Mutex<bool>>,
    /// Optionally hold a connected librespot Session
    session: Arc<Mutex<Option<Session>>>,
}

impl SpotifySessionManager {
    /// Create a new Spotify session manager
    pub fn new(client_id: String) -> Self {
        Self {
            access_token: Arc::new(Mutex::new(None)),
            client_id,
            session_ready: Arc::new(Mutex::new(false)),
            session: Arc::new(Mutex::new(None)),
        }
    }

    /// Check if session is initialized
    pub async fn is_initialized(&self) -> bool {
        *self.session_ready.lock().await
    }

    /// Launch the librespot OAuth flow and return the obtained `OAuthToken`.
    ///
    /// This opens the browser for user authorization and waits for the token.
    pub async fn get_creds(&self) -> Option<OAuthToken> {
        tracing::info!("SpotifySessionManager: Starting OAuth flow to get credentials");
        let client_builder = OAuthClientBuilder::new(
            SPOTIFY_CLIENT_ID,
            DEFAULT_REDIRECT_URI,
            OAUTH_SCOPES.to_vec(),
        )
        .open_in_browser();

        let oauth_client = match client_builder.build() {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to build OAuth client: {:?}", e);
                return None;
            }
        };

        match oauth_client.get_access_token_async().await {
            Ok(token) => Some(token),
            Err(e) => {
                tracing::error!("Failed to obtain OAuth token: {:?}", e);
                None
            }
        }
    }

    /// Initialize session with OAuth access token
    ///
    /// Creates a new librespot session using the provided OAuth access token.
    /// This allows playing full Spotify tracks for premium users.
    /// Uses Credentials::with_access_token() as per spotify-player implementation.
    pub async fn initialize_with_oauth_token(&self, access_token: &str) -> Result<(), String> {
        tracing::info!("SpotifySessionManager: Starting session initialization with OAuth token");

        // Store the access token
        {
            let mut token = self.access_token.lock().await;
            *token = Some(access_token.to_string());
            tracing::info!(
                "SpotifySessionManager: Access token stored (len={})",
                access_token.len()
            );
        }

        // Create librespot Session using OAuth access token
        // This matches spotify-player's approach in auth.rs
        let session_config = SessionConfig::default();

        // Use Credentials::with_access_token() instead of with_password()
        // This is the correct method for OAuth tokens per spotify-player
        let credentials = Credentials::with_access_token(access_token.to_string());

        let cache = Cache::new::<&std::path::Path>(None, None, None, None)
            .map_err(|e| format!("Failed to create librespot cache: {}", e))?;

        let session = Session::new(session_config, Some(cache));

        // Connect the session with OAuth credentials
        // The third parameter (true) enables token caching
        match Session::connect(&session, credentials, true).await {
            Ok(()) => {
                tracing::info!("SpotifySessionManager: librespot Session connected successfully");
                {
                    let mut s = self.session.lock().await;
                    *s = Some(session);
                }
                let mut ready = self.session_ready.lock().await;
                *ready = true;
                tracing::info!("SpotifySessionManager: Session is ready for playback");
                Ok(())
            }
            Err(e) => {
                tracing::error!("SpotifySessionManager: failed to connect session: {:?}", e);
                Err(format!("Failed to initialize librespot session: {:?}", e))
            }
        }
    }

    /// Initialize session using a `librespot_oauth::OAuthToken` instance.
    pub async fn initialize_with_oauth_token_obj(&self, token: &OAuthToken) -> Result<(), String> {
        self.initialize_with_oauth_token(&token.access_token).await
    }

    /// Get the current access token
    pub async fn get_access_token(&self) -> Option<String> {
        self.access_token.lock().await.clone()
    }

    /// Retrieve a clone of the connected librespot session, if available
    pub async fn get_session(&self) -> Option<Session> {
        self.session.lock().await.clone()
    }

    /// Close and cleanup the session
    pub async fn close_session(&self) -> Result<(), String> {
        {
            let mut token = self.access_token.lock().await;
            *token = None;
        }

        {
            let mut ready = self.session_ready.lock().await;
            *ready = false;
        }

        // Shutdown librespot session if present
        if let Some(s) = self.session.lock().await.take() {
            s.shutdown();
        }

        tracing::info!("Session closed");
        Ok(())
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
