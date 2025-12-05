/// Reusable UI components
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph};

pub struct PlaybackBar {
    pub current_time: u64,
    pub total_time: u64,
}

impl PlaybackBar {
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let progress = if self.total_time > 0 {
            (self.current_time as f64 / self.total_time as f64 * 100.0) as u16
        } else {
            0
        };

        let gauge = Gauge::default()
            .block(Block::default().title("Progress").borders(Borders::ALL))
            .gauge_style(Style::default().fg(Color::Green))
            .percent(progress as u16);

        f.render_widget(gauge, area);
    }
}

pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub source: String,
}

impl TrackInfo {
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let text = vec![
            Line::from(format!("Title: {}", self.title)),
            Line::from(format!("Artist: {}", self.artist)),
            Line::from(format!("Album: {}", self.album)),
            Line::from(format!("Source: {}", self.source)),
        ];

        let paragraph =
            Paragraph::new(text).block(Block::default().title("Track Info").borders(Borders::ALL));

        f.render_widget(paragraph, area);
    }
}

pub struct PlaybackControls {
    pub playing: bool,
    pub shuffle: bool,
    pub repeat_mode: String,
}

impl PlaybackControls {
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let play_status = if self.playing {
            "⏸ Pause"
        } else {
            "▶ Play"
        };
        let shuffle_status = if self.shuffle {
            "🔀 Shuffle ON"
        } else {
            "🔀 Shuffle OFF"
        };

        let text = vec![
            Line::from(format!(
                "{} | {} | Repeat: {}",
                play_status, shuffle_status, self.repeat_mode
            )),
            Line::from(""),
            Line::from("Space: Play/Pause | N: Next | P: Previous | S: Shuffle | R: Repeat"),
        ];

        let paragraph = Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::TOP));

        f.render_widget(paragraph, area);
    }
}

pub struct SourceSelector {
    pub options: Vec<String>,
    pub selected: usize,
}

impl SourceSelector {
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let list_items: Vec<ListItem> = self
            .options
            .iter()
            .enumerate()
            .map(|(i, opt)| {
                let content = if i == self.selected {
                    format!("➤ {}", opt)
                } else {
                    format!("  {}", opt)
                };
                ListItem::new(content)
            })
            .collect();

        let list = List::new(list_items)
            .block(
                Block::default()
                    .title("Select Source")
                    .borders(Borders::ALL),
            )
            .style(Style::default());

        f.render_widget(list, area);
    }
}
