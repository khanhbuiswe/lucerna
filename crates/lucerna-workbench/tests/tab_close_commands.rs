//! Closing many tabs at once, Reopen Closed Editor, the close commands
//! and the tab bar's right-click menu, through the workbench.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, TabId, TabViewState, Workbench};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
        languages_generation: 0,
        icon_themes: lucerna_theme::IconThemeRegistry::build(&[]).0,
        icons: lucerna_theme::IconThemeRegistry::build(&[])
            .0
            .resolve("Lucide"),
    });
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn settle(cx: &mut VisualTestContext) {
    draw(cx);
    cx.run_until_parked();
    draw(cx);
}

/// Forty numbered lines.
fn long_text() -> String {
    (0..40).map(|i| format!("line {i}\n")).collect()
}

/// A workbench on a temporary folder holding `names`, each with
/// [`long_text`], none open.
fn workspace<'a>(
    names: &[&str],
    cx: &'a mut TestAppContext,
) -> (TempDir, Entity<Workbench>, &'a mut VisualTestContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    for name in names {
        fs::write(dir.path().join(name), long_text()).unwrap();
    }
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    (dir, wb, cx)
}

fn active_view(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<CodeView> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("the active tab has a body")
            .downcast::<CodeView>()
            .expect("tab bodies are code views")
    })
}

/// Opens `name` (pinned unless `mode` says otherwise) and focuses it.
fn open_as(
    wb: &Entity<Workbench>,
    name: &str,
    mode: OpenMode,
    cx: &mut VisualTestContext,
) -> Entity<CodeView> {
    wb.update(cx, |wb, cx| wb.open_path(Arc::from(Path::new(name)), mode, cx));
    settle(cx);
    let view = active_view(wb, cx);
    cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    settle(cx);
    view
}

fn open(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) -> Entity<CodeView> {
    open_as(wb, name, OpenMode::Pinned, cx)
}

fn input(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    settle(cx);
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    settle(cx);
}

fn run(
    wb: &Entity<Workbench>,
    command: &str,
    args: Option<serde_json::Value>,
    cx: &mut VisualTestContext,
) {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command(command, args.as_ref(), window, cx)
        })
    });
    settle(cx);
}

fn is_dirty(wb: &Entity<Workbench>, dir: &TempDir, name: &str, cx: &mut VisualTestContext) -> bool {
    let docs = cx.update(|_, cx| wb.read(cx).documents().clone());
    let path = dir.path().join(name);
    docs.read_with(cx, |d, _| d.is_dirty(&path))
}

/// The file names of every tab, group by group.
fn tabs(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<Vec<String>> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .groups()
            .iter()
            .map(|g| {
                g.tabs
                    .iter()
                    .map(|t| {
                        let path = t.path().unwrap();
                        path.file_name().unwrap().to_string_lossy().into_owned()
                    })
                    .collect()
            })
            .collect()
    })
}

/// The id of tab `ti` of group `gi`.
fn tab_id(wb: &Entity<Workbench>, gi: usize, ti: usize, cx: &mut VisualTestContext) -> TabId {
    cx.update(|_, cx| {
        wb.read(cx).editor_groups().read(cx).model().groups()[gi].tabs[ti].id
    })
}

fn active_name(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        let tab = groups.model().active_tab()?;
        Some(
            tab.path()?
                .file_name()?
                .to_string_lossy()
                .into_owned(),
        )
    })
}

fn closed_len(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).model().closed().len())
}

fn request_close_tabs(wb: &Entity<Workbench>, ids: Vec<TabId>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let groups = wb.read(cx).editor_groups().clone();
        groups.update(cx, |view, cx| view.request_close_tabs(ids, window, cx));
    });
    settle(cx);
}

