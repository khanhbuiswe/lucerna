//! Editing in the code view driven through the workbench: the selection
//! set, cursor movement keys, two views of one file, the editing context
//! keys, the caret blink, typing (with IME), edit commands, the clipboard
//! and undo / redo.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, EntityInputHandler as _, Focusable as _, TestAppContext, VisualTestContext};
use lucerna_editor::{Goal, Selection, SelectionSet};
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

fn text_of(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, _| {
        let s = v.snapshot();
        s.text_for_range(0..s.len_bytes())
    })
}

fn is_dirty(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    let docs = documents(wb, cx);
    docs.read_with(cx, |d, _| d.is_dirty(Path::new("file.txt")))
}

fn input(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    settle(cx);
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.run_command(command, None, window, cx)));
    settle(cx);
}

/// Cursors at `offsets`, the first one primary.
fn cursors(offsets: &[usize]) -> SelectionSet {
    let selections = offsets
        .iter()
        .enumerate()
        .map(|(id, &at)| Selection {
            id: id as u64,
            anchor: at,
            head: at,
            goal: Goal::None,
        })
        .collect();
    SelectionSet::from_selections(selections, 0, true)
}

#[gpui::test]
fn typing_inserts_text(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = editor("\nnext\n", cx);
    assert!(!is_dirty(&wb, cx));
    input("héllo", cx);
    assert_eq!(text_of(&view, cx), "héllo\nnext\n");
    assert_eq!(cursor(&view, cx), pt(0, 6));
    assert!(is_dirty(&wb, cx));
    // Typing replaces the selection.
    keys("shift-left shift-left", cx);
    input("p", cx);
    assert_eq!(text_of(&view, cx), "hélp\nnext\n");
}

#[gpui::test]
fn undo_groups_like_vscode(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("", cx);
    input("abc def", cx);
    assert_eq!(text_of(&view, cx), "abc def");
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "abc");
    assert_eq!(cursor(&view, cx), pt(0, 3));
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "");
    keys("cmd-shift-z", cx);
    assert_eq!(text_of(&view, cx), "abc");
    assert_eq!(cursor(&view, cx), pt(0, 3));
}

#[gpui::test]
fn cursor_move_closes_undo_group(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("", cx);
    input("ab", cx);
    keys("left", cx);
    input("c", cx);
    assert_eq!(text_of(&view, cx), "acb");
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "ab");
    assert_eq!(cursor(&view, cx), pt(0, 1));
}

#[gpui::test]
fn undo_back_to_saved_is_clean(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = editor("x\n", cx);
    input("yz", cx);
    assert!(is_dirty(&wb, cx));
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "x\n");
    assert!(!is_dirty(&wb, cx));
    keys("cmd-shift-z", cx);
    assert!(is_dirty(&wb, cx));
}

#[gpui::test]
fn edit_commands_delete_and_indent(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("foo bar baz\n", cx);
    keys("cmd-right backspace", cx);
    assert_eq!(text_of(&view, cx), "foo bar ba\n");
    keys("alt-backspace", cx);
    assert_eq!(text_of(&view, cx), "foo bar \n");
    keys("cmd-left delete", cx);
    assert_eq!(text_of(&view, cx), "oo bar \n");
    keys("alt-delete", cx);
    assert_eq!(text_of(&view, cx), " bar \n");
    keys("right ctrl-k", cx);
    assert_eq!(text_of(&view, cx), " \n");
    // To the next tab stop.
    keys("tab", cx);
    assert_eq!(text_of(&view, cx), "    \n");
    keys("shift-tab", cx);
    assert_eq!(text_of(&view, cx), "\n");
    input("ab", cx);
    keys("left ctrl-o", cx);
    assert_eq!(text_of(&view, cx), "a\nb\n");
    assert_eq!(cursor(&view, cx), pt(0, 1));
    keys("cmd-backspace", cx);
    assert_eq!(text_of(&view, cx), "\nb\n");
}

