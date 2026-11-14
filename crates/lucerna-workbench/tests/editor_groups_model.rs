//! The pure editor-groups model: preview tabs, splits, MRU and persistence.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lucerna_workbench::{
    EXTENSIONS_TAB_STATE, EditorGroups, FileView, GroupState, GroupsState,
    MARKDOWN_PREVIEW_DYNAMIC_STATE_PREFIX, MARKDOWN_PREVIEW_STATE_PREFIX, OpenMode,
    SEARCH_TAB_STATE, TabItem,
};

fn p(name: &str) -> Arc<Path> {
    Arc::from(Path::new(name))
}

/// `(path, pinned)` of every tab in group `g`, in order.
fn tabs(groups: &EditorGroups, g: usize) -> Vec<(String, bool)> {
    groups.groups()[g]
        .tabs
        .iter()
        .map(|t| (t.path().unwrap().display().to_string(), t.pinned))
        .collect()
}

fn active_path(groups: &EditorGroups) -> Option<String> {
    groups
        .active_tab()
        .map(|t| t.path().unwrap().display().to_string())
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
    assert_eq!(empty.groups()[0].active, None);
}

fn saved(tabs: &[(&str, bool)], active: Option<usize>) -> GroupState {
    GroupState {
        tabs: tabs.iter().map(|(p, b)| (PathBuf::from(p), *b)).collect(),
        active,
    }
}

#[test]
fn restore_remaps_active_group_past_dropped_groups() {
    // [A, empty, C, D] with D active: D is restored index 2.
    let state = GroupsState {
        groups: vec![
            saved(&[("a.rs", true)], Some(0)),
            saved(&[], None),
            saved(&[("c.rs", true)], Some(0)),
            saved(&[("d.rs", true)], Some(0)),
        ],
        active_group: 3,
    };
    let g = EditorGroups::restore(&state);
    assert_eq!(g.groups().len(), 3);
    assert_eq!(g.active_group(), 2);
    assert_eq!(active_path(&g).as_deref(), Some("d.rs"));

    // A dropped active group falls back to its left neighbour.
    let g = EditorGroups::restore(&GroupsState {
        active_group: 1,
        ..state.clone()
    });
    assert_eq!(active_path(&g).as_deref(), Some("a.rs"));
    let g = EditorGroups::restore(&GroupsState {
        active_group: 2,
        ..state.clone()
    });
    assert_eq!(active_path(&g).as_deref(), Some("c.rs"));
    // Out of range clamps to the last group.
    let g = EditorGroups::restore(&GroupsState {
        active_group: 9,
        ..state
    });
    assert_eq!(active_path(&g).as_deref(), Some("d.rs"));
}

#[test]
fn restore_without_active_tab_activates_last() {
    let state = GroupsState {
        groups: vec![saved(&[("a.rs", true), ("b.rs", true)], None)],
        active_group: 0,
    };
    let g = EditorGroups::restore(&state);
    assert_eq!(active_path(&g).as_deref(), Some("b.rs"));
    let state = GroupsState {
        groups: vec![saved(&[("a.rs", true), ("b.rs", true)], Some(7))],
        active_group: 0,
    };
    assert_eq!(
        active_path(&EditorGroups::restore(&state)).as_deref(),
        Some("b.rs")
    );
}

#[test]
fn restore_drops_duplicates_and_extra_previews() {
    let state = GroupsState {
        groups: vec![saved(
            &[
                ("a.rs", false),
                ("b.rs", true),
                ("a.rs", true),
                ("c.rs", false),
                ("d.rs", false),
            ],
            // The duplicate a.rs entry: resolves to the kept a.rs tab.
            Some(2),
        )],
        active_group: 0,
    };
    let g = EditorGroups::restore(&state);
    assert_eq!(
        tabs(&g, 0),
        owned(&[
            ("a.rs", true),
            ("b.rs", true),
            ("c.rs", true),
            ("d.rs", false)
        ])
    );
    assert_eq!(active_path(&g).as_deref(), Some("a.rs"));
    // The single preview is still replaced by the next preview open.
    let mut g = g;
    g.open(p("e.rs"), OpenMode::Preview, true);
    assert_eq!(tabs(&g, 0)[3], ("e.rs".to_string(), false));
    assert_eq!(tabs(&g, 0).len(), 4);
}

