//! The vim layer inside the workbench: settings, key routing, applying
//! vim's actions to the code view, context keys and the mode indicator.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    BorrowAppContext as _, Entity, Focusable as _, KeyDownEvent, Modifiers, TestAppContext,
    VisualTestContext,
};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::{Buffer, BufferSnapshot, Point};
use lucerna_theme::ThemeRegistry;
use lucerna_vim::Mode;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, FindWidgetEvent, OpenMode, TabItem, Workbench};

const VIM_ON: &str = r#"{ "vim.enabled": true }"#;

fn init(cx: &mut TestAppContext, user_settings: Option<&str>) {
    init_with_keymap(cx, user_settings, None);
}

fn init_with_keymap(
    cx: &mut TestAppContext,
    user_settings: Option<&str>,
    user_keymap: Option<&str>,
) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), user_keymap);
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

fn snapshot(text: &str) -> BufferSnapshot {
    Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot()
}

fn p(name: &str) -> Arc<Path> {
    Arc::from(Path::new(name))
}

fn pt(row: u32, column: u32) -> Point {
    Point { row, column }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

/// A workbench whose tabs get code views; the caller loads them.
fn workbench(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut VisualTestContext) {
    let (wb, cx) =
        cx.add_window_view(|window, cx| Workbench::new(PathBuf::from("/tmp/project"), window, cx));
    wb.update(cx, |wb, cx| {
        wb.editor_groups().update(cx, |groups, cx| {
            groups.set_tab_body_factory(Rc::new(CodeView::tab_body), cx)
        })
    });
    (wb, cx)
}

/// Opens `path` in the workbench and loads `text` into its code view.
fn open(
    wb: &Entity<Workbench>,
    path: &str,
    text: &str,
    cx: &mut VisualTestContext,
) -> Entity<CodeView> {
    wb.update(cx, |wb, cx| wb.open_path(p(path), OpenMode::Pinned, cx));
    // Tab bodies are made while drawing.
    draw(cx);
    let view = active_code_view(wb, cx).expect("the active tab shows a code view");
    let snapshot = snapshot(text);
    view.update(cx, |v, cx| v.set_buffer(snapshot, cx));
    draw(cx);
    view
}

fn active_code_view(
    wb: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> Option<Entity<CodeView>> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()?
            .downcast::<CodeView>()
            .ok()
    })
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    let ran =
        cx.update(|window, cx| wb.update(cx, |wb, cx| wb.run_command(command, None, window, cx)));
    assert!(ran, "{command} is registered");
}

/// Space-separated keystrokes, e.g. `"j j shift-g"`. GPUI's test platform
/// fills `key_char` for printable keys, as macOS does.
fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    draw(cx);
}

/// Types `text` into the focused text input (the find widget's).
fn type_text(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    draw(cx);
}

fn find_visible(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, _| v.find_widget().is_some())
}

/// A workbench with vim on and one file open.
fn vim_file<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (
    Entity<Workbench>,
    Entity<CodeView>,
    &'a mut VisualTestContext,
) {
    init(cx, Some(VIM_ON));
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.rs", text, cx);
    (wb, view, cx)
}

fn cursor(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Point {
    view.read_with(cx, |v, _| v.cursor())
}

fn overlay(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Option<Point> {
    view.read_with(cx, |v, _| v.cursor_overlay().map(|o| o.at))
}

fn mode(
    wb: &Entity<Workbench>,
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
) -> Option<Mode> {
    let id = view.entity_id();
    cx.update(|_, cx| wb.read(cx).vim().read(cx).mode_for(id))
}

fn indicator(
    wb: &Entity<Workbench>,
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
) -> Option<String> {
    let id = view.entity_id();
    cx.update(|_, cx| wb.read(cx).vim().read(cx).indicator_text(id))
}

fn set_user_settings(json: &str, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        cx.update_global::<AppState, _>(|state, _| {
            state.settings.set_layer(Layer::User, json).unwrap();
        })
    });
    draw(cx);
}

fn reload(view: &Entity<CodeView>, old: &str, new: &str, cx: &mut VisualTestContext) {
    let mut buffer = Buffer::from_bytes(old.as_bytes(), false).unwrap();
    let reload = buffer.reload(new.as_bytes()).unwrap();
    let snapshot = buffer.snapshot();
    view.update(cx, |v, cx| v.apply_reload(snapshot, reload, cx));
    draw(cx);
}

