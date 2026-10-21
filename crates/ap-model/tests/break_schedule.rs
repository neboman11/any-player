//! Port of Android `BreakScheduleTest` plus the `break_schedule.json` golden vectors.

mod common;

use std::sync::Arc;

use ap_model::break_schedule::{reduce, reduce_with};
use ap_model::{BreakSchedule, QueueEntry, QueueOps, QueueState, ScheduleEvent, SourceType, Track};
use common::{fixture_json, rng};
use serde_json::Value;

fn queue(count: usize) -> QueueState {
    let tracks = (0..count)
        .map(|i| Track {
            url: Some(format!("file:///song{i}.wav")),
            ..Track::new(i.to_string(), i.to_string(), "Artist", SourceType::Custom)
        })
        .collect();
    QueueOps::replace(&QueueState::default(), tracks, 0, &mut rng(0), false)
}

fn enabled(q: &QueueState) -> BreakSchedule {
    reduce(&BreakSchedule::default(), ScheduleEvent::Enabled(true), q, &mut rng(9))
}

fn at(q: &QueueState, cursor: isize) -> QueueState {
    QueueState { cursor, ..q.clone() }
}

fn moved(s: &BreakSchedule, q: &QueueState, cursor: isize) -> BreakSchedule {
    reduce(s, ScheduleEvent::CursorMoved, &at(q, cursor), &mut rng(0))
}

fn map_tracks(q: &QueueState, f: impl Fn(usize, &QueueEntry) -> Track) -> QueueState {
    let entries: Arc<[QueueEntry]> =
        q.entries.iter().enumerate().map(|(i, e)| QueueEntry { uid: e.uid, track: f(i, e) }).collect();
    QueueState { entries, ..q.clone() }
}

fn pos(n: i32) -> usize {
    usize::try_from(n).unwrap()
}

#[test]
fn enable_prepares_future_target_immediately_with_random_interval() {
    let q = queue(20);
    for seed in 0..=100 {
        let s = reduce(&BreakSchedule::default(), ScheduleEvent::Enabled(true), &q, &mut rng(seed));
        assert!((3..=5).contains(&s.songs_until_break));
        assert_eq!(Some(q.entry_at(pos(s.songs_until_break) + 1).uid), s.target_uid);
        assert_eq!(s.songs_away(&q), Some(s.songs_until_break));
    }
}

#[test]
fn forward_jump_counts_every_real_song() {
    let q = queue(20);
    let s = enabled(&q);
    let m = moved(&s, &q, 2);
    assert_eq!(m.songs_until_break, s.songs_until_break - 2);
    assert_eq!(m.target_uid, s.target_uid);
}

#[test]
fn repeated_cursor_event_does_not_count() {
    let q = queue(20);
    let s = enabled(&q);
    assert_eq!(moved(&s, &q, 0), s);
}

#[test]
fn backward_movement_reanchors_without_counting() {
    let q = queue(20);
    let s = moved(&enabled(&q), &q, 2);
    let back = moved(&s, &q, 0);
    assert_eq!(back.songs_until_break, s.songs_until_break);
    assert_eq!(back.anchor_order_pos, 0);
}

#[test]
fn one_song_backward_bounce_preserves_ready_target_and_counts_only_new_advances() {
    let q = queue(20);
    let s = moved(&enabled(&q), &q, 2);
    let back = moved(&s, &q, 1);
    assert_eq!(back.target_uid, s.target_uid);
    assert_eq!(back.anchor_order_pos, s.anchor_order_pos);
    assert_eq!(back.songs_until_break, s.songs_until_break);
    assert_eq!(moved(&back, &q, 2), s);
    let forward = moved(&back, &q, 3);
    assert_eq!(forward.target_uid, s.target_uid);
    assert_eq!(forward.songs_until_break, s.songs_until_break - 1);
}

#[test]
fn content_edits_reanchor_without_counting() {
    let q = queue(20);
    let s = enabled(&q);
    let edited = QueueState { cursor: 2, content_version: q.content_version + 1, ..q.clone() };
    let result = reduce(&s, ScheduleEvent::CursorMoved, &edited, &mut rng(0));
    assert_eq!(result.songs_until_break, s.songs_until_break);
    assert_eq!(result.order_version, i64::try_from(edited.content_version).unwrap());
}

#[test]
fn dj_entries_never_count_and_are_never_targets() {
    let q = queue(20);
    let s = enabled(&q);
    let mixed = map_tracks(&q, |i, e| Track { is_dj_filler: (1..=3).contains(&i), ..e.track.clone() });
    let result = moved(&s, &mixed, 3);
    assert_eq!(result.songs_until_break, s.songs_until_break);
    let target = mixed.entries.iter().find(|e| Some(e.uid) == result.target_uid).unwrap();
    assert!(!target.track.is_dj_filler);
}

#[test]
fn unplayable_entries_are_not_break_targets() {
    let q = queue(20);
    let s = enabled(&q);
    let p = q.order_pos_of_uid(s.target_uid.unwrap()).unwrap();
    let mixed = map_tracks(&q, |i, e| Track { url: if i == p { None } else { e.track.url.clone() }, ..e.track.clone() });
    let result = reduce(&s, ScheduleEvent::CursorMoved, &mixed, &mut rng(0));
    assert_eq!(result.target_uid, Some(mixed.entry_at(p + 1).uid));
}

