//! The pure editor-groups model: preview tabs, splits, MRU and persistence.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lucerna_workbench::{EditorGroups, GroupState, GroupsState, OpenMode, TabId};

fn p(name: &str) -> Arc<Path> {
    Arc::from(Path::new(name))
}

/// `(path, pinned)` of every tab in group `g`, in order.
fn tabs(groups: &EditorGroups, g: usize) -> Vec<(String, bool)> {
    groups.groups()[g]
        .tabs
        .iter()
        .map(|t| (t.path.display().to_string(), t.pinned))
        .collect()
}

fn active_path(groups: &EditorGroups) -> Option<String> {
    groups.active_tab().map(|t| t.path.display().to_string())
}

fn owned(v: &[(&str, bool)]) -> Vec<(String, bool)> {
    v.iter().map(|(s, b)| (s.to_string(), *b)).collect()
}

#[test]
fn preview_replaces_preview() {
    let mut g = EditorGroups::default();
    let a = g.open(p("a.rs"), OpenMode::Preview, true);
    let b = g.open(p("b.rs"), OpenMode::Preview, true);
    assert_ne!(a, b);
    assert_eq!(tabs(&g, 0), owned(&[("b.rs", false)]));
    assert_eq!(active_path(&g).as_deref(), Some("b.rs"));
    assert_eq!(g.groups()[0].mru, vec![b]);
}

#[test]
fn pinned_not_replaced() {
    let mut g = EditorGroups::default();
    g.open(p("a.rs"), OpenMode::Pinned, true);
    g.open(p("b.rs"), OpenMode::Preview, true);
    g.open(p("c.rs"), OpenMode::Preview, true);
    assert_eq!(tabs(&g, 0), owned(&[("a.rs", true), ("c.rs", false)]));

    // Pinning the preview tab protects it too.
    let c = g.active_tab().unwrap().id;
    g.pin(c);
    g.open(p("d.rs"), OpenMode::Preview, true);
    assert_eq!(
        tabs(&g, 0),
        owned(&[("a.rs", true), ("c.rs", true), ("d.rs", false)])
    );
}

#[test]
fn open_existing_activates() {
    let mut g = EditorGroups::default();
    let a = g.open(p("a.rs"), OpenMode::Pinned, true);
    let b = g.open(p("b.rs"), OpenMode::Preview, true);
    assert_eq!(g.open(p("a.rs"), OpenMode::Preview, true), a);
    assert_eq!(active_path(&g).as_deref(), Some("a.rs"));
    assert_eq!(tabs(&g, 0).len(), 2);

    // Opening the preview tab's file pinned pins it in place.
    assert_eq!(g.open(p("b.rs"), OpenMode::Pinned, true), b);
    assert_eq!(tabs(&g, 0), owned(&[("a.rs", true), ("b.rs", true)]));
    assert_eq!(g.groups()[0].mru, vec![b, a]);
}

#[test]
fn preview_disabled_always_pins() {
    let mut g = EditorGroups::default();
    g.open(p("a.rs"), OpenMode::Preview, false);
    g.open(p("b.rs"), OpenMode::Preview, false);
    assert_eq!(tabs(&g, 0), owned(&[("a.rs", true), ("b.rs", true)]));
}

#[test]
fn to_side_creates_group() {
    let mut g = EditorGroups::default();
    g.open(p("a.rs"), OpenMode::Pinned, true);
    g.open(p("b.rs"), OpenMode::ToSide, true);
    assert_eq!(g.groups().len(), 2);
    assert_eq!(g.active_group(), 1);
    assert_eq!(tabs(&g, 1), owned(&[("b.rs", true)]));

    // From the first group, ToSide reuses the existing group to the right.
    assert!(g.focus_group(0));
    g.open(p("c.rs"), OpenMode::ToSide, true);
    assert_eq!(g.groups().len(), 2);
    assert_eq!(g.active_group(), 1);
    assert_eq!(tabs(&g, 1), owned(&[("b.rs", true), ("c.rs", true)]));
    assert_eq!(tabs(&g, 0), owned(&[("a.rs", true)]));
}

#[test]
fn close_activates_mru() {
    let mut g = EditorGroups::default();
    let a = g.open(p("a.rs"), OpenMode::Pinned, true);
    let b = g.open(p("b.rs"), OpenMode::Pinned, true);
    let c = g.open(p("c.rs"), OpenMode::Pinned, true);
    // MRU (newest first): a, c, b — not tab order.
    g.open(p("a.rs"), OpenMode::Preview, true);
    assert_eq!(g.close(a), Some(c));
    assert_eq!(active_path(&g).as_deref(), Some("c.rs"));
    assert_eq!(g.groups()[0].mru, vec![c, b]);

    // Closing an inactive tab keeps the active one.
    assert_eq!(g.close(b), Some(c));
    assert_eq!(g.close(c), None);
    assert!(g.groups()[0].tabs.is_empty());
    assert_eq!(g.groups()[0].active, None);
    assert_eq!(g.groups().len(), 1);
}

