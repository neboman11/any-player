use crate::commands::{AppState, PlaybackStatus, TrackInfo};
use crate::{PlaybackManager, PlaybackState, RepeatMode};
use futures::{SinkExt, StreamExt};
use serde::Serialize;
use std::sync::Arc;
use tauri::Manager;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, Mutex};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{accept_async, WebSocketStream};

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
    let authenticated = providers.is_spotify_authenticated().await;
    let premium = if authenticated {
        providers.is_spotify_premium().await
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
    let authenticated = providers.is_jellyfin_authenticated().await;
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
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|e| format!("WebSocket accept error: {}", e))?;
        let sender_clone = sender.clone();
        let handle_clone = app_handle.clone();

        tokio::spawn(async move {
            let ws_stream = match accept_async(stream).await {
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
                while let Ok(message) = rx.recv().await {
                    if ws_write.send(Message::Text(message)).await.is_err() {
                        break;
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

    let playback_message = serde_json::to_string(&WsMessage {
        event: "playback-status",
        data: playback_status,
    })
    .unwrap_or_else(|_| "".to_string());

    let spotify_message = serde_json::to_string(&WsMessage {
        event: "spotify-auth-status",
        data: spotify_status,
    })
    .unwrap_or_else(|_| "".to_string());

    let jellyfin_message = serde_json::to_string(&WsMessage {
        event: "jellyfin-auth-status",
        data: jellyfin_status,
    })
    .unwrap_or_else(|_| "".to_string());

    if !playback_message.is_empty() {
        ws_write.send(Message::Text(playback_message)).await?;
    }
    if !spotify_message.is_empty() {
        ws_write.send(Message::Text(spotify_message)).await?;
    }
    if !jellyfin_message.is_empty() {
        ws_write.send(Message::Text(jellyfin_message)).await?;
    }

    Ok(())
}

async fn build_spotify_status(state: &AppState) -> SpotifyAuthStatus {
    let providers = state.providers.lock().await;
    let authenticated = providers.is_spotify_authenticated().await;
    let premium = if authenticated {
        providers.is_spotify_premium().await
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
    let authenticated = providers.is_jellyfin_authenticated().await;
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