#[gpui::test]
fn ime_composition_is_one_step(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("x\n", cx);
    keys("cmd-right", cx);
    view.update_in(cx, |v, window, cx| {
        v.replace_and_mark_text_in_range(None, "a", None, window, cx);
        assert_eq!(v.marked_text_range(window, cx), Some(1..2));
        v.replace_and_mark_text_in_range(None, "ă", None, window, cx);
        v.replace_and_mark_text_in_range(None, "ằ", None, window, cx);
        assert_eq!(v.marked_text_range(window, cx), Some(1..2));
        let sel = v.selected_text_range(false, window, cx).unwrap();
        assert_eq!(sel.range, 2..2);
        v.replace_text_in_range(None, "ằ", window, cx);
        assert_eq!(v.marked_text_range(window, cx), None);
    });
    settle(cx);
    assert_eq!(text_of(&view, cx), "xằ\n");
    assert_eq!(cursor(&view, cx), pt(0, 1 + "ằ".len() as u32));
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "x\n");
}

#[gpui::test]
fn ime_composes_at_every_cursor(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("a\nb\n", cx);
    view.update(cx, |v, cx| v.set_selections(cursors(&[1, 3]), false, cx));
    view.update_in(cx, |v, window, cx| {
        v.replace_and_mark_text_in_range(None, "d", None, window, cx);
        v.replace_and_mark_text_in_range(None, "đ", None, window, cx);
        // The primary's range, in UTF-16.
        assert_eq!(v.marked_text_range(window, cx), Some(1..2));
        v.replace_text_in_range(None, "đ", window, cx);
    });
    settle(cx);
    assert_eq!(text_of(&view, cx), "ađ\nbđ\n");
    let heads: Vec<usize> = view.read_with(cx, |v, _| {
        v.selections().all().iter().map(|s| s.head).collect()
    });
    assert_eq!(heads, [3, 7]);
}

#[gpui::test]
fn undo_cancels_composition(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("x\n", cx);
    view.update_in(cx, |v, window, cx| {
        v.replace_and_mark_text_in_range(None, "a", None, window, cx);
    });
    settle(cx);
    assert_eq!(text_of(&view, cx), "ax\n");
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "x\n");
    view.update_in(cx, |v, window, cx| {
        assert_eq!(v.marked_text_range(window, cx), None);
    });
}

#[gpui::test]
fn cut_paste_whole_line(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("one\ntwo\n", cx);
    keys("right cmd-x", cx);
    assert_eq!(text_of(&view, cx), "two\n");
    let clip = cx.read_from_clipboard().expect("cut wrote the clipboard");
    assert_eq!(clip.text().as_deref(), Some("one\n"));
    // Pasted with an empty cursor, a whole line goes above the cursor's.
    keys("right cmd-v", cx);
    assert_eq!(text_of(&view, cx), "one\ntwo\n");
    assert_eq!(cursor(&view, cx), pt(1, 1));
    // Copy with an empty selection copies the line too.
    keys("cmd-c down cmd-v", cx);
    assert_eq!(text_of(&view, cx), "one\ntwo\ntwo\n");
}

#[gpui::test]
fn copy_from_several_selections_writes_pieces(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("ab\ncd\n", cx);
    let set = SelectionSet::from_selections(
        vec![
            Selection {
                id: 0,
                anchor: 0,
                head: 1,
                goal: Goal::None,
            },
            Selection {
                id: 1,
                anchor: 3,
                head: 4,
                goal: Goal::None,
            },
        ],
        0,
        true,
    );
    view.update(cx, |v, cx| v.set_selections(set, false, cx));
    keys("cmd-c", cx);
    let clip = cx.read_from_clipboard().unwrap();
    assert_eq!(clip.text().as_deref(), Some("a\nc"));
    let meta: serde_json::Value = serde_json::from_str(clip.metadata().unwrap()).unwrap();
    assert_eq!(
        meta,
        serde_json::json!({ "pieces": ["a", "c"], "wholeLine": false })
    );
    // Pasted at two cursors, each gets its piece.
    keys("right", cx);
    view.update(cx, |v, cx| v.set_selections(cursors(&[2, 5]), false, cx));
    keys("cmd-v", cx);
    assert_eq!(text_of(&view, cx), "aba\ncdc\n");
}

#[gpui::test]
fn enter_keeps_indent(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, _wb, view, cx) = editor("    foo\n", cx);
    keys("cmd-right enter", cx);
    assert_eq!(text_of(&view, cx), "    foo\n    \n");
    assert_eq!(cursor(&view, cx), pt(1, 4));
    input("bar", cx);
    assert_eq!(text_of(&view, cx), "    foo\n    bar\n");
}

