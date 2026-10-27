//! Store adapters shared by production playback and DJ services.
use ap_playback::ports::{BoxError, PlaybackPersistence, PlayerSettings};
use ap_store::{Db, settings::SettingsStore};
use async_trait::async_trait;
use std::sync::Arc;

pub struct Persistence {
    pub db: Db,
    pub settings: Arc<SettingsStore>,
}
#[async_trait]
impl PlaybackPersistence for Persistence {
    async fn load_queue(&self) -> Result<ap_model::QueueState, BoxError> {
        Ok(self.db.read(ap_store::queue::load).await?)
    }
    async fn save_queue_content(&self, queue: &ap_model::QueueState) -> Result<(), BoxError> {
        let queue = queue.clone();
        self.db
            .call(move |c| ap_store::queue::write_content(c, &queue))
            .await?;
        Ok(())
    }
    async fn save_cursor(&self, uid: Option<u64>, version: u64) -> Result<(), BoxError> {
        self.db
            .call(move |c| ap_store::queue::write_cursor(c, uid, version))
            .await?;
        Ok(())
    }
    async fn load_settings(&self) -> Result<PlayerSettings, BoxError> {
        let s = self.settings.get().player;
        Ok(PlayerSettings {
            volume: s.volume,
            shuffle: s.shuffle,
            repeat: s.repeat,
            position_ms: s.position_ms,
            normalization_enabled: s.normalization_enabled,
            normalization_strict: s.normalization_strict,
        })
    }
    async fn save_settings(&self, s: &PlayerSettings) -> Result<(), BoxError> {
        self.settings
            .update(|v| {
                v.player.volume = s.volume;
                v.player.shuffle = s.shuffle;
                v.player.repeat = s.repeat;
                v.player.position_ms = s.position_ms;
                v.player.normalization_enabled = s.normalization_enabled;
                v.player.normalization_strict = s.normalization_strict;
            })
            .await?;
        Ok(())
    }
    async fn track_gain(&self, key: &str) -> Option<f32> {
        let key = key.to_owned();
        self.db
            .read(move |c| ap_store::track_gain::get(c, &key))
            .await
            .ok()
            .flatten()
            .map(|v| v.gain)
    }
    async fn put_track_gain(&self, key: &str, gain: f32) {
        let key = key.to_owned();
        let analyzed_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
        if let Err(error) = self
            .db
            .call(move |c| {
                ap_store::track_gain::put(
                    c,
                    &key,
                    ap_store::track_gain::TrackGain {
                        gain,
                        analyzed_at_ms,
                    },
                )
            })
            .await
        {
            tracing::warn!(%error,"Could not persist track gain");
        }
    }
}
impl ap_dj::coordinator::DjSettingsStore for Persistence {
    fn load(&self) -> ap_dj::passages::StoreFuture<'_, ap_dj::coordinator::DjSettings> {
        Box::pin(async {
            let s = self.settings.get().dj;
            Ok(ap_dj::coordinator::DjSettings {
                enabled: s.enabled,
                show_entries_in_queue: s.show_entries_in_queue,
                selected_voice_id: s.voice_id,
                voice_level: super::voice_level(s.voice_level),
            })
        })
    }
    fn save(&self, s: ap_dj::coordinator::DjSettings) -> ap_dj::passages::StoreFuture<'_, ()> {
        Box::pin(async move {
            self.settings
                .update(|v| {
                    v.dj.enabled = s.enabled;
                    v.dj.show_entries_in_queue = s.show_entries_in_queue;
                    v.dj.voice_id = s.selected_voice_id;
                    v.dj.voice_level = f64::from(s.voice_level);
                })
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        })
    }
}
impl ap_dj::passages::PassageStateStore for Persistence {
    fn read<'a>(&'a self, key: &'a str) -> ap_dj::passages::StoreFuture<'a, Option<String>> {
        let key = key.to_owned();
        Box::pin(async move {
            Ok(self
                .db
                .read(move |c| ap_store::dj_state::load(c, &key))
                .await?)
        })
    }
    fn write<'a>(&'a self, key: &'a str, value: String) -> ap_dj::passages::StoreFuture<'a, ()> {
        let key = key.to_owned();
        Box::pin(async move {
            self.db
                .call(move |c| ap_store::dj_state::save(c, &key, &value))
                .await?;
            Ok(())
        })
    }
}