#[gpui::test]
fn disabled_by_default_keys_pass_through(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.rs", "a\nb\nc\n", cx);
    keys("j", cx);
    assert_eq!(cursor(&view, cx), pt(0, 0));
    assert_eq!(overlay(&view, cx), None);
    assert_eq!(indicator(&wb, &view, cx), None);
    keys("cmd-p", cx);
    assert!(cx.update(|_, cx| wb.read(cx).quick_open().is_some()));
}

#[gpui::test]
fn j_moves_cursor_when_enabled(cx: &mut TestAppContext) {
    let (_wb, view, cx) = vim_file("a\nb\nc", cx);
    keys("j j", cx);
    assert_eq!(cursor(&view, cx), pt(2, 0));
    assert_eq!(overlay(&view, cx), Some(pt(2, 0)));
    keys("k", cx);
    assert_eq!(cursor(&view, cx), pt(1, 0));
}

#[gpui::test]
fn vim_mode_alias_enables(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "vim_mode": true }"#));
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.rs", "a\nb\nc", cx);
    keys("j", cx);
    assert_eq!(cursor(&view, cx), pt(1, 0));
}

#[gpui::test]
fn explicit_vim_enabled_false_beats_alias(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "vim.enabled": false, "vim_mode": true }"#));
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.rs", "a\nb\nc", cx);
    keys("j", cx);
    assert_eq!(cursor(&view, cx), pt(0, 0));
    assert!(!cx.update(|_, cx| wb.read(cx).vim().read(cx).enabled()));
}

#[gpui::test]
fn keys_go_to_quick_open_input(cx: &mut TestAppContext) {
    let (wb, view, cx) = vim_file("a\nb\nc", cx);
    keys("cmd-p", cx);
    keys("j k", cx);
    let query = cx.update(|_, cx| {
        let qo = wb
            .read(cx)
            .quick_open()
            .cloned()
            .expect("quick open is shown");
        qo.read(cx).query(cx)
    });
    assert_eq!(query, "jk");
    assert_eq!(cursor(&view, cx), pt(0, 0));
}

#[gpui::test]
fn escape_still_closes_find_widget(cx: &mut TestAppContext) {
    let (wb, view, cx) = vim_file("foo\nbar\n", cx);
    run(&wb, "actions.find", cx);
    draw(cx);
    assert!(view.read_with(cx, |v, _| v.find_widget().is_some()));
    keys("escape", cx);
    assert!(view.read_with(cx, |v, _| v.find_widget().is_none()));
}

#[gpui::test]
fn visual_yank_writes_clipboard(cx: &mut TestAppContext) {
    let (_wb, view, cx) = vim_file("abc\ndef", cx);
    keys("v j", cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.selection()),
        Some(pt(0, 0)..pt(1, 1))
    );
    keys("y", cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("abc\nd"));
    assert_eq!(view.read_with(cx, |v, _| v.selection()), None);
    assert_eq!(overlay(&view, cx), Some(pt(0, 0)));
}

#[gpui::test]
fn mode_indicator_text(cx: &mut TestAppContext) {
    let (wb, view, cx) = vim_file("abc\ndef\n", cx);
    assert_eq!(indicator(&wb, &view, cx).as_deref(), Some("-- NORMAL --"));
    assert!(cx.debug_bounds("vim-mode-indicator").is_some());
    keys("shift-v", cx);
    assert_eq!(
        indicator(&wb, &view, cx).as_deref(),
        Some("-- VISUAL LINE --")
    );
    keys("escape 2", cx);
    assert_eq!(
        indicator(&wb, &view, cx).as_deref(),
        Some("-- NORMAL --  2")
    );
}

#[gpui::test]
fn context_key_vim_mode_in_user_keybinding(cx: &mut TestAppContext) {
    init_with_keymap(
        cx,
        Some(VIM_ON),
        Some(
            r#"[{ "key": "ctrl+k", "command": "editor.action.toggleWordWrap", "when": "vim.mode == 'Normal'" }]"#,
        ),
    );
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.rs", "abc\n", cx);
    assert!(!view.read_with(cx, |v, _| v.word_wrap()));
    keys("ctrl-k", cx);
    assert!(view.read_with(cx, |v, _| v.word_wrap()));
    // In Visual mode the binding does not apply.
    keys("v ctrl-k", cx);
    assert!(view.read_with(cx, |v, _| v.word_wrap()));
}

