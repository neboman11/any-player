mod support;
use ap_model::{BackendKind, SourceType};
use ap_playback::spotify::*;
use ap_playback::{Backend, BackendEvent, BackendRun, ChangeOrigin, RunItem};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use support::{settle, track};
const ID: &str = "0123456789012345678901";
const NEXT: &str = "1123456789012345678901";
struct Api {
    states: Mutex<VecDeque<Option<SpotifyPlayback>>>,
    last: Mutex<Option<SpotifyPlayback>>,
    devices: Mutex<Vec<SpotifyDevice>>,
    calls: Mutex<Vec<String>>,
    plays: Mutex<Vec<PlayRequest>>,
    device_calls: AtomicUsize,
    settings_failures: AtomicUsize,
}
impl Api {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            states: Mutex::default(),
            last: Mutex::new(None),
            devices: Mutex::new(vec![device("computer", "Computer", true)]),
            calls: Mutex::default(),
            plays: Mutex::default(),
            device_calls: AtomicUsize::new(0),
            settings_failures: AtomicUsize::new(0),
        })
    }
    fn state(&self, s: Option<SpotifyPlayback>) {
        *self.last.lock().unwrap() = s;
    }
    fn setting_result(&self) -> Result<(), SpotifyError> {
        if self
            .settings_failures
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok()
        {
            Err(SpotifyError::Http {
                status: 404,
                message: "Device is still activating".into(),
            })
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl SpotifyPlayerApi for Api {
    async fn devices(&self) -> Result<Vec<SpotifyDevice>, SpotifyError> {
        self.device_calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.devices.lock().unwrap().clone())
    }
    async fn current_playback(&self) -> Result<Option<SpotifyPlayback>, SpotifyError> {
        if let Some(s) = self.states.lock().unwrap().pop_front() {
            *self.last.lock().unwrap() = s;
        }
        Ok(self.last.lock().unwrap().clone())
    }
    async fn play(&self, r: PlayRequest) -> Result<(), SpotifyError> {
        self.plays.lock().unwrap().push(r);
        Ok(())
    }
    async fn pause(&self, _: &str) -> Result<(), SpotifyError> {
        self.calls.lock().unwrap().push("pause".into());
        Ok(())
    }
    async fn seek(&self, p: i64, _: &str) -> Result<(), SpotifyError> {
        self.calls.lock().unwrap().push(format!("seek:{p}"));
        Ok(())
    }
    async fn volume(&self, v: i32, _: &str) -> Result<(), SpotifyError> {
        self.calls.lock().unwrap().push(format!("volume:{v}"));
        self.setting_result()
    }
    async fn shuffle(&self, on: bool, _: &str) -> Result<(), SpotifyError> {
        self.calls.lock().unwrap().push(format!("shuffle:{on}"));
        Ok(())
    }
    async fn repeat(&self, on: bool, _: &str) -> Result<(), SpotifyError> {
        self.calls.lock().unwrap().push(format!("repeat:{on}"));
        self.setting_result()
    }
}
struct Launcher {
    calls: AtomicUsize,
    api: Arc<Api>,
    available: bool,
}
#[async_trait]
impl SpotifyLauncher for Launcher {
    fn hostname(&self) -> String {
        "hostname".into()
    }
    async fn launch(&self) -> Result<(), SpotifyError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.available {
            *self.api.devices.lock().unwrap() = vec![device("hostname", "Computer", false)];
            Ok(())
        } else {
            Err(SpotifyError::NotInstalled)
        }
    }
}
fn device(name: &str, kind: &str, active: bool) -> SpotifyDevice {
    SpotifyDevice {
        id: name.into(),
        name: name.into(),
        device_type: kind.into(),
        is_active: active,
        is_restricted: false,
    }
}
fn state(id: &str, p: i64, playing: bool) -> SpotifyPlayback {
    SpotifyPlayback {
        id: id.into(),
        linked_from: None,
        position_ms: p,
        duration_ms: 60_000,
        is_playing: playing,
        shuffle: false,
    }
}
fn run(ids: &[&str]) -> BackendRun {
    BackendRun {
        kind: BackendKind::Spotify,
        items: ids
            .iter()
            .enumerate()
            .map(|(i, id)| RunItem::Song {
                uid: i as u64 + 1,
                track: track(id, SourceType::Spotify),
            })
            .collect(),
    }
}
fn rig() -> (
    Arc<Api>,
    Arc<Launcher>,
    Arc<SpotifyDeviceResolver>,
    SpotifyBackend,
) {
    let api = Api::new();
    let launcher = Arc::new(Launcher {
        calls: AtomicUsize::new(0),
        api: api.clone(),
        available: true,
    });
    let devices = Arc::new(SpotifyDeviceResolver::new(api.clone(), launcher.clone()));
    let backend = SpotifyBackend::new(api.clone(), devices.clone());
    (api, launcher, devices, backend)
}
async fn tick(ms: u64) {
    tokio::time::advance(Duration::from_millis(ms)).await;
    settle().await;
}
#[tokio::test(start_paused = true)]
async fn adopt_shuffle_repeat_and_relink() {
    let (api, _, _, backend) = rig();
    let mut s = state("relinked", 1000, true);
    s.linked_from = Some(ID.into());
    s.shuffle = true;
    api.state(Some(s));
    backend.set_repeat_one(true).await;
    backend
        .load(1, run(&[ID]), 0, 0, true, ChangeOrigin::User)
        .await;
    assert!(api.plays.lock().unwrap().is_empty());
    assert!(api.calls.lock().unwrap().contains(&"shuffle:false".into()));
    assert!(api.calls.lock().unwrap().contains(&"repeat:true".into()));
    assert_eq!(backend.position_ms(), 1000);
}
#[tokio::test(start_paused = true)]
async fn load_sends_native_uri_offset_and_saved_position() {
    let (api, _, _, backend) = rig();
    backend
        .load(5, run(&[ID, NEXT]), 1, 1234, true, ChangeOrigin::User)
        .await;
    let p = api.plays.lock().unwrap()[0].clone();
    assert_eq!(
        p.uris,
        vec![
            format!("spotify:track:{ID}"),
            format!("spotify:track:{NEXT}")
        ]
    );
    assert_eq!(p.offset, 1);
    assert_eq!(p.position_ms, 1234);
}
#[tokio::test(start_paused = true)]
async fn restore_paused_does_not_resolve_or_start() {
    let (api, launcher, _, backend) = rig();
    backend
        .load(1, run(&[ID]), 0, 1234, false, ChangeOrigin::Restore)
        .await;
    tick(20_000).await;
    assert_eq!(api.device_calls.load(Ordering::Relaxed), 0);
    assert!(api.plays.lock().unwrap().is_empty());
    assert_eq!(launcher.calls.load(Ordering::Relaxed), 0);
    backend.play(ChangeOrigin::User).await;
    assert_eq!(api.plays.lock().unwrap()[0].position_ms, 1234);
}
#[tokio::test(start_paused = true)]
async fn settle_ignores_wrong_track_and_unknown_requires_two() {
    let (api, _, _, backend) = rig();
    let mut events = backend.subscribe();
    backend
        .load(1, run(&[ID]), 0, 0, true, ChangeOrigin::User)
        .await;
    api.state(Some(state("unknown", 0, true)));
    tick(2000).await;
    assert!(
        !std::iter::from_fn(|| events.try_recv().ok())
            .any(|e| matches!(e, BackendEvent::RunEnded { .. }))
    );
    api.state(Some(state(ID, 4000, true)));
    tick(2000).await;
    api.state(Some(state("unknown", 0, true)));
    tick(2000).await;
    assert!(
        !std::iter::from_fn(|| events.try_recv().ok())
            .any(|e| matches!(e, BackendEvent::RunEnded { .. }))
    );
    tick(2000).await;
    assert!(
        std::iter::from_fn(|| events.try_recv().ok())
            .any(|e| matches!(e, BackendEvent::RunEnded { load_id: 1 }))
    );
}
#[tokio::test(start_paused = true)]
async fn unknown_before_playing_does_not_end() {
    let (api, _, _, backend) = rig();
    let mut events = backend.subscribe();
    backend
        .load(1, run(&[ID]), 0, 0, true, ChangeOrigin::User)
        .await;
    api.state(Some(state("unknown", 0, false)));
    for _ in 0..4 {
        tick(2000).await;
    }
    assert!(
        !std::iter::from_fn(|| events.try_recv().ok())
            .any(|e| matches!(e, BackendEvent::RunEnded { .. }))
    );
}
#[tokio::test(start_paused = true)]
async fn end_lead_pauses_before_run_end() {
    let (api, _, _, backend) = rig();
    api.state(Some(state(ID, 59_000, true)));
    let mut events = backend.subscribe();
    backend
        .load(8, run(&[ID]), 0, 59_000, true, ChangeOrigin::User)
        .await;
    api.state(Some(state(ID, 59_700, true)));
    tick(700).await;
    let events = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(api.calls.lock().unwrap().contains(&"pause".into()));
    assert!(events.contains(&BackendEvent::RunEnded { load_id: 8 }));
}
#[tokio::test(start_paused = true)]
async fn external_pause_is_adopted() {
    let (api, _, _, backend) = rig();
    api.state(Some(state(ID, 0, true)));
    let mut events = backend.subscribe();
    backend
        .load(1, run(&[ID]), 0, 0, true, ChangeOrigin::User)
        .await;
    api.state(Some(state(ID, 1000, false)));
    tick(2000).await;
    assert!(
        std::iter::from_fn(|| events.try_recv().ok()).any(|e| matches!(
            e,
            BackendEvent::PlayingChanged {
                is_playing: false,
                by_player: true,
                ..
            }
        ))
    );
}
#[tokio::test(start_paused = true)]
async fn volume_and_prefix_reload_debounce() {
    let (api, _, _, backend) = rig();
    api.state(Some(state(ID, 0, true)));
    backend
        .load(1, run(&[ID, NEXT]), 0, 0, true, ChangeOrigin::User)
        .await;
    backend.set_volume(10).await;
    backend.set_volume(20).await;
    tick(299).await;
    assert!(!api.calls.lock().unwrap().contains(&"volume:20".into()));
    tick(1).await;
    assert!(api.calls.lock().unwrap().contains(&"volume:20".into()));
    backend
        .update_upcoming(
            1,
            run(&[ID, "2123456789012345678901"]),
            ap_playback::ItemKey::Song(1),
        )
        .await;
    tick(299).await;
    assert!(api.plays.lock().unwrap().is_empty());
    tick(1).await;
    assert_eq!(api.plays.lock().unwrap().len(), 1);
}
#[tokio::test(start_paused = true)]
async fn stalled_playing_recovers_boundedly() {
    let (api, _, _, backend) = rig();
    api.state(Some(state(ID, 0, true)));
    let mut events = backend.subscribe();
    backend
        .load(1, run(&[ID]), 0, 0, true, ChangeOrigin::User)
        .await;
    for _ in 0..40 {
        tick(2000).await;
    }
    assert!(api.plays.lock().unwrap().len() <= MAX_RECOVERY_ATTEMPTS);
    assert!(
        std::iter::from_fn(|| events.try_recv().ok())
            .any(|e| matches!(e, BackendEvent::Error { .. }))
    );
}
#[tokio::test(start_paused = true)]
async fn device_preference_active_hostname_and_cache() {
    let (api, _, devices, _) = rig();
    *api.devices.lock().unwrap() = vec![
        device("other", "Speaker", true),
        device("hostname", "Computer", false),
    ];
    assert_eq!(devices.resolve(ChangeOrigin::Sync).await.unwrap(), "other");
    devices
        .set_preferred(Some(PreferredDevice {
            id: "missing".into(),
            name: "hostname".into(),
        }))
        .await;
    assert_eq!(
        devices.resolve(ChangeOrigin::Sync).await.unwrap(),
        "hostname"
    );
    let calls = api.device_calls.load(Ordering::Relaxed);
    assert_eq!(
        devices.resolve(ChangeOrigin::User).await.unwrap(),
        "hostname"
    );
    assert_eq!(api.device_calls.load(Ordering::Relaxed), calls);
    tick(30_001).await;
    devices.resolve(ChangeOrigin::Sync).await.unwrap();
    assert!(api.device_calls.load(Ordering::Relaxed) > calls);
}
#[tokio::test(start_paused = true)]
async fn device_launch_single_flight_and_no_sync_launch() {
    let (api, launcher, devices, _) = rig();
    api.devices.lock().unwrap().clear();
    assert_eq!(
        devices.resolve(ChangeOrigin::Sync).await,
        Err(SpotifyError::NoDevice)
    );
    let (a, b) = tokio::join!(
        devices.resolve(ChangeOrigin::User),
        devices.resolve(ChangeOrigin::User)
    );
    assert_eq!(a.unwrap(), "hostname");
    assert_eq!(b.unwrap(), "hostname");
    assert_eq!(launcher.calls.load(Ordering::Relaxed), 1);
}
#[tokio::test(start_paused = true)]
async fn shortened_prefix_does_not_reload() {
    let (api, _, _, backend) = rig();
    api.state(Some(state(ID, 0, true)));
    backend
        .load(1, run(&[ID, NEXT]), 0, 0, true, ChangeOrigin::User)
        .await;
    backend
        .update_upcoming(1, run(&[ID]), ap_playback::ItemKey::Song(1))
        .await;
    tick(300).await;
    assert!(api.plays.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn accepted_play_survives_transient_device_setting_rejections() {
    let (api, _, _, backend) = rig();
    api.settings_failures.store(2, Ordering::Relaxed);
    let mut events = backend.subscribe();
    backend
        .load(1, run(&[ID]), 0, 0, true, ChangeOrigin::User)
        .await;
    api.state(Some(state(ID, 2000, true)));
    tick(2000).await;
    assert!(
        !std::iter::from_fn(|| events.try_recv().ok())
            .any(|event| matches!(event, BackendEvent::Error { .. }))
    );
    assert_eq!(
        api.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.starts_with("volume:"))
            .count(),
        2
    );
    assert_eq!(
        api.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.starts_with("repeat:"))
            .count(),
        2
    );
    assert!(!api.calls.lock().unwrap().contains(&"pause".into()));
}

struct EmptyLauncher {
    calls: AtomicUsize,
}
#[async_trait]
impl SpotifyLauncher for EmptyLauncher {
    fn hostname(&self) -> String {
        "hostname".into()
    }
    async fn launch(&self) -> Result<(), SpotifyError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
#[tokio::test(start_paused = true)]
async fn device_launch_wait_is_five_bounded_polls() {
    let api = Api::new();
    api.devices.lock().unwrap().clear();
    let launcher = Arc::new(EmptyLauncher {
        calls: AtomicUsize::new(0),
    });
    let resolver = SpotifyDeviceResolver::new(api.clone(), launcher.clone());
    let at = tokio::time::Instant::now();
    assert_eq!(
        resolver.resolve(ChangeOrigin::User).await,
        Err(SpotifyError::NoDevice)
    );
    assert_eq!(
        at.elapsed(),
        DEVICE_WAIT_INTERVAL * u32::try_from(DEVICE_WAIT_ATTEMPTS).unwrap()
    );
    assert_eq!(api.device_calls.load(Ordering::Relaxed), 6);
    assert_eq!(launcher.calls.load(Ordering::Relaxed), 1);
}
