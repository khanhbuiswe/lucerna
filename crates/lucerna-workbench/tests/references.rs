//! Find references (spec §7.5): ⇧F12 lists several results in the
//! locations picker; ⇧⌥F12 shows them in the References tab, an excerpt
//! list like the Search tab's.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_lsp::position::path_to_uri;
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_text::Point;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{ReferencesView, TabItem, Workbench};
use serde_json::{Value, json};
use support::lsp::{
    TRUSTED, active_view, editor_text_focus, focus_view, lsp_workbench_with, open, sent, settle,
};
use tempfile::TempDir;

const MAIN: &str = "fn main() {\n    b::helper();\n    b::helper();\n}\n";
const B: &str = "// b\npub fn helper() {}\npub fn other() {}\n";
const REFERENCES: &str = "textDocument/references";

fn rust_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), MAIN).unwrap();
    fs::write(dir.path().join("src/b.rs"), B).unwrap();
    dir
}

fn location(dir: &Path, name: &str, line: u32, start: u32, end: u32) -> Value {
    json!({
        "uri": path_to_uri(&dir.join("src").join(name)).as_str(),
        "range": { "start": { "line": line, "character": start },
                   "end": { "line": line, "character": end } },
    })
}

fn pt(row: u32, column: u32) -> Point {
    Point { row, column }
}

/// `helper` is used twice in main.rs and defined in b.rs; `main` has only
/// its definition.
fn references(dir: &Path) -> FakeServer {
    let helper = json!([
        location(dir, "main.rs", 1, 7, 13),
        location(dir, "main.rs", 2, 7, 13),
        location(dir, "b.rs", 1, 7, 13),
    ]);
    let main = json!([location(dir, "main.rs", 0, 3, 7)]);
    FakeServer::new().on_request(REFERENCES, move |params| {
        if params["position"]["line"] == 0 {
            Ok(main.clone())
        } else {
            Ok(helper.clone())
        }
    })
}

/// The project, a workbench on it with main.rs open and the cursor on
/// the first `helper`.
fn setup(
    cx: &mut TestAppContext,
) -> (
    TempDir,
    FakeServer,
    Entity<Workbench>,
    Entity<CodeView>,
    &mut VisualTestContext,
) {
    let dir = rust_project();
    let fake = references(dir.path());
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher, TRUSTED, cx);
    let view = open(&wb, "src/main.rs", cx);
    view.update(cx, |v, cx| v.set_cursor(pt(1, 8), cx));
    settle(cx);
    (dir, fake, wb, view, cx)
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    settle(cx);
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(wb.run_command(command, None, window, cx), "{command}");
        })
    });
    settle(cx);
}

fn refs_view(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<ReferencesView> {
    cx.update(|_, cx| wb.read(cx).references_view().clone())
}

fn active_item(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<TabItem> {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups.model().active_tab().map(|t| t.item.clone())
    })
}

fn references_tabs(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups
            .model()
            .groups()
            .iter()
            .flat_map(|g| &g.tabs)
            .filter(|t| t.item == TabItem::References)
            .count()
    })
}

fn tab_title(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups.references_title().map(|t| t.to_string())
    })
}

/// The active editor's file name and cursor.
fn at(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> (String, Point) {
    let view = active_view(wb, cx);
    view.read_with(cx, |v, _| {
        let name = v.path().file_name().unwrap().to_string_lossy().into_owned();
        (name, v.cursor())
    })
}

/// `(path, [(line number, text, ranges)])` of each file shown.
type Shown = Vec<(PathBuf, Vec<(u32, String, Vec<std::ops::Range<usize>>)>)>;

fn shown(view: &Entity<ReferencesView>, cx: &mut VisualTestContext) -> Shown {
    cx.update(|_, cx| {
        view.read(cx)
            .results(cx)
            .files()
            .iter()
            .map(|f| {
                let lines = f
                    .excerpts
                    .iter()
                    .flat_map(|e| &e.lines)
                    .map(|l| (l.line_number, l.line_text.clone(), l.ranges.clone()))
                    .collect();
                (f.path.to_path_buf(), lines)
            })
            .collect()
    })
}

#[gpui::test]
fn shift_f12_multiple_opens_picker(cx: &mut TestAppContext) {
    let (_dir, fake, wb, _view, cx) = setup(cx);
    keys("shift-f12", cx);
    let requests = sent(&fake, REFERENCES);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["context"]["includeDeclaration"], true);
    let picker = cx
        .update(|_, cx| wb.read(cx).locations_picker().cloned())
        .expect("several references open the picker");
    let rows = picker.read_with(cx, |p, _| p.row_texts());
    assert_eq!(
        rows.iter().filter(|r| !r.starts_with("header:")).count(),
        3,
        "{rows:?}"
    );
    // Nothing jumped and no tab opened.
    assert_eq!(at(&wb, cx), ("main.rs".into(), pt(1, 8)));
    assert_eq!(references_tabs(&wb, cx), 0);
}

