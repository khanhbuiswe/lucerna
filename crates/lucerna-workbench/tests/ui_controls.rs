//! The shared controls of `ui.rs` the onboarding page is built from: each
//! draws under its debug selectors, and a stepper button at its limit takes
//! no clicks.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Context, IntoElement, Modifiers, ParentElement as _, Render, Styled as _, TestAppContext,
    VisualTestContext, Window, div, prelude::*,
};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::{IconThemeRegistry, ThemeRegistry};
use lucerna_workbench::AppState;
use lucerna_workbench::ui;

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
        icon_themes: IconThemeRegistry::build(&[]).0,
        icons: IconThemeRegistry::build(&[]).0.resolve("Lucide"),
    });
}

/// Draws one of each control; `split` gives the theme preview two halves.
struct Harness {
    split: bool,
    /// Stepper clicks: (minus, plus).
    clicks: Rc<Cell<(u32, u32)>>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app_state = cx.global::<AppState>();
        let light = app_state.themes.get_or_default("Lucerna Light");
        let dark = app_state.themes.get_or_default("Lucerna Dark");
        let segments = ui::segmented_control("seg", &["Light", "Dark", "System"], 1, cx);
        let (value, minus, plus) = ui::number_stepper("stepper", 6, true, false, cx);
        let clicks = self.clicks.clone();
        let minus = minus.on_click(move |_, _, _| {
            let (m, p) = clicks.get();
            clicks.set((m + 1, p));
        });
        let clicks = self.clicks.clone();
        let plus = plus.on_click(move |_, _, _| {
            let (m, p) = clicks.get();
            clicks.set((m, p + 1));
        });
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(ui::segmented_group(cx).children(segments))
            .child(ui::setting_row(
                "Font Size",
                Some("The editor font size"),
                div().flex().child(minus).child(value).child(plus),
                cx,
            ))
            .child(ui::setting_row(
                "Vim Mode",
                None,
                ui::switch("vim", true, cx),
                cx,
            ))
            .child(ui::theme_preview(
                "preview",
                &light,
                self.split.then_some(&*dark),
                cx,
            ))
    }
}

fn open(split: bool, cx: &mut TestAppContext) -> (Rc<Cell<(u32, u32)>>, &mut VisualTestContext) {
    init(cx);
    let clicks = Rc::new(Cell::new((0, 0)));
    let shared = clicks.clone();
    let (_, cx) = cx.add_window_view(move |_, _| Harness {
        split,
        clicks: shared,
    });
    draw(cx);
    (clicks, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

fn drawn(selector: &'static str, cx: &mut VisualTestContext) -> bool {
    cx.debug_bounds(selector).is_some()
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let at = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .center();
    cx.simulate_click(at, Modifiers::none());
    draw(cx);
}

#[gpui::test]
fn segmented_draws_each_option(cx: &mut TestAppContext) {
    let (_, cx) = open(false, cx);
    assert!(drawn("seg-0", cx));
    assert!(drawn("seg-1", cx));
    assert!(drawn("seg-2", cx));
    assert!(!drawn("seg-3", cx));
    // Side by side, in order.
    let first = cx.debug_bounds("seg-0").unwrap();
    let second = cx.debug_bounds("seg-1").unwrap();
    assert!(first.right() <= second.left());
    assert_eq!(first.top(), second.top());
}

#[gpui::test]
fn stepper_buttons_exist(cx: &mut TestAppContext) {
    let (_, cx) = open(false, cx);
    assert!(drawn("stepper", cx));
    assert!(drawn("stepper-minus", cx));
    assert!(drawn("stepper-plus", cx));
}

#[gpui::test]
fn stepper_button_at_limit_takes_no_click(cx: &mut TestAppContext) {
    let (clicks, cx) = open(false, cx);
    click("stepper-minus", cx);
    assert_eq!(clicks.get(), (0, 0), "minus is at the minimum");
    click("stepper-plus", cx);
    assert_eq!(clicks.get(), (0, 1));
}

#[gpui::test]
fn switch_draws(cx: &mut TestAppContext) {
    let (_, cx) = open(false, cx);
    let track = cx.debug_bounds("vim").expect("switch drawn");
    let knob = cx.debug_bounds("vim-knob").expect("knob drawn");
    assert_eq!(track.size.width, gpui::px(28.));
    assert_eq!(track.size.height, gpui::px(16.));
    // On: the knob sits at the right.
    assert!(knob.center().x > track.center().x);
}

#[gpui::test]
fn theme_preview_split_has_two_halves(cx: &mut TestAppContext) {
    let (_, cx) = open(true, cx);
    let preview = cx.debug_bounds("preview").expect("preview drawn");
    assert_eq!(preview.size.height, gpui::px(120.));
    let left = cx.debug_bounds("preview-left").expect("left half");
    let right = cx.debug_bounds("preview-right").expect("right half");
    assert!(left.right() <= right.left());
}

#[gpui::test]
fn theme_preview_single_has_one_half(cx: &mut TestAppContext) {
    let (_, cx) = open(false, cx);
    assert!(drawn("preview-left", cx));
    assert!(!drawn("preview-right", cx));
}
