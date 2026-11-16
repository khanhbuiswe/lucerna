use std::path::{Path, PathBuf};
use std::sync::Arc;

use lucerna_project::{Change, ChangeKind, Coalesced, RESCAN_THRESHOLD, RawEvent, coalesce};

const ROOT: &str = "/proj";

fn p(rel: &str) -> PathBuf {
    Path::new(ROOT).join(rel)
}

fn change(rel: &str, kind: ChangeKind) -> Change {
    Change {
        path: Arc::from(Path::new(rel)),
        kind,
    }
}

fn run(events: Vec<RawEvent>) -> Coalesced {
    coalesce(events, Path::new(ROOT))
}

#[test]
fn added_then_removed_dropped() {
    let out = run(vec![RawEvent::Created(p("a")), RawEvent::Removed(p("a"))]);
    assert_eq!(out, Coalesced::Changes(vec![]));
}

#[test]
fn removed_then_added_is_modified() {
    let out = run(vec![RawEvent::Removed(p("a")), RawEvent::Created(p("a"))]);
    assert_eq!(
        out,
        Coalesced::Changes(vec![change("a", ChangeKind::Modified)])
    );
}

#[test]
fn added_then_modified_stays_added() {
    let out = run(vec![RawEvent::Created(p("a")), RawEvent::Modified(p("a"))]);
    assert_eq!(
        out,
        Coalesced::Changes(vec![change("a", ChangeKind::Added)])
    );
}

#[test]
fn modified_then_removed_is_removed() {
    let out = run(vec![RawEvent::Modified(p("a")), RawEvent::Removed(p("a"))]);
    assert_eq!(
        out,
        Coalesced::Changes(vec![change("a", ChangeKind::Removed)])
    );
}

#[test]
fn atomic_write_via_rename() {
    let out = run(vec![
        RawEvent::Created(p("f.tmp")),
        RawEvent::Modified(p("f.tmp")),
        RawEvent::Renamed {
            from: p("f.tmp"),
            to: p("f"),
        },
    ]);
    assert_eq!(
        out,
        Coalesced::Changes(vec![change("f", ChangeKind::Modified)])
    );
}

#[test]
fn plain_rename_removes_old_path() {
    let out = run(vec![RawEvent::Renamed {
        from: p("a"),
        to: p("b"),
    }]);
    let Coalesced::Changes(changes) = out else {
        panic!("expected changes");
    };
    assert!(changes.contains(&change("a", ChangeKind::Removed)));
}

#[test]
fn over_threshold_rescans() {
    let events = (0..=RESCAN_THRESHOLD)
        .map(|i| RawEvent::Modified(p(&format!("f{i}"))))
        .collect::<Vec<_>>();
    assert_eq!(events.len(), RESCAN_THRESHOLD + 1);
    assert_eq!(run(events), Coalesced::Rescan);
}

#[test]
fn at_threshold_does_not_rescan() {
    let events = (0..RESCAN_THRESHOLD)
        .map(|i| RawEvent::Modified(p(&format!("f{i}"))))
        .collect::<Vec<_>>();
    assert!(matches!(run(events), Coalesced::Changes(_)));
}

#[test]
fn overflow_rescans() {
    let out = run(vec![RawEvent::Modified(p("a")), RawEvent::Overflow]);
    assert_eq!(out, Coalesced::Rescan);
}

#[test]
fn outside_root_dropped() {
    let out = run(vec![
        RawEvent::Modified(PathBuf::from("/elsewhere/x")),
        RawEvent::Modified(p("in/side")),
    ]);
    assert_eq!(
        out,
        Coalesced::Changes(vec![change("in/side", ChangeKind::Modified)])
    );
}

/// T4-1: a report about the root itself (renamed away, back, chmod) comes
/// out as the empty path, which the worktree takes as a rescan signal.
#[test]
fn root_itself_is_kept_as_the_empty_path() {
    let out = run(vec![
        RawEvent::Removed(PathBuf::from(ROOT)),
        RawEvent::Modified(p("in/side")),
    ]);
    assert_eq!(
        out,
        Coalesced::Changes(vec![
            change("", ChangeKind::Removed),
            change("in/side", ChangeKind::Modified),
        ])
    );
}
