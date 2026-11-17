//! Vim editing inside the workbench: one undo step per command, typing
//! recorded for `.`, edits moving marks and the cursors of other views,
//! the clipboard rules and the V2a settings.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    ClipboardItem, Entity, EntityInputHandler as _, Focusable as _, TestAppContext,
    VisualTestContext,
};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_vim::{Key, Mode};
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, Workbench};
use tempfile::TempDir;

const VIM_ON: &str = r#"{ "vim.enabled": true }"#;

fn init(cx: &mut TestAppContext, user_settings: &str) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    settings.set_layer(Layer::User, user_settings).unwrap();
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

fn pt(row: u32, column: u32) -> Point {
    Point { row, column }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
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
    draw(cx);
}

/// A workbench with `settings` on a temporary folder holding `file.txt`
/// with `text`, the file open and its view focused.
fn vim_editor_with<'a>(
    settings: &str,
    text: &str,
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<CodeView>,
    &'a mut VisualTestContext,
) {
    init(cx, settings);
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("file.txt"), text).unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("file.txt")), OpenMode::Pinned, cx)
    });
    draw(cx);
    let view = active_view(&wb, cx);
    focus(&view, cx);
    (dir, wb, view, cx)
}

fn vim_editor<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<CodeView>,
    &'a mut VisualTestContext,
) {
    vim_editor_with(VIM_ON, text, cx)
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    draw(cx);
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    let ran =
        cx.update(|window, cx| wb.update(cx, |wb, cx| wb.run_command(command, None, window, cx)));
    assert!(ran, "{command} is registered");
    draw(cx);
}

fn text_of(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, _| {
        let s = v.snapshot();
        s.text_for_range(0..s.len_bytes())
    })
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

fn mode(
    wb: &Entity<Workbench>,
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
) -> Option<Mode> {
    let id = view.entity_id();
    cx.update(|_, cx| wb.read(cx).vim().read(cx).mode_for(id))
}

/// Types `text` as the platform input handler commits it.
fn commit(view: &Entity<CodeView>, text: &str, cx: &mut VisualTestContext) {
    view.update_in(cx, |v, window, cx| {
        v.replace_text_in_range(None, text, window, cx)
    });
    draw(cx);
}

fn clipboard(cx: &mut VisualTestContext) -> Option<(String, Option<serde_json::Value>)> {
    let item = cx.read_from_clipboard()?;
    let meta = item
        .metadata()
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok());
    Some((item.text()?, meta))
}

/// Writes `text` to `file.txt` on disk and tells the workbench, as the
/// file watcher would.
fn change_on_disk(wb: &Entity<Workbench>, dir: &TempDir, text: &str, cx: &mut VisualTestContext) {
    fs::write(dir.path().join("file.txt"), text).unwrap();
    let change = Change {
        path: Arc::from(Path::new("file.txt")),
        kind: ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    draw(cx);
}

#[gpui::test]
fn three_dd_is_one_undo_step(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("a\nb\nc\nd\n", cx);
    keys("3 d d", cx);
    assert_eq!(text_of(&view, cx), "d\n");
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "a\nb\nc\nd\n");
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "a\nb\nc\nd\n");
    // And redo brings the three lines' deletion back as one step.
    keys("ctrl-r", cx);
    assert_eq!(text_of(&view, cx), "d\n");
}

#[gpui::test]
fn ciw_typing_escape_is_one_undo_step(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("one two\n", cx);
    keys("w c i w", cx);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Insert));
    assert_eq!(text_of(&view, cx), "one \n");
    commit(&view, "f", cx);
    commit(&view, "oo", cx);
    keys("escape", cx);
    assert_eq!(text_of(&view, cx), "one foo\n");
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "one two\n");
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "one two\n");
}

#[gpui::test]
fn ime_commit_records_final_text_for_dot(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("one two\n", cx);
    keys("c i w", cx);
    view.update_in(cx, |v, window, cx| {
        v.replace_and_mark_text_in_range(None, "chà", None, window, cx);
        v.replace_text_in_range(None, "chào", window, cx);
    });
    draw(cx);
    keys("escape", cx);
    assert_eq!(text_of(&view, cx), "chào two\n");
    keys("w .", cx);
    assert_eq!(text_of(&view, cx), "chào chào\n");
    // The replay is one undo step.
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "chào two\n");
}

