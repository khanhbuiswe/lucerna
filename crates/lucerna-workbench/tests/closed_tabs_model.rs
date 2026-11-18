//! The closed-tab history of the editor groups model (Reopen Closed
//! Editor) and the tab sets the close commands act on.

use std::path::Path;
use std::sync::Arc;

use lucerna_text::Point;
use lucerna_workbench::{EditorGroups, OpenMode, TabItem, TabViewState};

fn p(name: &str) -> Arc<Path> {
    Arc::from(Path::new(name))
}

/// The file names of group `g`'s tabs, in order.
fn names(groups: &EditorGroups, g: usize) -> Vec<String> {
    groups.groups()[g]
        .tabs
        .iter()
        .map(|t| t.path().unwrap().display().to_string())
        .collect()
}

fn active_name(groups: &EditorGroups) -> Option<String> {
    groups
        .active_tab()
        .map(|t| t.path().unwrap().display().to_string())
}

fn closed_names(groups: &EditorGroups) -> Vec<String> {
    groups
        .closed()
        .iter()
        .map(|e| match &e.item {
            TabItem::File(p) | TabItem::MarkdownPreview(p) => p.display().to_string(),
            other => format!("{other:?}"),
        })
        .collect()
}

fn pinned(g: &mut EditorGroups, name: &str) -> lucerna_workbench::TabId {
    g.open(p(name), OpenMode::Pinned, true)
}

#[test]
fn close_records_entry() {
    let mut g = EditorGroups::default();
    pinned(&mut g, "a.rs");
    let b = pinned(&mut g, "b.rs");
    pinned(&mut g, "c.rs");
    g.activate(b);
    g.close(b);
    let closed = g.closed();
    assert_eq!(closed.len(), 1);
    let entry = &closed[0];
    assert_eq!(entry.item, TabItem::File(p("b.rs")));
    assert_eq!(entry.index, 1);
    assert!(entry.was_active);
    assert_eq!(entry.tab, b);
    assert_eq!(entry.view, None);
}

#[test]
fn history_caps_at_20() {
    let mut g = EditorGroups::default();
    for i in 0..25 {
        let id = pinned(&mut g, &format!("f{i}.rs"));
        g.close(id);
    }
    assert_eq!(g.closed().len(), 20);
    assert_eq!(closed_names(&g)[0], "f5.rs");
    assert_eq!(closed_names(&g)[19], "f24.rs");
}

#[test]
fn reclosing_same_item_in_group_keeps_one_entry() {
    let mut g = EditorGroups::default();
    pinned(&mut g, "keep.rs");
    let a = pinned(&mut g, "a.rs");
    g.close(a);
    let a = pinned(&mut g, "a.rs");
    g.close(a);
    assert_eq!(closed_names(&g), ["a.rs"]);
}

#[test]
fn not_recorded() {
    // A preview replaced by another preview.
    let mut g = EditorGroups::default();
    g.open(p("a.rs"), OpenMode::Preview, true);
    g.open(p("b.rs"), OpenMode::Preview, true);
    assert!(g.closed().is_empty(), "replaced preview");

    // A tab replaced in place (source ↔ Markdown preview).
    let mut g = EditorGroups::default();
    let id = pinned(&mut g, "/w/a.md");
    g.replace_item(id, TabItem::MarkdownPreview(p("/w/a.md")));
    assert!(g.closed().is_empty(), "replace_item");

    // An Untitled document.
    let mut g = EditorGroups::default();
    let id = g.open(
        lucerna_workbench::untitled::path(1),
        OpenMode::Pinned,
        true,
    );
    g.close(id);
    assert!(g.closed().is_empty(), "untitled");

    // A tab that is not a file.
    let mut g = EditorGroups::default();
    let id = g.open_search();
    g.close(id);
    assert!(g.closed().is_empty(), "search tab");

    // A close made by code.
    let mut g = EditorGroups::default();
    let id = pinned(&mut g, "a.rs");
    g.remove(id);
    assert!(g.closed().is_empty(), "remove");
    assert!(g.groups()[0].tabs.is_empty());
}

