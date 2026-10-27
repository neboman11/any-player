//! Bridge between provider Web API DTOs and playback's Connect port.
use ap_playback::spotify::{self as player, SpotifyError};
use ap_providers::spotify::{self as web, SpotifyPlayerApi as _};
use async_trait::async_trait;
use std::sync::Arc;
pub struct SpotifyApi(web::WebSpotifyPlayerApi);
impl SpotifyApi {
    pub fn new(tokens: Arc<web::SpotifyTokenProvider>) -> Self {
        Self(web::WebSpotifyPlayerApi::new(
            reqwest::Client::new(),
            tokens,
        ))
    }
}
fn error(e: web::SpotifyCommandError) -> SpotifyError {
    match e.kind {
        web::SpotifyErrorKind::NotLinked | web::SpotifyErrorKind::TokenRefreshFailed => {
            SpotifyError::Unauthorized
        }
        web::SpotifyErrorKind::PremiumRequired => SpotifyError::PremiumRequired,
        web::SpotifyErrorKind::NoActiveDevice => SpotifyError::NoDevice,
        _ => SpotifyError::Http {
            status: e.http_status.unwrap_or(0),
            message: e.message,
        },
    }
}
#[async_trait]
impl player::SpotifyPlayerApi for SpotifyApi {
    async fn devices(&self) -> Result<Vec<player::SpotifyDevice>, SpotifyError> {
        Ok(self
            .0
            .devices()
            .await
            .map_err(error)?
            .into_iter()
            .map(|d| player::SpotifyDevice {
                id: d.id,
                name: d.name,
                device_type: d.device_type,
                is_active: d.is_active,
                is_restricted: d.is_restricted,
            })
            .collect())
    }
    async fn current_playback(&self) -> Result<Option<player::SpotifyPlayback>, SpotifyError> {
        Ok(self
            .0
            .playback_state()
            .await
            .map_err(error)?
            .map(|p| player::SpotifyPlayback {
                id: p.track_id.unwrap_or_default(),
                linked_from: p.linked_from_id,
                position_ms: p.progress_ms,
                duration_ms: p.duration_ms,
                is_playing: p.is_playing,
                shuffle: p.shuffle,
            }))
    }
    async fn play(&self, r: player::PlayRequest) -> Result<(), SpotifyError> {
        self.0
            .start_playback(
                Some(&r.device_id),
                &r.uris,
                r.offset,
                u64::try_from(r.position_ms).unwrap_or(0),
            )
            .await
            .map_err(error)
    }
    async fn pause(&self, id: &str) -> Result<(), SpotifyError> {
        self.0.pause(Some(id)).await.map_err(error)
    }
    async fn seek(&self, pos: i64, id: &str) -> Result<(), SpotifyError> {
        self.0
            .seek(u64::try_from(pos).unwrap_or(0), Some(id))
            .await
            .map_err(error)
    }
    async fn volume(&self, p: i32, id: &str) -> Result<(), SpotifyError> {
        self.0.set_volume(p, Some(id)).await.map_err(error)
    }
    async fn shuffle(&self, e: bool, id: &str) -> Result<(), SpotifyError> {
        self.0.set_shuffle(e, Some(id)).await.map_err(error)
    }
    async fn repeat(&self, one: bool, id: &str) -> Result<(), SpotifyError> {
        self.0
            .set_repeat(
                if one {
                    ap_model::RepeatMode::One
                } else {
                    ap_model::RepeatMode::Off
                },
                Some(id),
            )
            .await
            .map_err(error)
    }
}
