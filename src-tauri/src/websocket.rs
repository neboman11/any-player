use crate::commands::{AppState, PlaybackStatus, TrackInfo};
use crate::{PlaybackManager, PlaybackState, RepeatMode};
use futures::{SinkExt, StreamExt};
use serde::Serialize;
use std::sync::Arc;
use tauri::Manager;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, Mutex};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{accept_hdr_async, WebSocketStream};

const WS_ADDR: &str = "127.0.0.1:8990";

#[derive(Debug, Serialize)]
struct WsMessage<T: Serialize> {
    event: &'static str,
    data: T,
}

#[derive(Debug, Serialize, Clone)]
pub struct SpotifyAuthStatus {
    pub authenticated: bool,
    pub premium: Option<bool>,
    pub session_ready: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct JellyfinAuthStatus {
    pub authenticated: bool,
}

pub fn broadcast_event<T: Serialize>(
    sender: &broadcast::Sender<String>,
    event: &'static str,
    data: T,
) {
    if let Ok(text) = serde_json::to_string(&WsMessage { event, data }) {
        let _ = sender.send(text);
    }
}

pub async fn emit_spotify_status(state: &AppState) {
    let providers = state.providers.lock().await;
    let authenticated = providers.is_authenticated(crate::Source::Spotify).await;
    let premium = if authenticated {
        providers.premium_status(crate::Source::Spotify).await
    } else {
        None
    };
    drop(providers);

    let playback = state.playback.lock().await;
    let session_ready = playback.is_spotify_session_ready().await;
    drop(playback);

    broadcast_event(
        &state.ws_sender,
        "spotify-auth-status",
        SpotifyAuthStatus {
            authenticated,
            premium,
            session_ready,
        },
    );
}

pub async fn emit_jellyfin_status(state: &AppState) {
    let providers = state.providers.lock().await;
    let authenticated = providers.is_authenticated(crate::Source::Jellyfin).await;
    drop(providers);

    broadcast_event(
        &state.ws_sender,
        "jellyfin-auth-status",
        JellyfinAuthStatus { authenticated },
    );
}

pub async fn start_ws_server(
    app_handle: tauri::AppHandle,
    sender: broadcast::Sender<String>,
) -> Result<(), String> {
    let listener = TcpListener::bind(WS_ADDR)
        .await
        .map_err(|e| format!("Failed to bind websocket server: {}", e))?;

    tracing::info!("WebSocket server listening on {}", WS_ADDR);

    loop {
        let (stream, peer_addr) = listener
            .accept()
            .await
            .map_err(|e| format!("WebSocket accept error: {}", e))?;
        let sender_clone = sender.clone();
        let handle_clone = app_handle.clone();

        tokio::spawn(async move {
            let ws_stream = match accept_hdr_async(stream, |req: &Request, response: Response| {
                // Validate that the connection is from localhost
                // Check Origin header if present
                if let Some(origin) = req.headers().get("Origin") {
                    if let Ok(origin_str) = origin.to_str() {
                        // Parse origin URL and validate hostname
                        let is_allowed = if let Ok(url) = url::Url::parse(origin_str) {
                            if let Some(host) = url.host_str() {
                                // Allow connections from localhost, 127.0.0.1, or ::1 (IPv6)
                                host == "localhost" || host == "127.0.0.1" || host == "::1"
                            } else {
                                false
                            }
                        } else if origin_str == "tauri://localhost" {
                            // Allow tauri://localhost specifically for Tauri applications
                            true
                        } else {
                            false
                        };

                        if !is_allowed {
                            tracing::warn!(
                                "Rejected WebSocket connection from unauthorized origin: {}",
                                origin_str
                            );
                            return Err(Response::builder()
                                .status(403)
                                .body(Some(
                                    "Forbidden: WebSocket connections only allowed from localhost"
                                        .into(),
                                ))
                                .unwrap());
                        }
                    }
                }
                Ok(response)
            })
            .await
            {
                Ok(ws) => ws,
                Err(err) => {
                    tracing::warn!(?err, "Failed to accept websocket connection");
                    return;
                }
            };

            let (mut ws_write, mut ws_read) = ws_stream.split();

            if let Err(err) = send_initial_state(&handle_clone, &mut ws_write).await {
                tracing::warn!(?err, "Failed to send initial websocket state");
                return;
            }

            let mut rx = sender_clone.subscribe();
            let write_task = tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(message) => {
                            if ws_write.send(Message::Text(message.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            // The receiver has fallen behind; skip dropped messages and continue.
                            tracing::warn!(peer = %peer_addr, "WebSocket client lagged, skipped {} messages", skipped);
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            // The sender has been closed; exit the loop.
                            break;
                        }
                    }
                }
            });

            while let Some(message) = ws_read.next().await {
                if message.is_err() {
                    break;
                }
            }

            write_task.abort();
        });
    }
}