#[test]
fn static_markdown_preview_is_recorded() {
    let mut g = EditorGroups::default();
    let id = g.open_file(
        p("/w/a.md"),
        lucerna_workbench::FileView::Preview,
        OpenMode::Pinned,
        true,
        false,
    );
    g.close(id);
    assert_eq!(g.closed()[0].item, TabItem::MarkdownPreview(p("/w/a.md")));
}

#[test]
fn close_many_is_one_batch_with_pre_batch_indexes() {
    let mut g = EditorGroups::default();
    let a = pinned(&mut g, "A");
    let b = pinned(&mut g, "B");
    let c = pinned(&mut g, "C");
    let d = pinned(&mut g, "D");
    g.activate(c);
    g.close_many(&[a, b, d]);
    assert_eq!(names(&g, 0), ["C"]);
    let closed = g.closed();
    let indexes: Vec<usize> = closed.iter().map(|e| e.index).collect();
    assert_eq!(indexes, [0, 1, 3]);
    assert!(closed.iter().all(|e| e.batch == closed[0].batch));
    assert!(closed.iter().all(|e| !e.was_active));
}

#[test]
fn reopen_restores_batch_in_place() {
    let mut g = EditorGroups::default();
    let a = pinned(&mut g, "A");
    let b = pinned(&mut g, "B");
    let c = pinned(&mut g, "C");
    let d = pinned(&mut g, "D");
    g.activate(c);
    g.close_many(&[a, b, d]);
    let reopened = g.reopen_closed(|_| true);
    assert_eq!(reopened.len(), 3);
    assert_eq!(names(&g, 0), ["A", "B", "C", "D"]);
    assert!(g.groups()[0].tabs.iter().all(|t| t.pinned));
    assert_eq!(active_name(&g).as_deref(), Some("C"));
    assert!(g.closed().is_empty());
}

#[test]
fn reopen_single_activates_it() {
    let mut g = EditorGroups::default();
    let a = pinned(&mut g, "a");
    let b = pinned(&mut g, "b");
    pinned(&mut g, "c");
    g.activate(b);
    g.close(b);
    g.activate(a);
    g.reopen_closed(|_| true);
    assert_eq!(names(&g, 0), ["a", "b", "c"]);
    assert_eq!(active_name(&g).as_deref(), Some("b"));
}

#[test]
fn reopen_preview_comes_back_pinned() {
    let mut g = EditorGroups::default();
    pinned(&mut g, "a");
    let b = g.open(p("b"), OpenMode::Preview, true);
    g.close(b);
    g.reopen_closed(|_| true);
    assert!(g.groups()[0].tabs.iter().all(|t| t.pinned));
}

#[test]
fn reopen_goes_to_old_group() {
    let mut g = EditorGroups::default();
    let a = pinned(&mut g, "a");
    pinned(&mut g, "b");
    g.activate(a);
    g.split_active(); // group 1 holds a copy of a, and is active
    pinned(&mut g, "c");
    let b0 = g.groups()[0].tabs[1].id;
    g.close(b0);
    assert_eq!(g.active_group(), 1);
    g.reopen_closed(|_| true);
    assert_eq!(names(&g, 0), ["a", "b"]);
    assert_eq!(names(&g, 1), ["a", "c"]);
    assert_eq!(g.active_group(), 0);
    assert_eq!(active_name(&g).as_deref(), Some("b"));
}

#[test]
fn reopen_into_missing_group_goes_to_active_group() {
    let mut g = EditorGroups::default();
    let a = pinned(&mut g, "a");
    pinned(&mut g, "b");
    g.activate(a);
    g.split_active();
    let copy = g.active_tab().unwrap().id;
    pinned(&mut g, "c");
    g.close(copy); // group 1: [c]
    let ids: Vec<_> = g
        .groups()
        .iter()
        .flat_map(|gr| gr.tabs.iter().map(|t| t.id))
        .collect();
    g.close_many(&ids);
    assert_eq!(g.groups().len(), 1);
    assert!(g.groups()[0].tabs.is_empty());
    let reopened = g.reopen_closed(|_| true);
    assert_eq!(reopened.len(), 3);
    let mut all = names(&g, 0);
    assert_eq!(all.len(), 3);
    // a and b keep their order; c (index 0 of a group now gone) lands
    // where its index puts it.
    let a_at = all.iter().position(|n| n == "a").unwrap();
    let b_at = all.iter().position(|n| n == "b").unwrap();
    assert!(a_at < b_at);
    all.sort();
    assert_eq!(all, ["a", "b", "c"]);
}