#[gpui::test]
fn mouse_click_resets_to_normal(cx: &mut TestAppContext) {
    let (wb, view, cx) = vim_file("abc\ndef\nghi\n", cx);
    keys("v", cx);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Visual));
    let bounds = cx
        .debug_bounds("editor-groups")
        .expect("the editor groups are drawn");
    cx.simulate_click(bounds.center(), Modifiers::none());
    draw(cx);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
}

#[gpui::test]
fn ime_key_without_char_is_ignored(cx: &mut TestAppContext) {
    let (_wb, view, cx) = vim_file("abc def\n", cx);
    // `e` would move to the end of the word; without a char it is an IME
    // composition key and not vim's.
    cx.simulate_event(KeyDownEvent {
        keystroke: gpui::Keystroke {
            modifiers: Modifiers::none(),
            key: "e".into(),
            key_char: None,
        },
        is_held: false,
        prefer_character_input: false,
    });
    draw(cx);
    assert_eq!(cursor(&view, cx), pt(0, 0));
    assert_eq!(overlay(&view, cx), Some(pt(0, 0)));
}

#[gpui::test]
fn pending_state_is_per_view(cx: &mut TestAppContext) {
    init(cx, Some(VIM_ON));
    let (wb, cx) = workbench(cx);
    let a = open(&wb, "a.rs", "a\nb\nc\nd\n", cx);
    keys("cmd-\\", cx);
    let b = open(&wb, "b.rs", "1\n2\n3\n4\n", cx);
    assert_ne!(a.entity_id(), b.entity_id());
    keys("j j", cx);
    assert_eq!(cursor(&b, cx), pt(2, 0));
    keys("cmd-1", cx);
    keys("j g", cx);
    assert_eq!(cursor(&a, cx), pt(1, 0));
    keys("cmd-2", cx);
    keys("g", cx);
    assert_eq!(cursor(&b, cx), pt(2, 0), "b had no pending g");
    keys("g", cx);
    assert_eq!(cursor(&b, cx), pt(0, 0), "gg in b");
}

#[gpui::test]
fn reload_in_visual_clamps_selection(cx: &mut TestAppContext) {
    let text = "a\nb\nc\nd\ne\n";
    let (wb, view, cx) = vim_file(text, cx);
    keys("shift-v j j", cx);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::VisualLine));
    reload(&view, text, "x", cx);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::VisualLine));
    assert_eq!(overlay(&view, cx).map(|p| p.row), Some(0));
    keys("j", cx);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::VisualLine));
    assert_eq!(overlay(&view, cx).map(|p| p.row), Some(0));
}

#[gpui::test]
fn disabling_vim_clears_overlay_and_selection(cx: &mut TestAppContext) {
    let (wb, view, cx) = vim_file("abc\ndef\n", cx);
    keys("v l", cx);
    assert!(view.read_with(cx, |v, _| v.selection()).is_some());
    set_user_settings(r#"{ "vim.enabled": false }"#, cx);
    assert_eq!(view.read_with(cx, |v, _| v.selection()), None);
    assert_eq!(overlay(&view, cx), None);
    assert_eq!(indicator(&wb, &view, cx), None);
    assert!(cx.debug_bounds("vim-mode-indicator").is_none());
    let before = cursor(&view, cx);
    keys("j", cx);
    assert_eq!(cursor(&view, cx), before);
    // Turning it back on starts in Normal mode.
    set_user_settings(VIM_ON, cx);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
}

#[gpui::test]
fn reopened_file_keeps_marks_new_state(cx: &mut TestAppContext) {
    let text = "a\nb\nc\nd\n";
    let (wb, view, cx) = vim_file(text, cx);
    keys("j j m a v", cx);
    let old = view.entity_id();
    keys("cmd-w", cx);
    let view = open(&wb, "a.rs", text, cx);
    assert_ne!(view.entity_id(), old);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    keys("g g ' a", cx);
    assert_eq!(cursor(&view, cx), pt(2, 0));
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
}

#[gpui::test]
fn fold_keys_fold_and_move_cursor(cx: &mut TestAppContext) {
    let (_wb, view, cx) = vim_file("fn a() {\n    x\n    y\n}\nlast\n", cx);
    assert!(
        view.read_with(cx, |v, _| !v.fold_ranges().is_empty()),
        "the folds were computed"
    );
    keys("j j", cx);
    assert_eq!(cursor(&view, cx), pt(2, 0));
    keys("z c", cx);
    assert!(view.read_with(cx, |v, _| v.is_folded(0)));
    assert_eq!(cursor(&view, cx).row, 0);
    assert_eq!(overlay(&view, cx).map(|p| p.row), Some(0));
}

#[gpui::test]
fn star_highlights_matches(cx: &mut TestAppContext) {
    let (_wb, view, cx) = vim_file("foo bar\nbaz foo\n", cx);
    keys("*", cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.match_highlights().to_vec()),
        vec![0..3, 12..15]
    );
    assert_eq!(cursor(&view, cx), pt(1, 4));
}

