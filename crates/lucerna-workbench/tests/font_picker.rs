//! The font family picker: list order, filtering, selection and events.
//! It runs standalone; the onboarding page hosts it.

use std::sync::{Arc, Mutex};

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::{IconThemeRegistry, ThemeRegistry};
use lucerna_workbench::{AppState, FontPicker, FontPickerEvent};

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
        icon_themes: IconThemeRegistry::build(&[]).0,
        icons: IconThemeRegistry::build(&[]).0.resolve("Lucide"),
    });
}

type Events = Arc<Mutex<Vec<FontPickerEvent>>>;

fn open<'a>(
    names: &[&str],
    current: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<FontPicker>, Events, &'a mut VisualTestContext) {
    init(cx);
    let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    let current = current.to_string();
    let (picker, cx) =
        cx.add_window_view(move |window, cx| FontPicker::new(names, &current, window, cx));
    let events: Events = Arc::default();
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&picker, move |_, event: &FontPickerEvent, _| {
            sink.lock().unwrap().push(event.clone());
        })
        .detach();
    });
    draw(cx);
    let focus = cx.update(|_, cx| gpui::Focusable::focus_handle(picker.read(cx), cx));
    cx.update(|window, cx| window.focus(&focus, cx));
    (picker, events, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn texts(picker: &Entity<FontPicker>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| picker.read(cx).row_texts())
}

fn selected(picker: &Entity<FontPicker>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| picker.read(cx).selected_index())
}

#[gpui::test]
fn lilex_first_and_sorted_deduped(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(&["Menlo", "Lilex", "Fira Code", "Menlo"], "Lilex", cx);
    assert_eq!(texts(&picker, cx), vec!["Lilex", "Fira Code", "Menlo"]);
}

#[gpui::test]
fn sorts_case_insensitively(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(&["menlo", "Source Code Pro", "Fira Code"], "Lilex", cx);
    assert_eq!(
        texts(&picker, cx),
        vec!["Lilex", "Fira Code", "menlo", "Source Code Pro"]
    );
}

#[gpui::test]
fn only_suggestions_until_typed(cx: &mut TestAppContext) {
    let names = [
        "Arial",
        "Menlo",
        "JetBrains Mono",
        "Comic Sans MS",
        "Al Bayan",
    ];
    let (picker, _, cx) = open(&names, "Lilex", cx);
    assert_eq!(texts(&picker, cx), vec!["Lilex", "JetBrains Mono", "Menlo"]);
}

#[gpui::test]
fn typing_searches_every_font(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(&["Menlo", "Comic Sans MS"], "Lilex", cx);
    let input = cx.update(|_, cx| picker.read(cx).input().clone());
    cx.update(|window, cx| input.update(cx, |input, cx| input.set_text("comic", window, cx)));
    draw(cx);
    assert_eq!(texts(&picker, cx), vec!["Comic Sans MS"]);
    cx.update(|window, cx| input.update(cx, |input, cx| input.set_text("", window, cx)));
    draw(cx);
    assert_eq!(texts(&picker, cx), vec!["Lilex", "Menlo"]);
}

#[gpui::test]
fn current_font_is_listed_and_selected(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(&["Menlo", "Comic Sans MS"], "Comic Sans MS", cx);
    assert_eq!(texts(&picker, cx), vec!["Lilex", "Comic Sans MS", "Menlo"]);
    assert_eq!(selected(&picker, cx), 1);
}

#[gpui::test]
fn filters_fuzzily(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(&["Menlo", "Fira Code"], "Lilex", cx);
    let input = cx.update(|_, cx| picker.read(cx).input().clone());
    cx.update(|window, cx| input.update(cx, |input, cx| input.set_text("fir", window, cx)));
    draw(cx);
    assert_eq!(texts(&picker, cx), vec!["Fira Code"]);
}

#[gpui::test]
fn current_is_selected(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(&["Menlo", "Fira Code"], "Menlo", cx);
    assert_eq!(selected(&picker, cx), 2);
}

#[gpui::test]
fn enter_confirms_selected(cx: &mut TestAppContext) {
    let (picker, events, cx) = open(&["Menlo", "Fira Code"], "Lilex", cx);
    cx.simulate_keystrokes("down");
    assert_eq!(selected(&picker, cx), 1);
    assert!(events.lock().unwrap().is_empty());
    cx.simulate_keystrokes("enter");
    assert_eq!(
        *events.lock().unwrap(),
        vec![FontPickerEvent::Confirmed("Fira Code".into())]
    );
}

#[gpui::test]
fn escape_dismisses(cx: &mut TestAppContext) {
    let (_, events, cx) = open(&["Menlo"], "Lilex", cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(*events.lock().unwrap(), vec![FontPickerEvent::Dismissed]);
}

#[gpui::test]
fn empty_names_only_lilex(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(&[], "Lilex", cx);
    assert_eq!(texts(&picker, cx), vec!["Lilex"]);
}

/// How many `font-picker-row-{i}` rows the last frame drew, of `count`.
fn drawn_rows(count: usize, cx: &mut VisualTestContext) -> Vec<usize> {
    (0..count)
        .filter(|i| {
            // `debug_bounds` takes a `&'static str`.
            let name: &'static str = format!("font-picker-row-{i}").leak();
            cx.debug_bounds(name).is_some()
        })
        .collect()
}

/// Opens the picker over 2000 made-up families (none is a suggestion)
/// and types "font" so all of them are listed.
fn open_many(cx: &mut TestAppContext) -> (Entity<FontPicker>, &mut VisualTestContext) {
    let names: Vec<String> = (0..2000).map(|i| format!("Font {i:04}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (picker, _, cx) = open(&refs, "Lilex", cx);
    let input = cx.update(|_, cx| picker.read(cx).input().clone());
    cx.update(|window, cx| input.update(cx, |input, cx| input.set_text("font", window, cx)));
    draw(cx);
    (picker, cx)
}

#[gpui::test]
fn draws_only_visible_rows(cx: &mut TestAppContext) {
    let (picker, cx) = open_many(cx);
    assert_eq!(texts(&picker, cx).len(), 2000);
    let drawn = drawn_rows(2000, cx);
    assert!(!drawn.is_empty(), "no rows drawn");
    assert!(drawn.len() <= 14, "drew {} rows", drawn.len());
    assert_eq!(drawn[0], 0);
}

#[gpui::test]
fn scrolls_to_selected(cx: &mut TestAppContext) {
    let (picker, cx) = open_many(cx);
    assert_eq!(selected(&picker, cx), 0);
    // Up from the first row wraps to the last.
    cx.simulate_keystrokes("up");
    draw(cx);
    assert_eq!(selected(&picker, cx), 1999);
    assert!(drawn_rows(2000, cx).contains(&1999));
}
