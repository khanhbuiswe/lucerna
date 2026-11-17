//! Multiple cursors driven through the workbench's keys: `⌘D`, `⌘K ⌘D`,
//! `⇧⌘L`, `⌘F2`, cursors above / below, column selection, the two Escape
//! steps, `⌘U`, the Find bar and the cursor limit.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Context, Keymap, Layer, Resolution, SettingsStore, parse_keys, parse_when};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, Workbench};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext, user_settings: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    if let Some(json) = user_settings {
        settings.set_layer(Layer::User, json).unwrap();
    }
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
    cx.set_global(AppState {
        settings,
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

fn focus(view: &Entity<CodeView>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    settle(cx);
}

/// A workbench on a temporary folder holding `file.txt` with `text`, the
/// file open and its view focused.
fn editor<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<CodeView>,
    &'a mut VisualTestContext,
) {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("file.txt"), text).unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("file.txt")), OpenMode::Pinned, cx)
    });
    settle(cx);
    let view = active_view(&wb, cx);
    focus(&view, cx);
    (dir, wb, view, cx)
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    settle(cx);
}

fn input(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    settle(cx);
}

fn context(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Context {
    cx.update(|window, cx| wb.read(cx).context_keys(window, cx))
}

fn holds(ctx: &Context, when: &str) -> bool {
    parse_when(when).unwrap().eval(ctx)
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.run_command(command, None, window, cx)));
    settle(cx);
}

fn text_of(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, _| {
        let s = v.snapshot();
        s.text_for_range(0..s.len_bytes())
    })
}

/// The selected text of every selection, in document order.
fn selected(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, _| {
        v.selections()
            .all()
            .iter()
            .map(|s| v.snapshot().text_for_range(s.range()))
            .collect()
    })
}

/// The byte ranges of every selection, in document order.
fn ranges(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Vec<std::ops::Range<usize>> {
    view.read_with(cx, |v, _| {
        v.selections().all().iter().map(|s| s.range()).collect()
    })
}

fn count(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |v, _| v.selections().len())
}

fn set_cursor(view: &Entity<CodeView>, offset: usize, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| {
        let p = v.snapshot().offset_to_point(offset);
        v.set_cursor(p, cx);
    });
    settle(cx);
}

fn status_message(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|window, cx| {
        lucerna_workbench::status_message::current(window.window_handle().window_id(), cx)
            .map(|m| m.to_string())
    })
}

const FOOS: &str = "foo bar foo baz foo";

#[gpui::test]
fn cmd_d_three_times_then_type(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor(FOOS, cx);
    set_cursor(&view, 1, cx);
    keys("cmd-d", cx);
    assert_eq!(ranges(&view, cx), vec![0..3]);
    keys("cmd-d cmd-d", cx);
    assert_eq!(ranges(&view, cx), vec![0..3, 8..11, 16..19]);
    input("x", cx);
    assert_eq!(text_of(&view, cx), "x bar x baz x");
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), FOOS);
}

#[gpui::test]
fn cmd_k_cmd_d_skips(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor(FOOS, cx);
    keys("cmd-d cmd-d cmd-k cmd-d", cx);
    assert_eq!(ranges(&view, cx), vec![0..3, 16..19]);
    input("x", cx);
    assert_eq!(text_of(&view, cx), "x bar foo baz x");
}

#[gpui::test]
fn cmd_u_undoes_cmd_d(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor(FOOS, cx);
    keys("cmd-d cmd-d", cx);
    assert_eq!(count(&view, cx), 2);
    keys("cmd-u", cx);
    assert_eq!(ranges(&view, cx), vec![0..3]);
}

#[gpui::test]
fn shift_cmd_l_and_cmd_f2(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor(FOOS, cx);
    set_cursor(&view, 9, cx);
    keys("shift-cmd-l", cx);
    assert_eq!(ranges(&view, cx), vec![0..3, 8..11, 16..19]);
    assert_eq!(
        view.read_with(cx, |v, _| v.selections().primary().range()),
        8..11,
        "the match at the cursor stays primary"
    );
    set_cursor(&view, 17, cx);
    assert_eq!(count(&view, cx), 1);
    keys("cmd-f2", cx);
    assert_eq!(selected(&view, cx), vec!["foo", "foo", "foo"]);
}

#[gpui::test]
fn alt_cmd_down_then_up_removes(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("one\ntwo\nsix\nten\n", cx);
    set_cursor(&view, 1, cx);
    keys("alt-cmd-down alt-cmd-down", cx);
    assert_eq!(ranges(&view, cx), vec![1..1, 5..5, 9..9]);
    keys("alt-cmd-up", cx);
    assert_eq!(ranges(&view, cx), vec![1..1, 5..5]);
    // The layer below is the first `⌥⌘↓`'s: `⌥⌘↑` removes it too.
    keys("alt-cmd-up", cx);
    assert_eq!(ranges(&view, cx), vec![1..1]);
}

#[gpui::test]
fn escape_two_steps(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = editor(FOOS, cx);
    keys("cmd-d cmd-d", cx);
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "editorHasMultipleSelections"));
    keys("escape", cx);
    assert_eq!(ranges(&view, cx), vec![0..3]);
    let ctx = context(&wb, cx);
    assert!(!holds(&ctx, "editorHasMultipleSelections"));
    assert!(holds(&ctx, "editorHasSelection"));
    keys("escape", cx);
    assert_eq!(ranges(&view, cx), vec![3..3]);
    let ctx = context(&wb, cx);
    assert!(!holds(&ctx, "editorHasMultipleSelections"));
    assert!(!holds(&ctx, "editorHasSelection"));
}