#[test]
fn close_last_tab_removes_non_first_group() {
    let mut g = EditorGroups::default();
    let a = g.open(p("a.rs"), OpenMode::Pinned, true);
    let b = g.open(p("b.rs"), OpenMode::ToSide, true);
    assert_eq!(g.groups().len(), 2);
    assert_eq!(g.close(b), Some(a));
    assert_eq!(g.groups().len(), 1);
    assert_eq!(g.active_group(), 0);

    // The first group survives losing its last tab.
    assert_eq!(g.close(a), None);
    assert_eq!(g.groups().len(), 1);
}

#[test]
fn focus_group_out_of_range_false() {
    let mut g = EditorGroups::default();
    assert!(g.focus_group(0));
    assert!(!g.focus_group(1));
    g.open(p("a.rs"), OpenMode::Pinned, true);
    g.split_active();
    assert!(g.focus_group(0));
    assert_eq!(g.active_group(), 0);
    assert!(!g.focus_group(2));
    assert_eq!(g.active_group(), 0);
}

#[test]
fn split_duplicates_active() {
    let mut g = EditorGroups::default();
    g.open(p("a.rs"), OpenMode::Pinned, true);
    let b = g.open(p("b.rs"), OpenMode::Preview, true);
    assert_eq!(g.split_active(), 1);
    assert_eq!(g.active_group(), 1);
    assert_eq!(tabs(&g, 1), owned(&[("b.rs", false)]));
    let copy = g.active_tab().unwrap().id;
    assert_ne!(copy, b);
    // The original group is unchanged.
    assert_eq!(tabs(&g, 0), owned(&[("a.rs", true), ("b.rs", false)]));

    // A split from the first group lands directly to its right.
    g.focus_group(0);
    g.open(p("a.rs"), OpenMode::Preview, true);
    assert_eq!(g.split_active(), 1);
    assert_eq!(g.groups().len(), 3);
    assert_eq!(tabs(&g, 1), owned(&[("a.rs", true)]));
    assert_eq!(tabs(&g, 2), owned(&[("b.rs", false)]));
}

#[test]
fn cycle_mru_alternates_last_two() {
    let mut g = EditorGroups::default();
    assert_eq!(g.cycle_mru(), None);
    let a = g.open(p("a.rs"), OpenMode::Pinned, true);
    assert_eq!(g.cycle_mru(), None);
    let b = g.open(p("b.rs"), OpenMode::Pinned, true);
    let c = g.open(p("c.rs"), OpenMode::Pinned, true);
    assert_eq!(g.cycle_mru(), Some(b));
    assert_eq!(g.cycle_mru(), Some(c));
    assert_eq!(g.cycle_mru(), Some(b));
    assert_eq!(g.groups()[0].mru, vec![b, c, a]);
    assert_eq!(g.groups()[0].active, Some(b));
}

#[test]
fn state_roundtrip() {
    let mut g = EditorGroups::default();
    g.open(p("a.rs"), OpenMode::Pinned, true);
    g.open(p("b.rs"), OpenMode::Preview, true);
    g.open(p("a.rs"), OpenMode::Preview, true);
    g.open(p("c.rs"), OpenMode::ToSide, true);
    g.focus_group(0);

    let state = g.state();
    assert_eq!(
        state,
        GroupsState {
            groups: vec![
                GroupState {
                    tabs: vec![
                        (PathBuf::from("a.rs"), true),
                        (PathBuf::from("b.rs"), false)
                    ],
                    active: Some(0),
                },
                GroupState {
                    tabs: vec![(PathBuf::from("c.rs"), true)],
                    active: Some(0),
                },
            ],
            active_group: 0,
        }
    );

    let json = serde_json::to_string(&state).unwrap();
    let parsed: GroupsState = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, state);

    let restored = EditorGroups::restore(&parsed);
    assert_eq!(restored.state(), state);
    assert_eq!(active_path(&restored).as_deref(), Some("a.rs"));
    assert_eq!(
        restored.groups()[0].active,
        Some(restored.groups()[0].tabs[0].id)
    );

    // Out-of-range indices from a stale file are clamped, not trusted.
    let stale = GroupsState {
        groups: vec![GroupState {
            tabs: vec![(PathBuf::from("x.rs"), true)],
            active: Some(5),
        }],
        active_group: 3,
    };
    let restored = EditorGroups::restore(&stale);
    assert_eq!(restored.active_group(), 0);
    assert_eq!(active_path(&restored).as_deref(), Some("x.rs"));
    let empty = EditorGroups::restore(&GroupsState {
        groups: vec![],
        active_group: 0,
    });
    assert_eq!(empty.groups().len(), 1);
    let _: Option<TabId> = empty.groups()[0].active;
}