/// Opens a.txt, b.txt and c.txt, with a and b changed.
fn two_dirty_one_clean(
    cx: &mut TestAppContext,
) -> (TempDir, Entity<Workbench>, &mut VisualTestContext) {
    let (dir, wb, cx) = workspace(&["a.txt", "b.txt", "c.txt"], cx);
    open(&wb, "a.txt", cx);
    input("x", cx);
    open(&wb, "b.txt", cx);
    input("y", cx);
    open(&wb, "c.txt", cx);
    (dir, wb, cx)
}

fn all_ids(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<TabId> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .groups()
            .iter()
            .flat_map(|g| g.tabs.iter().map(|t| t.id))
            .collect()
    })
}

#[gpui::test]
fn closing_tab_captures_cursor_and_scroll(cx: &mut TestAppContext) {
    let (_dir, wb, cx) = workspace(&["a.txt", "b.txt"], cx);
    open(&wb, "a.txt", cx);
    let view = open(&wb, "b.txt", cx);
    cx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.set_cursor(Point { row: 5, column: 0 }, cx);
            v.set_scroll_top_row(2, cx);
        })
    });
    settle(cx);
    drop(view);
    run(&wb, "workbench.action.closeActiveEditor", None, cx);
    let view = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups.model().closed().back().and_then(|e| e.view)
    });
    assert_eq!(
        view,
        Some(TabViewState {
            cursor: Point { row: 5, column: 0 },
            scroll_top_row: 2,
        })
    );
}

#[gpui::test]
fn close_many_dirty_asks_once(cx: &mut TestAppContext) {
    let (dir, wb, cx) = two_dirty_one_clean(cx);
    let ids = all_ids(&wb, cx);
    request_close_tabs(&wb, ids, cx);
    let (message, _) = cx.pending_prompt().expect("a prompt");
    assert_eq!(
        message,
        "Do you want to save the changes to the following 2 files?"
    );
    assert_eq!(tabs(&wb, cx).concat().len(), 3, "open until answered");
    cx.simulate_prompt_answer("Save All");
    settle(cx);
    assert!(cx.pending_prompt().is_none(), "asked once");
    assert!(fs::read_to_string(dir.path().join("a.txt")).unwrap().starts_with('x'));
    assert!(fs::read_to_string(dir.path().join("b.txt")).unwrap().starts_with('y'));
    assert!(tabs(&wb, cx).concat().is_empty());
    let batches: Vec<u64> = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups.model().closed().iter().map(|e| e.batch).collect()
    });
    assert_eq!(batches.len(), 3);
    assert!(batches.iter().all(|b| *b == batches[0]));
}

#[gpui::test]
fn dont_save_closes_and_discards(cx: &mut TestAppContext) {
    let (dir, wb, cx) = two_dirty_one_clean(cx);
    let ids = all_ids(&wb, cx);
    request_close_tabs(&wb, ids, cx);
    cx.simulate_prompt_answer("Don't Save");
    settle(cx);
    assert!(tabs(&wb, cx).concat().is_empty());
    assert!(!is_dirty(&wb, &dir, "a.txt", cx));
    assert!(!is_dirty(&wb, &dir, "b.txt", cx));
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), long_text());
}

#[gpui::test]
fn cancel_closes_nothing_and_records_nothing(cx: &mut TestAppContext) {
    let (dir, wb, cx) = two_dirty_one_clean(cx);
    let ids = all_ids(&wb, cx);
    request_close_tabs(&wb, ids, cx);
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert_eq!(tabs(&wb, cx), [["a.txt", "b.txt", "c.txt"]]);
    assert_eq!(closed_len(&wb, cx), 0);
    assert!(is_dirty(&wb, &dir, "a.txt", cx));
}

#[gpui::test]
fn close_others_does_not_ask_for_document_shown_elsewhere(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workspace(&["a.txt", "b.txt"], cx);
    open(&wb, "b.txt", cx);
    open(&wb, "a.txt", cx);
    input("x", cx);
    run(&wb, "workbench.action.splitEditor", None, cx);
    assert_eq!(tabs(&wb, cx), [vec!["b.txt", "a.txt"], vec!["a.txt"]]);
    let group0: Vec<TabId> = vec![tab_id(&wb, 0, 0, cx), tab_id(&wb, 0, 1, cx)];
    request_close_tabs(&wb, group0, cx);
    assert!(cx.pending_prompt().is_none());
    assert_eq!(tabs(&wb, cx), [["a.txt"]]);
    assert!(is_dirty(&wb, &dir, "a.txt", cx));
}

