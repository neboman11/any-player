#![allow(dead_code)]

use std::path::PathBuf;

use ap_model::{QueueState, SourceType, Track};
use rand::SeedableRng;
use rand::rngs::StdRng;

pub fn rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

/// Jellyfin track with a playable http url.
pub fn jf(id: &str) -> Track {
    Track { url: Some(format!("http://host/{id}.flac")), ..Track::new(id, format!("Song {id}"), "Artist", SourceType::Jellyfin) }
}

pub fn jf_titled(id: &str, title: &str, artist: &str) -> Track {
    Track { url: Some(format!("http://host/{id}.flac")), ..Track::new(id, title, artist, SourceType::Jellyfin) }
}

/// Spotify track with a valid 22-char id derived from `name`.
pub fn sp(name: &str) -> Track {
    Track::new(format!("{name:x<22}"), format!("Spotify {name}"), "Artist", SourceType::Spotify)
}

/// A Plex track without url (unplayable).
pub fn dead(id: &str) -> Track {
    Track::new(id, format!("No url {id}"), "Artist", SourceType::Plex)
}

/// Short label: Spotify ids lose their padding.
pub fn label(t: &Track) -> String {
    if t.source == SourceType::Spotify { t.id.trim_end_matches('x').to_owned() } else { t.id.clone() }
}

pub fn play_order_ids(s: &QueueState) -> Vec<String> {
    s.play_order().map(|t| t.id.clone()).collect()
}

pub fn ids<'a>(entries: impl Iterator<Item = &'a ap_model::QueueEntry>) -> Vec<String> {
    entries.map(|e| e.track.id.clone()).collect()
}

pub fn fixture(name: &str) -> String {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "..", "fixtures", name].iter().collect();
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("fixture {}: {e}", path.display()))
}

pub fn fixture_json(name: &str) -> serde_json::Value {
    serde_json::from_str(&fixture(name)).expect("fixture is JSON")
}