#[gpui::test]
fn dot_repeats_typing_with_enter_and_backspace(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("a\nb\n", cx);
    // `A`, typed `xq`, Backspace, `y`, Enter, `z`.
    keys("shift-a", cx);
    commit(&view, "x", cx);
    commit(&view, "q", cx);
    keys("backspace", cx);
    commit(&view, "y", cx);
    keys("enter", cx);
    commit(&view, "z", cx);
    keys("escape", cx);
    assert_eq!(text_of(&view, cx), "axy\nz\nb\n");
    keys("j j .", cx);
    assert_eq!(text_of(&view, cx), "axy\nz\nbxy\nz\n");
}

#[gpui::test]
fn counted_insert_is_one_undo_step(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("x\n", cx);
    keys("3 i", cx);
    commit(&view, "ab", cx);
    keys("escape", cx);
    assert_eq!(text_of(&view, cx), "abababx\n");
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "x\n");
}

#[gpui::test]
fn arrow_key_in_insert_starts_a_new_record(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("abc\nabc\n", cx);
    keys("i", cx);
    commit(&view, "x", cx);
    keys("right", cx);
    commit(&view, "y", cx);
    keys("escape", cx);
    assert_eq!(text_of(&view, cx), "xaybc\nabc\n");
    // `.` repeats only the typing after the arrow key, as an `i`.
    keys("j 0 .", cx);
    assert_eq!(text_of(&view, cx), "xaybc\nyabc\n");
}

#[gpui::test]
fn dot_after_external_reload_uses_new_text(cx: &mut TestAppContext) {
    let (dir, wb, view, cx) = vim_editor("alpha beta gamma\n", cx);
    keys("d w", cx);
    assert_eq!(text_of(&view, cx), "beta gamma\n");
    run(&wb, "workbench.action.files.save", cx);
    change_on_disk(&wb, &dir, "top line\nbeta gamma\n", cx);
    assert_eq!(text_of(&view, cx), "top line\nbeta gamma\n");
    // The cursor stayed on `beta`, now on row 1.
    assert_eq!(cursor(&view, cx), pt(1, 0));
    keys(".", cx);
    assert_eq!(text_of(&view, cx), "top line\ngamma\n");
}

#[gpui::test]
fn edit_in_one_view_remaps_other_view_and_marks(cx: &mut TestAppContext) {
    let text: String = (0..8).map(|i| format!("line {i}\n")).collect();
    let (_dir, wb, view_a, cx) = vim_editor(&text, cx);
    run(&wb, "workbench.action.splitEditor", cx);
    let view_b = active_view(&wb, cx);
    assert_ne!(view_a.entity_id(), view_b.entity_id(), "split made a view");
    focus(&view_b, cx);
    keys("5 j l m a v l", cx);
    assert_eq!(cursor(&view_b, cx), pt(5, 2));
    assert_eq!(mode(&wb, &view_b, cx), Some(Mode::Visual));

    run(&wb, "workbench.action.focusFirstEditorGroup", cx);
    assert_eq!(active_view(&wb, cx), view_a);
    keys("g g j d d", cx);
    assert!(!text_of(&view_a, cx).contains("line 1\n"));
    // B's visual selection and its mark stay on `line 5`, a row up.
    assert_eq!(mode(&wb, &view_b, cx), Some(Mode::Visual));
    assert_eq!(selection(&view_b, cx), Some(pt(4, 1)..pt(4, 3)));

    run(&wb, "workbench.action.focusSecondEditorGroup", cx);
    assert_eq!(active_view(&wb, cx), view_b);
    keys("escape g g ` a", cx);
    assert_eq!(cursor(&view_b, cx), pt(4, 1));
    keys("u", cx);
    assert_eq!(text_of(&view_b, cx), text);
    keys("g g ` a", cx);
    assert_eq!(cursor(&view_b, cx), pt(5, 1));
}

