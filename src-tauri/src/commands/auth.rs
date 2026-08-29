/// Authentication commands for Spotify and Jellyfin
use crate::commands::AppState;
use crate::{providers::ProviderAuthRequest, Source};
use tauri::State;

/// Initialize Spotify OAuth flow and get authorization URL (no credentials needed)
#[tauri::command]
pub async fn get_spotify_auth_url(state: State<'_, AppState>) -> Result<String, String> {
    let mut providers = state.providers.lock().await;

    let auth_response = providers
        .begin_auth(Source::Spotify, ProviderAuthRequest::default())
        .await
        .map_err(|e| format!("Failed to get auth URL: {}", e))?;

    let auth_url = auth_response
        .auth_url()
        .ok_or_else(|| "Provider did not return an auth URL".to_string())?
        .to_string();

    Ok(auth_url)
}

/// Complete Spotify OAuth authentication with authorization code
#[tauri::command]
pub async fn authenticate_spotify(state: State<'_, AppState>, code: String) -> Result<(), String> {
    tracing::info!("Starting Spotify authentication with authorization code");

    let mut providers = state.providers.lock().await;
    providers
        .complete_auth(
            Source::Spotify,
            ProviderAuthRequest::from_pairs([("code", code)]),
        )
        .await
        .map_err(|e| format!("Failed to authenticate: {}", e))?;
    drop(providers);

    tracing::info!("Spotify authentication successful");

    // Warm Spotify playback state for premium users when possible
    super::helpers::initialize_premium_session_if_needed(&state).await?;

    crate::websocket::emit_spotify_status(&state).await;

    Ok(())
}

/// Check if Spotify is connected and authenticated
#[tauri::command]
pub async fn is_spotify_authenticated(state: State<'_, AppState>) -> Result<bool, String> {
    let providers = state.providers.lock().await;
    let authenticated = providers.is_authenticated(Source::Spotify).await;
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
        .premium_status(Source::Spotify)
        .await
        .ok_or_else(|| "Spotify not authenticated".to_string())
}

/// Initialize Spotify warm-up state using the stored provider access token.
/// This convenience command lets the frontend ask the backend to initialize
/// playback warm-up state using the provider-managed OAuth token, avoiding
/// the need for the frontend to pass the token value across IPC.
#[tauri::command]
pub async fn initialize_spotify_session_from_provider(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let providers = state.providers.lock().await;
    if let Some(access_token) = providers.get_access_token(Source::Spotify).await {
        drop(providers);
        let playback = state.playback.lock().await;
        playback
            .initialize_spotify_session(&access_token)
            .await
            .map_err(|e| format!("Failed to initialize Spotify warm-up state: {}", e))?;
        drop(playback);

        crate::websocket::emit_spotify_status(&state).await;

        Ok(())
    } else {
        Err("No Spotify access token available in provider registry".to_string())
    }
}

/// Check if Spotify warm-up state is initialized and ready.
#[tauri::command]
pub async fn is_spotify_session_ready(state: State<'_, AppState>) -> Result<bool, String> {
    let playback = state.playback.lock().await;
    Ok(playback.is_spotify_session_ready().await)
}

/// Refresh Spotify OAuth token and reinitialize warm-up state if needed.
///
/// Called periodically or when token expires to maintain active authentication
/// and playback readiness metadata.
#[tauri::command]
pub async fn refresh_spotify_token(state: State<'_, AppState>) -> Result<(), String> {
    let mut providers = state.providers.lock().await;
    providers
        .refresh_auth(Source::Spotify)
        .await
        .map_err(|e| format!("Failed to refresh Spotify token: {}", e))?;

    // If token was refreshed and user is premium, refresh warm-up state
    if let Some(true) = providers.premium_status(Source::Spotify).await {
        if let Some(access_token) = providers.get_access_token(Source::Spotify).await {
            drop(providers); // Release providers lock
            let playback = state.playback.lock().await;
            match playback.initialize_spotify_session(&access_token).await {
                Ok(()) => {
                    tracing::info!("Spotify warm-up state reinitialized after token refresh");
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to reinitialize Spotify warm-up state after token refresh: {}",
                        e
                    );
                }
            }
            drop(playback);
        }
    } else {
        drop(providers);
    }

    crate::websocket::emit_spotify_status(&state).await;

    Ok(())
}

