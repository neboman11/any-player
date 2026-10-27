//! Port of Android `RunPlanningTest` (`EXO` = `Local`).

mod common;

use ap_model::planning::DEFAULT_MAX_RUN_ITEMS;
use ap_model::{
    BackendKind, PendingBreak, PlanItem, QueueOps, QueueState, RepeatMode, Run, Track, backend_for,
    break_run, plan_run,
};
use common::{dead, jf, label, play_order_ids, rng, sp};

fn q(tracks: Vec<Track>, cursor: isize) -> QueueState {
    QueueOps::replace(&QueueState::default(), tracks, cursor, &mut rng(1), false)
}

fn describe(s: &QueueState, run: Option<Run>) -> Vec<String> {
    run.expect("run")
        .items
        .iter()
        .map(|item| match item {
            PlanItem::Song { order_pos, .. } => {
                format!("{}@{order_pos}", label(&s.entry_at(*order_pos).track))
            }
            PlanItem::Break { break_id, .. } => format!("break:{break_id}"),
        })
        .collect()
}

fn plan(s: &QueueState, from: usize, pending: Option<&PendingBreak>) -> Vec<String> {
    describe(s, plan_run(s, from, pending, DEFAULT_MAX_RUN_ITEMS))
}

fn with_url(t: Track, url: &str) -> Track {
    Track {
        url: Some(url.into()),
        ..t
    }
}

fn with_id(t: Track, id: &str) -> Track {
    Track { id: id.into(), ..t }
}

fn pending(s: &QueueState, entry: usize, path: &str) -> PendingBreak {
    PendingBreak {
        before_uid: s.entries[entry].uid,
        break_id: "brk".into(),
        path: path.into(),
    }
}

#[test]
fn backend_for_classifies_by_source_and_scheme() {
    assert_eq!(backend_for(&sp("s")), BackendKind::Spotify);
    assert_eq!(
        backend_for(&with_id(sp("s"), "spotify:track:4uLU6hMCjMI75M1A2tKUQC")),
        BackendKind::Spotify
    );
    assert_eq!(backend_for(&jf("a")), BackendKind::Local);
    for url in [
        "https://h/x",
        "HTTP://h/x",
        "file:///music/x.mp3",
        " https://h/y ",
    ] {
        assert_eq!(
            backend_for(&with_url(dead("x"), url)),
            BackendKind::Local,
            "{url}"
        );
    }
    // Android-only schemes are unplayable on desktop.
    for url in [
        "content://media/1",
        "android.resource://pkg/raw/x",
        "",
        "any-player://local/x",
    ] {
        assert_eq!(
            backend_for(&with_url(dead("x"), url)),
            BackendKind::Unplayable,
            "{url}"
        );
    }
    assert_eq!(backend_for(&dead("x")), BackendKind::Unplayable);
}

#[test]
fn backend_for_invalid_spotify_id_is_unplayable() {
    let base = sp("s");
    assert_eq!(
        backend_for(&with_id(base.clone(), "short")),
        BackendKind::Unplayable
    );
    assert_eq!(
        backend_for(&with_id(base.clone(), "4uLU6hMCjMI75M1A2tKUQ-")),
        BackendKind::Unplayable
    );
    assert_eq!(
        backend_for(&with_id(base.clone(), "4uLU6hMCjMI75M1A2tKUQCX")),
        BackendKind::Unplayable
    );
    assert_eq!(
        backend_for(&with_url(with_id(base.clone(), ""), "https://x")),
        BackendKind::Unplayable
    );
    assert_eq!(
        backend_for(&with_id(base, " 4uLU6hMCjMI75M1A2tKUQC ")),
        BackendKind::Spotify
    );
}

#[test]
fn run_boundaries_split_by_backend_kind() {
    let s = q(vec![jf("a"), jf("b"), sp("s1"), sp("s2"), jf("c")], 0);
    let local = plan_run(&s, 0, None, DEFAULT_MAX_RUN_ITEMS).unwrap();
    assert_eq!(local.kind, BackendKind::Local);
    assert_eq!(plan(&s, 0, None), ["a@0", "b@1"]);
    assert_eq!(
        plan_run(&s, 2, None, DEFAULT_MAX_RUN_ITEMS).unwrap().kind,
        BackendKind::Spotify
    );
    assert_eq!(plan(&s, 2, None), ["s1@2", "s2@3"]);
    assert_eq!(plan(&s, 4, None), ["c@4"]);
}

#[test]
fn unplayable_entries_are_skipped_inside_run_without_ending_it() {
    let s = q(
        vec![
            jf("a"),
            dead("u1"),
            jf("b"),
            dead("u2"),
            sp("s1"),
            dead("u3"),
            sp("s2"),
        ],
        0,
    );
    assert_eq!(plan(&s, 0, None), ["a@0", "b@2"]);
    assert_eq!(plan(&s, 3, None), ["s1@4", "s2@6"]);
    assert_eq!(
        plan_run(&s, 3, None, DEFAULT_MAX_RUN_ITEMS).unwrap().kind,
        BackendKind::Spotify
    );
}

#[test]
fn unplayable_start_skips_to_first_playable() {
    let s = q(vec![dead("u1"), dead("u2"), jf("a")], 0);
    assert_eq!(
        plan_run(&s, 0, None, DEFAULT_MAX_RUN_ITEMS).unwrap().kind,
        BackendKind::Local
    );
    assert_eq!(plan(&s, 0, None), ["a@2"]);
}