#[gpui::test]
fn clipboard_rules_end_to_end(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("one\ntwo\nthree\n", cx);
    keys("y y", cx);
    let (text, meta) = clipboard(cx).expect("yy wrote the clipboard");
    assert_eq!(text, "one\n");
    assert_eq!(meta.unwrap()["wholeLine"], serde_json::Value::Bool(true));

    // A delete writes the register, not the clipboard.
    keys("j d d", cx);
    assert_eq!(text_of(&view, cx), "one\nthree\n");
    assert_eq!(clipboard(cx).map(|c| c.0).as_deref(), Some("one\n"));
    // `p` pastes the deleted line: the clipboard did not change since.
    keys("p", cx);
    assert_eq!(text_of(&view, cx), "one\nthree\ntwo\n");

    // Something copied elsewhere is what `p` pastes, charwise.
    cx.write_to_clipboard(ClipboardItem::new_string("zzz".into()));
    keys("g g p", cx);
    assert_eq!(text_of(&view, cx), "ozzzne\nthree\ntwo\n");
}

#[gpui::test]
fn use_system_clipboard_false_keeps_yank_off_clipboard(cx: &mut TestAppContext) {
    let settings = r#"{ "vim.enabled": true, "vim.useSystemClipboard": false }"#;
    let (_dir, _wb, view, cx) = vim_editor_with(settings, "one\ntwo\n", cx);
    cx.write_to_clipboard(ClipboardItem::new_string("outside".into()));
    keys("y y", cx);
    assert_eq!(clipboard(cx).map(|c| c.0).as_deref(), Some("outside"));
    // `"+` still reaches it.
    keys("j \" + y y", cx);
    assert_eq!(clipboard(cx).map(|c| c.0).as_deref(), Some("two\n"));
    // The unnamed register is the internal one.
    keys("p", cx);
    assert_eq!(text_of(&view, cx), "one\ntwo\ntwo\n");
}

#[gpui::test]
fn cmd_c_line_then_p_is_linewise(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("one\ntwo\n", cx);
    run(&wb, "editor.action.clipboardCopyAction", cx);
    let (text, meta) = clipboard(cx).expect("the copy wrote the clipboard");
    assert_eq!(text, "one\n");
    assert_eq!(meta.unwrap()["wholeLine"], serde_json::Value::Bool(true));
    keys("p", cx);
    assert_eq!(text_of(&view, cx), "one\none\ntwo\n");
    assert_eq!(cursor(&view, cx), pt(1, 0));
}

#[gpui::test]
fn readonly_preview_dd_does_nothing(cx: &mut TestAppContext) {
    let (_dir, wb, _view, cx) = vim_editor("one\ntwo\n", cx);
    let docs = cx.update(|_, cx| wb.read(cx).documents().clone());
    let preview = docs.update(cx, |d, cx| {
        d.open_preview(Arc::from(Path::new("file.txt")), cx)
    });
    draw(cx);
    assert!(preview.read_with(cx, |v, _| v.is_readonly()));
    let vim = cx.update(|_, cx| wb.read(cx).vim().clone());
    let feed = |keys: &[Key], cx: &mut VisualTestContext| {
        vim.update(cx, |vim, cx| vim.feed_keys(&preview, keys, cx));
        draw(cx);
    };
    feed(&[Key::Char('d'), Key::Char('d'), Key::Char('x')], cx);
    assert_eq!(text_of(&preview, cx), "one\ntwo\n");
    // `y` still works.
    feed(&[Key::Char('y'), Key::Char('y')], cx);
    assert_eq!(clipboard(cx).map(|c| c.0).as_deref(), Some("one\n"));
}

#[gpui::test]
fn u_moves_cursor_to_change_start(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("one two three\nfour\n", cx);
    keys("w d w", cx);
    assert_eq!(text_of(&view, cx), "one three\nfour\n");
    keys("j $", cx);
    assert_eq!(cursor(&view, cx), pt(1, 3));
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "one two three\nfour\n");
    assert_eq!(cursor(&view, cx), pt(0, 4));
    keys("ctrl-r", cx);
    assert_eq!(text_of(&view, cx), "one three\nfour\n");
    assert_eq!(cursor(&view, cx), pt(0, 4));
}

#[gpui::test]
fn joinspaces_setting_is_read(cx: &mut TestAppContext) {
    let settings = r#"{ "vim.enabled": true, "vim.joinspaces": false }"#;
    let (_dir, _wb, view, cx) = vim_editor_with(settings, "end.\nnext\n", cx);
    keys("shift-j", cx);
    assert_eq!(text_of(&view, cx), "end. next\n");
}