#[test]
fn to_side_pins_already_open_tab() {
    let mut g = EditorGroups::default();
    g.open(p("a.rs"), OpenMode::Pinned, true);
    let b = g.open(p("b.rs"), OpenMode::ToSide, true);
    g.open(p("c.rs"), OpenMode::Preview, true);
    g.focus_group(0);
    // c.rs is a preview in group 1; ToSide from group 0 activates and pins it.
    let c = g.open(p("c.rs"), OpenMode::ToSide, true);
    assert_eq!(g.active_group(), 1);
    assert_eq!(tabs(&g, 1), owned(&[("b.rs", true), ("c.rs", true)]));
    assert_eq!(g.groups()[1].mru, vec![c, b]);
    assert_eq!(tabs(&g, 0), owned(&[("a.rs", true)]));
}

#[test]
fn extensions_tab_is_single_pinned_and_not_split() {
    let mut g = EditorGroups::default();
    g.open(p("a.rs"), OpenMode::Preview, true);
    let ext = g.open_extensions();
    assert_eq!(g.extensions_tab(), Some(ext));
    let tab = g.active_tab().unwrap();
    assert_eq!(tab.item, TabItem::Extensions);
    assert!(tab.pinned);
    assert_eq!(tab.path(), None);
    assert_eq!(tab.title(), "Extensions");
    // A preview open replaces the preview a.rs, not the page.
    g.open(p("b.rs"), OpenMode::Preview, true);
    let items: Vec<&TabItem> = g.groups()[0].tabs.iter().map(|t| &t.item).collect();
    assert_eq!(items, [&TabItem::File(p("b.rs")), &TabItem::Extensions]);

    // From another group, the existing tab is focused, not copied.
    g.open(p("c.rs"), OpenMode::ToSide, true);
    assert_eq!(g.active_group(), 1);
    assert_eq!(g.open_extensions(), ext);
    assert_eq!(g.active_group(), 0);
    assert_eq!(g.active_tab().map(|t| t.id), Some(ext));
    assert_eq!(g.split_active(), 0, "the page is never split");
    assert_eq!(g.groups().len(), 2);
    g.close(ext);
    assert_eq!(g.extensions_tab(), None);
}

#[test]
fn extensions_tab_saved_and_restored_once() {
    let mut g = EditorGroups::default();
    g.open(p("/w/a.rs"), OpenMode::Pinned, true);
    g.open_extensions();
    let state = g.state();
    assert_eq!(
        state.groups[0].tabs,
        [
            (PathBuf::from("/w/a.rs"), true),
            (PathBuf::from(EXTENSIONS_TAB_STATE), true)
        ]
    );
    let restored = EditorGroups::restore(&state);
    assert_eq!(restored.state(), state);

    // A hand-edited file with two (one unpinned) keeps the first, pinned.
    let state = GroupsState {
        groups: vec![
            saved(&[(EXTENSIONS_TAB_STATE, false)], Some(0)),
            saved(&[("b.rs", true), (EXTENSIONS_TAB_STATE, true)], Some(1)),
        ],
        active_group: 1,
    };
    let restored = EditorGroups::restore(&state);
    let ext: Vec<(usize, bool)> = restored
        .groups()
        .iter()
        .enumerate()
        .flat_map(|(i, g)| {
            g.tabs
                .iter()
                .filter(|t| t.item == TabItem::Extensions)
                .map(move |t| (i, t.pinned))
        })
        .collect();
    assert_eq!(ext, [(0, true)]);
    assert_eq!(active_path(&restored).as_deref(), Some("b.rs"));
}