#[gpui::test]
fn shift_alt_f12_opens_single_tab_reused(cx: &mut TestAppContext) {
    let (_dir, fake, wb, view, cx) = setup(cx);
    keys("shift-alt-f12", cx);
    assert_eq!(active_item(&wb, cx), Some(TabItem::References));
    assert_eq!(references_tabs(&wb, cx), 1);
    assert_eq!(tab_title(&wb, cx).as_deref(), Some("References to helper"));
    assert!(cx.debug_bounds("references-view").is_some(), "drawn");

    // Again from `main`: the same tab, re-titled, with the new results.
    wb.update(cx, |wb, cx| {
        wb.open_path(
            Arc::from(Path::new("src/main.rs")),
            lucerna_workbench::OpenMode::Pinned,
            cx,
        )
    });
    settle(cx);
    view.update(cx, |v, cx| v.set_cursor(pt(0, 4), cx));
    settle(cx);
    keys("shift-alt-f12", cx);
    assert_eq!(sent(&fake, REFERENCES).len(), 2);
    assert_eq!(references_tabs(&wb, cx), 1);
    assert_eq!(active_item(&wb, cx), Some(TabItem::References));
    assert_eq!(tab_title(&wb, cx).as_deref(), Some("References to main"));
    let refs = refs_view(&wb, cx);
    assert_eq!(
        cx.update(|_, cx| refs.read(cx).summary()),
        Some("1 reference in 1 file".into())
    );

    // The tab is never saved with the workspace.
    let saved = cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).model().state());
    let tabs: Vec<&PathBuf> = saved
        .groups
        .iter()
        .flat_map(|g| &g.tabs)
        .map(|(p, _)| p)
        .collect();
    assert!(tabs.iter().all(|p| p.is_absolute()), "{tabs:?}");
}

#[gpui::test]
fn summary_and_headers(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(cx);
    run(&wb, "references-view.findReferences", cx);
    let refs = refs_view(&wb, cx);
    assert_eq!(
        cx.update(|_, cx| refs.read(cx).summary()),
        Some("3 references in 2 files".into())
    );
    assert!(cx.debug_bounds("references-summary").is_some());
    assert!(cx.debug_bounds("references-file-0").is_some(), "headers");
    assert!(cx.debug_bounds("references-file-1").is_some(), "headers");
    let ctx = |n: u32, text: &str| (n, text.to_owned(), Vec::new());
    // Every `helper` spans bytes 7..13 of its line.
    let hit = |n: u32, text: &str| (n, text.to_owned(), std::iter::once(7..13).collect());
    assert_eq!(
        shown(&refs, cx),
        [
            (
                PathBuf::from("src/b.rs"),
                vec![
                    ctx(1, "// b"),
                    hit(2, "pub fn helper() {}"),
                    ctx(3, "pub fn other() {}"),
                ]
            ),
            (
                PathBuf::from("src/main.rs"),
                // Both uses and their context lines in one excerpt.
                vec![
                    ctx(1, "fn main() {"),
                    hit(2, "    b::helper();"),
                    hit(3, "    b::helper();"),
                    ctx(4, "}"),
                ]
            ),
        ]
    );
    // The collapse-all button folds every file.
    let bounds = cx
        .debug_bounds("references-collapse-all")
        .expect("collapse-all drawn");
    cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    settle(cx);
    let folded = cx.update(|_, cx| {
        refs.read(cx)
            .results(cx)
            .files()
            .iter()
            .all(|f| f.collapsed)
    });
    assert!(folded);
}

#[gpui::test]
fn enter_opens_reference(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(cx);
    keys("shift-alt-f12", cx);
    // The list has the focus: the second line is b.rs's `helper`.
    keys("down down", cx);
    keys("enter", cx);
    assert_eq!(at(&wb, cx), ("b.rs".into(), pt(1, 7)));
    // A new lookup starts with nothing selected; a context line opens at
    // its start.
    run(&wb, "references-view.findReferences", cx);
    keys("down", cx);
    keys("enter", cx);
    assert_eq!(at(&wb, cx), ("b.rs".into(), pt(0, 0)));
}