#[gpui::test]
fn mark_at_column_0_follows_its_line(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("a\nb\nc\nd\n", cx);
    keys("2 j m a", cx);
    // `kdd` then `u`: the line comes back above the mark's line.
    keys("k d d u", cx);
    assert_eq!(text_of(&view, cx), "a\nb\nc\nd\n");
    keys("g g ` a", cx);
    assert_eq!(cursor(&view, cx), pt(2, 0));
    // `P` of a line above the marked line pushes the mark down.
    keys("g g y y j j shift-p", cx);
    assert_eq!(text_of(&view, cx), "a\nb\na\nc\nd\n");
    keys("g g ` a", cx);
    assert_eq!(cursor(&view, cx), pt(3, 0));
}

#[gpui::test]
fn unmarked_replacement_at_caret_records_backspaces(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("x\ny\n", cx);
    keys("i", cx);
    commit(&view, "chao", cx);
    // An IME that does not mark replaces `ao` before the caret by `ào`.
    view.update_in(cx, |v, window, cx| {
        v.replace_text_in_range(Some(2..4), "ào", window, cx)
    });
    draw(cx);
    keys("escape", cx);
    assert_eq!(text_of(&view, cx), "chàox\ny\n");
    keys("j 0 .", cx);
    assert_eq!(text_of(&view, cx), "chàox\nchàoy\n");
}

#[gpui::test]
fn unmarked_replacement_away_from_caret_breaks_the_record(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("x\ny\n", cx);
    keys("i", cx);
    commit(&view, "ab", cx);
    // A replacement of the first char, not before the caret.
    view.update_in(cx, |v, window, cx| {
        v.replace_text_in_range(Some(0..1), "A", window, cx)
    });
    draw(cx);
    commit(&view, "c", cx);
    keys("escape", cx);
    // The caret followed the replaced text.
    assert_eq!(text_of(&view, cx), "Acbx\ny\n");
    // Only the typing after the break repeats.
    keys("j 0 .", cx);
    assert_eq!(text_of(&view, cx), "Acbx\ncy\n");
}

#[gpui::test]
fn paste_in_insert_breaks_the_record(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("x\ny\n", cx);
    cx.write_to_clipboard(ClipboardItem::new_string("ZZ".into()));
    keys("i", cx);
    commit(&view, "ab", cx);
    keys("cmd-v", cx);
    commit(&view, "c", cx);
    keys("escape", cx);
    assert_eq!(text_of(&view, cx), "abZZcx\ny\n");
    // `.` does not replay `abc` as if nothing was pasted.
    keys("j 0 .", cx);
    assert_eq!(text_of(&view, cx), "abZZcx\ncy\n");
}

#[gpui::test]
fn use_system_clipboard_true_writes_deletes(cx: &mut TestAppContext) {
    let settings = r#"{ "vim.enabled": true, "vim.useSystemClipboard": true }"#;
    let (_dir, _wb, _view, cx) = vim_editor_with(settings, "one\ntwo\n", cx);
    keys("d d", cx);
    assert_eq!(clipboard(cx).map(|c| c.0).as_deref(), Some("one\n"));
}

#[gpui::test]
fn invalid_use_system_clipboard_is_on_yank(cx: &mut TestAppContext) {
    let settings = r#"{ "vim.enabled": true, "vim.useSystemClipboard": "sometimes" }"#;
    let (_dir, _wb, _view, cx) = vim_editor_with(settings, "one\ntwo\n", cx);
    keys("y y", cx);
    assert_eq!(clipboard(cx).map(|c| c.0).as_deref(), Some("one\n"));
    keys("j d d", cx);
    assert_eq!(clipboard(cx).map(|c| c.0).as_deref(), Some("one\n"));
}

#[gpui::test]
fn dot_stops_when_its_insert_does_not_start(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("f(a)\nno pair\n", cx);
    keys("l l c i (", cx);
    commit(&view, "b", cx);
    keys("escape", cx);
    assert_eq!(text_of(&view, cx), "f(b)\nno pair\n");
    // No `(` pair on the second line: `ci(` fails, and its typing and
    // Escape are dropped.
    keys("j 0 .", cx);
    assert_eq!(text_of(&view, cx), "f(b)\nno pair\n");
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    // The undo step of the first change is intact.
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "f(a)\nno pair\n");
}

