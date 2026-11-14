//! The single-line text input: editing keys, clipboard paste, change
//! notifications and IME composition (Vietnamese Telex), driven through
//! GPUI's test platform.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    ClipboardItem, Context, Entity, EntityInputHandler, Focusable, IntoElement, Render,
    TestAppContext, VisualTestContext, Window, div, prelude::*,
};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, TextInput};

fn init(cx: &mut TestAppContext) {
    let (keymap, _) = Keymap::load(Keymap::default_bindings(), None);
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

struct Host {
    input: Entity<TextInput>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.input.clone())
    }
}

/// A focused, drawn input and the texts its `on_change` callback saw.
fn setup(
    cx: &mut TestAppContext,
) -> (
    Entity<TextInput>,
    Rc<RefCell<Vec<String>>>,
    &mut VisualTestContext,
) {
    init(cx);
    let changes = Rc::new(RefCell::new(Vec::new()));
    let seen = changes.clone();
    let (host, cx) = cx.add_window_view(move |window, cx| {
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.on_change(move |text, _, _| seen.borrow_mut().push(text.to_string()));
            input
        });
        window.focus(&input.focus_handle(cx), cx);
        Host { input }
    });
    let input = host.read_with(cx, |host, _| host.input.clone());
    draw(cx);
    (input, changes, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn text(input: &Entity<TextInput>, cx: &mut VisualTestContext) -> String {
    input.read_with(cx, |input, _| input.text().to_string())
}

fn selection(input: &Entity<TextInput>, cx: &mut VisualTestContext) -> std::ops::Range<usize> {
    input.read_with(cx, |input, _| input.selected_range())
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    draw(cx);
}

#[gpui::test]
fn typing_inserts_at_caret(cx: &mut TestAppContext) {
    let (input, changes, cx) = setup(cx);
    cx.simulate_input("hello");
    draw(cx);
    assert_eq!(text(&input, cx), "hello");
    assert_eq!(selection(&input, cx), 5..5);
    assert_eq!(changes.borrow().last().map(String::as_str), Some("hello"));
    assert_eq!(changes.borrow().len(), 5, "one change per keystroke");
}

#[gpui::test]
fn caret_movement_and_deletion(cx: &mut TestAppContext) {
    let (input, _, cx) = setup(cx);
    cx.simulate_input("abcd");
    keys("left left", cx);
    assert_eq!(selection(&input, cx), 2..2);
    keys("backspace", cx);
    assert_eq!(text(&input, cx), "acd");
    keys("delete", cx);
    assert_eq!(text(&input, cx), "ad");
    assert_eq!(selection(&input, cx), 1..1);
    keys("home", cx);
    assert_eq!(selection(&input, cx), 0..0);
    keys("backspace", cx);
    assert_eq!(
        text(&input, cx),
        "ad",
        "backspace at the start does nothing"
    );
    keys("end", cx);
    assert_eq!(selection(&input, cx), 2..2);
    keys("delete", cx);
    assert_eq!(text(&input, cx), "ad", "delete at the end does nothing");
    keys("right", cx);
    assert_eq!(selection(&input, cx), 2..2);
    cx.simulate_input("X");
    assert_eq!(text(&input, cx), "adX");
}

#[gpui::test]
fn moves_over_whole_characters(cx: &mut TestAppContext) {
    let (input, _, cx) = setup(cx);
    cx.simulate_input("aé");
    keys("left", cx);
    assert_eq!(selection(&input, cx), 1..1);
    keys("right", cx);
    assert_eq!(selection(&input, cx), 3..3);
    keys("backspace", cx);
    assert_eq!(text(&input, cx), "a");
}

#[gpui::test]
fn select_all_then_type_replaces(cx: &mut TestAppContext) {
    let (input, _, cx) = setup(cx);
    cx.simulate_input("old text");
    keys("cmd-a", cx);
    assert_eq!(selection(&input, cx), 0..8);
    cx.simulate_input("n");
    draw(cx);
    assert_eq!(text(&input, cx), "n");

    cx.simulate_input("ew");
    keys("shift-left shift-left", cx);
    assert_eq!(selection(&input, cx), 1..3);
    keys("backspace", cx);
    assert_eq!(text(&input, cx), "n");
}

#[gpui::test]
fn paste_from_clipboard(cx: &mut TestAppContext) {
    let (input, changes, cx) = setup(cx);
    cx.simulate_input("ab");
    keys("left", cx);
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("X\nY".into())));
    keys("cmd-v", cx);
    assert_eq!(text(&input, cx), "aX Yb", "newlines become spaces");
    assert_eq!(selection(&input, cx), 4..4);
    assert_eq!(changes.borrow().last().map(String::as_str), Some("aX Yb"));
}