#[gpui::test]
fn marks_follow_reload(cx: &mut TestAppContext) {
    let old: String = (0..10).map(|i| format!("line {i}\n")).collect();
    let (_wb, view, cx) = vim_file(&old, cx);
    keys("5 j m a", cx);
    assert_eq!(cursor(&view, cx), pt(5, 0));
    let new = format!("new\nnew\n{old}");
    reload(&view, &old, &new, cx);
    keys("g g ` a", cx);
    assert_eq!(cursor(&view, cx), pt(7, 0));
}

#[gpui::test]
fn star_in_large_file_searches_in_background(cx: &mut TestAppContext) {
    // Over `BACKGROUND_SEARCH_BYTES`: the key is fed again once the
    // background search is done.
    let filler = "x".repeat(99) + "\n";
    let text = format!("needle\n{}needle\n", filler.repeat(11_000));
    assert!(text.len() > lucerna_workbench::BACKGROUND_SEARCH_BYTES);
    let (_wb, view, cx) = vim_file(&text, cx);
    keys("*", cx);
    assert_eq!(view.read_with(cx, |v, _| v.match_highlights().len()), 2);
    assert_eq!(cursor(&view, cx), pt(11_001, 0));
}

#[gpui::test]
fn visual_block_selects_rows(cx: &mut TestAppContext) {
    let (wb, view, cx) = vim_file("abcd\nefgh\nijkl\n", cx);
    keys("l ctrl-v j l", cx);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::VisualBlock));
    assert_eq!(
        view.read_with(cx, |v, _| v.copy_selection()).as_deref(),
        Some("bc\nfg")
    );
    assert_eq!(overlay(&view, cx), Some(pt(1, 2)));
    keys("escape", cx);
    assert_eq!(view.read_with(cx, |v, _| v.copy_selection()), None);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
}

#[gpui::test]
fn block_cursor_shows_before_first_key(cx: &mut TestAppContext) {
    let (wb, view, cx) = vim_file("abc\ndef\n", cx);
    assert_eq!(overlay(&view, cx), Some(pt(0, 0)));
    assert_eq!(indicator(&wb, &view, cx).as_deref(), Some("-- NORMAL --"));
}

#[gpui::test]
fn enabling_vim_at_runtime_draws_cursor(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.rs", "abc\ndef\n", cx);
    assert_eq!(overlay(&view, cx), None);
    set_user_settings(VIM_ON, cx);
    assert_eq!(overlay(&view, cx), Some(pt(0, 0)));
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    keys("j", cx);
    assert_eq!(cursor(&view, cx), pt(1, 0));
}

#[gpui::test]
fn modified_keys_go_to_keymap(cx: &mut TestAppContext) {
    init_with_keymap(
        cx,
        Some(VIM_ON),
        Some(r#"[{ "key": "alt+up", "command": "editor.action.toggleWordWrap" }]"#),
    );
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.rs", "a\nb\nc\nd\n", cx);
    let wrap = |cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.word_wrap());
    keys("j", cx);
    keys("alt-down shift-down ctrl-down", cx);
    assert_eq!(cursor(&view, cx), pt(1, 0), "modified arrows are not vim's");
    keys("alt-up", cx);
    assert!(wrap(cx), "the user binding on alt+up ran");
    keys("alt-z", cx);
    assert!(!wrap(cx), "alt+z toggles wrap back");
    keys("down", cx);
    assert_eq!(cursor(&view, cx), pt(2, 0), "a plain arrow is vim's j");
}

