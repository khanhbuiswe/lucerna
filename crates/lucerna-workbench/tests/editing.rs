//! Editing in the code view driven through the workbench: the selection
//! set, cursor movement keys, two views of one file, the editing context
//! keys and the caret blink.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Context, Keymap, Layer, SettingsStore, parse_when};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, Documents, OpenMode, Workbench};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext, user_settings: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    if let Some(json) = user_settings {
        settings.set_layer(Layer::User, json).unwrap();
    }
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
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

fn pt(row: u32, column: u32) -> Point {
    Point { row, column }
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn settle(cx: &mut VisualTestContext) {
    draw(cx);
    cx.run_until_parked();
    draw(cx);
}

fn documents(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<Documents> {
    cx.update(|_, cx| wb.read(cx).documents().clone())
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

fn cursor(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Point {
    view.read_with(cx, |v, _| v.cursor())
}

fn selection(
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
) -> Option<std::ops::Range<Point>> {
    view.read_with(cx, |v, _| v.selection())
}

fn context(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Context {
    cx.update(|window, cx| wb.read(cx).context_keys(window, cx))
}

fn holds(ctx: &Context, when: &str) -> bool {
    parse_when(when).unwrap().eval(ctx)
}

fn insert(
    wb: &Entity<Workbench>,
    origin: &Entity<CodeView>,
    offset: usize,
    text: &str,
    cx: &mut VisualTestContext,
) {
    let docs = documents(wb, cx);
    let id = origin.entity_id();
    docs.update(cx, |d, cx| {
        d.apply_edits(
            Path::new("file.txt"),
            vec![(offset..offset, Arc::from(text))],
            None,
            None,
            id,
            cx,
        )
    })
    .expect("the document is loaded");
    settle(cx);
}

#[gpui::test]
fn arrow_keys_move_cursor(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("ab\ncd", cx);
    keys("right right down", cx);
    assert_eq!(cursor(&view, cx), pt(1, 2));
    keys("left up", cx);
    assert_eq!(cursor(&view, cx), pt(0, 1));
    // Emacs-style keys move too.
    keys("ctrl-f ctrl-n", cx);
    assert_eq!(cursor(&view, cx), pt(1, 2));
    keys("ctrl-a", cx);
    assert_eq!(cursor(&view, cx), pt(1, 0));
    keys("ctrl-e", cx);
    assert_eq!(cursor(&view, cx), pt(1, 2));
}

#[gpui::test]
fn shift_arrows_extend(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("abc\ndef", cx);
    keys("shift-right shift-right", cx);
    assert_eq!(selection(&view, cx), Some(pt(0, 0)..pt(0, 2)));
    assert_eq!(cursor(&view, cx), pt(0, 2));
    // A plain arrow collapses the selection to its side.
    keys("left", cx);
    assert_eq!(selection(&view, cx), None);
    assert_eq!(cursor(&view, cx), pt(0, 0));
    keys("cmd-a", cx);
    assert_eq!(selection(&view, cx), Some(pt(0, 0)..pt(1, 3)));
}

#[gpui::test]
fn cmd_up_down_go_top_bottom(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("one\ntwo\nthree", cx);
    keys("right cmd-down", cx);
    assert_eq!(cursor(&view, cx), pt(2, 5));
    keys("cmd-up", cx);
    assert_eq!(cursor(&view, cx), pt(0, 0));
    keys("cmd-shift-down", cx);
    assert_eq!(selection(&view, cx), Some(pt(0, 0)..pt(2, 5)));
    keys("cmd-left", cx);
    assert_eq!(cursor(&view, cx), pt(2, 0));
    keys("cmd-right", cx);
    assert_eq!(cursor(&view, cx), pt(2, 5));
    keys("alt-left", cx);
    assert_eq!(cursor(&view, cx), pt(2, 0));
    keys("alt-right", cx);
    assert_eq!(cursor(&view, cx), pt(2, 5));
}

#[gpui::test]
fn edit_in_one_view_remaps_the_other(cx: &mut TestAppContext) {
    init(cx, None);
    let text: String = (0..8).map(|i| format!("line {i}\n")).collect();
    let (_dir, wb, view_a, cx) = editor(&text, cx);
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("workbench.action.splitEditor", None, window, cx)
        })
    });
    settle(cx);
    let view_b = active_view(&wb, cx);
    assert_ne!(view_a.entity_id(), view_b.entity_id(), "split made a view");
    view_b.update(cx, |v, cx| v.set_cursor(pt(3, 1), cx));

    // An edit before view B's cursor shifts it down a row.
    insert(&wb, &view_a, 0, "x\n", cx);
    assert_eq!(cursor(&view_b, cx), pt(4, 1));

    // An edit after it leaves it alone.
    let after = view_b.read_with(cx, |v, _| v.snapshot().point_to_offset(pt(6, 0)));
    insert(&wb, &view_a, after, "y\n", cx);
    assert_eq!(cursor(&view_b, cx), pt(4, 1));
}

#[gpui::test]
fn context_keys_report_selection_and_dirty(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("abc\ndef\n", cx);
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "editorTextFocus && !editorReadonly"));
    assert!(!holds(
        &ctx,
        "editorHasSelection || editorHasMultipleSelections"
    ));
    assert!(!holds(&ctx, "activeEditorIsDirty"));

    keys("shift-right", cx);
    assert!(holds(&context(&wb, cx), "editorHasSelection"));

    insert(&wb, &view, 0, "x", cx);
    assert!(holds(&context(&wb, cx), "activeEditorIsDirty"));

    // A file that failed to load (here, a binary one) is read-only.
    fs::write(dir.path().join("bin.dat"), b"\0\x01\x02binary").unwrap();
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("bin.dat")), OpenMode::Pinned, cx)
    });
    settle(cx);
    let bin = active_view(&wb, cx);
    focus(&bin, cx);
    assert!(holds(&context(&wb, cx), "editorReadonly"));
}