#[gpui::test]
fn enter_and_tab_insert_nothing(cx: &mut TestAppContext) {
    let (input, _, cx) = setup(cx);
    cx.simulate_input("a");
    keys("enter tab", cx);
    assert_eq!(text(&input, cx), "a");
}

/// Telex: typing `d` then `d` composes `đ`. The IME marks `d`, replaces
/// the marked text with `đ`, then commits `đ`.
#[gpui::test]
fn ime_composition_telex_dd(cx: &mut TestAppContext) {
    let (input, changes, cx) = setup(cx);
    cx.simulate_input("a");
    input.update_in(cx, |input, window, cx| {
        input.replace_and_mark_text_in_range(None, "d", None, window, cx);
    });
    input.update_in(cx, |input, window, cx| {
        assert_eq!(input.text(), "ad");
        assert_eq!(input.marked_text_range(window, cx), Some(1..2));
        input.replace_and_mark_text_in_range(None, "đ", Some(1..1), window, cx);
    });
    input.update_in(cx, |input, window, cx| {
        assert_eq!(input.text(), "ađ");
        // UTF-16 offsets: đ is one code unit, two UTF-8 bytes.
        assert_eq!(input.marked_text_range(window, cx), Some(1..2));
        assert_eq!(input.selected_range(), 3..3);
        input.replace_text_in_range(None, "đ", window, cx);
    });
    draw(cx);
    input.update_in(cx, |input, window, cx| {
        assert_eq!(input.text(), "ađ");
        assert_eq!(input.marked_text_range(window, cx), None);
        assert_eq!(input.selected_range(), 3..3);
        let sel = input.selected_text_range(false, window, cx).unwrap();
        assert_eq!(sel.range, 2..2);
        assert_eq!(input.text_length_utf16(window, cx), Some(2));
        let mut actual = None;
        assert_eq!(
            input
                .text_for_range(1..2, &mut actual, window, cx)
                .as_deref(),
            Some("đ")
        );
    });
    assert_eq!(changes.borrow().last().map(String::as_str), Some("ađ"));

    // Typing after the commit goes on at the caret.
    cx.simulate_input("o");
    draw(cx);
    assert_eq!(text(&input, cx), "ađo");
}

/// VNI-style: a composition that is cancelled leaves the text as it was.
#[gpui::test]
fn ime_unmark_keeps_text(cx: &mut TestAppContext) {
    let (input, _, cx) = setup(cx);
    input.update_in(cx, |input, window, cx| {
        input.replace_and_mark_text_in_range(None, "ô", None, window, cx);
        input.unmark_text(window, cx);
    });
    input.update_in(cx, |input, window, cx| {
        assert_eq!(input.text(), "ô");
        assert_eq!(input.marked_text_range(window, cx), None);
        // Out-of-range requests from the IME are clamped, not panics.
        let mut actual = None;
        assert_eq!(
            input
                .text_for_range(0..10, &mut actual, window, cx)
                .as_deref(),
            Some("ô")
        );
        assert_eq!(actual, Some(0..1));
    });
}

#[gpui::test]
fn set_text_moves_caret_to_end(cx: &mut TestAppContext) {
    let (input, changes, cx) = setup(cx);
    input.update_in(cx, |input, window, cx| input.set_text(">", window, cx));
    draw(cx);
    assert_eq!(text(&input, cx), ">");
    assert_eq!(selection(&input, cx), 1..1);
    assert_eq!(changes.borrow().last().map(String::as_str), Some(">"));
}

#[gpui::test]
fn select_range_selects_bytes(cx: &mut TestAppContext) {
    let (input, _, cx) = setup(cx);
    input.update_in(cx, |input, window, cx| {
        input.set_text("foo.rs", window, cx);
        input.select_range(0..3, window, cx);
    });
    draw(cx);
    assert_eq!(selection(&input, cx), 0..3);
}

#[gpui::test]
fn select_range_snaps_to_char_boundaries(cx: &mut TestAppContext) {
    let (input, _, cx) = setup(cx);
    input.update_in(cx, |input, window, cx| {
        // `ả` is 3 bytes: 1 is inside it.
        input.set_text("ảnh.rs", window, cx);
        input.select_range(1..5, window, cx);
    });
    draw(cx);
    assert_eq!(selection(&input, cx), 0..5);
}
