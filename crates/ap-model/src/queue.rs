//! The app-owned queue (Android `Queue.kt`).

use std::sync::Arc;

use rand::Rng;
use rand::seq::SliceRandom;

use crate::enums::RepeatMode;
use crate::models::Track;
use crate::planning::{BackendKind, backend_for};

/// One queue slot. `uid` is unique within the queue even when the same track appears twice.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QueueEntry {
    pub uid: u64,
    pub track: Track,
}

/// The queue for every source mix.
///
/// `entries` is insertion order, `order` holds indices into `entries` in play order and `cursor`
/// is a position in `order` (`-1` = none). With shuffle off `order` is the identity permutation.
/// `content_version` bumps only when `entries` or `order` change; cursor moves never bump it.
/// Lists are shared (`Arc`) so cursor-only transitions are cheap to clone and publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueState {
    pub entries: Arc<[QueueEntry]>,
    pub order: Arc<[usize]>,
    pub cursor: isize,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub play_next_offset: usize,
    pub content_version: u64,
    pub next_uid: u64,
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            entries: Arc::new([]),
            order: Arc::new([]),
            cursor: -1,
            shuffle: false,
            repeat: RepeatMode::Off,
            play_next_offset: 0,
            content_version: 0,
            next_uid: 1,
        }
    }
}

impl QueueState {
    /// The cursor as an order position, when it points into `order`.
    #[must_use]
    pub fn cursor_pos(&self) -> Option<usize> {
        usize::try_from(self.cursor).ok().filter(|&c| c < self.order.len())
    }

    #[must_use]
    pub fn current(&self) -> Option<&QueueEntry> {
        self.cursor_pos().map(|c| self.entry_at(c))
    }

    /// The entry at play-order position `order_pos`. Panics when out of range.
    #[must_use]
    pub fn entry_at(&self, order_pos: usize) -> &QueueEntry {
        &self.entries[self.order[order_pos]]
    }

    /// Entries after the cursor in play order (everything when there is no cursor).
    pub fn upcoming(&self) -> impl Iterator<Item = &QueueEntry> {
        let skip = usize::try_from(self.cursor).map_or(0, |c| c + 1);
        self.order.iter().skip(skip).map(|&i| &self.entries[i])
    }

    /// Entries before the cursor in play order.
    pub fn history(&self) -> impl Iterator<Item = &QueueEntry> {
        let take = usize::try_from(self.cursor).unwrap_or(0);
        self.order.iter().take(take).map(|&i| &self.entries[i])
    }

    #[must_use]
    pub fn order_pos_of_uid(&self, uid: u64) -> Option<usize> {
        self.order.iter().position(|&i| self.entries[i].uid == uid)
    }

    /// Track ids in play order (test and debug helper).
    pub fn play_order(&self) -> impl Iterator<Item = &Track> {
        self.order.iter().map(|&i| &self.entries[i].track)
    }
}

/// Engine reaction to "previous" (Android `DefaultPlaybackEngine.previous`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Previous {
    /// Restart the current item.
    SeekToStart,
    /// Move the cursor to this play-order position.
    MoveTo(usize),
}

/// Positions beyond this restart the current item instead of moving back.
pub const PREVIOUS_RESTART_THRESHOLD_MS: i64 = 3_000;

fn to_isize(v: usize) -> isize {
    isize::try_from(v).expect("queue length fits isize")
}

fn shuffled<R: Rng + ?Sized>(mut items: Vec<usize>, rng: &mut R) -> Vec<usize> {
    items.shuffle(rng);
    items
}

/// Pure queue transitions. Every function returns a new state; `None` means "no move".
pub struct QueueOps;

impl QueueOps {
    /// Replaces the queue with `tracks`, current = `tracks[start_index]` (clamped). With shuffle
    /// on the current entry is placed first and the rest shuffled. `random_start` (user-initiated
    /// play with `start_index == 0`) picks a random start when shuffle is on and there is more
    /// than one track; restore/sync paths keep the given start.
    #[must_use]
    pub fn replace<R: Rng + ?Sized>(
        s: &QueueState,
        tracks: Vec<Track>,
        start_index: isize,
        rng: &mut R,
        random_start: bool,
    ) -> QueueState {
        let len = tracks.len();
        let next_uid = s.next_uid;
        let entries: Arc<[QueueEntry]> = tracks
            .into_iter()
            .zip(next_uid..)
            .map(|(track, uid)| QueueEntry { uid, track })
            .collect();
        let base = QueueState {
            entries,
            play_next_offset: 0,
            content_version: s.content_version + 1,
            next_uid: next_uid + len as u64,
            ..s.clone()
        };
        if len == 0 {
            return QueueState { order: Arc::new([]), cursor: -1, ..base };
        }
        let start = if random_start && s.shuffle && start_index == 0 && len > 1 {
            rng.random_range(0..len)
        } else {
            usize::try_from(start_index.clamp(0, to_isize(len - 1))).unwrap_or(0)
        };
        if s.shuffle {
            let rest = shuffled((0..len).filter(|&i| i != start).collect(), rng);
            let order: Arc<[usize]> = std::iter::once(start).chain(rest).collect();
            QueueState { order, cursor: 0, ..base }
        } else {
            QueueState { order: (0..len).collect(), cursor: to_isize(start), ..base }
        }
    }

