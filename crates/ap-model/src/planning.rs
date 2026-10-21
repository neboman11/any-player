//! Run planning (Android `RunPlanning.kt`). Android's `EXO` backend is the desktop `Local` one.

use crate::enums::SourceType;
use crate::ids::is_valid_spotify_track_id;
use crate::models::Track;
use crate::queue::QueueState;
use crate::text::kotlin_trim;

/// The backend that can play a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// Local decoder (Android `EXO`): http, https or file URLs.
    Local,
    Spotify,
    Unplayable,
}

const LOCAL_SCHEMES: [&str; 3] = ["http", "https", "file"];

/// SPOTIFY source with a valid 22-char track id → `Spotify`; an http/https/file url → `Local`;
/// anything else → `Unplayable`.
///
/// Android also accepts `content` and `android.resource` URLs; those cannot occur on desktop.
#[must_use]
pub fn backend_for(track: &Track) -> BackendKind {
    if track.source == SourceType::Spotify {
        return if is_valid_spotify_track_id(&track.id) {
            BackendKind::Spotify
        } else {
            BackendKind::Unplayable
        };
    }
    let scheme = track
        .url
        .as_deref()
        .and_then(|url| kotlin_trim(url).split_once(':'))
        .map(|(scheme, _)| scheme.to_lowercase());
    match scheme {
        Some(s) if LOCAL_SCHEMES.contains(&s.as_str()) => BackendKind::Local,
        _ => BackendKind::Unplayable,
    }
}

/// A DJ break that must play immediately before the entry `before_uid`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingBreak {
    pub before_uid: u64,
    pub break_id: String,
    pub path: String,
}

/// One item of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanItem {
    Song { uid: u64, order_pos: usize },
    Break { break_id: String, path: String },
}

/// A play-order slice owned by one backend: `kind` is `Local` or `Spotify`, never
/// `Unplayable`. Break items only appear in `Local` runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub kind: BackendKind,
    pub items: Vec<PlanItem>,
}

/// Default cap on planned items per run.
pub const DEFAULT_MAX_RUN_ITEMS: usize = 50;

/// The single-item `Local` run that plays `pending` at a Spotify run boundary.
#[must_use]
pub fn break_run(pending: &PendingBreak) -> Run {
    Run {
        kind: BackendKind::Local,
        items: vec![PlanItem::Break { break_id: pending.break_id.clone(), path: pending.path.clone() }],
    }
}

/// Plans the run starting at `from_order_pos`, without wrapping, at most `max_items` items.
///
/// Unplayable entries are skipped (also before the first item) and never end a run; the run's
/// kind is that of its first playable entry and it ends at the first playable entry of another
/// kind. Returns `None` when no playable entry exists at or after `from_order_pos`.
///
/// With a `pending_break`, a `Local` run splices the break right before `before_uid`; a Spotify
/// run stops before `before_uid` unless it is the run's first item (the engine plays
/// [`break_run`] before starting a run at the break target).
///
/// # Panics
/// When `max_items` is zero.
#[must_use]
pub fn plan_run(
    state: &QueueState,
    from_order_pos: usize,
    pending_break: Option<&PendingBreak>,
    max_items: usize,
) -> Option<Run> {
    assert!(max_items > 0, "max_items must be positive");
    let mut kind: Option<BackendKind> = None;
    let mut items = Vec::new();
    let mut pos = from_order_pos;
    while pos < state.order.len() && items.len() < max_items {
        let entry = state.entry_at(pos);
        let entry_kind = backend_for(&entry.track);
        if entry_kind == BackendKind::Unplayable {
            pos += 1;
            continue;
        }
        match kind {
            None => kind = Some(entry_kind),
            Some(k) if k != entry_kind => break,
            Some(_) => {}
        }
        if let Some(pending) = pending_break.filter(|p| p.before_uid == entry.uid) {
            if kind == Some(BackendKind::Local) {
                items.push(PlanItem::Break {
                    break_id: pending.break_id.clone(),
                    path: pending.path.clone(),
                });
                if items.len() >= max_items {
                    break;
                }
            } else if !items.is_empty() {
                break;
            }
        }
        items.push(PlanItem::Song { uid: entry.uid, order_pos: pos });
        pos += 1;
    }
    kind.map(|kind| Run { kind, items })
}