#[gpui::test]
fn readonly_view_ignores_typing(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, _view, cx) = editor("abc\n", cx);
    let docs = documents(&wb, cx);
    let preview = docs.update(cx, |d, cx| {
        d.open_preview(Arc::from(Path::new("file.txt")), cx)
    });
    settle(cx);
    assert!(preview.read_with(cx, |v, _| v.is_passive()));
    // The preview is in no window: the input reaches it directly.
    cx.update(|window, cx| {
        preview.update(cx, |v, cx| {
            v.replace_text_in_range(None, "x", window, cx);
            v.replace_and_mark_text_in_range(None, "y", None, window, cx);
            assert_eq!(v.marked_text_range(window, cx), None);
        })
    });
    settle(cx);
    assert_eq!(text_of(&preview, cx), "abc\n");
    assert!(!is_dirty(&wb, cx));
}

#[gpui::test]
fn vim_normal_mode_does_not_type(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "vim.enabled": true }"#));
    let (_dir, wb, view, cx) = editor("abc\n", cx);
    keys("backspace tab enter", cx);
    view.update_in(cx, |v, window, cx| {
        v.replace_text_in_range(None, "x", window, cx)
    });
    settle(cx);
    assert_eq!(text_of(&view, cx), "abc\n");
    assert!(!is_dirty(&wb, cx));
}

#[gpui::test]
fn cursor_undo_restores_previous_selection(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = editor("abcd\n", cx);
    keys("right right shift-right", cx);
    keys("cmd-u", cx);
    assert_eq!(selection(&view, cx), None);
    assert_eq!(cursor(&view, cx), pt(0, 2));
    keys("cmd-u", cx);
    assert_eq!(cursor(&view, cx), pt(0, 1));
    run(&wb, "cursorRedo", cx);
    assert_eq!(cursor(&view, cx), pt(0, 2));
    run(&wb, "cursorRedo", cx);
    assert_eq!(selection(&view, cx), Some(pt(0, 2)..pt(0, 3)));
    // An edit clears the stack.
    input("x", cx);
    keys("cmd-u", cx);
    assert_eq!(cursor(&view, cx), pt(0, 3));
}

/// A keystroke in a 10k-line Rust file reparses incrementally: the items
/// below the edit keep their tree-sitter nodes.
#[gpui::test]
fn typing_reparses_incrementally(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = TempDir::new().unwrap();
    let text = "fn f(x: u32) -> u32 { x }\n".repeat(10_000);
    fs::write(dir.path().join("big.rs"), &text).unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("big.rs")), OpenMode::Pinned, cx)
    });
    settle(cx);
    let view = active_view(&wb, cx);
    focus(&view, cx);
    let last_item = |syntax: &lucerna_syntax::SyntaxState| {
        let root = syntax.tree().expect("parsed").root_node();
        root.child(root.child_count() - 1).unwrap().id()
    };
    // Kept alive, so no new node can reuse an old node's address.
    let before = view
        .read_with(cx, |v, _| v.syntax().cloned())
        .expect("parsed");
    // `fn gf(…`: a valid edit, so no error recovery spreads it.
    keys("right right right", cx);
    input("g", cx);
    let after = view
        .read_with(cx, |v, _| v.syntax().cloned())
        .expect("parsed");
    let tree = after.tree().unwrap();
    assert!(!tree.root_node().has_changes(), "the new text was parsed");
    assert_eq!(last_item(&after), last_item(&before), "reused, not rebuilt");
}

/// Final review, finding 3 (spec §16): a panic while building an edit or
/// moving the cursors applies nothing, closes the undo step, and the view
/// keeps working.
#[gpui::test]
fn panicking_edit_or_motion_is_contained(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, view, cx) = editor("abc\n", cx);
    input("x", cx);
    let applied = view.update(cx, |v, cx| v.edit(cx, |_, _| panic!("plan builder")));
    assert!(!applied);
    assert_eq!(text_of(&view, cx), "xabc\n");
    view.update(cx, |v, cx| v.move_with(|_| panic!("motion"), cx));
    assert_eq!(cursor(&view, cx), pt(0, 1), "the cursor stays");
    // Still editable; the panic closed the step, so typing starts a new one.
    input("y", cx);
    assert_eq!(text_of(&view, cx), "xyabc\n");
    keys("cmd-z", cx);
    assert_eq!(text_of(&view, cx), "xabc\n");
    keys("right", cx);
    assert_eq!(cursor(&view, cx), pt(0, 2));
    assert!(is_dirty(&wb, cx));
}