#[gpui::test]
fn escape_closes_find_first(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = editor(FOOS, cx);
    keys("cmd-f", cx);
    focus(&view, cx);
    keys("cmd-d cmd-d", cx);
    assert!(holds(&context(&wb, cx), "findWidgetVisible"));
    assert_eq!(count(&view, cx), 2);
    keys("escape", cx);
    assert!(!holds(&context(&wb, cx), "findWidgetVisible"));
    assert_eq!(count(&view, cx), 2);
}

#[gpui::test]
fn cmd_d_explorer_still_duplicates(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, _view, cx) = editor(FOOS, cx);
    let resolve = |ctx: &Context, cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let keys = parse_keys("cmd+d").unwrap();
            match cx.global::<AppState>().keymap.resolve(&keys, ctx) {
                Resolution::Command { command, .. } => Some(command),
                _ => None,
            }
        })
    };
    let ctx = context(&wb, cx);
    assert_eq!(
        resolve(&ctx, cx).as_deref(),
        Some("editor.action.addSelectionToNextFindMatch")
    );
    run(&wb, "workbench.view.explorer", cx);
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "filesExplorerFocus"));
    assert_eq!(
        resolve(&ctx, cx).as_deref(),
        Some("filesExplorer.duplicate")
    );
}

#[gpui::test]
fn cmd_d_seeds_open_find(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor(FOOS, cx);
    set_cursor(&view, 9, cx);
    keys("cmd-f", cx);
    focus(&view, cx);
    keys("cmd-d", cx);
    let query = view.read_with(cx, |v, cx| {
        let find = v.find_widget().expect("the Find bar is open").read(cx);
        find.input().read(cx).text().to_owned()
    });
    assert_eq!(query, "foo");
    // Seeding searches, but leaves the selection alone.
    assert_eq!(ranges(&view, cx), vec![8..11]);
    let matches = view.read_with(cx, |v, cx| {
        v.find_widget().unwrap().read(cx).matches().len()
    });
    assert_eq!(matches, 3);
}

#[gpui::test]
fn cmd_d_after_external_reload(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor(FOOS, cx);
    // On the second `foo`, so the new line does not touch the selection.
    set_cursor(&view, 9, cx);
    keys("cmd-d", cx);
    assert_eq!(ranges(&view, cx), vec![8..11]);
    let new = format!("new line\n{FOOS}");
    fs::write(dir.path().join("file.txt"), &new).unwrap();
    let change = Change {
        path: Arc::from(Path::new("file.txt")),
        kind: ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    settle(cx);
    assert_eq!(text_of(&view, cx), new);
    keys("cmd-d", cx);
    assert_eq!(ranges(&view, cx), vec![17..20, 25..28]);
    assert_eq!(selected(&view, cx), vec!["foo", "foo"]);
}

#[gpui::test]
fn select_highlights_limit_large_file(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "editor.multiCursorLimit": 100 }"#));
    let text = "foo ".repeat(512 * 1024);
    assert!(text.len() > lucerna_workbench::BACKGROUND_SEARCH_BYTES);
    let (_dir, _wb, view, cx) = editor(&text, cx);
    keys("shift-cmd-l", cx);
    cx.run_until_parked();
    assert_eq!(count(&view, cx), 100);
    let message = status_message(cx).unwrap_or_default();
    assert!(message.contains("limited to 100"), "{message:?}");
    assert_eq!(message, "The number of cursors has been limited to 100.");
}

#[gpui::test]
fn alt_shift_cmd_arrows_column(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("abcd\nefgh\nij\n", cx);
    keys(
        "shift-alt-cmd-right shift-alt-cmd-right shift-alt-cmd-down",
        cx,
    );
    assert_eq!(selected(&view, cx), vec!["ab", "ef"]);
    // The next row is shorter than the start column: a cursor at its end.
    keys("shift-alt-cmd-down shift-alt-cmd-right", cx);
    assert_eq!(selected(&view, cx), vec!["abc", "efg", "ij"]);
}

#[gpui::test]
fn shift_alt_i_puts_cursors_at_line_ends(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("one\ntwo\nsix\n", cx);
    keys("shift-down shift-down shift-right", cx);
    keys("shift-alt-i", cx);
    assert_eq!(ranges(&view, cx), vec![3..3, 7..7, 9..9]);
    input("!", cx);
    assert_eq!(text_of(&view, cx), "one!\ntwo!\ns!ix\n");
}

#[gpui::test]
fn merge_overlapping_setting_reaches_the_selections(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "editor.multiCursorMergeOverlapping": false }"#),
    );
    let (_dir, _wb, view, cx) = editor("ab\n", cx);
    set_cursor(&view, 1, cx);
    assert!(!view.read_with(cx, |v, _| v.selections().merge_overlapping()));
}

#[gpui::test]
fn palette_titles(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, _view, cx) = editor(FOOS, cx);
    let title = |id: &str, cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            wb.read(cx)
                .commands()
                .all()
                .find(|(i, _)| *i == id)
                .map(|(_, title)| title.to_owned())
        })
    };
    for (id, expected) in [
        (
            "editor.action.addSelectionToPreviousFindMatch",
            "Add Selection to Previous Find Match",
        ),
        (
            "editor.action.moveSelectionToPreviousFindMatch",
            "Move Last Selection to Previous Find Match",
        ),
        (
            "editor.action.selectHighlights",
            "Select All Occurrences of Find Match",
        ),
        ("editor.action.insertCursorBelow", "Add Cursor Below"),
    ] {
        assert_eq!(title(id, cx).as_deref(), Some(expected), "{id}");
    }
}
