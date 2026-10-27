//! Main-thread desktop lifecycle, native tray and OS media controls.
//!
//! Keep this handle alive until the event loop exits. Startup must select Slint's
//! winit backend and use `run_event_loop_until_quit`, because hiding must not quit.
use crate::MainWindow;
use ap_model::{PlaybackStateType, QueueState};
use ap_platform::{
    media_controls::{MediaCommand, MediaControls, PlaybackState},
    tray::{TrayEvent, TrayState},
};
use ap_playback::{ChangeOrigin, Engine, PlayerState};
use ap_store::settings::SettingsStore;
use slint::{ComponentHandle, Timer, TimerMode, winit_030::WinitWindowAccessor};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, watch};

const TRAY_NOTICE: &str = "Any Player is still running in the tray.";

#[derive(Debug, thiserror::Error)]
pub enum DesktopError {
    #[error("Desktop integration requires an entered Tokio runtime: {0}")]
    Runtime(#[from] tokio::runtime::TryCurrentError),
}

/// Native resources are deliberately !Send and stay on Slint's thread.
pub struct DesktopIntegration {
    state: Rc<RefCell<DesktopState>>,
    _initialize: Timer,
    _poll: Timer,
    #[cfg(target_os = "linux")]
    stop_tray: Option<tokio::sync::oneshot::Sender<()>>,
}

impl DesktopIntegration {
    pub fn start(
        window: &MainWindow,
        engine: Engine,
        settings: Arc<SettingsStore>,
        show_rx: mpsc::UnboundedReceiver<()>,
        minimized: bool,
        deferred: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, DesktopError> {
        let runtime = tokio::runtime::Handle::try_current()?;
        let (tray_tx, tray_rx) = mpsc::unbounded_channel();
        let (media_tx, media_rx) = mpsc::unbounded_channel();
        let available = Arc::new(AtomicBool::new(false));
        #[cfg(target_os = "linux")]
        let (updates, updates_rx) = watch::channel(TrayState::default());
        #[cfg(target_os = "linux")]
        let (ready_tx, ready_rx) = mpsc::unbounded_channel();
        #[cfg(target_os = "linux")]
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let state = Rc::new(RefCell::new(DesktopState {
            player: engine.player(),
            queue: engine.queue(),
            settings: settings.subscribe(),
            engine,
            runtime: runtime.clone(),
            _settings_store: settings,
            show_rx,
            tray_rx,
            media_rx,
            media_tx,
            available,
            visible: false,
            initialized: false,
            quitting: false,
            notice_shown: false,
            deferred_started: false,
            deferred,
            media: None,
            metadata: None,
            last_playback: None,
            last_volume: None,
            #[cfg(target_os = "linux")]
            updates,
            #[cfg(target_os = "linux")]
            ready_rx,
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            tray: None,
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            tray_tx: tray_tx.clone(),
        }));
        #[cfg(target_os = "linux")]
        runtime.spawn(run_linux_tray(updates_rx, tray_tx, ready_tx, stop_rx));
        #[cfg(not(target_os = "linux"))]
        drop(tray_tx);

        let weak_state = Rc::downgrade(&state);
        let weak_window = window.as_weak();
        window.window().on_close_requested(move || {
            if let (Some(state), Some(window)) = (weak_state.upgrade(), weak_window.upgrade()) {
                let mut state = state.borrow_mut();
                let close_to_tray = state.settings.borrow().ui.close_to_tray;
                match close_decision(
                    close_to_tray,
                    state.available.load(Ordering::Acquire),
                    state.notice_shown,
                ) {
                    CloseDecision::Hide { notify } => {
                        state.hide(&window);
                        if notify {
                            state.notice_shown = true;
                            window.set_toast(TRAY_NOTICE.into());
                            state.runtime.spawn(async {
                                let _ = rfd::AsyncMessageDialog::new()
                                    .set_title("Any Player")
                                    .set_description(TRAY_NOTICE)
                                    .set_level(rfd::MessageLevel::Info)
                                    .set_buttons(rfd::MessageButtons::Ok)
                                    .show()
                                    .await;
                            });
                        }
                    }
                    CloseDecision::Quit => state.quit(),
                }
            }
            // Quit waits for the engine checkpoint rather than losing debounced state.
            slint::CloseRequestResponse::KeepWindowShown
        });

        let initialize = Timer::default();
        let weak_state = Rc::downgrade(&state);
        let weak_window = window.as_weak();
        initialize.start(TimerMode::SingleShot, Duration::ZERO, move || {
            if let (Some(state), Some(window)) = (weak_state.upgrade(), weak_window.upgrade()) {
                state.borrow_mut().initialize(&window, minimized);
            }
        });
        let poll = Timer::default();
        let weak_state = Rc::downgrade(&state);
        let weak_window = window.as_weak();
        poll.start(TimerMode::Repeated, Duration::from_millis(250), move || {
            if let (Some(state), Some(window)) = (weak_state.upgrade(), weak_window.upgrade()) {
                state.borrow_mut().poll(&window);
            }
        });
        Ok(Self {
            state,
            _initialize: initialize,
            _poll: poll,
            #[cfg(target_os = "linux")]
            stop_tray: Some(stop_tx),
        })
    }

    #[must_use]
    pub fn tray_available(&self) -> bool {
        self.state.borrow().available.load(Ordering::Acquire)
    }

    /// Settings adapters can observe changes without borrowing UI-thread resources.
    #[must_use]
    pub fn availability(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.state.borrow().available)
    }
}

impl Drop for DesktopIntegration {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(stop) = self.stop_tray.take() {
            let _ = stop.send(());
        }
    }
}