#[test]
fn failed_generation_retries_after_three_more_real_songs() {
    let q = queue(20);
    let failed = reduce(&enabled(&q), ScheduleEvent::GenerationFailed, &q, &mut rng(0));
    assert_eq!(failed.songs_until_break, 3);
    assert!(failed.failed_on_current);
    assert_eq!(moved(&failed, &q, 0), failed);
    let next = moved(&failed, &q, 1);
    assert!(!next.failed_on_current);
    assert_eq!(next.songs_until_break, 2);
}

#[test]
fn missing_due_break_resets_to_exactly_three() {
    let q = queue(20);
    let s = enabled(&q);
    let passed = moved(&s, &q, isize::try_from(s.songs_until_break).unwrap() + 1);
    assert_eq!(passed.songs_until_break, 3);
    assert!(passed.failed_on_current);
}

#[test]
fn break_start_immediately_schedules_next_break() {
    let q = queue(20);
    let s = enabled(&q);
    let current = at(&q, isize::try_from(s.songs_until_break).unwrap());
    let next = reduce(&s, ScheduleEvent::BreakStarted, &current, &mut rng(1));
    assert!((3..=5).contains(&next.songs_until_break));
    assert_ne!(next.target_uid, s.target_uid);
}

#[test]
fn disable_clears_target_and_ui_distance() {
    let q = queue(20);
    let off = reduce(&enabled(&q), ScheduleEvent::Enabled(false), &q, &mut rng(0));
    assert_eq!(off.target_uid, None);
    assert_eq!(off.songs_away(&q), None);
    assert_eq!(off.songs_until_break, 3);
}

#[test]
fn short_queue_has_no_target_but_retains_schedule() {
    let q = queue(2);
    let s = enabled(&q);
    assert_eq!(s.target_uid, None);
    assert!(s.songs_away(&q).is_some());
}

#[test]
fn duplicate_track_ids_target_unique_queue_slot() {
    let q = map_tracks(&queue(20), |_, e| Track { id: "duplicate".into(), ..e.track.clone() });
    let s = enabled(&q);
    assert_eq!(s.target_uid, Some(q.entry_at(pos(s.songs_until_break) + 1).uid));
}

#[test]
fn queue_replacement_rerolls_and_clears_failure() {
    let q = queue(20);
    let failed = reduce(&enabled(&q), ScheduleEvent::GenerationFailed, &q, &mut rng(0));
    let s = reduce(&failed, ScheduleEvent::QueueReplaced, &q, &mut rng(2));
    assert!((3..=5).contains(&s.songs_until_break));
    assert!(!s.failed_on_current);
}

fn fixture_queue(v: &Value) -> QueueState {
    let entries: Arc<[QueueEntry]> = v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| QueueEntry {
            uid: e["uid"].as_u64().unwrap(),
            track: serde_json::from_value(e["track"].clone()).unwrap(),
        })
        .collect();
    let order: Arc<[usize]> =
        v["order"].as_array().unwrap().iter().map(|i| usize::try_from(i.as_u64().unwrap()).unwrap()).collect();
    QueueState {
        entries,
        order,
        cursor: isize::try_from(v["cursor"].as_i64().unwrap()).unwrap(),
        content_version: v["content_version"].as_u64().unwrap(),
        ..QueueState::default()
    }
}

fn fixture_event(name: &str) -> ScheduleEvent {
    match name {
        "enable" => ScheduleEvent::Enabled(true),
        "disable" => ScheduleEvent::Enabled(false),
        "queue_replaced" => ScheduleEvent::QueueReplaced,
        "cursor_moved" => ScheduleEvent::CursorMoved,
        "break_started" => ScheduleEvent::BreakStarted,
        "generation_failed" => ScheduleEvent::GenerationFailed,
        other => panic!("unknown event {other}"),
    }
}

#[test]
fn golden_vectors_from_android() {
    let root = fixture_json("break_schedule.json");
    let cases = root["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let mut intervals = case["intervals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| i32::try_from(v.as_i64().unwrap()).unwrap())
            .collect::<Vec<_>>()
            .into_iter();
        let mut s = BreakSchedule::default();
        let events = case["events"].as_array().unwrap();
        let expected = case["expected"].as_array().unwrap();
        assert_eq!(events.len(), expected.len(), "{name}");
        for (step, (event, want)) in events.iter().zip(expected).enumerate() {
            let q = fixture_queue(&event["queue"]);
            let ev = fixture_event(event["event"].as_str().unwrap());
            s = reduce_with(&s, ev, &q, || intervals.next().unwrap_or_else(|| panic!("{name}: interval exhausted")));
            let ctx = format!("{name} step {step}");
            assert_eq!(s.songs_away(&q).map(i64::from), want["songs_away"].as_i64(), "{ctx}: songs_away");
            assert_eq!(s.target_uid, want["target_uid"].as_u64(), "{ctx}: target_uid");
            assert_eq!(i64::from(s.songs_until_break), want["songs_until_break"].as_i64().unwrap(), "{ctx}");
            assert_eq!(
                i64::try_from(s.anchor_order_pos).unwrap(),
                want["anchor_order_pos"].as_i64().unwrap(),
                "{ctx}: anchor"
            );
            assert_eq!(s.order_version, want["order_version"].as_i64().unwrap(), "{ctx}: order_version");
            assert_eq!(s.failed_on_current, want["failed_on_current"].as_bool().unwrap(), "{ctx}: failed");
            assert_eq!(s.enabled, want["enabled"].as_bool().unwrap(), "{ctx}: enabled");
        }
        assert!(intervals.next().is_none(), "{name}: every scripted interval is consumed");
    }
}