#[test]
fn open_search_is_singleton_and_pinned() {
    let mut g = EditorGroups::default();
    assert_eq!(g.search_tab(), None);
    g.open(p("a.rs"), OpenMode::Preview, true);
    let search = g.open_search();
    assert_eq!(g.search_tab(), Some(search));
    let tab = g.active_tab().unwrap();
    assert_eq!(tab.item, TabItem::Search);
    assert!(tab.pinned);
    assert_eq!(tab.path(), None);
    assert_eq!(tab.title(), "Search");
    // A preview open replaces the preview a.rs, not the Search tab.
    g.open(p("b.rs"), OpenMode::Preview, true);
    let items: Vec<&TabItem> = g.groups()[0].tabs.iter().map(|t| &t.item).collect();
    assert_eq!(items, [&TabItem::File(p("b.rs")), &TabItem::Search]);

    // From another group, the existing tab is focused, not copied.
    g.open(p("c.rs"), OpenMode::ToSide, true);
    assert_eq!(g.active_group(), 1);
    assert_eq!(g.open_search(), search);
    assert_eq!(g.active_group(), 0);
    assert_eq!(g.active_tab().map(|t| t.id), Some(search));
    assert_eq!(g.split_active(), 0, "the Search tab is never split");
    assert_eq!(g.groups().len(), 2);
    // The Extensions tab is another singleton.
    g.open_extensions();
    assert_eq!(g.search_tab(), Some(search));
    g.close(search);
    assert_eq!(g.search_tab(), None);
}

#[test]
fn restore_keeps_first_search_tab() {
    let mut g = EditorGroups::default();
    g.open(p("/w/a.rs"), OpenMode::Pinned, true);
    g.open_search();
    let state = g.state();
    assert_eq!(
        state.groups[0].tabs,
        [
            (PathBuf::from("/w/a.rs"), true),
            (PathBuf::from(SEARCH_TAB_STATE), true)
        ]
    );
    let restored = EditorGroups::restore(&state);
    assert_eq!(restored.state(), state);

    // A hand-edited file with two (one unpinned) keeps the first, pinned.
    let state = GroupsState {
        groups: vec![
            saved(&[(SEARCH_TAB_STATE, false)], Some(0)),
            saved(
                &[
                    ("b.rs", true),
                    (SEARCH_TAB_STATE, true),
                    (EXTENSIONS_TAB_STATE, true),
                ],
                Some(0),
            ),
        ],
        active_group: 1,
    };
    let restored = EditorGroups::restore(&state);
    let search: Vec<(usize, bool)> = restored
        .groups()
        .iter()
        .enumerate()
        .flat_map(|(i, g)| {
            g.tabs
                .iter()
                .filter(|t| t.item == TabItem::Search)
                .map(move |t| (i, t.pinned))
        })
        .collect();
    assert_eq!(search, [(0, true)]);
    assert!(restored.extensions_tab().is_some());
    assert_eq!(active_path(&restored).as_deref(), Some("b.rs"));
}

fn items(groups: &EditorGroups, g: usize) -> Vec<TabItem> {
    groups.groups()[g]
        .tabs
        .iter()
        .map(|t| t.item.clone())
        .collect()
}

#[test]
fn plain_open_reuses_source_tab() {
    let mut g = EditorGroups::default();
    let a = g.open(p("a.md"), OpenMode::Pinned, true);
    let b = g.open_file(p("a.md"), FileView::Preview, OpenMode::Preview, true, true);
    assert_eq!(a, b);
    assert_eq!(items(&g, 0), vec![TabItem::File(p("a.md"))]);
}

#[test]
fn plain_open_reuses_static_preview() {
    let mut g = EditorGroups::default();
    let a = g.open_file(p("a.md"), FileView::Preview, OpenMode::Pinned, true, false);
    g.open(p("b.md"), OpenMode::Pinned, true);
    let again = g.open_file(p("a.md"), FileView::Source, OpenMode::Preview, true, true);
    assert_eq!(a, again);
    assert_eq!(g.active_tab().unwrap().id, a);
    assert_eq!(g.groups()[0].tabs.len(), 2);
}