struct DesktopState {
    engine: Engine,
    runtime: tokio::runtime::Handle,
    player: watch::Receiver<PlayerState>,
    queue: watch::Receiver<QueueState>,
    settings: watch::Receiver<ap_store::settings::Settings>,
    show_rx: mpsc::UnboundedReceiver<()>,
    _settings_store: Arc<SettingsStore>,
    tray_rx: mpsc::UnboundedReceiver<TrayEvent>,
    media_rx: mpsc::UnboundedReceiver<MediaCommand>,
    media_tx: mpsc::UnboundedSender<MediaCommand>,
    available: Arc<AtomicBool>,
    visible: bool,
    initialized: bool,
    quitting: bool,
    notice_shown: bool,
    deferred_started: bool,
    deferred: Arc<dyn Fn() + Send + Sync>,
    media: Option<MediaControls>,
    metadata: Option<Metadata>,
    last_playback: Option<PlaybackState>,
    last_volume: Option<i32>,
    #[cfg(target_os = "linux")]
    updates: watch::Sender<TrayState>,
    #[cfg(target_os = "linux")]
    ready_rx: mpsc::UnboundedReceiver<bool>,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    tray: Option<ap_platform::tray::TrayHandle>,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    tray_tx: mpsc::UnboundedSender<TrayEvent>,
}

impl DesktopState {
    fn initialize(&mut self, window: &MainWindow, minimized: bool) {
        // Realize the native window inside the active event loop, before HWND/SMTC access.
        if let Err(error) = window.show() {
            eprintln!("Cannot create Any Player window: {error}");
            self.quit();
            return;
        }
        self.initialized = true;
        self.visible = true;
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        {
            let icon = native_tray_icon();
            match icon.and_then(|icon| {
                ap_platform::tray::TrayHandle::new(TrayState::default(), self.tray_tx.clone(), icon)
                    .map_err(|error| error.to_string())
            }) {
                Ok(tray) => {
                    self.tray = Some(tray);
                    self.set_available(window, true);
                }
                Err(error) => {
                    eprintln!("System tray unavailable: {error}");
                    self.set_available(window, false);
                }
            }
        }
        match MediaControls::new(native_hwnd(window), self.media_tx.clone()) {
            Ok(media) => self.media = Some(media),
            Err(error) => {
                window.set_toast(format!("OS media controls unavailable: {error}").into());
            }
        }
        if minimized {
            self.hide(window);
            #[cfg(not(target_os = "linux"))]
            if !self.available.load(Ordering::Acquire) {
                self.show(window);
            }
        } else {
            self.show(window);
        }
        self.refresh(window, true);
    }

    fn activate_deferred(&mut self) {
        if !self.deferred_started {
            self.deferred_started = true;
            (self.deferred)();
        }
    }

    fn show(&mut self, window: &MainWindow) {
        if let Err(error) = window.show() {
            window.set_toast(format!("Cannot show window: {error}").into());
            return;
        }
        window.window().with_winit_window(|native| {
            native.set_visible(true);
            native.set_minimized(false);
            native.focus_window();
        });
        self.visible = true;
        self.activate_deferred();
        self.publish_tray(window);
    }

    fn hide(&mut self, window: &MainWindow) {
        // Do not destroy the native HWND: Windows media controls retain it.
        if window
            .window()
            .with_winit_window(|native| native.set_visible(false))
            .is_none()
            && let Err(error) = window.hide()
        {
            window.set_toast(format!("Cannot hide window: {error}").into());
            return;
        }
        self.visible = false;
        self.publish_tray(window);
    }

