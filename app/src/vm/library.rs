//! Library page view model. Snapshot updates are consumed by the main-thread presenter.
use super::{LibraryCommand, LibraryPort, LibrarySnapshot, Result};
use std::sync::Arc;
use tokio::sync::watch;

pub struct LibraryVm {
    pub port: Arc<dyn LibraryPort>,
    pub state: watch::Receiver<LibrarySnapshot>,
}
impl LibraryVm {
    pub fn new(port: Arc<dyn LibraryPort>) -> Self {
        let state = port.subscribe();
        Self { port, state }
    }
    pub async fn dispatch(&self, command: LibraryCommand) -> Result {
        self.port.library(command).await
    }
}

/// Select an occurrence, mapping a duplicate to its retained keeper under distinct playback.
pub fn playback_selection(
    rows: &[super::TrackOccurrence],
    key: &str,
    distinct: bool,
) -> Option<(Vec<ap_model::Track>, usize)> {
    let selected = rows.iter().position(|row| row.key == key)?;
    let selected_key =
        ap_model::dedupe::key(&rows[selected].track.title, &rows[selected].track.artist);
    let mut tracks = Vec::with_capacity(rows.len());
    let mut seen = std::collections::HashSet::new();
    let mut start = selected;
    for row in rows {
        if distinct {
            let key = ap_model::dedupe::key(&row.track.title, &row.track.artist);
            let is_selected = key == selected_key;
            if !seen.insert(key) {
                continue;
            }
            if is_selected {
                start = tracks.len();
            }
        }
        tracks.push(row.track.clone());
    }
    Some((tracks, start))
}
