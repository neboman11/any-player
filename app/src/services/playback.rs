//! Engine snapshots and user commands for Slint.
use crate::vm::{PlaybackCommand, PlaybackPort, PlaybackSnapshot, Result};
use crate::vm::playback::PlaybackNotice;
use ap_playback::{Engine, state::{ChangeOrigin, EngineEvent}};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, watch};
pub struct PlaybackAdapter {
    engine: Engine,
    state: watch::Sender<PlaybackSnapshot>,
    notice: Mutex<PlaybackNotice>,
    errors: broadcast::Sender<String>,
}
fn snapshot(engine: &Engine) -> PlaybackSnapshot {
    let p = engine.player().borrow().clone();
    PlaybackSnapshot {
        queue: engine.queue().borrow().clone(),
        playing: p.rate() > 0.0,
        buffering: p.play_requested && p.status == ap_model::PlaybackStateType::Buffering,
        error: String::new(),
        position_ms: engine.position_ms(),
        at: std::time::Instant::now(),
        volume: u8::try_from(p.volume).unwrap_or(50),
        disabled_reason: p.disabled_reason.unwrap_or_default(),
        device: String::new(),
    }
}
impl PlaybackAdapter {
    pub fn new(engine: Engine) -> Arc<Self> {
        let mut notice = PlaybackNotice::default();
        let player = engine.player().borrow().clone();
        notice.observe(player.error.as_deref(), player.status == ap_model::PlaybackStateType::Playing);
        let mut initial = snapshot(&engine);
        notice.message().clone_into(&mut initial.error);
        let mut events = engine.events();
        let adapter = Arc::new(Self {
            state: watch::Sender::new(initial),
            notice: Mutex::new(notice),
            errors: broadcast::channel(32).0,
            engine,
        });
        let this = adapter.clone();
        tokio::spawn(async move {
            let mut queue = this.engine.queue();
            let mut player = this.engine.player();
            loop {
                let player_changed = tokio::select! {
                    r = queue.changed() => { if r.is_err() { break } false },
                    r = player.changed() => { if r.is_err() { break } true },
                    event = events.recv() => match event {
                        Ok(EngineEvent::Error { message, .. }) => {
                            let mut notice = this.notice.lock().expect("playback notice");
                            if let Some(message) = notice.event(&message) {
                                let _ = this.errors.send(message);
                            }
                            false
                        }
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => false,
                        Err(broadcast::error::RecvError::Closed) => break,
                    },
                };
                let mut notice = this.notice.lock().expect("playback notice");
                if player_changed {
                    let p = player.borrow().clone();
                    notice.observe(p.error.as_deref(), p.status == ap_model::PlaybackStateType::Playing);
                }
                let mut state = snapshot(&this.engine);
                notice.message().clone_into(&mut state.error);
                this.state.send_replace(state);
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
    fn errors(&self) -> broadcast::Receiver<String> {
        self.errors.subscribe()
    }
    async fn playback(&self, c: PlaybackCommand) -> Result {
        match c {
            PlaybackCommand::DismissError => {
                self.notice.lock().expect("playback notice").dismiss();
                self.state.send_modify(|state| state.error.clear());
            }
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