    fn set_available(&mut self, window: &MainWindow, available: bool) {
        self.available.store(available, Ordering::Release);
        window.set_tray_available(available);
        if !available {
            window.set_toast("No tray host available. Closing quits the app.".into());
            if self.initialized && !self.visible {
                self.show(window);
            }
        }
    }

    fn quit(&mut self) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        let engine = self.engine.clone();
        self.runtime.spawn(async move {
            engine.flush().await;
            let _ = slint::invoke_from_event_loop(|| {
                let _ = slint::quit_event_loop();
            });
        });
    }

    fn poll(&mut self, window: &MainWindow) {
        if !self.initialized || self.quitting {
            return;
        }
        #[cfg(target_os = "linux")]
        while let Ok(available) = self.ready_rx.try_recv() {
            self.set_available(window, available);
        }
        while self.show_rx.try_recv().is_ok() {
            self.show(window);
        }
        while let Ok(event) = self.tray_rx.try_recv() {
            match event {
                TrayEvent::ToggleWindow => {
                    if self.visible {
                        self.hide(window);
                    } else {
                        self.show(window);
                    }
                }
                TrayEvent::TogglePlayback => self.engine.toggle(),
                TrayEvent::Next => self.engine.next(),
                TrayEvent::Previous => self.engine.previous(),
                TrayEvent::Quit => self.quit(),
                TrayEvent::TrayUnavailable => self.set_available(window, false),
            }
        }
        while let Ok(command) = self.media_rx.try_recv() {
            dispatch_media(&self.engine, &command);
        }
        self.refresh(window, false);
    }

    fn refresh(&mut self, window: &MainWindow, force: bool) {
        let changed = force
            || self.player.has_changed().unwrap_or(false)
            || self.queue.has_changed().unwrap_or(false);
        if changed {
            let metadata = metadata(
                &self.queue.borrow_and_update(),
                &self.player.borrow_and_update(),
            );
            if self.metadata.as_ref() != Some(&metadata) {
                if let Some(media) = self.media.as_mut()
                    && let Err(error) = media.set_metadata(souvlaki::MediaMetadata {
                        title: Some(&metadata.title),
                        artist: metadata.artist.as_deref(),
                        album: metadata.album.as_deref(),
                        cover_url: metadata.art_url.as_deref(),
                        duration: metadata.duration,
                    })
                {
                    window.set_toast(format!("Cannot update OS media metadata: {error}").into());
                }
                self.metadata = Some(metadata);
            }
            self.publish_tray(window);
        }
        let (playback, play_requested, volume) = {
            let state = self.player.borrow();
            (
                media_playback(&state, tokio::time::Instant::now()),
                state.play_requested,
                state.volume,
            )
        };
        if play_requested {
            self.activate_deferred();
        }
        if let Some(media) = self.media.as_mut() {
            if self.last_playback != Some(playback) {
                match media.set_playback(playback) {
                    Ok(()) => self.last_playback = Some(playback),
                    Err(error) => {
                        window
                            .set_toast(format!("Cannot update OS playback state: {error}").into());
                    }
                }
            }
            if self.last_volume != Some(volume) {
                match media.set_volume(f64::from(volume.clamp(0, 100)) / 100.0) {
                    Ok(()) => self.last_volume = Some(volume),
                    Err(error) => {
                        window.set_toast(format!("Cannot update OS media volume: {error}").into());
                    }
                }
            }
        }
    }

    fn publish_tray(&self, window: &MainWindow) {
        let state = TrayState {
            window_visible: self.visible,
            playing: self.player.borrow().play_requested,
            tooltip: self
                .metadata
                .as_ref()
                .map_or_else(|| "Any Player".into(), Metadata::tooltip),
        };
        #[cfg(target_os = "linux")]
        {
            let _ = window;
            self.updates.send_replace(state);
        }
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        if let Some(tray) = self.tray.as_ref() {
            if let Err(error) = tray.update(state) {
                window.set_toast(error.to_string().into());
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Metadata {
    title: String,
    artist: Option<String>,
    album: Option<String>,
    art_url: Option<String>,
    duration: Option<Duration>,
}
impl Metadata {
    fn tooltip(&self) -> String {
        self.artist.as_ref().map_or_else(
            || self.title.clone(),
            |artist| format!("{} — {artist}", self.title),
        )
    }
}
fn metadata(queue: &QueueState, state: &PlayerState) -> Metadata {
    if let Some(active_break) = state.active_break.as_ref() {
        return Metadata {
            title: "AnyPlayer DJ".into(),
            artist: Some("Any Player".into()),
            album: None,
            art_url: None,
            duration: positive_duration(active_break.duration_ms),
        };
    }
    queue.current().map_or_else(
        || Metadata {
            title: "Any Player".into(),
            artist: None,
            album: None,
            art_url: None,
            duration: None,
        },
        |entry| Metadata {
            title: entry.track.title.clone(),
            artist: Some(entry.track.artist.clone()),
            album: entry.track.album.clone(),
            art_url: entry.track.image_url.clone(),
            duration: positive_duration(state.duration_ms)
                .or_else(|| entry.track.duration_ms.and_then(positive_duration)),
        },
    )
}
fn positive_duration(ms: i64) -> Option<Duration> {
    u64::try_from(ms)
        .ok()
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
}
fn media_playback(state: &PlayerState, now: tokio::time::Instant) -> PlaybackState {
    if state.current_uid.is_none() && state.active_break.is_none() {
        return PlaybackState::Stopped;
    }
    let position = u64::try_from(state.position_at(now)).unwrap_or_default();
    if state.status == PlaybackStateType::Playing {
        // MPRIS extrapolates position; publish at most once per second between state changes.
        PlaybackState::Playing(Duration::from_secs(position / 1000))
    } else {
        PlaybackState::Paused(Duration::from_millis(position))
    }
}
#[derive(Debug, PartialEq, Eq)]
enum CloseDecision {
    Hide { notify: bool },
    Quit,
}
fn close_decision(enabled: bool, available: bool, notice_shown: bool) -> CloseDecision {
    if enabled && available {
        CloseDecision::Hide {
            notify: !notice_shown,
        }
    } else {
        CloseDecision::Quit
    }
}
fn dispatch_media(engine: &Engine, command: &MediaCommand) {
    match *command {
        MediaCommand::Play => engine.play(ChangeOrigin::User),
        MediaCommand::Pause => engine.pause(ChangeOrigin::User),
        MediaCommand::Toggle => engine.toggle(),
        MediaCommand::Next => engine.next(),
        MediaCommand::Previous => engine.previous(),
        MediaCommand::SetPosition(position) => {
            engine.seek(duration_ms(position), ChangeOrigin::User);
        }
        MediaCommand::Seek { forward, amount } => engine.seek(
            seek_position(
                engine.position_ms(),
                forward,
                amount.unwrap_or(Duration::from_secs(5)),
            ),
            ChangeOrigin::User,
        ),
        MediaCommand::Volume(volume) => {
            if let Some(volume) = volume_percent(volume) {
                engine.set_volume(volume, ChangeOrigin::User);
            }
        }
    }
}
fn duration_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
fn seek_position(current: i64, forward: bool, amount: Duration) -> i64 {
    let position = if forward {
        current.saturating_add(duration_ms(amount))
    } else {
        current.saturating_sub(duration_ms(amount))
    };
    position.max(0)
}
#[expect(
    clippy::cast_possible_truncation,
    reason = "Finite normalized OS volume is clamped to 0..=100 before conversion"
)]
fn volume_percent(volume: f64) -> Option<i32> {
    volume
        .is_finite()
        .then(|| (volume.clamp(0.0, 1.0) * 100.0).round() as i32)
}

#[cfg(not(target_os = "windows"))]
fn native_hwnd(_: &MainWindow) -> Option<*mut std::ffi::c_void> {
    None
}
#[cfg(target_os = "windows")]
fn native_hwnd(window: &MainWindow) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    window
        .window()
        .with_winit_window(|native| {
            native
                .window_handle()
                .ok()
                .and_then(|handle| match handle.as_raw() {
                    RawWindowHandle::Win32(handle) => {
                        Some(handle.hwnd.get() as *mut std::ffi::c_void)
                    }
                    _ => None,
                })
        })
        .flatten()
}
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn native_tray_icon() -> Result<tray_icon::Icon, String> {
    let mut rgba = vec![0; 32 * 32 * 4];
    for y in 0_usize..32 {
        for x in 0_usize..32 {
            let offset = (y * 32 + x) * 4;
            if (4..28).contains(&x) && (4..28).contains(&y) {
                rgba[offset..offset + 4].copy_from_slice(&[21, 154, 131, 255]);
                if [9, 13, 17, 21].contains(&x) && (10..22).contains(&y) {
                    rgba[offset..offset + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, 32, 32).map_err(|error| error.to_string())
}
#[cfg(target_os = "linux")]
async fn run_linux_tray(
    mut updates: watch::Receiver<TrayState>,
    events: mpsc::UnboundedSender<TrayEvent>,
    ready: mpsc::UnboundedSender<bool>,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) {
    let initial = updates.borrow_and_update().clone();
    let tray = tokio::select! {
        result = ap_platform::tray::TrayHandle::new(initial, events) => result,
        _ = &mut stop => return,
    };
    match tray {
        Ok(tray) => {
            let _ = ready.send(true);
            loop {
                tokio::select! {
                    _ = &mut stop => break,
                    changed = updates.changed() => {
                        if changed.is_err() { break; }
                        let state = updates.borrow_and_update().clone();
                        tray.update(state).await;
                    }
                }
            }
            tray.shutdown().await;
        }
        Err(error) => {
            eprintln!("System tray unavailable: {error}");
            let _ = ready.send(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_policy_never_hides_without_a_tray_and_notifies_once() {
        assert_eq!(
            close_decision(true, true, false),
            CloseDecision::Hide { notify: true }
        );
        assert_eq!(
            close_decision(true, true, true),
            CloseDecision::Hide { notify: false }
        );
        for enabled in [false, true] {
            assert_eq!(close_decision(enabled, false, false), CloseDecision::Quit);
        }
        assert_eq!(close_decision(false, true, false), CloseDecision::Quit);
    }
    #[test]
    fn playback_mapping_preserves_paused_position_and_stopped_state() {
        assert_eq!(
            media_playback(&PlayerState::default(), tokio::time::Instant::now()),
            PlaybackState::Stopped
        );
        let mut state = PlayerState {
            current_uid: Some(7),
            status: PlaybackStateType::Paused,
            ..PlayerState::default()
        };
        state.anchor.position_ms = 12_345;
        assert_eq!(
            media_playback(&state, state.anchor.at),
            PlaybackState::Paused(Duration::from_millis(12_345))
        );
        state.status = PlaybackStateType::Playing;
        assert_eq!(
            media_playback(&state, state.anchor.at + Duration::from_millis(1000)),
            PlaybackState::Playing(Duration::from_secs(13))
        );
    }
    #[test]
    fn dj_metadata_does_not_leak_previous_song_artwork() {
        let state = PlayerState {
            active_break: Some(ap_playback::ActiveBreak {
                break_id: "break".into(),
                duration_ms: 4500,
            }),
            ..PlayerState::default()
        };
        let metadata = metadata(&QueueState::default(), &state);
        assert_eq!(metadata.title, "AnyPlayer DJ");
        assert_eq!(metadata.art_url, None);
        assert_eq!(metadata.duration, Some(Duration::from_millis(4500)));
        assert_eq!(metadata.tooltip(), "AnyPlayer DJ — Any Player");
    }
    #[test]
    fn song_metadata_includes_art_duration_and_track_tooltip() {
        let mut track =
            ap_model::Track::new("track", "Title", "Artist", ap_model::SourceType::Jellyfin);
        track.album = Some("Album".into());
        track.image_url = Some("https://music.test/cover.png".into());
        track.duration_ms = Some(12_345);
        let queue = QueueState {
            entries: vec![ap_model::QueueEntry { uid: 7, track }].into(),
            order: vec![0].into(),
            cursor: 0,
            ..QueueState::default()
        };
        let mapped = metadata(&queue, &PlayerState::default());
        assert_eq!(mapped.title, "Title");
        assert_eq!(mapped.artist.as_deref(), Some("Artist"));
        assert_eq!(mapped.album.as_deref(), Some("Album"));
        assert_eq!(
            mapped.art_url.as_deref(),
            Some("https://music.test/cover.png")
        );
        assert_eq!(mapped.duration, Some(Duration::from_millis(12_345)));
        assert_eq!(mapped.tooltip(), "Title — Artist");
    }
    #[test]
    fn media_seek_saturates_and_volume_rejects_nonfinite_values() {
        assert_eq!(seek_position(1000, false, Duration::from_secs(5)), 0);
        assert_eq!(
            seek_position(i64::MAX - 1, true, Duration::from_secs(5)),
            i64::MAX
        );
        assert_eq!(volume_percent(0.555), Some(56));
        assert_eq!(volume_percent(-1.0), Some(0));
        assert_eq!(volume_percent(2.0), Some(100));
        assert_eq!(volume_percent(f64::NAN), None);
        assert_eq!(volume_percent(f64::INFINITY), None);
    }
}