#[test]
fn no_playable_entry_returns_none() {
    assert!(
        plan_run(
            &q(vec![dead("u1"), with_id(sp("s"), "bad")], 0),
            0,
            None,
            50
        )
        .is_none()
    );
    assert!(plan_run(&q(vec![jf("a")], 0), 5, None, 50).is_none());
    assert!(plan_run(&QueueState::default(), 0, None, 50).is_none());
}

#[test]
fn run_starting_mid_run_starts_at_requested_position() {
    let s = q(vec![jf("a"), jf("b"), jf("c")], 0);
    assert_eq!(plan(&s, 1, None), ["b@1", "c@2"]);
}

#[test]
fn run_does_not_wrap_even_with_repeat_all() {
    let s = QueueOps::set_repeat(&q(vec![jf("a"), jf("b")], 1), RepeatMode::All);
    assert_eq!(plan(&s, 1, None), ["b@1"]);
}

#[test]
fn run_follows_shuffled_play_order() {
    let s = QueueOps::replace(
        &QueueState {
            shuffle: true,
            ..QueueState::default()
        },
        vec![jf("a"), jf("b"), jf("c"), jf("d")],
        2,
        &mut rng(8),
        false,
    );
    let run = plan_run(&s, 0, None, 50).unwrap();
    let positions: Vec<usize> = run
        .items
        .iter()
        .map(|i| match i {
            PlanItem::Song { order_pos, .. } => *order_pos,
            PlanItem::Break { .. } => panic!("no breaks"),
        })
        .collect();
    assert_eq!(positions, [0, 1, 2, 3]);
    let ids: Vec<String> = positions
        .iter()
        .map(|&p| s.entry_at(p).track.id.clone())
        .collect();
    assert_eq!(ids, play_order_ids(&s));
}

#[test]
fn max_items_caps_run() {
    let ten: Vec<Track> = (0..10).map(|i| jf(&format!("t{i}"))).collect();
    let s = q(ten, 0);
    assert_eq!(
        describe(&s, plan_run(&s, 0, None, 3)),
        ["t0@0", "t1@1", "t2@2"]
    );
    let eighty: Vec<Track> = (0..80).map(|i| jf(&format!("t{i}"))).collect();
    assert_eq!(
        plan_run(&q(eighty, 0), 0, None, DEFAULT_MAX_RUN_ITEMS)
            .unwrap()
            .items
            .len(),
        50
    );
}

#[test]
fn max_items_counts_only_planned_items_not_skipped_entries() {
    let s = q(vec![jf("a"), dead("u"), dead("v"), jf("b"), jf("c")], 0);
    assert_eq!(describe(&s, plan_run(&s, 0, None, 2)), ["a@0", "b@3"]);
}

#[test]
fn break_is_spliced_before_target_in_local_run() {
    let s = q(vec![jf("a"), jf("b"), jf("c")], 0);
    let p = pending(&s, 2, "file:///dj/brk.wav");
    let run = plan_run(&s, 0, Some(&p), 50).unwrap();
    assert_eq!(
        run.items[2],
        PlanItem::Break {
            break_id: "brk".into(),
            path: "file:///dj/brk.wav".into()
        }
    );
    assert_eq!(describe(&s, Some(run)), ["a@0", "b@1", "break:brk", "c@2"]);
}

#[test]
fn break_at_run_start_is_spliced_first_in_local_run() {
    let s = q(vec![jf("a"), jf("b")], 0);
    let p = pending(&s, 0, "file:///b.wav");
    assert_eq!(plan(&s, 0, Some(&p)), ["break:brk", "a@0", "b@1"]);
}

#[test]
fn break_counts_towards_max_items() {
    let s = q(vec![jf("a"), jf("b"), jf("c")], 0);
    let p = pending(&s, 1, "file:///b.wav");
    assert_eq!(
        describe(&s, plan_run(&s, 0, Some(&p), 2)),
        ["a@0", "break:brk"]
    );
}

#[test]
fn break_for_duplicate_track_id_uses_uid_not_track_id() {
    let s = q(vec![jf("a"), jf("a"), jf("a")], 0);
    let p = pending(&s, 2, "file:///b.wav");
    assert_eq!(plan(&s, 0, Some(&p)), ["a@0", "a@1", "break:brk", "a@2"]);
}

#[test]
fn spotify_run_stops_before_break_target() {
    let s = q(vec![sp("s1"), sp("s2"), sp("s3")], 0);
    let p = pending(&s, 2, "file:///b.wav");
    let run = plan_run(&s, 0, Some(&p), 50).unwrap();
    assert_eq!(run.kind, BackendKind::Spotify);
    assert!(run.items.iter().all(|i| matches!(i, PlanItem::Song { .. })));
    assert_eq!(describe(&s, Some(run)), ["s1@0", "s2@1"]);
    assert_eq!(
        break_run(&p),
        Run {
            kind: BackendKind::Local,
            items: vec![PlanItem::Break {
                break_id: "brk".into(),
                path: "file:///b.wav".into()
            }]
        }
    );
    // After the break the next Spotify run starts at the target.
    assert_eq!(plan(&s, 2, Some(&p)), ["s3@2"]);
}

#[test]
fn break_target_outside_run_is_ignored() {
    let s = q(vec![jf("a"), sp("s1"), jf("b")], 0);
    let p = pending(&s, 2, "file:///b.wav");
    assert_eq!(plan(&s, 0, Some(&p)), ["a@0"]);
}