#[gpui::test]
fn mark_comes_back_with_its_deleted_line(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("a\nb\nc\nd\n", cx);
    keys("2 j m a d d", cx);
    assert_eq!(text_of(&view, cx), "a\nb\nd\n");
    keys("u g g ` a", cx);
    assert_eq!(cursor(&view, cx), pt(2, 0));
}

#[gpui::test]
fn join_and_enter_move_mark_with_its_char(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("ab\ncd\n", cx);
    keys("j l m a k shift-j", cx);
    assert_eq!(text_of(&view, cx), "ab cd\n");
    keys("0 ` a", cx);
    assert_eq!(cursor(&view, cx), pt(0, 4));
    // Enter before the mark on its line: the mark goes to the second line.
    keys("0 l a", cx);
    keys("enter escape", cx);
    assert_eq!(text_of(&view, cx), "ab\n cd\n");
    keys("g g ` a", cx);
    assert_eq!(cursor(&view, cx), pt(1, 2));
}

#[gpui::test]
fn unmarked_replacement_in_indent_breaks_the_record(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("x\ny\n", cx);
    keys("i", cx);
    commit(&view, "  ", cx);
    // The two spaces before the caret, in the indentation, replaced.
    view.update_in(cx, |v, window, cx| {
        v.replace_text_in_range(Some(0..2), "--", window, cx)
    });
    draw(cx);
    commit(&view, "q", cx);
    keys("escape", cx);
    assert_eq!(text_of(&view, cx), "--qx\ny\n");
    keys("j 0 .", cx);
    assert_eq!(text_of(&view, cx), "--qx\nqy\n");
}

#[gpui::test]
fn unmarked_replacement_counts_graphemes(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("x\ny\n", cx);
    keys("i", cx);
    // `e` with a combining acute accent: one grapheme, two chars.
    commit(&view, "ae\u{301}", cx);
    view.update_in(cx, |v, window, cx| {
        v.replace_text_in_range(Some(1..3), "é", window, cx)
    });
    draw(cx);
    keys("escape", cx);
    assert_eq!(text_of(&view, cx), "aéx\ny\n");
    keys("j 0 .", cx);
    assert_eq!(text_of(&view, cx), "aéx\naéy\n");
}

#[gpui::test]
fn mark_follows_a_line_an_agent_rewrote(cx: &mut TestAppContext) {
    let (dir, wb, view, cx) = vim_editor("a\nb\nc\nd\n", cx);
    keys("2 j m a", cx);
    change_on_disk(&wb, &dir, "a\nb\nC\nd\n", cx);
    assert_eq!(text_of(&view, cx), "a\nb\nC\nd\n");
    change_on_disk(&wb, &dir, "a\nb\nnew\nC\nd\n", cx);
    assert_eq!(text_of(&view, cx), "a\nb\nnew\nC\nd\n");
    keys("g g ` a", cx);
    assert_eq!(cursor(&view, cx), pt(3, 0));
}

#[gpui::test]
fn unmarked_replacement_starting_in_indent_breaks_the_record(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("x\ny\n", cx);
    keys("i", cx);
    commit(&view, "  a", cx);
    // ` a` (from inside the indentation) replaced, with the caret at col 3.
    view.update_in(cx, |v, window, cx| {
        v.replace_text_in_range(Some(1..3), "-A", window, cx)
    });
    draw(cx);
    commit(&view, "q", cx);
    keys("escape", cx);
    assert_eq!(text_of(&view, cx), " -Aqx\ny\n");
    // Only the typing after the break repeats.
    keys("j 0 .", cx);
    assert_eq!(text_of(&view, cx), " -Aqx\nqy\n");
}

/// Types `text` into the focused text input (the find widget's).
fn type_text(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    draw(cx);
}

fn find_visible(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, _| v.find_widget().is_some())
}

#[gpui::test]
fn d_slash_through_find_widget(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("abc foo\nxyz foo\n", cx);
    keys("d /", cx);
    assert!(find_visible(&view, cx));
    type_text("foo", cx);
    // The incremental search does not edit.
    assert_eq!(text_of(&view, cx), "abc foo\nxyz foo\n");
    keys("enter", cx);
    assert!(!find_visible(&view, cx));
    assert_eq!(text_of(&view, cx), "foo\nxyz foo\n");
    assert_eq!(cursor(&view, cx), pt(0, 0));
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    // `.` reruns the search with no widget, as one undo step.
    keys("j l .", cx);
    assert!(!find_visible(&view, cx));
    assert_eq!(text_of(&view, cx), "foo\nxfoo\n");
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "foo\nxyz foo\n");
    // The `d/foo` is one undo step.
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "abc foo\nxyz foo\n");
}

