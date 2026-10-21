//! AI DJ break schedule reducer (Android `BreakSchedule.kt`).

use rand::Rng;

use crate::planning::{BackendKind, backend_for};
use crate::queue::QueueState;

/// Interval drawn after a reset: uniformly in `3..=5` real songs.
pub const INTERVAL_RANGE: std::ops::RangeInclusive<i32> = 3..=5;
/// Interval after a failure or a missed break.
pub const RETRY_INTERVAL: i32 = 3;

/// `songs_until_break` counts real songs between the current song and the break's target.
/// Zero means the break plays on the next advance, before the next playable song.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakSchedule {
    pub enabled: bool,
    pub songs_until_break: i32,
    pub anchor_order_pos: isize,
    /// Queue `content_version` the anchor refers to; `-1` = none.
    pub order_version: i64,
    pub failed_on_current: bool,
    pub target_uid: Option<u64>,
}

impl Default for BreakSchedule {
    fn default() -> Self {
        Self {
            enabled: false,
            songs_until_break: RETRY_INTERVAL,
            anchor_order_pos: -1,
            order_version: -1,
            failed_on_current: false,
            target_uid: None,
        }
    }
}

/// Inputs to [`reduce`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleEvent {
    Enabled(bool),
    QueueReplaced,
    CursorMoved,
    BreakStarted,
    GenerationFailed,
}

fn version(queue: &QueueState) -> i64 {
    i64::try_from(queue.content_version).unwrap_or(i64::MAX)
}

/// Real (non-filler) songs at play-order positions `from..to` (exclusive).
fn real_songs(queue: &QueueState, from: isize, to: isize) -> i32 {
    let count = (from.max(0)..to)
        .filter_map(|p| usize::try_from(p).ok())
        .filter(|&p| !queue.entry_at(p).track.is_dj_filler)
        .count();
    i32::try_from(count).unwrap_or(i32::MAX)
}

impl BreakSchedule {
    /// Real songs until the break target (UI distance); `None` when disabled.
    #[must_use]
    pub fn songs_away(&self, queue: &QueueState) -> Option<i32> {
        if !self.enabled {
            return None;
        }
        let pos = self
            .target_uid
            .and_then(|uid| queue.order_pos_of_uid(uid))
            .map_or(-1, |p| isize::try_from(p).unwrap_or(isize::MAX));
        Some(if pos > queue.cursor {
            real_songs(queue, queue.cursor + 1, pos)
        } else {
            self.songs_until_break
        })
    }
}

/// [`reduce_with`] drawing the interval from `rng` (`3..=5`).
#[must_use]
pub fn reduce<R: Rng + ?Sized>(
    s: &BreakSchedule,
    event: ScheduleEvent,
    queue: &QueueState,
    rng: &mut R,
) -> BreakSchedule {
    reduce_with(s, event, queue, || rng.random_range(INTERVAL_RANGE))
}

/// The reducer. `draw_interval` is called exactly when Android calls `random.nextInt(3, 6)`:
/// on every reset event (enable, queue replaced, break started).
#[must_use]
pub fn reduce_with(
    s: &BreakSchedule,
    event: ScheduleEvent,
    queue: &QueueState,
    draw_interval: impl FnOnce() -> i32,
) -> BreakSchedule {
    match event {
        ScheduleEvent::Enabled(false) => return BreakSchedule::default(),
        ScheduleEvent::Enabled(on) if on == s.enabled => return s.clone(),
        _ => {}
    }
    let reset = matches!(
        event,
        ScheduleEvent::Enabled(_) | ScheduleEvent::QueueReplaced | ScheduleEvent::BreakStarted
    );
    let mut next = if reset {
        BreakSchedule {
            enabled: if let ScheduleEvent::Enabled(on) = event { on } else { s.enabled },
            songs_until_break: draw_interval(),
            anchor_order_pos: queue.cursor,
            order_version: version(queue),
            failed_on_current: false,
            target_uid: s.target_uid,
        }
    } else {
        s.clone()
    };
    if !next.enabled {
        return BreakSchedule { target_uid: None, ..next };
    }
    if event == ScheduleEvent::GenerationFailed {
        next = BreakSchedule {
            songs_until_break: RETRY_INTERVAL,
            anchor_order_pos: queue.cursor,
            order_version: version(queue),
            failed_on_current: true,
            ..next
        };
    } else if !reset {
        let reanchor = next.order_version != version(queue)
            || next.anchor_order_pos < 0
            || queue.cursor < next.anchor_order_pos - 1;
        let delta = if reanchor || queue.cursor < 0 {
            0
        } else {
            real_songs(queue, next.anchor_order_pos + 1, queue.cursor + 1)
        };
        let remaining = next.songs_until_break - delta;
        next = BreakSchedule {
            songs_until_break: if remaining < 0 { RETRY_INTERVAL } else { remaining },
            anchor_order_pos: if reanchor {
                queue.cursor
            } else {
                next.anchor_order_pos.max(queue.cursor)
            },
            order_version: version(queue),
            failed_on_current: if remaining < 0 {
                true
            } else if delta > 0 {
                false
            } else {
                next.failed_on_current
            },
            ..next
        };
    }
    let mut target = None;
    if let Ok(cursor) = usize::try_from(queue.cursor) {
        let anchor = usize::try_from(next.anchor_order_pos).unwrap_or(0);
        let mut real = 0;
        for pos in anchor.max(cursor) + 1..queue.order.len() {
            let entry = queue.entry_at(pos);
            if entry.track.is_dj_filler {
                continue;
            }
            if real >= next.songs_until_break && backend_for(&entry.track) != BackendKind::Unplayable {
                target = Some(entry.uid);
                break;
            }
            real += 1;
        }
    }
    BreakSchedule { target_uid: target, ..next }
}