pub async fn start_playback_broadcast(
    playback: Arc<Mutex<PlaybackManager>>,
    sender: broadcast::Sender<String>,
) {
    let mut last_payload = String::new();

    loop {
        // Only poll and broadcast if there are active subscribers
        if sender.receiver_count() > 0 {
            let status = build_playback_status(&playback).await;
            if let Ok(text) = serde_json::to_string(&WsMessage {
                event: "playback-status",
                data: status,
            }) {
                if text != last_payload {
                    let _ = sender.send(text.clone());
                    last_payload = text;
                }
            }
        }

        sleep(Duration::from_millis(500)).await;
    }
}

async fn send_initial_state(
    app_handle: &tauri::AppHandle,
    ws_write: &mut futures::stream::SplitSink<
        WebSocketStream<tokio::net::TcpStream>,
        tokio_tungstenite::tungstenite::Message,
    >,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    let state = app_handle.state::<AppState>();

    let playback_status = build_playback_status(&state.playback).await;
    let spotify_status = build_spotify_status(&state).await;
    let jellyfin_status = build_jellyfin_status(&state).await;

    let playback_message = match serde_json::to_string(&WsMessage {
        event: "playback-status",
        data: playback_status,
    }) {
        Ok(msg) => Some(msg),
        Err(e) => {
            tracing::error!(
                "Failed to serialize playback-status WebSocket message: {}",
                e
            );
            None
        }
    };

    let spotify_message = match serde_json::to_string(&WsMessage {
        event: "spotify-auth-status",
        data: spotify_status,
    }) {
        Ok(msg) => Some(msg),
        Err(e) => {
            tracing::error!(
                "Failed to serialize spotify-auth-status WebSocket message: {}",
                e
            );
            None
        }
    };

    let jellyfin_message = match serde_json::to_string(&WsMessage {
        event: "jellyfin-auth-status",
        data: jellyfin_status,
    }) {
        Ok(msg) => Some(msg),
        Err(e) => {
            tracing::error!(
                "Failed to serialize jellyfin-auth-status WebSocket message: {}",
                e
            );
            None
        }
    };

    if let Some(playback_message) = playback_message {
        ws_write
            .send(Message::Text(playback_message.into()))
            .await?;
    }
    if let Some(spotify_message) = spotify_message {
        ws_write.send(Message::Text(spotify_message.into())).await?;
    }
    if let Some(jellyfin_message) = jellyfin_message {
        ws_write
            .send(Message::Text(jellyfin_message.into()))
            .await?;
    }

    Ok(())
}

async fn build_spotify_status(state: &AppState) -> SpotifyAuthStatus {
    let providers = state.providers.lock().await;
    let authenticated = providers.is_authenticated(crate::Source::Spotify).await;
    let premium = if authenticated {
        providers.premium_status(crate::Source::Spotify).await
    } else {
        None
    };
    drop(providers);

    let playback = state.playback.lock().await;
    let session_ready = playback.is_spotify_session_ready().await;
    drop(playback);

    SpotifyAuthStatus {
        authenticated,
        premium,
        session_ready,
    }
}

async fn build_jellyfin_status(state: &AppState) -> JellyfinAuthStatus {
    let providers = state.providers.lock().await;
    let authenticated = providers.is_authenticated(crate::Source::Jellyfin).await;
    drop(providers);

    JellyfinAuthStatus { authenticated }
}

async fn build_playback_status(playback: &Arc<Mutex<PlaybackManager>>) -> PlaybackStatus {
    let info = {
        let playback = playback.lock().await;
        playback.get_info().await
    };

    let state = match info.state {
        PlaybackState::Playing => "playing".to_string(),
        PlaybackState::Paused => "paused".to_string(),
        PlaybackState::Stopped => "stopped".to_string(),
    };

    let repeat_mode = match info.repeat_mode {
        RepeatMode::Off => "off".to_string(),
        RepeatMode::One => "one".to_string(),
        RepeatMode::All => "all".to_string(),
    };

    let duration = info
        .current_track
        .as_ref()
        .map(|t| t.duration_ms)
        .unwrap_or(0);
    let current_track = info.current_track.map(|t| TrackInfo {
        id: t.id,
        title: t.title,
        artist: t.artist,
        album: t.album,
        duration: t.duration_ms,
        source: t.source.to_string(),
        url: t.url,
        image_url: t.image_url,
    });

    let queue = if info.shuffle && !info.shuffle_order.is_empty() {
        info.shuffle_order
            .iter()
            .skip(info.current_index + 1)
            .filter_map(|&idx| info.queue.get(idx))
            .map(|t| TrackInfo {
                id: t.id.clone(),
                title: t.title.clone(),
                artist: t.artist.clone(),
                album: t.album.clone(),
                duration: t.duration_ms,
                source: t.source.to_string(),
                url: t.url.clone(),
                image_url: t.image_url.clone(),
            })
            .collect()
    } else {
        info.queue
            .into_iter()
            .skip(info.current_index + 1)
            .map(|t| TrackInfo {
                id: t.id,
                title: t.title,
                artist: t.artist,
                album: t.album,
                duration: t.duration_ms,
                source: t.source.to_string(),
                url: t.url,
                image_url: t.image_url,
            })
            .collect()
    };

    PlaybackStatus {
        state,
        current_track,
        position: info.position_ms,
        volume: info.volume,
        shuffle: info.shuffle,
        repeat_mode,
        duration,
        queue,
    }
}