#[gpui::test]
fn d_slash_in_large_file_then_a_key_drops_the_operator(cx: &mut TestAppContext) {
    // Over `BACKGROUND_SEARCH_BYTES`: the matches come from a background
    // search, and a key typed before it ends drops the waiting `d`.
    let filler = "x".repeat(99) + "\n";
    let text = format!("abc foo\n{}foo\n", filler.repeat(11_000));
    assert!(text.len() > lucerna_workbench::BACKGROUND_SEARCH_BYTES);
    let (_dir, wb, view, cx) = vim_editor(&text, cx);
    keys("d /", cx);
    type_text("foo", cx);
    // Enter, then `w` before the background search is done.
    keys("enter w", cx);
    assert_eq!(text_of(&view, cx), text);
    assert_eq!(cursor(&view, cx), pt(0, 4));
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    // Without a key in between, the search finishes the operator.
    keys("0 d /", cx);
    type_text("foo", cx);
    keys("enter", cx);
    assert!(text_of(&view, cx).starts_with("foo\n"));
}

#[gpui::test]
fn key_reaching_the_view_during_d_slash_drops_the_operator(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("abc foo\n", cx);
    keys("d /", cx);
    type_text("foo", cx);
    assert!(find_visible(&view, cx));
    // The user goes back to the text with the widget still open.
    focus(&view, cx);
    keys("w", cx);
    assert_eq!(text_of(&view, cx), "abc foo\n");
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    let id = view.entity_id();
    let pending = cx.update(|_, cx| wb.read(cx).vim().read(cx).indicator_text(id));
    assert_eq!(pending.as_deref(), Some("-- NORMAL --"));
}

#[gpui::test]
fn c_slash_then_escape_cancels(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("abc foo\n", cx);
    keys("c /", cx);
    type_text("foo", cx);
    keys("escape", cx);
    assert!(!find_visible(&view, cx));
    assert_eq!(text_of(&view, cx), "abc foo\n");
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    assert_eq!(cursor(&view, cx), pt(0, 0));
    // The operator is gone: `w` only moves.
    keys("w", cx);
    assert_eq!(text_of(&view, cx), "abc foo\n");
    assert_eq!(cursor(&view, cx), pt(0, 4));
}

#[gpui::test]
fn c_slash_typing_escape_is_one_undo_step(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("ab foo\ncd foo\n", cx);
    keys("c /", cx);
    type_text("foo", cx);
    keys("enter", cx);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Insert));
    assert_eq!(text_of(&view, cx), "foo\ncd foo\n");
    commit(&view, "X", cx);
    keys("escape", cx);
    assert_eq!(text_of(&view, cx), "Xfoo\ncd foo\n");
    keys("j 0 .", cx);
    assert_eq!(text_of(&view, cx), "Xfoo\nXfoo\n");
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "Xfoo\ncd foo\n");
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "ab foo\ncd foo\n");
}

#[gpui::test]
fn d_slash_in_large_file_dropped_by_an_outside_edit(cx: &mut TestAppContext) {
    // An outside edit (an agent, another view) lands while the background
    // search of a `d/` runs: the matches are stale, and the operator must
    // not stay armed for the next key.
    let filler = "x".repeat(99) + "\n";
    let text = format!("abc foo\n{}foo\n", filler.repeat(11_000));
    assert!(text.len() > lucerna_workbench::BACKGROUND_SEARCH_BYTES);
    let (_dir, wb, view, cx) = vim_editor(&text, cx);
    keys("d /", cx);
    type_text("foo", cx);
    // Enter starts the background search; the edit comes before it ends
    // (no parking in between).
    cx.update(|window, cx| window.dispatch_keystroke(gpui::Keystroke::parse("enter").unwrap(), cx));
    let origin = wb.entity_id();
    let documents = wb.read_with(cx, |wb, _| wb.documents().clone());
    documents.update(cx, |d, cx| {
        let edit = (0..0, Arc::<str>::from("Z"));
        d.apply_edits(Path::new("file.txt"), vec![edit], None, None, origin, cx)
            .expect("the file is loaded");
    });
    draw(cx);
    let edited = format!("Z{text}");
    assert_eq!(text_of(&view, cx), edited);
    keys("w", cx);
    assert_eq!(text_of(&view, cx), edited);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    let id = view.entity_id();
    let pending = cx.update(|_, cx| wb.read(cx).vim().read(cx).indicator_text(id));
    assert_eq!(pending.as_deref(), Some("-- NORMAL --"));
}