#[test]
fn point_open_adds_source_next_to_preview() {
    let mut g = EditorGroups::default();
    g.open_file(p("a.md"), FileView::Preview, OpenMode::Pinned, true, false);
    g.open(p("a.md"), OpenMode::Pinned, true);
    assert_eq!(
        items(&g, 0),
        vec![
            TabItem::MarkdownPreview(p("a.md")),
            TabItem::File(p("a.md"))
        ]
    );
}

#[test]
fn replace_item_keeps_slot_and_pin() {
    let mut g = EditorGroups::default();
    g.open(p("a.md"), OpenMode::Pinned, true);
    let mid = g.open(p("b.md"), OpenMode::Preview, true);
    g.open(p("c.md"), OpenMode::Pinned, true);
    g.activate(mid);
    let new = g
        .replace_item(mid, TabItem::MarkdownPreview(p("b.md")))
        .unwrap();
    assert_eq!(
        g.groups()[0].tabs[1].item,
        TabItem::MarkdownPreview(p("b.md"))
    );
    assert!(!g.groups()[0].tabs[1].pinned);
    assert_eq!(g.active_tab().unwrap().id, new);
    assert!(g.groups()[0].mru.contains(&new) && !g.groups()[0].mru.contains(&mid));
}

#[test]
fn replace_item_into_existing_closes_old() {
    let mut g = EditorGroups::default();
    let src = g.open(p("a.md"), OpenMode::Pinned, true);
    let prev = g.open_file(p("a.md"), FileView::Preview, OpenMode::Pinned, true, false);
    g.activate(src);
    let got = g.replace_item(src, TabItem::MarkdownPreview(p("a.md")));
    assert_eq!(got, Some(prev));
    assert_eq!(items(&g, 0), vec![TabItem::MarkdownPreview(p("a.md"))]);
    assert_eq!(g.active_tab().unwrap().id, prev);
}

#[test]
fn dynamic_preview_is_singleton_and_keeps_focus() {
    let mut g = EditorGroups::default();
    g.open(p("a.md"), OpenMode::Pinned, true);
    let id = g.open_dynamic_preview(p("a.md"));
    assert_eq!(g.open_dynamic_preview(p("b.md")), id);
    assert_eq!(g.groups().len(), 2);
    assert_eq!(
        items(&g, 1),
        vec![TabItem::MarkdownPreviewDynamic(p("b.md"))]
    );
    assert_eq!(g.groups()[1].tabs[0].title(), "Preview b.md");
    assert!(g.groups()[1].tabs[0].pinned);
    assert_eq!(g.active_group(), 0);
    assert_eq!(g.dynamic_preview().map(|(i, t)| (i, t.id)), Some((1, id)));
    assert!(!g.retarget_dynamic(p("b.md")));
    assert!(g.retarget_dynamic(p("c.md")));
}

#[test]
fn previews_saved_and_restored() {
    let mut g = EditorGroups::default();
    g.open_file(
        p("/x/a.md"),
        FileView::Preview,
        OpenMode::Pinned,
        true,
        false,
    );
    g.open_dynamic_preview(p("/x/b.md"));
    let state = g.state();
    let saved: Vec<_> = state.groups.iter().flat_map(|g| &g.tabs).collect();
    assert_eq!(
        saved[0].0,
        PathBuf::from(format!("{MARKDOWN_PREVIEW_STATE_PREFIX}/x/a.md"))
    );
    assert_eq!(
        saved[1].0,
        PathBuf::from(format!("{MARKDOWN_PREVIEW_DYNAMIC_STATE_PREFIX}/x/b.md"))
    );
    let restored = EditorGroups::restore(&state);
    assert_eq!(
        items(&restored, 0),
        vec![TabItem::MarkdownPreview(p("/x/a.md"))]
    );
    assert_eq!(
        items(&restored, 1),
        vec![TabItem::MarkdownPreviewDynamic(p("/x/b.md"))]
    );

    let two = GroupsState {
        groups: vec![GroupState {
            tabs: vec![
                (
                    PathBuf::from(format!("{MARKDOWN_PREVIEW_DYNAMIC_STATE_PREFIX}/x/b.md")),
                    true,
                ),
                (
                    PathBuf::from(format!("{MARKDOWN_PREVIEW_DYNAMIC_STATE_PREFIX}/x/c.md")),
                    true,
                ),
            ],
            active: Some(1),
        }],
        active_group: 0,
    };
    let restored = EditorGroups::restore(&two);
    assert_eq!(
        items(&restored, 0),
        vec![TabItem::MarkdownPreviewDynamic(p("/x/b.md"))]
    );
}

