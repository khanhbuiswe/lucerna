//! Vim's `:` command line through Quick Open: the rows it shows, what
//! Enter runs, the status bar messages, saving and closing, and a large
//! `:s` in the background.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use gpui::{Entity, Focusable as _, Keystroke, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, QuickOpen, Workbench};
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
fn editor_with<'a>(
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
    editor_with(VIM_ON, text, cx)
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    draw(cx);
}

/// Types `text` into the focused text input (Quick Open's).
fn type_text(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    draw(cx);
}

/// `:`, the command `line`, Enter.
fn ex(line: &str, cx: &mut VisualTestContext) {
    keys(":", cx);
    type_text(line, cx);
    keys("enter", cx);
}

fn quick_open(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<Entity<QuickOpen>> {
    cx.update(|_, cx| wb.read(cx).quick_open().cloned())
}

fn rows(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    let qo = quick_open(wb, cx).expect("quick open is shown");
    cx.update(|_, cx| qo.read(cx).row_texts())
}

fn query(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> String {
    let qo = quick_open(wb, cx).expect("quick open is shown");
    cx.update(|_, cx| qo.read(cx).query(cx))
}

/// Shows Quick Open with `text` in its input, as if typed.
fn set_query(wb: &Entity<Workbench>, text: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.show_quick_open(text, window, cx)));
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

fn indicator(
    wb: &Entity<Workbench>,
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
) -> String {
    let id = view.entity_id();
    cx.update(|_, cx| wb.read(cx).vim().read(cx).indicator_text(id))
        .expect("vim is on")
}

fn tab_count(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups.model().groups().iter().map(|g| g.tabs.len()).sum()
    })
}

fn disk(dir: &TempDir) -> String {
    fs::read_to_string(dir.path().join("file.txt")).unwrap()
}

fn is_dirty(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    let docs = wb.read_with(cx, |wb, _| wb.documents().clone());
    docs.read_with(cx, |d, _| d.is_dirty(Path::new("file.txt")))
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

/// Over `BACKGROUND_SUBSTITUTE_BYTES`: `abc 1` lines, then filler.
fn large_text() -> String {
    let mut text: String = (0..60).map(|i| format!("abc {}\n", i % 10)).collect();
    text.push_str(&("z".repeat(99) + "\n").repeat(11_000));
    assert!(text.len() > lucerna_vim::BACKGROUND_SUBSTITUTE_BYTES);
    text
}

#[gpui::test]
fn colon_opens_quick_open_with_rows(cx: &mut TestAppContext) {
    let (_dir, wb, _view, cx) = vim_editor("foo\n", cx);
    keys(":", cx);
    assert_eq!(query(&wb, cx), ":");
    assert_eq!(rows(&wb, cx), ["Type a Vim command…"]);
    type_text("%s/foo/bar/g", cx);
    assert_eq!(rows(&wb, cx), [":%s/foo/bar/g — Replace in all lines"]);
    set_query(&wb, ":w", cx);
    assert_eq!(rows(&wb, cx), [":w — Save"]);
    set_query(&wb, ":12", cx);
    assert_eq!(rows(&wb, cx), [":12 — Go to line 12"]);
    set_query(&wb, ":foo", cx);
    assert_eq!(rows(&wb, cx), ["Not an editor command: foo"]);
}

fn twenty_lines() -> String {
    (1..=20)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[gpui::test]
fn colon_dispatch_vim_off(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = editor_with("{}", &twenty_lines(), cx);
    keys("ctrl-g", cx);
    assert_eq!(query(&wb, cx), ":");
    type_text("-1", cx);
    keys("enter", cx);
    assert_eq!(cursor(&view, cx).row, 19);
    set_query(&wb, ":12,5", cx);
    keys("enter", cx);
    assert_eq!(cursor(&view, cx), pt(11, 4));
}

#[gpui::test]
fn colon_dispatch_vim_on(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor(&twenty_lines(), cx);
    keys("1 0 shift-g", cx);
    assert_eq!(cursor(&view, cx).row, 9);
    ex("-1", cx);
    assert_eq!(cursor(&view, cx).row, 8);
    // A reversed range is swapped; it goes to the range's last line.
    ex("12,5", cx);
    assert_eq!(cursor(&view, cx).row, 11);
    // With no editor open, `:` is go to line.
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("workbench.action.closeActiveEditor", None, window, cx)
        })
    });
    draw(cx);
    assert_eq!(tab_count(&wb, cx), 0);
    set_query(&wb, ":12", cx);
    assert_eq!(rows(&wb, cx), ["Open a file to go to a line"]);
}

#[gpui::test]
fn visual_colon_prefills_marks(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("a\nb\nc\n", cx);
    keys("shift-v j :", cx);
    assert_eq!(query(&wb, cx), ":'<,'>");
    type_text("d", cx);
    keys("enter", cx);
    assert_eq!(text_of(&view, cx), "c\n");
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "a\nb\nc\n");
}

