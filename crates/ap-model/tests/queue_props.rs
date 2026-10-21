//! Property invariants for `QueueOps`.

mod common;

use ap_model::{QueueOps, QueueState, RepeatMode, Track};
use common::{jf, rng};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Op {
    Replace { len: usize, start: isize, random_start: bool },
    PlayNext,
    Advance { wrap: bool },
    Back,
    Jump(usize),
    Shuffle(bool),
    Repeat(u8),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0usize..12, -2isize..14, any::<bool>()).prop_map(|(len, start, random_start)| Op::Replace {
            len,
            start,
            random_start
        }),
        Just(Op::PlayNext),
        any::<bool>().prop_map(|wrap| Op::Advance { wrap }),
        Just(Op::Back),
        (0usize..40).prop_map(Op::Jump),
        any::<bool>().prop_map(Op::Shuffle),
        (0u8..3).prop_map(Op::Repeat),
    ]
}

fn tracks(len: usize, salt: u64) -> Vec<Track> {
    (0..len).map(|i| jf(&format!("t{salt}-{}", i % 4))).collect()
}

fn assert_invariants(s: &QueueState) {
    let mut sorted = s.order.to_vec();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..s.entries.len()).collect::<Vec<_>>(), "order is a permutation of entries");
    if s.entries.is_empty() {
        assert_eq!(s.cursor, -1);
    } else {
        assert!(s.cursor >= -1 && s.cursor < isize::try_from(s.order.len()).unwrap());
    }
    if !s.shuffle {
        assert_eq!(s.order.to_vec(), (0..s.entries.len()).collect::<Vec<_>>(), "shuffle off ⇒ identity");
    }
    let mut uids: Vec<u64> = s.entries.iter().map(|e| e.uid).collect();
    uids.sort_unstable();
    uids.dedup();
    assert_eq!(uids.len(), s.entries.len(), "uids are unique");
    assert!(s.entries.iter().all(|e| e.uid < s.next_uid), "next_uid is fresh");
}

fn apply(s: &QueueState, op: &Op, seed: u64) -> QueueState {
    let mut r = rng(seed);
    match *op {
        Op::Replace { len, start, random_start } => QueueOps::replace(s, tracks(len, seed), start, &mut r, random_start),
        Op::PlayNext => QueueOps::play_next(s, jf(&format!("n{seed}"))),
        Op::Advance { wrap } => QueueOps::advance(s, wrap).unwrap_or_else(|| s.clone()),
        Op::Back => QueueOps::back(s).unwrap_or_else(|| s.clone()),
        Op::Jump(i) => {
            let uid = s.entries.get(i % s.entries.len().max(1)).map_or(999_999, |e| e.uid);
            QueueOps::jump_to_uid(s, uid)
        }
        Op::Shuffle(on) => QueueOps::set_shuffle(s, on, &mut r),
        Op::Repeat(m) => QueueOps::set_repeat(s, [RepeatMode::Off, RepeatMode::One, RepeatMode::All][usize::from(m)]),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn ops_preserve_queue_invariants(ops in proptest::collection::vec(op(), 1..40), seed in any::<u64>()) {
        let mut s = QueueState::default();
        for (i, op) in ops.iter().enumerate() {
            let next = apply(&s, op, seed.wrapping_add(i as u64));
            assert_invariants(&next);
            let content_changed = next.entries != s.entries || next.order != s.order;
            let bumped = next.content_version != s.content_version;
            match op {
                // Content ops always bump, even when the result happens to be equal.
                Op::Replace { .. } | Op::PlayNext => prop_assert_eq!(next.content_version, s.content_version + 1),
                Op::Shuffle(on) if *on != s.shuffle => prop_assert_eq!(next.content_version, s.content_version + 1),
                _ => {
                    prop_assert!(!bumped, "{:?} bumped content_version", op);
                    prop_assert!(!content_changed, "{:?} changed content", op);
                }
            }
            if content_changed {
                prop_assert!(bumped, "{:?} changed content without a bump", op);
            }
            s = next;
        }
    }

    #[test]
    fn set_shuffle_preserves_current_identity_and_history(
        len in 1usize..15,
        start in 0isize..15,
        seed in any::<u64>(),
        shuffled_first in any::<bool>(),
    ) {
        let base = QueueState { shuffle: shuffled_first, ..QueueState::default() };
        let s = QueueOps::replace(&base, tracks(len, seed), start, &mut rng(seed), false);
        let current = s.current().map(|e| e.uid);
        let toggled = QueueOps::set_shuffle(&s, !s.shuffle, &mut rng(seed ^ 1));
        prop_assert_eq!(toggled.current().map(|e| e.uid), current);
        if toggled.shuffle {
            prop_assert_eq!(toggled.cursor, s.cursor);
            let before: Vec<u64> = s.history().map(|e| e.uid).collect();
            let after: Vec<u64> = toggled.history().map(|e| e.uid).collect();
            prop_assert_eq!(after, before);
        } else {
            prop_assert_eq!(toggled.order.to_vec(), (0..len).collect::<Vec<_>>());
        }
        let back = QueueOps::set_shuffle(&toggled, s.shuffle, &mut rng(seed ^ 2));
        prop_assert_eq!(back.current().map(|e| e.uid), current);
    }

    #[test]
    fn play_next_lands_right_after_current_in_both_orders(
        len in 1usize..12,
        start in 0isize..12,
        shuffle in any::<bool>(),
        stack in 1usize..4,
        seed in any::<u64>(),
    ) {
        let base = QueueState { shuffle, ..QueueState::default() };
        let mut s = QueueOps::replace(&base, tracks(len, seed), start, &mut rng(seed), false);
        let cursor = usize::try_from(s.cursor).unwrap();
        let current_entry = s.order[cursor];
        let mut added = Vec::new();
        for k in 0..stack {
            s = QueueOps::play_next(&s, jf(&format!("pn{k}")));
            added.push(s.next_uid - 1);
        }
        let play: Vec<u64> = s.order[cursor + 1..=cursor + stack].iter().map(|&i| s.entries[i].uid).collect();
        prop_assert_eq!(&play, &added);
        let inserted: Vec<u64> = s.entries[current_entry + 1..=current_entry + stack].iter().map(|e| e.uid).collect();
        prop_assert_eq!(&inserted, &added);
    }
}