#[gpui::test]
fn escape_in_normal_mode_reaches_keymap(cx: &mut TestAppContext) {
    init_with_keymap(
        cx,
        Some(VIM_ON),
        Some(
            r#"[{ "key": "escape", "command": "editor.action.toggleWordWrap", "when": "editorFocus && vim.mode == 'Normal'" }]"#,
        ),
    );
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.rs", "abc\n", cx);
    let wrap = |cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.word_wrap());
    // With a count pending, Escape only clears it.
    keys("2 escape", cx);
    assert!(!wrap(cx));
    assert_eq!(indicator(&wb, &view, cx).as_deref(), Some("-- NORMAL --"));
    keys("escape", cx);
    assert!(wrap(cx));
}

#[gpui::test]
fn pending_keymap_chord_is_not_vims(cx: &mut TestAppContext) {
    init_with_keymap(
        cx,
        Some(VIM_ON),
        Some(r#"[{ "key": "cmd+k j", "command": "editor.action.toggleWordWrap" }]"#),
    );
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.rs", "a\nb\n", cx);
    keys("cmd-k j", cx);
    assert!(view.read_with(cx, |v, _| v.word_wrap()));
    assert_eq!(cursor(&view, cx), pt(0, 0));
}

#[gpui::test]
fn reload_before_any_key_moves_marks(cx: &mut TestAppContext) {
    let text = "a\nb\nc\nd\n";
    let (wb, _view, cx) = vim_file(text, cx);
    keys("j j m a", cx);
    keys("cmd-w", cx);
    let view = open(&wb, "a.rs", text, cx);
    // An agent edits the file before the user presses a key.
    reload(&view, text, &format!("new\nnew\n{text}"), cx);
    keys("` a", cx);
    assert_eq!(cursor(&view, cx), pt(4, 0));
}

#[gpui::test]
fn reload_of_cursor_row_keeps_view_cursor_on_vim_cursor(cx: &mut TestAppContext) {
    let text = "a\nb\nc\nd\n";
    let (wb, view, cx) = vim_file(text, cx);
    keys("j j", cx);
    // An agent edits the cursor's line: the view drops its selection
    // there, and takes vim's cursor instead of falling back to (0,0).
    reload(&view, text, "a\nb\nC!\nd\n", cx);
    assert_eq!(overlay(&view, cx), Some(pt(2, 0)));
    assert_eq!(Some(cursor(&view, cx)), overlay(&view, cx));
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    keys("k", cx);
    assert_eq!(cursor(&view, cx), pt(1, 0));
}

#[gpui::test]
fn reload_keeps_a_mouse_selection_on_unchanged_rows(cx: &mut TestAppContext) {
    let text = "abc\ndef\nghi\n";
    let (wb, view, cx) = vim_file(text, cx);
    keys("j", cx);
    // A selection made outside vim, as the mouse does.
    view.update(cx, |v, cx| v.select(pt(0, 1)..pt(1, 2), cx));
    draw(cx);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    reload(&view, text, "new\nabc\ndef\nghi\n", cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.selection()),
        Some(pt(1, 1)..pt(2, 2))
    );
    assert_eq!(overlay(&view, cx), Some(pt(2, 2)));
}

#[gpui::test]
fn slash_search_jumps_and_n_continues(cx: &mut TestAppContext) {
    let (wb, view, cx) = vim_file("foo\nbar foo\nfoo", cx);
    keys("/", cx);
    assert!(find_visible(&view, cx));
    type_text("foo", cx);
    keys("enter", cx);
    assert_eq!(cursor(&view, cx), pt(1, 4));
    assert_eq!(overlay(&view, cx), Some(pt(1, 4)));
    assert!(!find_visible(&view, cx));
    assert!(!view.read_with(cx, |v, _| v.match_highlights().is_empty()));
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    keys("n", cx);
    assert_eq!(cursor(&view, cx), pt(2, 0));
}

