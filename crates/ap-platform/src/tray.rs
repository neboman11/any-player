//! Native tray integration. Native tray construction must occur on the UI thread.
use tokio::sync::mpsc;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    ToggleWindow,
    TogglePlayback,
    Next,
    Previous,
    Quit,
    TrayUnavailable,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrayState {
    pub window_visible: bool,
    pub playing: bool,
    pub tooltip: String,
}
impl TrayState {
    pub fn window_label(&self) -> &'static str {
        if self.window_visible { "Hide" } else { "Show" }
    }
    pub fn playback_label(&self) -> &'static str {
        if self.playing { "Pause" } else { "Play" }
    }
}
#[derive(Debug, thiserror::Error)]
#[error("System tray unavailable: {0}")]
pub struct TrayUnavailable(pub String);

#[cfg(target_os = "linux")]
mod platform {
    use super::{TrayEvent, TrayState, TrayUnavailable, mpsc};
    use ksni::TrayMethods;
    pub struct Tray {
        state: TrayState,
        events: mpsc::UnboundedSender<TrayEvent>,
    }
    impl ksni::Tray for Tray {
        fn id(&self) -> String {
            "any-player".into()
        }
        fn icon_name(&self) -> String {
            "multimedia-player".into()
        }
        fn activate(&mut self, _: i32, _: i32) {
            let _ = self.events.send(TrayEvent::ToggleWindow);
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: "Any Player".into(),
                description: self.state.tooltip.clone(),
                ..Default::default()
            }
        }
        fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
            let _ = self.events.send(TrayEvent::TrayUnavailable);
            true
        }
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::{MenuItem, StandardItem};
            let item = |label: &str, event| {
                StandardItem {
                    label: label.into(),
                    activate: Box::new(move |this: &mut Self| {
                        let _ = this.events.send(event);
                    }),
                    ..Default::default()
                }
                .into()
            };
            vec![
                item(self.state.window_label(), TrayEvent::ToggleWindow),
                item(self.state.playback_label(), TrayEvent::TogglePlayback),
                item("Next", TrayEvent::Next),
                item("Previous", TrayEvent::Previous),
                MenuItem::Separator,
                item("Quit", TrayEvent::Quit),
            ]
        }
    }
    pub struct TrayHandle(ksni::Handle<Tray>);
    impl TrayHandle {
        pub async fn new(
            state: TrayState,
            events: mpsc::UnboundedSender<TrayEvent>,
        ) -> Result<Self, TrayUnavailable> {
            Tray { state, events }
                .spawn()
                .await
                .map(Self)
                .map_err(|e| TrayUnavailable(e.to_string()))
        }
        pub async fn update(&self, state: TrayState) {
            self.0.update(|tray| tray.state = state).await;
        }
        pub async fn shutdown(&self) {
            self.0.shutdown().await;
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod platform {
    use super::{TrayEvent, TrayState, TrayUnavailable, mpsc};
    use tray_icon::{
        MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
        menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    };
    pub struct TrayHandle {
        icon: TrayIcon,
        window: MenuItem,
        playback: MenuItem,
    }
    impl TrayHandle {
        pub fn new(
            state: TrayState,
            events: mpsc::UnboundedSender<TrayEvent>,
            icon: tray_icon::Icon,
        ) -> Result<Self, TrayUnavailable> {
            let menu = Menu::new();
            let window = MenuItem::new(state.window_label(), true, None);
            let playback = MenuItem::new(state.playback_label(), true, None);
            let next = MenuItem::new("Next", true, None);
            let previous = MenuItem::new("Previous", true, None);
            let quit = MenuItem::new("Quit", true, None);
            menu.append_items(&[
                &window,
                &playback,
                &next,
                &previous,
                &PredefinedMenuItem::separator(),
                &quit,
            ])
            .map_err(|e| TrayUnavailable(e.to_string()))?;
            let mappings = [
                (window.id().clone(), TrayEvent::ToggleWindow),
                (playback.id().clone(), TrayEvent::TogglePlayback),
                (next.id().clone(), TrayEvent::Next),
                (previous.id().clone(), TrayEvent::Previous),
                (quit.id().clone(), TrayEvent::Quit),
            ];
            let menu_events = events.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some((_, action)) = mappings.iter().find(|(id, _)| *id == event.id) {
                    let _ = menu_events.send(*action);
                }
            }));
            TrayIconEvent::set_event_handler(Some(move |event| {
                if matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                ) {
                    let _ = events.send(TrayEvent::ToggleWindow);
                }
            }));
            let icon = TrayIconBuilder::new()
                .with_icon(icon)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .with_tooltip(&state.tooltip)
                .build()
                .map_err(|e| TrayUnavailable(e.to_string()))?;
            Ok(Self {
                icon,
                window,
                playback,
            })
        }
        pub fn update(&self, state: TrayState) -> Result<(), TrayUnavailable> {
            self.window.set_text(state.window_label());
            self.playback.set_text(state.playback_label());
            self.icon
                .set_tooltip(Some(state.tooltip))
                .map_err(|e| TrayUnavailable(e.to_string()))
        }
    }
}
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub use platform::TrayHandle;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_follow_state() {
        let mut state = TrayState::default();
        assert_eq!(
            (state.window_label(), state.playback_label()),
            ("Show", "Play")
        );
        state.window_visible = true;
        state.playing = true;
        assert_eq!(
            (state.window_label(), state.playback_label()),
            ("Hide", "Pause")
        );
    }
}