#[gpui::test]
fn reopen_restores_tab_cursor_and_scroll(cx: &mut TestAppContext) {
    let (_dir, wb, cx) = workspace(&["a.txt", "b.txt", "c.txt"], cx);
    open(&wb, "a.txt", cx);
    open(&wb, "b.txt", cx);
    open(&wb, "c.txt", cx);
    // Back to b, the middle tab.
    let view = open(&wb, "b.txt", cx);
    cx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.set_cursor(Point { row: 12, column: 3 }, cx);
            v.set_scroll_top_row(4, cx);
        })
    });
    settle(cx);
    drop(view);
    assert_eq!(tabs(&wb, cx), [["a.txt", "b.txt", "c.txt"]]);
    run(&wb, "workbench.action.closeActiveEditor", None, cx);
    assert_eq!(tabs(&wb, cx), [["a.txt", "c.txt"]]);
    keys("cmd-shift-t", cx);
    assert_eq!(tabs(&wb, cx), [["a.txt", "b.txt", "c.txt"]]);
    assert_eq!(active_name(&wb, cx).as_deref(), Some("b.txt"));
    let view = active_view(&wb, cx);
    let (cursor, top) = view.read_with(cx, |v, _| (v.cursor(), v.scroll_top_row()));
    assert_eq!(cursor, Point { row: 12, column: 3 });
    assert_eq!(top, 4);
}

#[gpui::test]
fn reopen_walks_back_and_stops(cx: &mut TestAppContext) {
    let (_dir, wb, cx) = workspace(&["a.txt", "b.txt", "c.txt"], cx);
    open(&wb, "a.txt", cx);
    open(&wb, "b.txt", cx);
    open(&wb, "c.txt", cx);
    run(&wb, "workbench.action.closeActiveEditor", None, cx); // c
    run(&wb, "workbench.action.closeActiveEditor", None, cx); // b
    assert_eq!(tabs(&wb, cx), [["a.txt"]]);
    keys("cmd-shift-t", cx);
    assert_eq!(tabs(&wb, cx), [["a.txt", "b.txt"]]);
    keys("cmd-shift-t", cx);
    assert_eq!(tabs(&wb, cx), [["a.txt", "b.txt", "c.txt"]]);
    keys("cmd-shift-t", cx);
    assert_eq!(tabs(&wb, cx), [["a.txt", "b.txt", "c.txt"]]);
}

#[gpui::test]
fn reopen_skips_deleted_file(cx: &mut TestAppContext) {
    let (dir, wb, cx) = workspace(&["a.txt", "b.txt"], cx);
    open(&wb, "a.txt", cx);
    open(&wb, "b.txt", cx);
    run(&wb, "workbench.action.closeActiveEditor", None, cx);
    fs::remove_file(dir.path().join("b.txt")).unwrap();
    keys("cmd-shift-t", cx);
    assert_eq!(tabs(&wb, cx), [["a.txt"]]);
}

#[gpui::test]
fn close_others_key_and_pin(cx: &mut TestAppContext) {
    let (_dir, wb, cx) = workspace(&["a.txt", "b.txt", "c.txt"], cx);
    open(&wb, "a.txt", cx);
    open(&wb, "b.txt", cx);
    open_as(&wb, "c.txt", OpenMode::Preview, cx);
    keys("alt-cmd-t", cx);
    assert_eq!(tabs(&wb, cx), [["c.txt"]]);
    let pinned = cx.update(|_, cx| {
        wb.read(cx).editor_groups().read(cx).model().groups()[0].tabs[0].pinned
    });
    assert!(pinned);
    assert_eq!(active_name(&wb, cx).as_deref(), Some("c.txt"));
    keys("cmd-shift-t", cx);
    assert_eq!(tabs(&wb, cx), [["a.txt", "b.txt", "c.txt"]]);
    assert_eq!(active_name(&wb, cx).as_deref(), Some("c.txt"));
}