#[gpui::test]
fn question_search_goes_backward(cx: &mut TestAppContext) {
    let (_wb, view, cx) = vim_file("foo\nbar foo\nfoo", cx);
    keys("shift-g", cx);
    assert_eq!(cursor(&view, cx), pt(2, 0));
    keys("?", cx);
    type_text("foo", cx);
    keys("enter", cx);
    assert_eq!(cursor(&view, cx), pt(1, 4));
    keys("n", cx);
    assert_eq!(cursor(&view, cx), pt(0, 0));
}

#[gpui::test]
fn escape_in_vim_search_restores_cursor(cx: &mut TestAppContext) {
    let (wb, view, cx) = vim_file("foo\nbar foo\nfoo", cx);
    keys("l", cx);
    keys("/", cx);
    type_text("bar", cx);
    // The incremental search selected the match; vim's cursor stays.
    let selected = view.read_with(cx, |v, _| v.selection());
    assert_eq!(selected.map(|r| r.start.row), Some(1));
    keys("escape", cx);
    assert!(!find_visible(&view, cx));
    assert_eq!(cursor(&view, cx), pt(0, 1));
    assert_eq!(overlay(&view, cx), Some(pt(0, 1)));
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    keys("j", cx);
    assert_eq!(cursor(&view, cx), pt(1, 1));
}

#[gpui::test]
fn count_slash(cx: &mut TestAppContext) {
    let (_wb, view, cx) = vim_file("foo\nbar foo\nfoo", cx);
    keys("2 /", cx);
    type_text("foo", cx);
    keys("enter", cx);
    assert_eq!(cursor(&view, cx), pt(2, 0));
}

#[gpui::test]
fn slash_search_in_large_file_searches_in_background(cx: &mut TestAppContext) {
    let filler = "x".repeat(99) + "\n";
    let text = format!("needle\n{}needle\n", filler.repeat(11_000));
    assert!(text.len() > lucerna_workbench::BACKGROUND_SEARCH_BYTES);
    let (_wb, view, cx) = vim_file(&text, cx);
    keys("/", cx);
    type_text("needle", cx);
    keys("enter", cx);
    assert_eq!(cursor(&view, cx), pt(11_001, 0));
    assert_eq!(view.read_with(cx, |v, _| v.match_highlights().len()), 2);
}

#[gpui::test]
fn plain_find_widget_unchanged(cx: &mut TestAppContext) {
    let (_wb, view, cx) = vim_file("foo\nbar foo\nfoo", cx);
    keys("cmd-f", cx);
    let find = view.read_with(cx, |v, _| v.find_widget().cloned().expect("find is shown"));
    let events = Rc::new(std::cell::Cell::new(0));
    let seen = events.clone();
    let _sub = cx.update(|_, cx| {
        cx.subscribe(&find, move |_, _: &FindWidgetEvent, _| {
            seen.set(seen.get() + 1)
        })
    });
    type_text("foo", cx);
    assert_eq!(find.read_with(cx, |f, _| f.active_index()), Some(0));
    keys("enter", cx);
    assert_eq!(find.read_with(cx, |f, _| f.active_index()), Some(1));
    assert!(find_visible(&view, cx));
    assert_eq!(
        view.read_with(cx, |v, _| v.selection()),
        Some(pt(1, 4)..pt(1, 7))
    );
    keys("escape", cx);
    assert_eq!(events.get(), 0, "plain find emits no vim event");
}

#[gpui::test]
fn reload_during_slash_then_escape_keeps_cursor_on_text(cx: &mut TestAppContext) {
    let text = "abc\nabc\nabc\nabcd";
    let (_wb, view, cx) = vim_file(text, cx);
    keys("j j j $", cx);
    assert_eq!(cursor(&view, cx), pt(3, 3));
    keys("/", cx);
    type_text("b", cx);
    // An agent shortens the file while the prompt is open.
    reload(&view, text, "ab\n", cx);
    keys("escape", cx);
    assert!(!find_visible(&view, cx));
    let at = overlay(&view, cx).expect("the cursor is drawn");
    assert_eq!(at, pt(0, 1), "clamped to the last char of the only line");
    assert_eq!(cursor(&view, cx), at);
}