#[gpui::test]
fn caret_blinks_and_solid_does_not(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("abc\n", cx);
    let visible = |cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.caret_visible());
    assert!(visible(cx));
    cx.executor().advance_clock(Duration::from_millis(600));
    assert!(!visible(cx), "hidden in the off half");
    cx.executor().advance_clock(Duration::from_millis(500));
    assert!(visible(cx), "back on at 1000 ms");

    // A move shows it again at once.
    cx.executor().advance_clock(Duration::from_millis(500));
    assert!(!visible(cx));
    keys("right", cx);
    assert!(visible(cx));
}

#[gpui::test]
fn solid_caret_does_not_blink(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "editor.cursorBlinking": "solid" }"#));
    let (_dir, _wb, view, cx) = editor("abc\n", cx);
    cx.executor().advance_clock(Duration::from_millis(600));
    assert!(view.read_with(cx, |v, _| v.caret_visible()));
    cx.executor().advance_clock(Duration::from_millis(500));
    assert!(view.read_with(cx, |v, _| v.caret_visible()));
}

#[gpui::test]
fn cursor_move_is_an_undo_stop(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = editor("abc\n", cx);
    let text = |cx: &mut VisualTestContext| {
        view.read_with(cx, |v, _| {
            let s = v.snapshot();
            s.text_for_range(0..s.len_bytes())
        })
    };
    let undo = |cx: &mut VisualTestContext| {
        let docs = documents(&wb, cx);
        docs.update(cx, |d, cx| d.undo(Path::new("file.txt"), cx));
        settle(cx);
    };
    // Two edits with no move between are one step.
    insert(&wb, &view, 0, "x", cx);
    insert(&wb, &view, 1, "y", cx);
    undo(cx);
    assert_eq!(text(cx), "abc\n");
    // A cursor move between them splits the step.
    insert(&wb, &view, 0, "x", cx);
    keys("right", cx);
    insert(&wb, &view, 1, "y", cx);
    undo(cx);
    assert_eq!(text(cx), "xabc\n");
}

/// Writes `text` to `file.txt` and lets the workbench reload it.
fn reload(wb: &Entity<Workbench>, dir: &TempDir, text: &str, cx: &mut VisualTestContext) {
    fs::write(dir.path().join("file.txt"), text).unwrap();
    let change = Change {
        path: Arc::from(Path::new("file.txt")),
        kind: ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    settle(cx);
}

#[gpui::test]
fn reload_of_the_cursor_line_keeps_the_cursor_on_it(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, view, cx) = editor("one\ntwo\nthree\nfour\n", cx);
    for column in [0, 2] {
        view.update(cx, |v, cx| v.set_cursor(pt(1, column), cx));
        let new = if column == 0 {
            "one\nTWO!\nthree\nfour\n"
        } else {
            "one\nzwei\nthree\nfour\n"
        };
        reload(&wb, &dir, new, cx);
        let text = view.read_with(cx, |v, _| {
            let s = v.snapshot();
            s.text_for_range(0..s.len_bytes())
        });
        assert_eq!(text, new);
        assert_eq!(cursor(&view, cx), pt(1, 0), "cursor at column {column}");
    }
}

#[gpui::test]
fn opening_a_file_focuses_its_view(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "abc\n").unwrap();
    fs::write(dir.path().join("b.txt"), "def\n").unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    let focused = |view: &Entity<CodeView>, cx: &mut VisualTestContext| {
        cx.update(|window, cx| view.read(cx).focus_handle(cx).is_focused(window))
    };
    for name in ["a.txt", "b.txt"] {
        wb.update(cx, |wb, cx| {
            wb.open_path(Arc::from(Path::new(name)), OpenMode::Pinned, cx)
        });
        settle(cx);
        let view = active_view(&wb, cx);
        assert!(focused(&view, cx), "{name} is focused");
        assert!(view.read_with(cx, |v, _| v.caret_visible()));
    }
    // Arrows reach the focused view, and its caret follows.
    keys("right", cx);
    assert_eq!(cursor(&active_view(&wb, cx), cx), pt(0, 1));
    // Focusing the editor area (as closing Quick Open does) focuses the
    // active view.
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.focus_editor_area(window, cx)));
    settle(cx);
    let view = active_view(&wb, cx);
    assert!(focused(&view, cx));
}