#[gpui::test]
fn f4_steps_references(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(cx);
    keys("shift-alt-f12", cx);
    keys("f4", cx);
    assert_eq!(at(&wb, cx), ("b.rs".into(), pt(1, 7)));
    // The editor has the focus now; F4 still walks the references.
    keys("f4", cx);
    assert_eq!(at(&wb, cx), ("main.rs".into(), pt(1, 7)));
    keys("f4", cx);
    assert_eq!(at(&wb, cx), ("main.rs".into(), pt(2, 7)));
    keys("f4", cx);
    assert_eq!(at(&wb, cx), ("b.rs".into(), pt(1, 7)), "wraps");
    keys("shift-f4", cx);
    assert_eq!(at(&wb, cx), ("main.rs".into(), pt(2, 7)));
    // Opened references are in the navigation history.
    run(&wb, "workbench.action.navigateBack", cx);
    assert_eq!(at(&wb, cx), ("b.rs".into(), pt(1, 7)));
}

#[gpui::test]
fn search_tab_unaffected(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(cx);
    keys("shift-alt-f12", cx);
    assert!(cx.debug_bounds("references-file-0").is_some());

    run(&wb, "workbench.action.findInFiles", cx);
    cx.simulate_input("other");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(150));
    settle(cx);
    assert_eq!(active_item(&wb, cx), Some(TabItem::Search));
    assert_eq!(references_tabs(&wb, cx), 1);
    let search = cx.update(|_, cx| wb.read(cx).search_view().clone());
    let files: Vec<PathBuf> = cx.update(|_, cx| {
        search
            .read(cx)
            .results(cx)
            .files()
            .iter()
            .map(|f| f.path.to_path_buf())
            .collect()
    });
    assert_eq!(files, [PathBuf::from("src/b.rs")]);
    // Each list draws under its own selectors.
    assert!(cx.debug_bounds("search-file-0").is_some());
    assert!(cx.debug_bounds("references-file-0").is_none());
    // F4 from the Search tab walks the search results.
    keys("f4", cx);
    assert_eq!(at(&wb, cx), ("b.rs".into(), pt(2, 7)));
    // The references are still there.
    let refs = refs_view(&wb, cx);
    assert_eq!(
        cx.update(|_, cx| refs.read(cx).summary()),
        Some("3 references in 2 files".into())
    );
}

#[gpui::test]
fn refresh_after_origin_preview_replaced(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = references(dir.path());
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher, TRUSTED, cx);
    // main.rs in the preview tab.
    wb.update(cx, |wb, cx| {
        wb.open_path(
            Arc::from(Path::new("src/main.rs")),
            lucerna_workbench::OpenMode::Preview,
            cx,
        )
    });
    settle(cx);
    let view = active_view(&wb, cx);
    view.update(cx, |v, cx| v.set_cursor(pt(1, 8), cx));
    drop(view);
    settle(cx);
    keys("shift-alt-f12", cx);
    // b.rs's `helper` takes main.rs's preview tab.
    keys("down down enter", cx);
    assert_eq!(at(&wb, cx), ("b.rs".into(), pt(1, 7)));
    let main_tab = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups
            .model()
            .groups()
            .iter()
            .flat_map(|g| &g.tabs)
            .any(|t| t.path().is_some_and(|p| p.ends_with("main.rs")))
    });
    assert!(!main_tab, "main.rs's tab was replaced");

    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| m.open_references())
    });
    settle(cx);
    let bounds = cx
        .debug_bounds("references-refresh")
        .expect("refresh drawn");
    cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    settle(cx);
    assert_eq!(sent(&fake, REFERENCES).len(), 2, "asked again");
    let refs = refs_view(&wb, cx);
    assert_eq!(
        cx.update(|_, cx| refs.read(cx).summary()),
        Some("3 references in 2 files".into())
    );
}

#[gpui::test]
fn refresh_without_server_keeps_results(cx: &mut TestAppContext) {
    let (_dir, fake, wb, _view, cx) = setup(cx);
    keys("shift-alt-f12", cx);
    let servers = support::lsp::language_servers(&wb, cx);
    servers.update(cx, |servers, cx| servers.stop_all(cx));
    settle(cx);
    let refs = refs_view(&wb, cx);
    refs.update(cx, |view, cx| view.refresh(cx));
    settle(cx);
    assert_eq!(sent(&fake, REFERENCES).len(), 1);
    assert_eq!(
        cx.update(|_, cx| refs.read(cx).summary()),
        Some("3 references in 2 files".into())
    );
    assert_eq!(shown(&refs, cx).len(), 2);
}

