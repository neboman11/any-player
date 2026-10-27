//! Engine snapshots and user commands for Slint.
use crate::vm::{PlaybackCommand, PlaybackPort, PlaybackSnapshot, Result};
use ap_playback::{Engine, state::ChangeOrigin};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::watch;
pub struct PlaybackAdapter {
    engine: Engine,
    state: watch::Sender<PlaybackSnapshot>,
}
fn snapshot(engine: &Engine) -> PlaybackSnapshot {
    let p = engine.player().borrow().clone();
    PlaybackSnapshot {
        queue: engine.queue().borrow().clone(),
        playing: p.rate() > 0.0,
        position_ms: engine.position_ms(),
        at: std::time::Instant::now(),
        volume: u8::try_from(p.volume).unwrap_or(50),
        disabled_reason: p.disabled_reason.unwrap_or_default(),
        device: String::new(),
    }
}
impl PlaybackAdapter {
    pub fn new(engine: Engine) -> Arc<Self> {
        let adapter = Arc::new(Self {
            state: watch::Sender::new(snapshot(&engine)),
            engine,
        });
        let this = adapter.clone();
        tokio::spawn(async move {
            let mut queue = this.engine.queue();
            let mut player = this.engine.player();
            loop {
                tokio::select! {r=queue.changed()=>if r.is_err(){break},r=player.changed()=>if r.is_err(){break}}
                this.state.send_replace(snapshot(&this.engine));
            }
        });
        adapter
    }
}
#[async_trait]
impl PlaybackPort for PlaybackAdapter {
    fn subscribe(&self) -> watch::Receiver<PlaybackSnapshot> {
        self.state.subscribe()
    }
    async fn playback(&self, c: PlaybackCommand) -> Result {
        match c {
            PlaybackCommand::Toggle => self.engine.toggle(),
            PlaybackCommand::Next => self.engine.next(),
            PlaybackCommand::Previous => self.engine.previous(),
            PlaybackCommand::Jump(uid) => self.engine.jump_to_uid(uid),
            PlaybackCommand::Seek(ms) => self.engine.seek(ms, ChangeOrigin::User),
            PlaybackCommand::Volume(v) => self.engine.set_volume(i32::from(v), ChangeOrigin::User),
            PlaybackCommand::Shuffle => {
                let enabled = !self.engine.queue().borrow().shuffle;
                self.engine.set_shuffle(enabled, ChangeOrigin::User);
            }
            PlaybackCommand::Repeat => {
                let next = match self.engine.queue().borrow().repeat {
                    ap_model::RepeatMode::Off => ap_model::RepeatMode::All,
                    ap_model::RepeatMode::All => ap_model::RepeatMode::One,
                    ap_model::RepeatMode::One => ap_model::RepeatMode::Off,
                };
                self.engine.set_repeat(next, ChangeOrigin::User);
            }
            PlaybackCommand::PlayNext(track) => self.engine.play_next(vec![track]),
            PlaybackCommand::SetQueue { tracks, start } => self.engine.set_queue(
                tracks,
                isize::try_from(start).unwrap_or(0),
                ChangeOrigin::User,
                true,
            ),
        }
        Ok(())
    }
}