#[gpui::test]
fn visual_line_change_then_p_puts_the_changed_lines(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("one\ntwo\n", cx);
    cx.write_to_clipboard(ClipboardItem::new_string("zzz".into()));
    keys("V R", cx);
    assert_eq!(text_of(&view, cx), "\ntwo\n");
    // The clipboard did not change since `R` wrote `"`: `p` puts the line.
    keys("escape p", cx);
    assert_eq!(text_of(&view, cx), "\none\ntwo\n");
}

#[gpui::test]
fn p_of_a_crlf_clipboard_puts_line_feeds(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("xy\n", cx);
    cx.write_to_clipboard(ClipboardItem::new_string("a\r\nb\rc".into()));
    keys("p", cx);
    assert_eq!(text_of(&view, cx), "xa\nb\ncy\n");
    // Multi-line text: the cursor goes to its start.
    assert_eq!(cursor(&view, cx), pt(0, 1));
}

#[gpui::test]
fn named_yank_leaves_the_clipboard_alone(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("one\ntwo\n", cx);
    cx.write_to_clipboard(ClipboardItem::new_string("outside".into()));
    keys("\" a y y", cx);
    assert_eq!(clipboard(cx).map(|c| c.0).as_deref(), Some("outside"));
    // `"` holds the named yank, and the clipboard has not changed since.
    keys("p", cx);
    assert_eq!(text_of(&view, cx), "one\none\ntwo\n");
    keys("\" a p", cx);
    assert_eq!(text_of(&view, cx), "one\none\none\ntwo\n");
}

fn flashed(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Vec<std::ops::Range<usize>> {
    view.read_with(cx, |v, _| v.flashed_ranges().to_vec())
}

#[gpui::test]
fn yank_flash_follows_settings(cx: &mut TestAppContext) {
    let on = r#"{ "vim.enabled": true, "vim.highlightedyank.enable": true }"#;
    let (_dir, _wb, view, cx) = vim_editor_with(on, "one two\n", cx);
    keys("w y i w", cx);
    assert_eq!(flashed(&view, cx), vec![4..7]);
    cx.executor().advance_clock(Duration::from_millis(100));
    assert_eq!(flashed(&view, cx), vec![4..7]);
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    assert!(flashed(&view, cx).is_empty());
}

#[gpui::test]
fn yank_flash_duration_is_read(cx: &mut TestAppContext) {
    let on = r#"{ "vim.enabled": true, "vim.highlightedyank.enable": true,
                  "vim.highlightedyank.duration": 1000 }"#;
    let (_dir, _wb, view, cx) = vim_editor_with(on, "one two\n", cx);
    keys("y i w", cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(flashed(&view, cx), vec![0..3]);
    cx.executor().advance_clock(Duration::from_millis(900));
    cx.run_until_parked();
    assert!(flashed(&view, cx).is_empty());
}

#[gpui::test]
fn yank_flash_is_off_by_default(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("one two\n", cx);
    keys("y i w", cx);
    assert!(flashed(&view, cx).is_empty());
}

#[gpui::test]
fn yank_flash_cleared_by_edit(cx: &mut TestAppContext) {
    let on = r#"{ "vim.enabled": true, "vim.highlightedyank.enable": true }"#;
    let (_dir, _wb, view, cx) = vim_editor_with(on, "one two\n", cx);
    keys("y i w", cx);
    assert_eq!(flashed(&view, cx), vec![0..3]);
    keys("x", cx);
    assert!(flashed(&view, cx).is_empty());
}

fn indicator(
    wb: &Entity<Workbench>,
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
) -> String {
    let id = view.entity_id();
    cx.update(|_, cx| wb.read(cx).vim().read(cx).indicator_text(id))
        .expect("vim is on")
}

#[gpui::test]
fn macro_record_and_count_is_one_undo_step(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("a\nb\nc\nd\n", cx);
    keys("q a", cx);
    assert_eq!(indicator(&wb, &view, cx), "-- NORMAL -- recording @a");
    keys("shift-a", cx);
    assert_eq!(indicator(&wb, &view, cx), "-- INSERT -- recording @a");
    commit(&view, ";", cx);
    keys("escape j q", cx);
    assert_eq!(indicator(&wb, &view, cx), "-- NORMAL --");
    assert_eq!(text_of(&view, cx), "a;\nb\nc\nd\n");
    // The third run's `j` fails on the last line, after its `;`.
    keys("3 @ a", cx);
    assert_eq!(text_of(&view, cx), "a;\nb;\nc;\nd;\n");
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    // The three runs are one undo step.
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "a;\nb\nc\nd\n");
}

