//! The vim layer inside the workbench: settings, key routing, applying
//! vim's actions to the code view, context keys and the mode indicator.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    BorrowAppContext as _, Entity, KeyDownEvent, Modifiers, TestAppContext, VisualTestContext,
};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::{Buffer, BufferSnapshot, Point};
use lucerna_theme::ThemeRegistry;
use lucerna_vim::Mode;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, Workbench};

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