#[test]
fn static_preview_has_path_dynamic_has_not() {
    let mut g = EditorGroups::default();
    let s = g.open_file(p("a.md"), FileView::Preview, OpenMode::Pinned, true, false);
    let d = g.open_dynamic_preview(p("a.md"));
    let tab = |id| {
        g.groups()
            .iter()
            .flat_map(|g| &g.tabs)
            .find(|t| t.id == id)
            .unwrap()
    };
    assert_eq!(tab(s).path(), Some(&p("a.md")));
    assert_eq!(tab(s).previewed_path(), Some(&p("a.md")));
    assert_eq!(tab(s).title(), "a.md");
    assert_eq!(tab(d).path(), None);
    assert_eq!(tab(d).previewed_path(), Some(&p("a.md")));
}

#[test]
fn retarget_moves_file_and_children() {
    let mut g = EditorGroups::default();
    let a = g.open(p("/r/a.rs"), OpenMode::Pinned, true);
    let x = g.open(p("/r/src/x.rs"), OpenMode::Pinned, true);
    let srcx = g.open(p("/r/srcx.rs"), OpenMode::Preview, true);
    assert!(g.retarget(Path::new("/r/src"), Path::new("/r/lib")));
    assert_eq!(
        tabs(&g, 0),
        owned(&[
            ("/r/a.rs", true),
            ("/r/lib/x.rs", true),
            ("/r/srcx.rs", false)
        ])
    );
    let ids: Vec<_> = g.groups()[0].tabs.iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![a, x, srcx]);
    // A file itself.
    assert!(g.retarget(Path::new("/r/a.rs"), Path::new("/r/b.rs")));
    assert_eq!(tabs(&g, 0)[0], ("/r/b.rs".to_string(), true));
    assert!(!g.retarget(Path::new("/r/none"), Path::new("/r/other")));
}

/// Renaming a Markdown file moves its static and dynamic previews too.
#[test]
fn retarget_moves_markdown_previews() {
    let mut g = EditorGroups::default();
    g.open_file(
        p("/r/doc/a.md"),
        FileView::Preview,
        OpenMode::Pinned,
        true,
        false,
    );
    g.open_dynamic_preview(p("/r/doc/a.md"));
    assert!(g.retarget(Path::new("/r/doc"), Path::new("/r/notes")));
    assert_eq!(
        items(&g, 0),
        vec![TabItem::MarkdownPreview(p("/r/notes/a.md"))]
    );
    assert_eq!(
        items(&g, 1),
        vec![TabItem::MarkdownPreviewDynamic(p("/r/notes/a.md"))]
    );
}

#[test]
fn file_diff_tabs_are_one_per_file_and_not_saved() {
    let mut g = EditorGroups::default();
    g.open(p("/w/a.rs"), OpenMode::Pinned, true);
    let a = g.open_file_diff(p("a.rs"));
    let b = g.open_file_diff(p("b.rs"));
    assert_ne!(a, b, "one tab per file");
    assert_eq!(g.open_file_diff(p("a.rs")), a, "reopening activates it");
    assert_eq!(g.active_tab().map(|t| t.id), Some(a));
    assert_eq!(g.file_diff_tab(Path::new("b.rs")), Some(b));
    let tab = g.active_tab().unwrap();
    assert!(tab.pinned, "never a preview");
    assert_eq!(tab.title(), "a.rs (Changes)");
    assert!(tab.item.is_singleton());
    let saved = &g.state().groups[0].tabs;
    assert_eq!(saved, &[(PathBuf::from("/w/a.rs"), true)], "not saved");
    assert!(matches!(tab.item, TabItem::FileDiff(_)));
}