#[test]
fn reopen_skips_missing_files_and_falls_back_a_batch() {
    let mut g = EditorGroups::default();
    pinned(&mut g, "keep.rs");
    let x = pinned(&mut g, "x.rs");
    g.close(x);
    let y = pinned(&mut g, "y.rs");
    let z = pinned(&mut g, "z.rs");
    g.close_many(&[y, z]);
    let reopened = g.reopen_closed(|p| p != Path::new("z.rs"));
    assert_eq!(reopened.len(), 1);
    assert_eq!(names(&g, 0), ["keep.rs", "y.rs"]);

    // A batch whose files are all gone gives way to the one before.
    let mut g = EditorGroups::default();
    pinned(&mut g, "keep.rs");
    let x = pinned(&mut g, "x.rs");
    g.close(x);
    let z = pinned(&mut g, "z.rs");
    g.close(z);
    let reopened = g.reopen_closed(|p| p != Path::new("z.rs"));
    assert_eq!(reopened.len(), 1);
    assert_eq!(names(&g, 0), ["keep.rs", "x.rs"]);

    // Nothing left: nothing happens.
    assert!(g.reopen_closed(|_| true).is_empty());
    assert_eq!(names(&g, 0), ["keep.rs", "x.rs"]);
}

#[test]
fn reopen_skips_tab_already_open_in_group() {
    let mut g = EditorGroups::default();
    pinned(&mut g, "keep.rs");
    let a = pinned(&mut g, "a.rs");
    g.close(a);
    pinned(&mut g, "a.rs");
    assert!(g.reopen_closed(|_| true).is_empty());
    assert_eq!(names(&g, 0), ["keep.rs", "a.rs"]);
    assert!(g.closed().is_empty());
}

#[test]
fn retarget_updates_history() {
    let mut g = EditorGroups::default();
    pinned(&mut g, "/w/keep.rs");
    let a = pinned(&mut g, "/w/a.rs");
    g.close(a);
    g.retarget(Path::new("/w/a.rs"), Path::new("/w/b.rs"));
    assert_eq!(g.closed()[0].item, TabItem::File(p("/w/b.rs")));
}

#[test]
fn set_closed_view_attaches_state() {
    let mut g = EditorGroups::default();
    pinned(&mut g, "keep.rs");
    let a = pinned(&mut g, "a.rs");
    g.close(a);
    let state = TabViewState {
        cursor: Point { row: 3, column: 2 },
        scroll_top_row: 1,
    };
    g.set_closed_view(a, state);
    let reopened = g.reopen_closed(|_| true);
    assert_eq!(reopened.len(), 1);
    assert_eq!(reopened[0].1, Some(state));
}

#[test]
fn tab_sets() {
    let mut g = EditorGroups::default();
    let a = pinned(&mut g, "a");
    let b = pinned(&mut g, "b");
    let c = pinned(&mut g, "c");
    let d = pinned(&mut g, "d");
    g.activate(d);
    g.split_active(); // group 1: [d copy]
    let e = g.active_tab().unwrap().id;
    assert_eq!(g.others(c), [a, b, d]);
    assert_eq!(g.to_the_right(c), [d]);
    assert_eq!(g.to_the_left(c), [a, b]);
    assert_eq!(g.group_tabs(c), [a, b, c, d]);
    assert!(g.to_the_right(d).is_empty());
    assert!(g.others(e).is_empty());
    assert_eq!(g.group_tabs(e), [e]);
    g.close(c);
    assert!(g.others(c).is_empty());
    assert!(g.to_the_right(c).is_empty());
    assert!(g.to_the_left(c).is_empty());
    assert!(g.group_tabs(c).is_empty());
}