#[gpui::test]
fn reload_during_slash_shifts_search_origin(cx: &mut TestAppContext) {
    let text = "foo\nbar\nfoo\nbar\n";
    let (_wb, view, cx) = vim_file(text, cx);
    keys("j", cx);
    keys("/", cx);
    // Two lines inserted above: the origin (1,0) moves to (3,0).
    reload(&view, text, &format!("new\nnew\n{text}"), cx);
    type_text("foo", cx);
    keys("enter", cx);
    assert_eq!(cursor(&view, cx), pt(4, 0));
}

#[gpui::test]
fn cmd_f_over_vim_search_makes_plain_widget(cx: &mut TestAppContext) {
    let (wb, view, cx) = vim_file("foo\nbar foo\nfoo", cx);
    keys("/", cx);
    type_text("foo", cx);
    keys("cmd-f", cx);
    let find = view.read_with(cx, |v, _| v.find_widget().cloned().expect("find is shown"));
    let before = find.read_with(cx, |f, _| f.active_index());
    keys("enter", cx);
    // Plain find: Enter steps and the widget stays.
    assert!(find_visible(&view, cx));
    assert_ne!(find.read_with(cx, |f, _| f.active_index()), before);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
}

#[gpui::test]
fn disabling_vim_during_slash_makes_plain_widget(cx: &mut TestAppContext) {
    let (_wb, view, cx) = vim_file("foo\nbar foo\nfoo", cx);
    keys("/", cx);
    type_text("foo", cx);
    set_user_settings(r#"{ "vim.enabled": false }"#, cx);
    let find = view.read_with(cx, |v, _| v.find_widget().cloned().expect("find is shown"));
    assert_eq!(find.read_with(cx, |f, _| f.active_index()), Some(0));
    keys("enter", cx);
    assert!(find_visible(&view, cx));
    assert_eq!(find.read_with(cx, |f, _| f.active_index()), Some(1));
}

#[gpui::test]
fn key_in_editor_during_slash_ends_search(cx: &mut TestAppContext) {
    let (wb, view, cx) = vim_file("abc\ndef\nghi\nfoo foo\n", cx);
    keys("/", cx);
    type_text("foo", cx);
    // The user clicks back into the editor at (1,1): focus and selection.
    cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    view.update(cx, |v, cx| v.select(pt(1, 1)..pt(1, 1), cx));
    draw(cx);
    keys("j", cx);
    assert_eq!(cursor(&view, cx), pt(2, 1), "vim took the clicked position");
    assert_eq!(mode(&wb, &view, cx), Some(Mode::Normal));
    // The widget, still shown, is a plain one now: Enter steps.
    let find = view.read_with(cx, |v, _| v.find_widget().cloned().expect("find is shown"));
    cx.update(|window, cx| {
        let focus = find.read(cx).input().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    draw(cx);
    let before = find.read_with(cx, |f, _| f.active_index());
    keys("enter", cx);
    assert!(find_visible(&view, cx));
    assert_ne!(find.read_with(cx, |f, _| f.active_index()), before);
}

#[gpui::test]
fn keys_go_to_search_panel_input(cx: &mut TestAppContext) {
    init(cx, Some(VIM_ON));
    let (wb, cx) = workbench(cx);
    // The Search tab shows the workbench's search view; files code views.
    wb.update(cx, |wb, cx| {
        let search = wb.search_view().clone();
        wb.editor_groups().update(cx, |groups, cx| {
            let body = move |tab: &lucerna_workbench::Tab,
                             window: &mut gpui::Window,
                             cx: &mut gpui::App| {
                match tab.item {
                    TabItem::Search => search.clone().into(),
                    _ => CodeView::tab_body(tab, window, cx),
                }
            };
            groups.set_tab_body_factory(Rc::new(body), cx)
        })
    });
    let view = open(&wb, "a.rs", "a\nb\nc", cx);
    keys("cmd-shift-f", cx);
    keys("j k", cx);
    let query = cx.update(|_, cx| {
        let panel = wb.read(cx).search_view().clone();
        panel.read(cx).query_input().read(cx).text().to_owned()
    });
    assert_eq!(query, "jk");
    assert_eq!(cursor(&view, cx), pt(0, 0));
}

#[gpui::test]
fn keys_go_to_extensions_page_input(cx: &mut TestAppContext) {
    init(cx, Some(VIM_ON));
    // The workbench's own tab bodies, so the Extensions tab shows its page.
    let (wb, cx) =
        cx.add_window_view(|window, cx| Workbench::new(PathBuf::from("/tmp/project"), window, cx));
    draw(cx);
    keys("cmd-shift-x", cx);
    keys("j k", cx);
    let query = cx.update(|_, cx| {
        let page = wb.read(cx).extensions_page().clone();
        page.read(cx).query_input().read(cx).text().to_owned()
    });
    assert_eq!(query, "jk");
    assert!(cx.update(|_, cx| wb.read(cx).vim().read(cx).enabled()));
}

#[gpui::test]
fn reload_in_visual_line_follows_moved_rows(cx: &mut TestAppContext) {
    let text = "line 0\nline 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\n";
    let (wb, view, cx) = vim_file(text, cx);
    keys("5 j shift-v j", cx);
    let copied = |cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.copy_selection());
    assert_eq!(copied(cx).as_deref(), Some("line 5\nline 6\n"));
    reload(&view, text, &format!("new a\nnew b\n{text}"), cx);
    assert_eq!(mode(&wb, &view, cx), Some(Mode::VisualLine));
    assert_eq!(copied(cx).as_deref(), Some("line 5\nline 6\n"));
    assert_eq!(overlay(&view, cx), Some(pt(8, 0)));
}

#[gpui::test]
fn reload_in_visual_block_follows_moved_rows(cx: &mut TestAppContext) {
    let text = "abcd\nefgh\nijkl\n";
    let (_wb, view, cx) = vim_file(text, cx);
    keys("j l ctrl-v j l", cx);
    reload(&view, text, &format!("new\n{text}"), cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.copy_selection()).as_deref(),
        Some("fg\njk")
    );
    assert_eq!(overlay(&view, cx), Some(pt(3, 2)));
}

#[gpui::test]
fn view_cursor_is_vim_cursor_in_visual_modes(cx: &mut TestAppContext) {
    let (_wb, view, cx) = vim_file("abcd\nefgh\nijkl\n", cx);
    keys("ctrl-v j j", cx);
    assert_eq!(overlay(&view, cx), Some(pt(2, 0)));
    assert_eq!(cursor(&view, cx), pt(2, 0));
    keys("escape j l v k", cx);
    assert_eq!(overlay(&view, cx), Some(pt(1, 1)));
    assert_eq!(cursor(&view, cx), pt(1, 1));
    keys("escape shift-v j", cx);
    assert_eq!(cursor(&view, cx), overlay(&view, cx).unwrap());
}

#[gpui::test]
fn disabling_vim_keeps_the_cursor(cx: &mut TestAppContext) {
    let (_wb, view, cx) = vim_file("abc\ndef\nghi\n", cx);
    keys("j l", cx);
    set_user_settings(r#"{ "vim.enabled": false }"#, cx);
    assert_eq!(cursor(&view, cx), pt(1, 1));
    // A visual block goes; the cursor stays on vim's.
    set_user_settings(VIM_ON, cx);
    keys("ctrl-v j l", cx);
    set_user_settings(r#"{ "vim.enabled": false }"#, cx);
    assert_eq!(view.read_with(cx, |v, _| v.copy_selection()), None);
    assert_eq!(cursor(&view, cx), pt(2, 2));
    // A selection made in Normal mode (with the mouse) is kept.
    set_user_settings(VIM_ON, cx);
    view.update(cx, |v, cx| v.select(pt(0, 0)..pt(0, 2), cx));
    draw(cx);
    set_user_settings(r#"{ "vim.enabled": false }"#, cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.selection()),
        Some(pt(0, 0)..pt(0, 2))
    );
}

/// With vim off the mode indicator takes no room: the pending chord sits
/// one status-bar gap after the panel toggles, as before vim existed.
#[gpui::test]
fn mode_indicator_takes_no_room_when_off(cx: &mut TestAppContext) {
    init(cx, None);
    let (_wb, cx) = workbench(cx);
    draw(cx);
    keys("cmd-k", cx);
    let toggles = cx
        .debug_bounds("panel-toggle-extensions")
        .expect("the extensions toggle is drawn");
    let chord = cx
        .debug_bounds("pending-chord")
        .expect("the pending chord is drawn");
    // The status bar's gap is 3 * GRID (4 px).
    assert_eq!(chord.left() - toggles.right(), gpui::px(12.));
}
