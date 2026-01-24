/// Authentication commands for Spotify and Jellyfin
use crate::commands::AppState;
use tauri::State;

/// Initialize Spotify OAuth flow and get authorization URL (no credentials needed)
#[tauri::command]
pub async fn get_spotify_auth_url(state: State<'_, AppState>) -> Result<String, String> {
    let mut providers = state.providers.lock().await;

    let auth_url = providers
        .get_spotify_auth_url_default()
        .map_err(|e| format!("Failed to get auth URL: {}", e))?;

    Ok(auth_url)
}

/// Complete Spotify OAuth authentication with authorization code
#[tauri::command]
pub async fn authenticate_spotify(state: State<'_, AppState>, code: String) -> Result<(), String> {
    tracing::info!("Starting Spotify authentication with authorization code");

    let providers = state.providers.lock().await;
    providers
        .authenticate_spotify(&code)
        .await
        .map_err(|e| format!("Failed to authenticate: {}", e))?;
    drop(providers);

    tracing::info!("Spotify authentication successful");

    // Initialize session for premium users
    super::helpers::initialize_premium_session_if_needed(&state).await?;

    Ok(())
}

/// Check if Spotify is connected and authenticated
#[tauri::command]
pub async fn is_spotify_authenticated(state: State<'_, AppState>) -> Result<bool, String> {
    let providers = state.providers.lock().await;
    let authenticated = providers.is_spotify_authenticated().await;
    tracing::debug!("is_spotify_authenticated query result: {}", authenticated);
    Ok(authenticated)
}

/// Check if user has Spotify Premium
///
/// Returns true if authenticated user has Spotify Premium, false otherwise
#[tauri::command]
pub async fn check_spotify_premium(state: State<'_, AppState>) -> Result<bool, String> {
    let providers = state.providers.lock().await;
    providers
        .is_spotify_premium()
        .await
        .ok_or_else(|| "Spotify not authenticated".to_string())
}

/// Initialize Spotify session for premium track streaming
///
/// This should be called after successful Spotify authentication to enable
/// full track streaming for premium users via librespot.
#[tauri::command]
pub async fn initialize_spotify_session(
    state: State<'_, AppState>,
    access_token: String,
) -> Result<(), String> {
    let playback = state.playback.lock().await;
    playback.initialize_spotify_session(&access_token).await
}

/// Initialize Spotify session using the stored provider access token
/// This convenience command lets the frontend ask the backend to initialize
/// the librespot session using the provider-managed OAuth token, avoiding
/// the need for the frontend to pass the token value across IPC.
#[tauri::command]
pub async fn initialize_spotify_session_from_provider(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let providers = state.providers.lock().await;
    if let Some(access_token) = providers.get_spotify_access_token().await {
        drop(providers);
        let playback = state.playback.lock().await;
        playback
            .initialize_spotify_session(&access_token)
            .await
            .map_err(|e| format!("Failed to initialize session: {}", e))
    } else {
        Err("No Spotify access token available in provider registry".to_string())
    }
}

/// Check if Spotify session is initialized and ready for playback
#[tauri::command]
pub async fn is_spotify_session_ready(state: State<'_, AppState>) -> Result<bool, String> {
    let playback = state.playback.lock().await;
    Ok(playback.is_spotify_session_ready().await)
}

/// Refresh Spotify OAuth token and reinitialize session if needed
///
/// Called periodically or when token expires to maintain active authentication
/// and session state for premium playback features.
#[tauri::command]
pub async fn refresh_spotify_token(state: State<'_, AppState>) -> Result<(), String> {
    let mut providers = state.providers.lock().await;
    providers
        .refresh_spotify_token()
        .await
        .map_err(|e| format!("Failed to refresh Spotify token: {}", e))?;

    // If token was refreshed and user is premium, reinitialize session
    if let Some(true) = providers.is_spotify_premium().await {
        if let Some(access_token) = providers.get_spotify_access_token().await {
            drop(providers); // Release providers lock
            let playback = state.playback.lock().await;
            match playback.initialize_spotify_session(&access_token).await {
                Ok(()) => {
                    tracing::info!("Spotify session reinitialized after token refresh");
                }
                Err(e) => {
                    tracing::warn!("Failed to reinitialize session after token refresh: {}", e);
                }
            }
        }
    }

    Ok(())
}

