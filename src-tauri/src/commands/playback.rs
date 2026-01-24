/// Playback control commands
use crate::commands::{AppState, PlaybackStatus, TrackInfo};
use crate::{PlaybackState, RepeatMode};
use tauri::State;

/// Get current playback status
#[tauri::command]
pub async fn get_playback_status(state: State<'_, AppState>) -> Result<PlaybackStatus, String> {
    let info = {
        let playback = state.playback.lock().await;
        playback.get_info().await
    };

    let state_str = match info.state {
        PlaybackState::Playing => "playing".to_string(),
        PlaybackState::Paused => "paused".to_string(),
        PlaybackState::Stopped => "stopped".to_string(),
    };

    let repeat_str = match info.repeat_mode {
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

    // Get queue tracks - only return tracks after the current index
    // If shuffle is enabled, use shuffle_order to determine the actual play order
    let queue_tracks: Vec<TrackInfo> = if info.shuffle && !info.shuffle_order.is_empty() {
        // Get remaining tracks in shuffle order
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
        // Get remaining tracks in normal order
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

    Ok(PlaybackStatus {
        state: state_str,
        current_track,
        position: info.position_ms,
        volume: info.volume,
        shuffle: info.shuffle,
        repeat_mode: repeat_str,
        duration,
        queue: queue_tracks,
    })
}

/// Play current track in queue
#[tauri::command]
pub async fn play(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.play().await;
    Ok(())
}

/// Pause playback
#[tauri::command]
pub async fn pause(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.pause().await;
    Ok(())
}

/// Toggle play/pause
#[tauri::command]
pub async fn toggle_play_pause(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.toggle_play_pause().await;
    Ok(())
}

/// Play next track
#[tauri::command]
pub async fn next_track(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    let _ = playback.next_track().await;

    // Trigger eager loading for upcoming tracks in the background
    let playback_arc = state.playback.clone();
    let providers_arc = state.providers.clone();
    tokio::spawn(async move {
        let pb = playback_arc.lock().await;
        let info = pb.get_info().await;
        let current_idx = info.current_index;
        drop(pb);

        super::helpers::enrich_queued_tracks_eager(playback_arc, providers_arc, current_idx).await;
    });

    Ok(())
}

/// Play previous track
#[tauri::command]
pub async fn previous_track(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    let _ = playback.previous_track().await;
    Ok(())
}

/// Seek to position in milliseconds
#[tauri::command]
pub async fn seek(state: State<'_, AppState>, position: u64) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.seek(position).await;
    Ok(())
}

/// Set volume (0-100)
#[tauri::command]
pub async fn set_volume(state: State<'_, AppState>, volume: u32) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.set_volume(volume).await;
    Ok(())
}

/// Toggle shuffle mode
#[tauri::command]
pub async fn toggle_shuffle(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.toggle_shuffle().await;
    Ok(())
}

/// Set repeat mode
#[tauri::command]
pub async fn set_repeat_mode(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    let repeat_mode = match mode.as_str() {
        "off" => RepeatMode::Off,
        "one" => RepeatMode::One,
        "all" => RepeatMode::All,
        _ => return Err("Invalid repeat mode".to_string()),
    };

    let playback = { state.playback.lock().await };
    playback.set_repeat_mode(repeat_mode).await;
    Ok(())
}

/// Clear the queue
#[tauri::command]
pub async fn clear_queue(state: State<'_, AppState>) -> Result<(), String> {
    let playback = { state.playback.lock().await };
    playback.clear_queue().await;
    Ok(())
}

/// Save current playback state to disk
#[tauri::command]
pub async fn save_playback_state(state: State<'_, AppState>) -> Result<(), String> {
    let playback = state.playback.lock().await;
    playback.save_state().await
}

/// Restore playback state from disk
#[tauri::command]
pub async fn restore_playback_state(state: State<'_, AppState>) -> Result<(), String> {
    let playback = state.playback.lock().await;
    playback.restore_state().await
}
