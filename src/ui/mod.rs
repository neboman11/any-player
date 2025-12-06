
pub mod pages;
pub mod components;
pub mod theme;

/// UI state for the application
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppPage {
    Home,
    SearchPlaylists,
    ViewPlaylist,
    NowPlaying,
    Queue,
    Settings,
}

/// Application UI state
pub struct AppState {
    pub current_page: AppPage,
    pub exit: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            current_page: AppPage::Home,
            exit: false,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