#[gpui::test]
fn colon_w_saves(cx: &mut TestAppContext) {
    let (dir, wb, view, cx) = vim_editor("abc\n", cx);
    keys("x", cx);
    ex("w", cx);
    assert_eq!(disk(&dir), "bc\n");
    assert!(!is_dirty(&wb, cx));
    assert_eq!(text_of(&view, cx), "bc\n");
}

#[gpui::test]
fn colon_q_dirty_shows_prompt(cx: &mut TestAppContext) {
    let (_dir, wb, _view, cx) = vim_editor("abc\n", cx);
    keys("x", cx);
    ex("q", cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    draw(cx);
    assert_eq!(tab_count(&wb, cx), 1);
}

#[gpui::test]
fn colon_q_closes_a_clean_tab(cx: &mut TestAppContext) {
    let (_dir, wb, _view, cx) = vim_editor("abc\n", cx);
    ex("q", cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(tab_count(&wb, cx), 0);
}

#[gpui::test]
fn colon_q_bang_reverts_and_closes(cx: &mut TestAppContext) {
    let (dir, wb, _view, cx) = vim_editor("abc\n", cx);
    keys("x", cx);
    ex("q!", cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(disk(&dir), "abc\n");
    assert_eq!(tab_count(&wb, cx), 0);
    assert!(!is_dirty(&wb, cx));
}

#[gpui::test]
fn colon_wq_saves_and_closes(cx: &mut TestAppContext) {
    let (dir, wb, _view, cx) = vim_editor("abc\n", cx);
    keys("x", cx);
    ex("wq", cx);
    assert_eq!(disk(&dir), "bc\n");
    assert_eq!(tab_count(&wb, cx), 0);
}

#[gpui::test]
fn colon_x_clean_closes_without_write(cx: &mut TestAppContext) {
    let (dir, wb, _view, cx) = vim_editor("abc\n", cx);
    let file = dir.path().join("file.txt");
    let before = fs::metadata(&file).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    ex("x", cx);
    assert_eq!(tab_count(&wb, cx), 0);
    assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), before);
}

#[gpui::test]
fn colon_x_dirty_saves_and_closes(cx: &mut TestAppContext) {
    let (dir, wb, _view, cx) = vim_editor("abc\n", cx);
    keys("x", cx);
    ex("x", cx);
    assert_eq!(disk(&dir), "bc\n");
    assert_eq!(tab_count(&wb, cx), 0);
}

fn open_other(wb: &Entity<Workbench>, dir: &TempDir, cx: &mut VisualTestContext) {
    fs::write(dir.path().join("other.txt"), "other\n").unwrap();
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("other.txt")), OpenMode::Pinned, cx)
    });
    draw(cx);
    let view = active_view(wb, cx);
    focus(&view, cx);
}