    /// Inserts `track` after the current entry + `play_next_offset` in insertion order and after
    /// the cursor + offset in play order, so consecutive calls stack in call order. An empty
    /// queue becomes a single current entry.
    #[must_use]
    pub fn play_next(s: &QueueState, track: Track) -> QueueState {
        let entry = QueueEntry { uid: s.next_uid, track };
        if s.entries.is_empty() {
            return QueueState {
                entries: Arc::new([entry]),
                order: Arc::new([0]),
                cursor: 0,
                play_next_offset: 0,
                content_version: s.content_version + 1,
                next_uid: s.next_uid + 1,
                ..s.clone()
            };
        }
        let offset = to_isize(s.play_next_offset);
        let order_insert = usize::try_from((s.cursor + 1 + offset).clamp(0, to_isize(s.order.len())))
            .unwrap_or(0);
        let current_entry_index = s.cursor_pos().map_or(-1, |c| to_isize(s.order[c]));
        let entry_insert =
            usize::try_from((current_entry_index + 1 + offset).clamp(0, to_isize(s.entries.len())))
                .unwrap_or(0);
        let mut entries = s.entries.to_vec();
        entries.insert(entry_insert, entry);
        let mut order: Vec<usize> =
            s.order.iter().map(|&i| if i >= entry_insert { i + 1 } else { i }).collect();
        order.insert(order_insert, entry_insert);
        QueueState {
            entries: entries.into(),
            order: order.into(),
            play_next_offset: s.play_next_offset + 1,
            content_version: s.content_version + 1,
            next_uid: s.next_uid + 1,
            ..s.clone()
        }
    }

    /// Moves to the next play-order position; repeat ALL wraps when `wrap`. `None` at the end.
    #[must_use]
    pub fn advance(s: &QueueState, wrap: bool) -> Option<QueueState> {
        if s.order.is_empty() {
            return None;
        }
        let next = if s.cursor + 1 < to_isize(s.order.len()) {
            s.cursor + 1
        } else if wrap && s.repeat == RepeatMode::All {
            0
        } else {
            return None;
        };
        Some(Self::move_cursor(s, next))
    }

    /// Moves to the previous play-order position; `None` at the start.
    #[must_use]
    pub fn back(s: &QueueState) -> Option<QueueState> {
        (s.cursor > 0).then(|| Self::move_cursor(s, s.cursor - 1))
    }

    /// Makes the entry with `uid` current; an unknown uid leaves the state unchanged.
    #[must_use]
    pub fn jump_to_uid(s: &QueueState, uid: u64) -> QueueState {
        match s.order_pos_of_uid(uid) {
            Some(pos) => Self::move_cursor(s, to_isize(pos)),
            None => s.clone(),
        }
    }

    /// On: the current entry stays current and keeps its play-order position; history before the
    /// cursor is kept and every other entry is shuffled after it.
    /// Off: order becomes the identity and the cursor follows the current entry.
    #[must_use]
    pub fn set_shuffle<R: Rng + ?Sized>(s: &QueueState, on: bool, rng: &mut R) -> QueueState {
        if on == s.shuffle {
            return s.clone();
        }
        let current_entry = s.cursor_pos().map(|c| s.order[c]);
        let order: Vec<usize> = if on {
            match (current_entry, s.cursor_pos()) {
                (Some(current), Some(c)) => {
                    let mut order = s.order[..c].to_vec();
                    order.push(current);
                    order.extend(shuffled(s.order[c + 1..].to_vec(), rng));
                    order
                }
                _ => shuffled((0..s.entries.len()).collect(), rng),
            }
        } else {
            (0..s.entries.len()).collect()
        };
        let cursor = current_entry
            .and_then(|e| order.iter().position(|&i| i == e))
            .map_or(-1, to_isize);
        QueueState {
            shuffle: on,
            order: order.into(),
            cursor,
            play_next_offset: 0,
            content_version: s.content_version + 1,
            ..s.clone()
        }
    }

    #[must_use]
    pub fn set_repeat(s: &QueueState, mode: RepeatMode) -> QueueState {
        QueueState { repeat: mode, ..s.clone() }
    }

    /// Android engine `previous()` for the queue: restart past 3 s, otherwise move to the closest
    /// earlier playable entry, or restart when there is none.
    #[must_use]
    pub fn previous(s: &QueueState, position_ms: i64) -> Previous {
        if position_ms > PREVIOUS_RESTART_THRESHOLD_MS {
            return Previous::SeekToStart;
        }
        let Some(cursor) = s.cursor_pos() else { return Previous::SeekToStart };
        (0..cursor)
            .rev()
            .find(|&pos| backend_for(&s.entry_at(pos).track) != BackendKind::Unplayable)
            .map_or(Previous::SeekToStart, Previous::MoveTo)
    }

    /// Cursor moves reset the "play next" stacking and never bump `content_version`.
    #[must_use]
    pub fn move_cursor(s: &QueueState, pos: isize) -> QueueState {
        QueueState { cursor: pos, play_next_offset: 0, ..s.clone() }
    }
}