/// Check for and process pending OAuth code
#[tauri::command]
pub async fn check_oauth_code(state: State<'_, AppState>) -> Result<bool, String> {
    let mut oauth_code = state.oauth_code.lock().await;

    if let Some(code) = oauth_code.take() {
        tracing::info!("OAuth code found in storage");
        drop(oauth_code);

        let mut providers = state.providers.lock().await;
        providers
            .complete_auth(
                Source::Spotify,
                ProviderAuthRequest::from_pairs([("code", code)]),
            )
            .await
            .map_err(|e| format!("Failed to authenticate: {}", e))?;
        drop(providers);

        tracing::info!("Provider authentication succeeded");

        // Warm Spotify playback state for premium users when possible
        super::helpers::initialize_premium_session_if_needed(&state).await?;

        crate::websocket::emit_spotify_status(&state).await;

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
        .disconnect(Source::Spotify)
        .await
        .map_err(|e| format!("Failed to disconnect Spotify: {}", e))?;
    drop(providers);

    state.playback.lock().await.close_spotify_session().await?;

    crate::websocket::emit_spotify_status(&state).await;

    Ok(())
}

/// Jellyfin authentication and connection
#[tauri::command]
pub async fn authenticate_jellyfin(
    state: State<'_, AppState>,
    url: String,
    api_key: String,
    page_size: Option<u32>,
) -> Result<(), String> {
    let mut providers = state.providers.lock().await;

    let mut auth_request =
        ProviderAuthRequest::from_pairs([("url", url.clone()), ("api_key", api_key.clone())]);

    if let Some(size) = page_size {
        auth_request.insert("page_size".to_string(), size.to_string());
    }

    providers
        .complete_auth(Source::Jellyfin, auth_request)
        .await
        .map_err(|e| format!("Failed to authenticate Jellyfin: {}", e))?;
    drop(providers);

    // Credentials are now automatically saved within complete_auth for consistency with Spotify

    crate::websocket::emit_jellyfin_status(&state).await;

    Ok(())
}

/// Check if Jellyfin is connected and authenticated
#[tauri::command]
pub async fn is_jellyfin_authenticated(state: State<'_, AppState>) -> Result<bool, String> {
    let providers = state.providers.lock().await;
    Ok(providers.is_authenticated(Source::Jellyfin).await)
}

/// Disconnect and revoke Jellyfin authentication
#[tauri::command]
pub async fn disconnect_jellyfin(state: State<'_, AppState>) -> Result<(), String> {
    let mut providers = state.providers.lock().await;

    providers
        .disconnect(Source::Jellyfin)
        .await
        .map_err(|e| format!("Failed to disconnect Jellyfin: {}", e))?;
    drop(providers);

    crate::websocket::emit_jellyfin_status(&state).await;

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

    let restored = providers
        .restore_session(Source::Jellyfin)
        .await
        .map_err(|e| format!("Failed to restore Jellyfin session: {}", e))?;
    drop(providers);

    crate::websocket::emit_jellyfin_status(&state).await;

    Ok(restored)
}

/// Plex authentication and connection
#[tauri::command]
pub async fn authenticate_plex(
    state: State<'_, AppState>,
    url: String,
    token: String,
    page_size: Option<u32>,
) -> Result<(), String> {
    let mut providers = state.providers.lock().await;

    let mut auth_request =
        ProviderAuthRequest::from_pairs([("url", url.clone()), ("token", token.clone())]);

    if let Some(size) = page_size {
        auth_request.insert("page_size".to_string(), size.to_string());
    }

    providers
        .complete_auth(Source::Plex, auth_request)
        .await
        .map_err(|e| format!("Failed to authenticate Plex: {}", e))?;
    drop(providers);

    crate::websocket::emit_plex_status(&state).await;

    Ok(())
}

/// Check if Plex is connected and authenticated
#[tauri::command]
pub async fn is_plex_authenticated(state: State<'_, AppState>) -> Result<bool, String> {
    let providers = state.providers.lock().await;
    Ok(providers.is_authenticated(Source::Plex).await)
}

/// Disconnect and revoke Plex authentication
#[tauri::command]
pub async fn disconnect_plex(state: State<'_, AppState>) -> Result<(), String> {
    let mut providers = state.providers.lock().await;

    providers
        .disconnect(Source::Plex)
        .await
        .map_err(|e| format!("Failed to disconnect Plex: {}", e))?;
    drop(providers);

    crate::websocket::emit_plex_status(&state).await;

    Ok(())
}

/// Get stored Plex credentials
#[tauri::command]
pub async fn get_plex_credentials(
    _state: State<'_, AppState>,
) -> Result<Option<(String, String)>, String> {
    use crate::config::Config;

    let tokens = Config::load_tokens().map_err(|e| format!("Failed to load tokens: {}", e))?;

    match (tokens.plex_url, tokens.plex_token) {
        (Some(url), Some(token)) => Ok(Some((url, token))),
        _ => Ok(None),
    }
}

/// Restore Plex session from saved credentials
#[tauri::command]
pub async fn restore_plex_session(state: State<'_, AppState>) -> Result<bool, String> {
    let mut providers = state.providers.lock().await;

    let restored = providers
        .restore_session(Source::Plex)
        .await
        .map_err(|e| format!("Failed to restore Plex session: {}", e))?;
    drop(providers);

    crate::websocket::emit_plex_status(&state).await;

    Ok(restored)
}