#[gpui::test]
fn colon_qa_closes_all(cx: &mut TestAppContext) {
    let (dir, wb, _view, cx) = vim_editor("abc\n", cx);
    open_other(&wb, &dir, cx);
    assert_eq!(tab_count(&wb, cx), 2);
    ex("qa", cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(tab_count(&wb, cx), 0);
}

#[gpui::test]
fn colon_qa_dirty_asks_first(cx: &mut TestAppContext) {
    let (dir, wb, _view, cx) = vim_editor("abc\n", cx);
    keys("x", cx);
    open_other(&wb, &dir, cx);
    keys("x", cx);
    ex("qa", cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    draw(cx);
    assert_eq!(tab_count(&wb, cx), 2);
    ex("qa", cx);
    cx.simulate_prompt_answer("Don't Save");
    draw(cx);
    assert_eq!(tab_count(&wb, cx), 0);
    assert_eq!(disk(&dir), "abc\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("other.txt")).unwrap(),
        "other\n"
    );
}

#[gpui::test]
fn colon_qa_save_all_saves_then_closes(cx: &mut TestAppContext) {
    let (dir, wb, _view, cx) = vim_editor("abc\n", cx);
    keys("x", cx);
    open_other(&wb, &dir, cx);
    keys("x", cx);
    ex("qa", cx);
    cx.simulate_prompt_answer("Save All");
    draw(cx);
    assert_eq!(tab_count(&wb, cx), 0);
    assert_eq!(disk(&dir), "bc\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("other.txt")).unwrap(),
        "ther\n"
    );
}

#[gpui::test]
fn w_bang_overwrites_without_prompt(cx: &mut TestAppContext) {
    let (dir, wb, view, cx) = vim_editor("abc\n", cx);
    keys("x", cx);
    change_on_disk(&wb, &dir, "disk\n", cx);
    assert_eq!(text_of(&view, cx), "bc\n");
    ex("w!", cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(disk(&dir), "bc\n");
    assert!(!is_dirty(&wb, cx));
}

#[gpui::test]
fn wq_conflict_cancel_keeps_tab(cx: &mut TestAppContext) {
    let (dir, wb, view, cx) = vim_editor("abc\n", cx);
    keys("x", cx);
    change_on_disk(&wb, &dir, "disk\n", cx);
    ex("wq", cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    draw(cx);
    assert_eq!(tab_count(&wb, cx), 1);
    assert!(is_dirty(&wb, cx));
    assert_eq!(text_of(&view, cx), "bc\n");
    assert_eq!(disk(&dir), "disk\n");
}

#[gpui::test]
fn wq_conflict_reload_keeps_tab(cx: &mut TestAppContext) {
    // Reload declines the write: the quit is aborted, as in Vim, though
    // the reloaded text is clean.
    let (dir, wb, view, cx) = vim_editor("abc\n", cx);
    keys("x", cx);
    change_on_disk(&wb, &dir, "disk\n", cx);
    ex("wq", cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Reload");
    draw(cx);
    assert_eq!(tab_count(&wb, cx), 1);
    assert_eq!(text_of(&view, cx), "disk\n");
    assert!(!is_dirty(&wb, cx));
    assert_eq!(disk(&dir), "disk\n");
}

#[gpui::test]
fn substitute_reports_in_status_bar(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("a a\na", cx);
    ex("%s/a/b/g", cx);
    assert_eq!(text_of(&view, cx), "b b\nb");
    assert_eq!(indicator(&wb, &view, cx), "3 substitutions on 2 lines");
    keys("j", cx);
    assert_eq!(indicator(&wb, &view, cx), "-- NORMAL --");
    keys("u", cx);
    assert_eq!(text_of(&view, cx), "a a\na");
}

#[gpui::test]
fn unknown_command_message(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = vim_editor("abc\n", cx);
    ex("foo", cx);
    assert_eq!(
        indicator(&wb, &view, cx),
        "E492: Not an editor command: foo"
    );
    assert_eq!(text_of(&view, cx), "abc\n");
}

#[gpui::test(iterations = 10)]
fn large_file_substitute_cancelled_by_outside_change(cx: &mut TestAppContext) {
    // The text changes (an agent's edit) while the background `:s` runs:
    // its result is for the old text and must not be applied. The change
    // is applied at once; a reload from disk would race the `:s` on the
    // test scheduler.
    let text = large_text();
    let (_dir, wb, view, cx) = vim_editor(&text, cx);
    keys(":", cx);
    type_text("%s/abc/X/g", cx);
    // Enter starts the background `:s`; the change comes before it ends
    // (no parking in between).
    cx.update(|window, cx| window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx));
    let origin = wb.entity_id();
    let documents = wb.read_with(cx, |wb, _| wb.documents().clone());
    documents.update(cx, |d, cx| {
        let edit = (0..text.len(), Arc::<str>::from("new\nabc\n"));
        d.apply_edits(Path::new("file.txt"), vec![edit], None, None, origin, cx)
            .expect("the file is loaded");
    });
    draw(cx);
    assert_eq!(text_of(&view, cx), "new\nabc\n");
    assert_eq!(indicator(&wb, &view, cx), "E: file changed, :s cancelled");
}

#[gpui::test]
fn large_file_substitute_runs_in_background(cx: &mut TestAppContext) {
    let text = large_text();
    let (_dir, wb, view, cx) = vim_editor(&text, cx);
    ex("%s/abc/X/g", cx);
    assert_eq!(text_of(&view, cx), text.replace("abc", "X"));
    assert_eq!(indicator(&wb, &view, cx), "60 substitutions on 60 lines");
    // The cursor is on the last line replaced; `n` finds the pattern.
    assert_eq!(cursor(&view, cx), pt(59, 0));
    keys("u", cx);
    assert_eq!(text_of(&view, cx), text);
}

#[gpui::test]
fn background_substitute_matches_the_sync_path(cx: &mut TestAppContext) {
    // The same `:s` on the same large text: once in the background (typed
    // in Quick Open while recording), once at once (a macro runs it).
    let text = large_text();
    let (_dir, _wb, view, cx) = vim_editor(&text, cx);
    keys("q a", cx);
    ex(r"1,40s/\(b\)c \(\d\)/\2\u\1\n/g", cx);
    keys("q", cx);
    let background = text_of(&view, cx);
    assert_ne!(background, text);
    assert!(
        background.starts_with("a0B\n\na1B\n\n"),
        "{}",
        &background[..20]
    );
    keys("u", cx);
    assert_eq!(text_of(&view, cx), text);
    keys("@ a", cx);
    assert_eq!(text_of(&view, cx), background);
    // One undo step there too.
    keys("u", cx);
    assert_eq!(text_of(&view, cx), text);
}

#[gpui::test]
fn noh_clears_highlights(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = vim_editor("foo bar foo\n", cx);
    keys("/", cx);
    type_text("foo", cx);
    keys("enter", cx);
    assert!(!view.read_with(cx, |v, _| v.match_highlights().is_empty()));
    ex("noh", cx);
    assert!(view.read_with(cx, |v, _| v.match_highlights().is_empty()));
}

#[gpui::test]
fn gdefault_setting_inverts_g(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = editor_with(
        r#"{ "vim.enabled": true, "vim.gdefault": true }"#,
        "aaa\n",
        cx,
    );
    ex("s/a/b/", cx);
    assert_eq!(text_of(&view, cx), "bbb\n");
}
