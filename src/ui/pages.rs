/// UI page components
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Padding, Paragraph};

pub struct HomePage {}

impl HomePage {
    pub fn render(&self, f: &mut Frame, area: Rect) {
        // Render home page with provider selection
        let block = Block::default()
            .title("Any Player - Multi-Source Music Client")
            .borders(Borders::ALL);
        f.render_widget(block, area);

        let text = vec![
            Line::from("Select a source:"),
            Line::from("  1) Spotify"),
            Line::from("  2) Jellyfin"),
            Line::from("  3) Both"),
            Line::from(""),
            Line::from("Commands:"),
            Line::from("  q) Quit"),
            Line::from("  /) Search"),
            Line::from("  p) Playlists"),
        ];

        let paragraph = Paragraph::new(text)
            .alignment(Alignment::Left)
            .block(Block::default().padding(Padding::uniform(1)));

        f.render_widget(paragraph, area);
    }
}

pub struct SearchPage {}

impl SearchPage {
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title("Search Playlists & Tracks")
            .borders(Borders::ALL);
        f.render_widget(block, area);
    }
}

pub struct PlaylistPage {}

impl PlaylistPage {
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let block = Block::default().title("Playlists").borders(Borders::ALL);
        f.render_widget(block, area);
    }
}

pub struct NowPlayingPage {}

impl NowPlayingPage {
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let block = Block::default().title("Now Playing").borders(Borders::ALL);
        f.render_widget(block, area);
    }
}

pub struct QueuePage {}

impl QueuePage {
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let block = Block::default().title("Queue").borders(Borders::ALL);
        f.render_widget(block, area);
    }
}
