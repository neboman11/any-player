//! Title|artist dedupe shared with Android and the cross-platform `dedup_spec.json`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::models::{PlaylistTrack, Track};
use crate::text::kotlin_trim;

/// A single duplicate occurrence within the original input list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateOccurrence {
    /// Zero-based index in the original input list.
    pub index: usize,
    pub track_id: String,
}

/// Tracks sharing one key; `occurrences` excludes the first (kept) occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    /// `"{lowercase_trimmed_title}|{lowercase_trimmed_artist}"`.
    pub key: String,
    pub first_occurrence_index: usize,
    pub occurrences: Vec<DuplicateOccurrence>,
}

/// Kept items (first occurrences, input order) and duplicate groups ordered by first occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deduplicated<T> {
    pub items: Vec<T>,
    pub duplicate_groups: Vec<DuplicateGroup>,
}

/// The dedupe key: Kotlin-trimmed, lowercased title and artist joined with `|`.
#[must_use]
pub fn key(title: &str, artist: &str) -> String {
    format!("{}|{}", kotlin_trim(title).to_lowercase(), kotlin_trim(artist).to_lowercase())
}

/// Dedupes playlist rows by title|artist.
#[must_use]
pub fn deduplicate(tracks: Vec<PlaylistTrack>) -> Deduplicated<PlaylistTrack> {
    dedupe_by(tracks, |t| key(&t.title, &t.artist), |t| t.track_id.clone())
}

/// Dedupes tracks by title|artist.
#[must_use]
pub fn deduplicate_tracks(tracks: Vec<Track>) -> Deduplicated<Track> {
    dedupe_by(tracks, |t| key(&t.title, &t.artist), |t| t.id.clone())
}

/// Generic first-occurrence-wins dedupe.
pub fn dedupe_by<T>(
    items: Vec<T>,
    mut key_of: impl FnMut(&T) -> String,
    mut id_of: impl FnMut(&T) -> String,
) -> Deduplicated<T> {
    // (key, first index) in first-occurrence order, plus key → group slot.
    let mut groups: Vec<DuplicateGroup> = Vec::new();
    let mut slot_of: HashMap<String, usize> = HashMap::new();
    let mut kept = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        let key = key_of(&item);
        if let Some(&slot) = slot_of.get(&key) {
            groups[slot].occurrences.push(DuplicateOccurrence { index, track_id: id_of(&item) });
        } else {
            slot_of.insert(key.clone(), groups.len());
            groups.push(DuplicateGroup { key, first_occurrence_index: index, occurrences: Vec::new() });
            kept.push(item);
        }
    }
    groups.retain(|g| !g.occurrences.is_empty());
    Deduplicated { items: kept, duplicate_groups: groups }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enums::SourceType;

    #[test]
    fn key_is_trimmed_lowercase_title_pipe_artist() {
        assert_eq!(key("  Hello World\t", " THE BAND "), "hello world|the band");
        assert_eq!(key("Instrumental", ""), "instrumental|");
    }

    #[test]
    fn groups_are_ordered_by_first_occurrence() {
        let t = |id: &str, title: &str, artist: &str| Track::new(id, title, artist, SourceType::Jellyfin);
        let result = deduplicate_tracks(vec![
            t("1", "Song A", "X"),
            t("2", "Song B", "Y"),
            t("3", "Song B", "Y"),
            t("4", "Song A", "X"),
        ]);
        assert_eq!(result.items.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["1", "2"]);
        assert_eq!(
            result.duplicate_groups.iter().map(|g| g.key.as_str()).collect::<Vec<_>>(),
            ["song a|x", "song b|y"]
        );
    }
}