#[gpui::test]
fn close_to_right_and_left_with_tab_arg(cx: &mut TestAppContext) {
    let (_dir, wb, cx) = workspace(&["a.txt", "b.txt", "c.txt", "d.txt"], cx);
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        open(&wb, name, cx);
    }
    let b = tab_id(&wb, 0, 1, cx);
    run(
        &wb,
        "workbench.action.closeEditorsToTheRight",
        Some(serde_json::json!({ "tab": b.as_u64() })),
        cx,
    );
    assert_eq!(tabs(&wb, cx), [["a.txt", "b.txt"]]);
    assert_eq!(active_name(&wb, cx).as_deref(), Some("b.txt"));

    keys("cmd-shift-t", cx);
    assert_eq!(tabs(&wb, cx), [["a.txt", "b.txt", "c.txt", "d.txt"]]);
    let c = tab_id(&wb, 0, 2, cx);
    run(
        &wb,
        "workbench.action.closeEditorsToTheLeft",
        Some(serde_json::json!({ "tab": c.as_u64() })),
        cx,
    );
    assert_eq!(tabs(&wb, cx), [["c.txt", "d.txt"]]);
    assert_eq!(active_name(&wb, cx).as_deref(), Some("c.txt"));
}

#[gpui::test]
fn close_saved_keeps_dirty(cx: &mut TestAppContext) {
    let (_dir, wb, cx) = workspace(&["a.txt", "b.txt", "c.txt"], cx);
    open(&wb, "a.txt", cx);
    input("x", cx);
    open(&wb, "b.txt", cx);
    open(&wb, "c.txt", cx);
    keys("cmd-k u", cx);
    assert!(cx.pending_prompt().is_none());
    assert_eq!(tabs(&wb, cx), [["a.txt"]]);
}

#[gpui::test]
fn close_group_keys(cx: &mut TestAppContext) {
    let (_dir, wb, cx) = workspace(&["a.txt", "b.txt", "c.txt"], cx);
    open(&wb, "a.txt", cx);
    open(&wb, "b.txt", cx);
    run(&wb, "workbench.action.splitEditor", None, cx);
    open(&wb, "c.txt", cx);
    assert_eq!(tabs(&wb, cx), [vec!["a.txt", "b.txt"], vec!["b.txt", "c.txt"]]);
    keys("cmd-k w", cx);
    assert_eq!(tabs(&wb, cx), [["a.txt", "b.txt"]]);
    keys("cmd-k cmd-w", cx);
    assert!(tabs(&wb, cx).concat().is_empty());
}

#[gpui::test]
fn registers_commands(cx: &mut TestAppContext) {
    let (_dir, wb, cx) = workspace(&[], cx);
    let all: Vec<(String, String)> = wb.read_with(cx, |wb, _| {
        wb.commands()
            .all()
            .map(|(id, title)| (id.to_string(), title.to_string()))
            .collect()
    });
    for (id, title) in [
        ("workbench.action.reopenClosedEditor", "View: Reopen Closed Editor"),
        ("workbench.action.closeOtherEditors", "View: Close Other Editors in Group"),
        (
            "workbench.action.closeEditorsToTheRight",
            "View: Close Editors to the Right in Group",
        ),
        (
            "workbench.action.closeEditorsToTheLeft",
            "View: Close Editors to the Left in Group",
        ),
        ("workbench.action.closeUnmodifiedEditors", "View: Close Saved Editors in Group"),
        ("workbench.action.closeEditorsInGroup", "View: Close All Editors in Group"),
    ] {
        assert!(
            all.contains(&(id.to_string(), title.to_string())),
            "missing {id} \"{title}\""
        );
    }
}
