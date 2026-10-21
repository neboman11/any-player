//! Port of Android `QueueOpsTest`.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use ap_model::queue::PREVIOUS_RESTART_THRESHOLD_MS;
use ap_model::{Previous, QueueEntry, QueueOps, QueueState, RepeatMode, Track};
use common::{dead, ids, jf, play_order_ids, rng};

fn tracks() -> Vec<Track> {
    ["a", "b", "c", "d", "e"].into_iter().map(jf).collect()
}

fn queue_with(start: isize, shuffle: bool, seed: u64) -> QueueState {
    QueueOps::replace(&QueueState { shuffle, ..QueueState::default() }, tracks(), start, &mut rng(seed), false)
}

fn queue(start: isize) -> QueueState {
    queue_with(start, false, 1)
}

fn current_id(s: &QueueState) -> Option<&str> {
    s.current().map(|e| e.track.id.as_str())
}

fn set(order: &[usize]) -> BTreeSet<usize> {
    order.iter().copied().collect()
}

#[test]
fn replace_without_shuffle_keeps_insertion_order_and_start_index() {
    let s = queue(2);
    assert_eq!(&*s.order, &[0, 1, 2, 3, 4]);
    assert_eq!(s.cursor, 2);
    assert_eq!(current_id(&s), Some("c"));
    assert_eq!(s.entries.iter().map(|e| e.uid).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    assert_eq!(s.next_uid, 6);
    assert_eq!(s.content_version, 1);
}

#[test]
fn replace_empty_tracks_clears_queue() {
    let s = QueueOps::replace(&queue(0), vec![], 0, &mut rng(1), false);
    assert!(s.entries.is_empty());
    assert_eq!(s.cursor, -1);
    assert!(s.current().is_none());
}

#[test]
fn replace_start_index_is_clamped() {
    assert_eq!(QueueOps::replace(&QueueState::default(), tracks(), 99, &mut rng(1), false).cursor, 4);
    assert_eq!(QueueOps::replace(&QueueState::default(), tracks(), -3, &mut rng(1), false).cursor, 0);
}

#[test]
fn replace_uids_keep_increasing_across_replacements() {
    let s = QueueOps::replace(&queue(0), tracks()[..2].to_vec(), 0, &mut rng(1), false);
    assert_eq!(s.entries.iter().map(|e| e.uid).collect::<Vec<_>>(), [6, 7]);
}

#[test]
fn replace_with_shuffle_and_explicit_start_puts_start_first_and_shuffles_rest() {
    let s = queue_with(3, true, 1);
    assert_eq!(s.cursor, 0);
    assert_eq!(current_id(&s), Some("d"));
    assert_eq!(set(&s.order), (0..5).collect());
    assert_eq!(s.order.len(), 5);
}

#[test]
fn replace_with_shuffle_single_track_starts_at_it() {
    let s = QueueOps::replace(&QueueState { shuffle: true, ..QueueState::default() }, vec![jf("x")], 0, &mut rng(3), false);
    assert_eq!(&*s.order, &[0]);
    assert_eq!(s.cursor, 0);
}

#[test]
fn replace_resets_play_next_offset() {
    let stacked = QueueOps::play_next(&queue(0), jf("n"));
    assert_eq!(stacked.play_next_offset, 1);
    assert_eq!(QueueOps::replace(&stacked, tracks(), 0, &mut rng(1), false).play_next_offset, 0);
}

#[test]
fn play_next_stacks_in_call_order_after_current() {
    let mut s = queue(1);
    for id in ["x", "y", "z"] {
        s = QueueOps::play_next(&s, jf(id));
    }
    assert_eq!(play_order_ids(&s), ["a", "b", "x", "y", "z", "c", "d", "e"]);
    assert_eq!(s.play_next_offset, 3);
    assert_eq!(current_id(&s), Some("b"));
    assert_eq!(s.order.to_vec(), (0..s.entries.len()).collect::<Vec<_>>());
}

#[test]
fn play_next_offset_resets_when_cursor_moves() {
    let mut s = QueueOps::play_next(&queue(0), jf("x"));
    s = QueueOps::advance(&s, true).unwrap();
    assert_eq!(current_id(&s), Some("x"));
    assert_eq!(s.play_next_offset, 0);
    s = QueueOps::play_next(&s, jf("y"));
    assert_eq!(play_order_ids(&s), ["a", "x", "y", "b", "c", "d", "e"]);
}

#[test]
fn play_next_on_empty_queue_creates_single_current_entry() {
    let s = QueueOps::play_next(&QueueState::default(), jf("x"));
    assert_eq!(play_order_ids(&s), ["x"]);
    assert_eq!(s.cursor, 0);
    assert_eq!(s.play_next_offset, 0);
    assert_eq!(s.entries.len(), 1);
    assert_eq!(s.entries[0].uid, 1);
}

#[test]
fn play_next_under_shuffle_inserts_after_current_in_play_order() {
    let mut s = queue_with(2, true, 7);
    let before = play_order_ids(&s);
    s = QueueOps::play_next(&s, jf("x"));
    s = QueueOps::play_next(&s, jf("y"));
    let mut expected = vec![before[0].clone(), "x".into(), "y".into()];
    expected.extend(before[1..].iter().cloned());
    assert_eq!(play_order_ids(&s), expected);
    assert_eq!(set(&s.order), (0..s.entries.len()).collect());
    // Unshuffling keeps the inserted entries right after the current entry.
    let off = QueueOps::set_shuffle(&s, false, &mut rng(1));
    assert_eq!(play_order_ids(&off), ["a", "b", "c", "x", "y", "d", "e"]);
    assert_eq!(current_id(&off), Some("c"));
}

#[test]
fn advance_moves_forward_and_returns_none_at_end_without_repeat() {
    let s = QueueOps::advance(&queue(3), true).unwrap();
    assert_eq!(current_id(&s), Some("e"));
    assert!(QueueOps::advance(&s, true).is_none());
}

#[test]
fn advance_repeat_all_wraps() {
    let s = QueueOps::set_repeat(&queue(4), RepeatMode::All);
    let wrapped = QueueOps::advance(&s, true).unwrap();
    assert_eq!(wrapped.cursor, 0);
    assert_eq!(current_id(&wrapped), Some("a"));
    assert!(QueueOps::advance(&s, false).is_none());
}

#[test]
fn advance_repeat_one_does_not_wrap() {
    let s = QueueOps::set_repeat(&queue(4), RepeatMode::One);
    assert!(QueueOps::advance(&s, true).is_none());
}

#[test]
fn advance_on_empty_queue_is_none() {
    assert!(QueueOps::advance(&QueueState::default(), true).is_none());
}

#[test]
fn back_moves_back_and_returns_none_at_start_even_with_repeat_all() {
    assert_eq!(current_id(&QueueOps::back(&queue(1)).unwrap()), Some("a"));
    let at_start = QueueOps::set_repeat(&queue(0), RepeatMode::All);
    assert!(QueueOps::back(&at_start).is_none());
}

#[test]
fn advance_and_back_follow_shuffled_order() {
    let s = queue_with(0, true, 11);
    let next = QueueOps::advance(&s, true).unwrap();
    assert_eq!(next.current(), Some(s.entry_at(1)));
    assert_eq!(QueueOps::back(&next).unwrap().current(), Some(s.entry_at(0)));
}

#[test]
fn set_shuffle_on_keeps_current_at_cursor_and_history() {
    let s = queue(2);
    let on = QueueOps::set_shuffle(&s, true, &mut rng(5));
    assert!(on.shuffle);
    assert_eq!(on.cursor, 2);
    assert_eq!(current_id(&on), Some("c"));
    assert_eq!(s.current(), on.current());
    assert_eq!(ids(on.history()), ["a", "b"]);
    assert_eq!(ids(on.upcoming()).into_iter().collect::<BTreeSet<_>>(), ["d".to_owned(), "e".to_owned()].into());
    assert_eq!(set(&on.order), (0..5).collect());
}

#[test]
fn set_shuffle_off_restores_identity_and_follows_current() {
    let on = queue_with(3, true, 9);
    let off = QueueOps::set_shuffle(&on, false, &mut rng(1));
    assert_eq!(&*off.order, &[0, 1, 2, 3, 4]);
    assert_eq!(off.cursor, 3);
    assert_eq!(current_id(&off), Some("d"));
}

#[test]
fn set_shuffle_unchanged_returns_same_state() {
    let s = queue(0);
    let same = QueueOps::set_shuffle(&s, false, &mut rng(1));
    assert_eq!(same, s);
    assert!(Arc::ptr_eq(&same.order, &s.order));
}

#[test]
fn set_shuffle_without_current_shuffles_everything() {
    let entries: Arc<[QueueEntry]> =
        tracks().into_iter().zip(1..).map(|(track, uid)| QueueEntry { uid, track }).collect();
    let s = QueueState { entries, order: (0..5).collect(), ..QueueState::default() };
    let on = QueueOps::set_shuffle(&s, true, &mut rng(2));
    assert_eq!(on.cursor, -1);
    assert_eq!(set(&on.order), (0..5).collect());
}

#[test]
fn jump_with_duplicate_track_ids_is_distinct_by_uid() {
    let s = QueueOps::replace(&QueueState::default(), vec![jf("a"), jf("b"), jf("a"), jf("a")], 0, &mut rng(1), false);
    let (third, fourth) = (s.entries[2].clone(), s.entries[3].clone());
    assert_eq!(third.track, fourth.track);
    assert_ne!(third.uid, fourth.uid);

    let j3 = QueueOps::jump_to_uid(&s, third.uid);
    assert_eq!(j3.cursor, 2);
    assert_eq!(j3.current().map(|e| e.uid), Some(third.uid));
    let j4 = QueueOps::jump_to_uid(&s, fourth.uid);
    assert_eq!(j4.cursor, 3);
    assert_eq!(j4.current().map(|e| e.uid), Some(fourth.uid));
    assert_eq!(ids(j3.upcoming()), ["a"]);
    assert_eq!(j3.upcoming().map(|e| e.uid).collect::<Vec<_>>(), [fourth.uid]);
}

#[test]
fn upcoming_and_history_follow_cursor() {
    let s = queue(2);
    assert_eq!(ids(s.history()), ["a", "b"]);
    assert_eq!(ids(s.upcoming()), ["d", "e"]);
    let none = QueueState { cursor: -1, ..s };
    assert_eq!(ids(none.upcoming()).len(), 5);
    assert_eq!(none.history().count(), 0);
}

#[test]
fn replace_random_start_picks_random_start_only_when_requested() {
    let shuffled = QueueState { shuffle: true, ..QueueState::default() };
    let starts: BTreeSet<String> = (1..=40)
        .map(|seed| current_id(&QueueOps::replace(&shuffled, tracks(), 0, &mut rng(seed), true)).unwrap().to_owned())
        .collect();
    assert!(starts.len() > 1, "random start should vary across seeds, got {starts:?}");
    for seed in 1..=40 {
        let kept = QueueOps::replace(&shuffled, tracks(), 0, &mut rng(seed), false);
        assert_eq!(current_id(&kept), Some("a"), "restore/sync keep the given start");
    }
    // Without shuffle random_start is ignored.
    assert_eq!(current_id(&QueueOps::replace(&QueueState::default(), tracks(), 0, &mut rng(3), true)), Some("a"));
    // An explicit non-zero start is never randomized.
    assert_eq!(current_id(&QueueOps::replace(&shuffled, tracks(), 2, &mut rng(3), true)), Some("c"));
}

#[test]
fn play_next_offset_resets_on_back_and_jump() {
    let stacked = QueueOps::play_next(&queue(2), jf("x"));
    assert_eq!(QueueOps::back(&stacked).unwrap().play_next_offset, 0);
    assert_eq!(QueueOps::jump_to_uid(&stacked, stacked.entries[4].uid).play_next_offset, 0);
}

#[test]
fn jump_to_unknown_uid_is_no_op() {
    let s = queue(0);
    assert_eq!(QueueOps::jump_to_uid(&s, 999), s);
}

#[test]
fn cursor_moves_do_not_bump_content_version() {
    let s = queue(1);
    let v = s.content_version;
    assert_eq!(QueueOps::advance(&s, true).unwrap().content_version, v);
    assert_eq!(QueueOps::back(&s).unwrap().content_version, v);
    assert_eq!(QueueOps::jump_to_uid(&s, s.entries[4].uid).content_version, v);
    assert_eq!(QueueOps::set_repeat(&s, RepeatMode::All).content_version, v);
    let advanced = QueueOps::advance(&s, true).unwrap();
    assert!(Arc::ptr_eq(&advanced.entries, &s.entries));
    assert!(Arc::ptr_eq(&advanced.order, &s.order));
}

#[test]
fn content_changes_bump_content_version() {
    let s = queue(1);
    let v = s.content_version;
    assert_eq!(QueueOps::play_next(&s, jf("x")).content_version, v + 1);
    assert_eq!(QueueOps::set_shuffle(&s, true, &mut rng(1)).content_version, v + 1);
    assert_eq!(QueueOps::replace(&s, tracks(), 0, &mut rng(1), false).content_version, v + 1);
}

#[test]
fn play_next_inserts_into_entries_after_current_plus_offset() {
    let mut s = queue_with(3, true, 5); // current = entry 3 ("d")
    s = QueueOps::play_next(&s, jf("x"));
    s = QueueOps::play_next(&s, jf("y"));
    assert_eq!(ids(s.entries.iter()), ["a", "b", "c", "d", "x", "y", "e"]);
    assert_eq!(play_order_ids(&s)[..3], ["d", "x", "y"]);
}

#[test]
fn previous_restarts_past_threshold_and_skips_unplayable() {
    let s = QueueOps::replace(&QueueState::default(), vec![jf("a"), dead("u"), jf("c")], 2, &mut rng(1), false);
    assert_eq!(QueueOps::previous(&s, PREVIOUS_RESTART_THRESHOLD_MS + 1), Previous::SeekToStart);
    assert_eq!(QueueOps::previous(&s, PREVIOUS_RESTART_THRESHOLD_MS), Previous::MoveTo(0));
    let first = QueueOps::jump_to_uid(&s, s.entries[0].uid);
    assert_eq!(QueueOps::previous(&first, 0), Previous::SeekToStart);
}