/// Check for and process pending OAuth code
#[tauri::command]
pub async fn check_oauth_code(state: State<'_, AppState>) -> Result<bool, String> {
    let mut oauth_code = state.oauth_code.lock().await;

    if let Some(code) = oauth_code.take() {
        tracing::info!("OAuth code found in storage");
        drop(oauth_code);

        let providers = state.providers.lock().await;
        providers
            .authenticate_spotify(&code)
            .await
            .map_err(|e| format!("Failed to authenticate: {}", e))?;
        drop(providers);

        tracing::info!("Provider authentication succeeded");

        // Initialize session for premium users
        super::helpers::initialize_premium_session_if_needed(&state).await?;

        Ok(true)
    } else {
        Ok(false)
    }
}

/// Disconnect and revoke Spotify authentication
#[tauri::command]
pub async fn disconnect_spotify(state: State<'_, AppState>) -> Result<(), String> {
    let mut providers = state.providers.lock().await;

    providers
        .disconnect_spotify()
        .await
        .map_err(|e| format!("Failed to disconnect Spotify: {}", e))
}

/// Restore Spotify session from saved tokens
#[tauri::command]
pub async fn restore_spotify_session(state: State<'_, AppState>) -> Result<bool, String> {
    let mut providers = state.providers.lock().await;

    providers
        .restore_spotify_session()
        .await
        .map_err(|e| format!("Failed to restore Spotify session: {}", e))
}

/// Clear saved Spotify session tokens and in-memory Spotify session state
#[tauri::command]
pub async fn clear_spotify_session(state: State<'_, AppState>) -> Result<(), String> {
    use crate::config::Config;

    let mut providers = state.providers.lock().await;
    providers
        .disconnect_spotify()
        .await
        .map_err(|e| format!("Failed to disconnect Spotify during session clear: {}", e))?;
    drop(providers);

    Config::clear_tokens().map_err(|e| format!("Failed to clear tokens: {}", e))
}

/// Jellyfin authentication and connection
#[tauri::command]
pub async fn authenticate_jellyfin(
    state: State<'_, AppState>,
    url: String,
    api_key: String,
) -> Result<(), String> {
    use crate::config::Config;

    let mut providers = state.providers.lock().await;

    providers
        .authenticate_jellyfin(&url, &api_key)
        .await
        .map_err(|e| format!("Failed to authenticate Jellyfin: {}", e))?;

    // Save credentials to secure storage after successful authentication
    let mut tokens = Config::load_tokens().map_err(|e| format!("Failed to load tokens: {}", e))?;
    tokens.jellyfin_api_key = Some(api_key);
    tokens.jellyfin_url = Some(url);
    Config::save_tokens(&tokens)
        .map_err(|e| format!("Failed to save Jellyfin credentials: {}", e))?;

    tracing::info!("Jellyfin credentials saved to secure storage");

    Ok(())
}

/// Check if Jellyfin is connected and authenticated
#[tauri::command]
pub async fn is_jellyfin_authenticated(state: State<'_, AppState>) -> Result<bool, String> {
    let providers = state.providers.lock().await;
    Ok(providers.is_jellyfin_authenticated().await)
}

/// Disconnect and revoke Jellyfin authentication
#[tauri::command]
pub async fn disconnect_jellyfin(state: State<'_, AppState>) -> Result<(), String> {
    use crate::config::Config;

    let mut providers = state.providers.lock().await;

    providers
        .disconnect_jellyfin()
        .await
        .map_err(|e| format!("Failed to disconnect Jellyfin: {}", e))?;

    // Clear stored Jellyfin credentials from secure storage
    let mut tokens = Config::load_tokens().map_err(|e| format!("Failed to load tokens: {}", e))?;
    tokens.jellyfin_api_key = None;
    tokens.jellyfin_url = None;
    Config::save_tokens(&tokens)
        .map_err(|e| format!("Failed to clear Jellyfin credentials: {}", e))?;

    tracing::info!("Jellyfin credentials cleared from secure storage");

    Ok(())
}

/// Get stored Jellyfin credentials
#[tauri::command]
pub async fn get_jellyfin_credentials(
    _state: State<'_, AppState>,
) -> Result<Option<(String, String)>, String> {
    use crate::config::Config;

    let tokens = Config::load_tokens().map_err(|e| format!("Failed to load tokens: {}", e))?;

    match (tokens.jellyfin_url, tokens.jellyfin_api_key) {
        (Some(url), Some(api_key)) => Ok(Some((url, api_key))),
        _ => Ok(None),
    }
}

/// Restore Jellyfin session from saved credentials
#[tauri::command]
pub async fn restore_jellyfin_session(state: State<'_, AppState>) -> Result<bool, String> {
    let mut providers = state.providers.lock().await;

    providers
        .restore_jellyfin_session()
        .await
        .map_err(|e| format!("Failed to restore Jellyfin session: {}", e))
}