#[gpui::test]
fn macro_with_vietnamese_ime_text_replays_text(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("one two\n", cx);
    keys("q a c i w", cx);
    view.update_in(cx, |v, window, cx| {
        v.replace_and_mark_text_in_range(None, "chà", None, window, cx);
    });
    commit(&view, "chào", cx);
    keys("escape q", cx);
    assert_eq!(text_of(&view, cx), "chào two\n");
    keys("w @ a", cx);
    assert_eq!(text_of(&view, cx), "chào chào\n");
}

#[gpui::test]
fn macro_types_through_auto_close(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("(a)\n(x)\n", cx);
    keys("q a c i (", cx);
    for c in ["f", "o", "o", "("] {
        commit(&view, c, cx);
    }
    keys("escape q", cx);
    let recorded = text_of(&view, cx);
    let first = recorded.lines().next().unwrap().to_string();
    assert_ne!(first, "(a)");
    keys("j 0 @ a", cx);
    let text = text_of(&view, cx);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines, [first.as_str(), first.as_str()]);
}

#[gpui::test]
fn macro_search_runs_without_widget(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("foo a\nbar foo\nbaz foo\n", cx);
    keys("q a /", cx);
    assert!(find_visible(&view, cx));
    type_text("foo", cx);
    keys("enter", cx);
    assert!(!find_visible(&view, cx));
    keys("x q", cx);
    assert_eq!(text_of(&view, cx), "foo a\nbar oo\nbaz foo\n");
    keys("@ a", cx);
    assert!(!find_visible(&view, cx));
    assert_eq!(text_of(&view, cx), "foo a\nbar oo\nbaz oo\n");
    assert_eq!(cursor(&view, cx), pt(2, 4));
}

#[gpui::test]
fn macro_stops_when_motion_fails(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("a\nb\nc\nd\ne\n", cx);
    keys("q a j x q", cx);
    assert_eq!(text_of(&view, cx), "a\n\nc\nd\ne\n");
    keys("1 0 0 @ a", cx);
    assert_eq!(text_of(&view, cx), "a\n\n\n\n\n");
    assert_eq!(cursor(&view, cx).row, 4);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    // The run ended: one undo step brings the three lines back.
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "a\n\nc\nd\ne\n");
}

#[gpui::test]
fn self_calling_macro_stops_at_depth_cap(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("\n", cx);
    keys("i", cx);
    commit(&view, "@a", cx);
    keys("escape 0 \" a y $ u", cx);
    assert_eq!(text_of(&view, cx), "\n");
    keys("@ a", cx);
    cx.run_until_parked();
    assert_eq!(text_of(&view, cx), "\n");
    assert_eq!(indicator(&wb, &view, cx), "-- NORMAL --");
    // Vim works on after the run: no macro is left running.
    keys("i", cx);
    commit(&view, "z", cx);
    keys("escape", cx);
    assert_eq!(text_of(&view, cx), "z\n");
}

#[gpui::test]
fn macro_with_colon_command(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("a\na\na\n", cx);
    keys("q a :", cx);
    type_text("s/a/b/", cx);
    keys("enter j q", cx);
    assert_eq!(text_of(&view, cx), "b\na\na\n");
    keys("@ a", cx);
    assert_eq!(text_of(&view, cx), "b\nb\na\n");
    assert_eq!(cursor(&view, cx), pt(2, 0));
    assert_eq!(indicator(&wb, &view, cx), "1 substitution on 1 line");
}