/// Whether `referencesViewFocus` holds.
fn references_focus(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    let ctx = cx.update(|window, cx| wb.read(cx).context_keys(window, cx));
    lucerna_settings::parse_when("referencesViewFocus")
        .unwrap()
        .eval(&ctx)
}

#[gpui::test]
fn f4_follows_the_results_tab_used_last(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(cx);
    keys("shift-alt-f12", cx);
    run(&wb, "workbench.action.findInFiles", cx);
    cx.simulate_input("b::");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(150));
    settle(cx);
    // Search's lines are `b::helper` at column 4.
    keys("f4", cx);
    assert_eq!(at(&wb, cx), ("main.rs".into(), pt(1, 4)));
    assert!(!references_focus(&wb, cx), "an editor has the focus");
    keys("f4", cx);
    assert_eq!(at(&wb, cx), ("main.rs".into(), pt(2, 4)), "Search's next");

    // A line opened from the References tab hands F4 to it.
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| m.open_references())
    });
    settle(cx);
    let refs = refs_view(&wb, cx);
    let focus = cx.update(|_, cx| refs.read(cx).list_focus_handle().clone());
    cx.update(|window, cx| window.focus(&focus, cx));
    settle(cx);
    keys("down down enter", cx);
    assert_eq!(at(&wb, cx), ("b.rs".into(), pt(1, 7)));
    assert!(references_focus(&wb, cx));
    keys("f4", cx);
    assert_eq!(
        at(&wb, cx),
        ("main.rs".into(), pt(1, 7)),
        "references' next"
    );
}

#[gpui::test]
fn second_lookup_replaces_one_still_running(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, view, cx) = setup(cx);
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(wb.run_command("references-view.findReferences", None, window, cx))
        })
    });
    // Before the first answer: a lookup on `main`.
    let main = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups
            .model()
            .groups()
            .iter()
            .flat_map(|g| &g.tabs)
            .find(|t| t.path().is_some_and(|p| p.ends_with("main.rs")))
            .map(|t| t.id)
            .unwrap()
    });
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| m.activate(main))
    });
    view.update(cx, |v, cx| v.set_cursor(pt(0, 4), cx));
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(wb.run_command("references-view.findReferences", None, window, cx))
        })
    });
    settle(cx);
    let refs = refs_view(&wb, cx);
    assert_eq!(
        cx.update(|_, cx| refs.read(cx).summary()),
        Some("1 reference in 1 file".into())
    );
    let files: Vec<PathBuf> = shown(&refs, cx).into_iter().map(|(p, _)| p).collect();
    assert_eq!(files, [PathBuf::from("src/main.rs")]);
    assert_eq!(tab_title(&wb, cx).as_deref(), Some("References to main"));
}

#[gpui::test]
fn references_enter_leaves_working_focus(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, view, cx) = setup(cx);
    focus_view(&view, cx);
    keys("shift-alt-f12", cx);
    keys("down down", cx);
    keys("enter", cx);
    assert_eq!(at(&wb, cx), ("b.rs".into(), pt(1, 7)));
    assert!(
        editor_text_focus(&wb, cx) || references_focus(&wb, cx),
        "focus is on something shown"
    );
}

#[gpui::test]
fn new_lookup_without_server_does_not_keep_old_count(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(cx);
    keys("shift-alt-f12", cx);
    let refs = refs_view(&wb, cx);
    assert_eq!(
        cx.update(|_, cx| refs.read(cx).summary()),
        Some("3 references in 2 files".into())
    );
    let servers = support::lsp::language_servers(&wb, cx);
    servers.update(cx, |servers, cx| servers.stop_all(cx));
    settle(cx);
    // A new lookup (another word) with no server to ask.
    let view = open(&wb, "src/main.rs", cx);
    view.update(cx, |v, cx| v.set_cursor(pt(0, 4), cx));
    settle(cx);
    run(&wb, "references-view.findReferences", cx);
    let summary = cx.update(|_, cx| refs.read(cx).summary());
    assert!(
        summary
            .as_deref()
            .is_none_or(|s| s.starts_with("No references found")),
        "{summary:?}"
    );
    assert_eq!(shown(&refs, cx).len(), 0);
}
