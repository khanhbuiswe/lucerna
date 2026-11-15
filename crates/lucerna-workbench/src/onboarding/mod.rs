//! The Get Started page: theme, editor font and Vim mode, each control
//! writing the user `settings.json` at once. One view for two hosts: the
//! root of a window opened without a folder, with a title bar, or a tab of
//! the workbench. Its host acts on [`OnboardingEvent`].

pub mod model;

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    AnyElement, Context, Entity, EventEmitter, FocusHandle, Focusable, HighlightStyle, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, Styled, StyledText, Subscription, Task,
    Window, div, prelude::*, px, svg,
};
use lucerna_settings::{Context as WhenContext, ContextValue, Keystroke, Resolution};
use serde_json::json;

use crate::config_files::{ConfigPaths, ConfigState, USER_SETTINGS_TEMPLATE, create_if_missing};
use crate::layout::{color, title_bar};
use crate::settings_edit::save_user_settings;
use crate::ui::{
    GRID, UI_FONT_FAMILY, UI_FONT_SIZE, UI_FONT_SIZE_SMALL, UI_FONT_SIZE_STATUS, dropdown_button,
    list_button, number_stepper, popover_overlay, primary_text_button, section_header,
    segmented_control, segmented_group, setting_row, subheader_card, switch, theme_preview,
    warning_row,
};
use crate::{AppState, FontPicker, FontPickerEvent, font_names, icons, keys};
use model::{
    FONT_SIZE_MAX, FONT_SIZE_MIN, ThemeMode, current_font_family, current_font_size, current_mode,
    mode_edits, preview_themes, stepped_font_size,
};

const FINISH: &str = "lucerna.onboarding.finish";
const OPEN_SETTINGS: &str = "workbench.action.openSettingsJson";
const OPEN_ONBOARDING: &str = "lucerna.openOnboarding";
const ABOUT: &str = "workbench.action.showAboutDialog";
const QUIT: &str = "workbench.action.quit";

/// The commands the page runs from its keys as a window root; keys bound
/// to any other command do nothing there.
pub const ONBOARDING_COMMANDS: &[&str] = &[FINISH, OPEN_SETTINGS, OPEN_ONBOARDING, ABOUT, QUIT];

const COLUMN_MAX_WIDTH: f32 = 640.;
const LOGO_SIZE: f32 = 45.;
const TITLE_FONT_SIZE: f32 = 20.;
const DROPDOWN_WIDTH: f32 = 48. * GRID;
const VIM_KEY: &str = "vim.enabled";
const FONT_SIZE_KEY: &str = "editor.fontSize";
const FONT_FAMILY_KEY: &str = "editor.fontFamily";

/// The theme buttons, in order: their labels and modes.
const THEME_LABELS: [&str; 3] = ["Light", "Dark", "System"];
const THEME_MODES: [ThemeMode; 3] = [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System];

/// Where the page is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// The root view of a window without a folder.
    Window,
    /// A workbench tab.
    Tab,
}

/// What the page's host does for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingEvent {
    /// Finish: the root turns into Welcome; the tab closes.
    Finish,
    /// Open the user settings file at this path in a new window (root).
    OpenSettingsInNewWindow(PathBuf),
    /// Run Open User Settings (JSON) in the workbench (tab).
    OpenSettingsInWorkbench,
}

/// One control of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Finish,
    Theme(ThemeMode),
    FontFamily,
    FontSizeMinus,
    FontSizePlus,
    Vim,
    OpenSettings,
}

/// The controls in focus order.
const FOCUS_ORDER: [Control; 9] = [
    Control::Finish,
    Control::Theme(ThemeMode::Light),
    Control::Theme(ThemeMode::Dark),
    Control::Theme(ThemeMode::System),
    Control::FontFamily,
    Control::FontSizeMinus,
    Control::FontSizePlus,
    Control::Vim,
    Control::OpenSettings,
];

pub struct OnboardingPage {
    host: Host,
    focus: FocusHandle,
    /// Index into [`FOCUS_ORDER`] of the focused control.
    focused: usize,
    /// Why the settings file could not be opened, shown atop the column.
    warning: Option<String>,
    picker: Option<(Entity<FontPicker>, Subscription)>,
    /// Lists the fonts for the picker about to open.
    loading_fonts: Option<Task<()>>,
    pending_chord: Vec<Keystroke>,
    /// The font size the last frame drew.
    drawn_font_size: Option<u32>,
    /// Whether a press began on the title bar, for its drag.
    title_bar_pressed: Rc<Cell<bool>>,
    _settings: Subscription,
}

impl EventEmitter<OnboardingEvent> for OnboardingPage {}

impl OnboardingPage {
    /// Focuses the page on the selected theme button. It draws from the
    /// settings each frame and redraws when they change.
    pub fn new(host: Host, window: &mut Window, cx: &mut Context<Self>) -> Self {
        if host == Host::Window {
            crate::config_files::follow_system_appearance(window, cx);
        }
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let app_state = cx.global::<AppState>();
        let mode = current_mode(&app_state.settings, &app_state.themes);
        let focused = FOCUS_ORDER
            .iter()
            .position(|c| *c == Control::Theme(mode))
            .unwrap_or(0);
        OnboardingPage {
            host,
            focus,
            focused,
            warning: None,
            picker: None,
            loading_fonts: None,
            pending_chord: Vec::new(),
            drawn_font_size: None,
            title_bar_pressed: Rc::default(),
            _settings: cx.observe_global::<AppState>(|_, cx| cx.notify()),
        }
    }

    pub fn font_picker(&self) -> Option<&Entity<FontPicker>> {
        self.picker.as_ref().map(|(picker, _)| picker)
    }

    /// The font size the last frame showed; `None` before the first.
    pub fn drawn_font_size(&self) -> Option<u32> {
        self.drawn_font_size
    }

    /// Runs one of [`ONBOARDING_COMMANDS`] (About and Quit only as a
    /// window root); `false` for any other command.
    pub fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match id {
            FINISH => cx.emit(OnboardingEvent::Finish),
            OPEN_SETTINGS => self.open_settings(cx),
            // Already here.
            OPEN_ONBOARDING => {}
            ABOUT if self.host == Host::Window => crate::about::show(window, cx),
            QUIT if self.host == Host::Window => crate::workbench::quit(cx),
            _ => return false,
        }
        true
    }

    fn press(&mut self, control: Control, window: &mut Window, cx: &mut Context<Self>) {
        match control {
            Control::Finish => cx.emit(OnboardingEvent::Finish),
            Control::Theme(mode) => self.set_mode(mode, cx),
            Control::FontFamily => self.open_font_picker(window, cx),
            Control::FontSizeMinus => self.step_font_size(-1, cx),
            Control::FontSizePlus => self.step_font_size(1, cx),
            Control::Vim => {
                let on: bool = cx
                    .global::<AppState>()
                    .settings
                    .get(VIM_KEY, None)
                    .unwrap_or(false);
                save(&[(VIM_KEY, json!(!on))], cx);
            }
            Control::OpenSettings => self.open_settings(cx),
        }
    }

    /// A click focuses the control and presses it.
    fn click(&mut self, control: Control, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = FOCUS_ORDER.iter().position(|c| *c == control) {
            self.focused = index;
        }
        self.press(control, window, cx);
        cx.notify();
    }

    /// Writes `mode` unless it is the one the settings describe already.
    fn set_mode(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        let app_state = cx.global::<AppState>();
        if current_mode(&app_state.settings, &app_state.themes) == mode {
            return;
        }
        let edits = mode_edits(mode, &app_state.settings);
        save(&edits, cx);
    }

    /// Writes the font size moved by `delta`; nothing at a limit.
    fn step_font_size(&mut self, delta: i32, cx: &mut Context<Self>) {
        let current = current_font_size(&cx.global::<AppState>().settings);
        if let Some(size) = stepped_font_size(current, delta) {
            save(&[(FONT_SIZE_KEY, json!(size))], cx);
        }
    }

    /// Root: makes sure the user settings file exists and asks for it in a
    /// new window. Tab: asks the workbench to open it.
    fn open_settings(&mut self, cx: &mut Context<Self>) {
        if self.host == Host::Tab {
            cx.emit(OnboardingEvent::OpenSettingsInWorkbench);
            return;
        }
        let path = cx
            .try_global::<ConfigState>()
            .map(|state| state.paths().user_settings())
            .unwrap_or_else(|| ConfigPaths::user_only().user_settings());
        match create_if_missing(&path, USER_SETTINGS_TEMPLATE) {
            Ok(_) => {
                self.warning = None;
                cx.emit(OnboardingEvent::OpenSettingsInNewWindow(path));
            }
            Err(err) => {
                tracing::error!(path = %path.display(), %err, "could not create the settings file");
                self.warning = Some(format!("Could not create {}: {err}", path.display()));
            }
        }
        cx.notify();
    }

    /// Lists the installed fonts in the background, then opens the picker.
    fn open_font_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.is_some() || self.loading_fonts.is_some() {
            return;
        }
        let names = font_names(cx);
        self.loading_fonts = Some(cx.spawn_in(window, async move |this, cx| {
            let names = names.await;
            this.update_in(cx, |page, window, cx| {
                page.loading_fonts = None;
                let current = current_font_family(&cx.global::<AppState>().settings);
                let picker = cx.new(|cx| FontPicker::new(names, &current, window, cx));
                let events = cx.subscribe_in(&picker, window, Self::on_picker_event);
                window.focus(&picker.focus_handle(cx), cx);
                page.picker = Some((picker, events));
                cx.notify();
            })
            .ok();
        }));
    }

    fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn on_picker_event(
        &mut self,
        _: &Entity<FontPicker>,
        event: &FontPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let FontPickerEvent::Confirmed(name) = event {
            save(&[(FONT_FAMILY_KEY, json!(name))], cx);
        }
        self.close_picker(window, cx);
    }

    /// As a window root, runs keys bound to a page command; then moves
    /// focus and presses controls. With the font picker open, keys go to
    /// it first and `onboardingFocus` is false.
    fn key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = keys::to_keystroke(ev);
        if self.host == Host::Window && self.dispatch_key(&stroke, window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.picker.is_none() && self.page_key(ev, window, cx) {
            cx.stop_propagation();
        }
    }

    /// Resolves `stroke` through the user keymap; whether it ran a page
    /// command or started a chord.
    fn dispatch_key(
        &mut self,
        stroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut ctx = WhenContext::default();
        ctx.set("onboardingFocus", ContextValue::Bool(self.picker.is_none()));
        ctx.set("inQuickOpen", ContextValue::Bool(self.picker.is_some()));
        let keymap = &cx.global::<AppState>().keymap;
        match keys::resolve_chord(&mut self.pending_chord, stroke, &ctx, keymap) {
            Resolution::Command { command, .. } => {
                ONBOARDING_COMMANDS.contains(&command.as_str())
                    && self.run_command(&command, window, cx)
            }
            Resolution::Pending => true,
            Resolution::NoMatch => false,
        }
    }

    /// Tab, ⇧Tab and ↑/↓ move focus; Enter and Space press; ←/→ on a
    /// theme button pick the theme beside it.
    fn page_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.platform || m.function {
            return false;
        }
        let focused = FOCUS_ORDER[self.focused];
        match ev.keystroke.key.as_str() {
            "tab" => self.move_focus(if m.shift { -1 } else { 1 }, cx),
            "up" if !m.shift => self.move_focus(-1, cx),
            "down" if !m.shift => self.move_focus(1, cx),
            "enter" | "space" if !m.shift => self.press(focused, window, cx),
            key @ ("left" | "right") if !m.shift => {
                let Control::Theme(mode) = focused else {
                    return false;
                };
                let index = THEME_MODES.iter().position(|t| *t == mode).unwrap_or(0);
                let next = if key == "left" {
                    index.checked_sub(1)
                } else {
                    Some(index + 1).filter(|i| *i < THEME_MODES.len())
                };
                if let Some(next) = next {
                    self.click(Control::Theme(THEME_MODES[next]), window, cx);
                }
            }
            _ => return false,
        }
        true
    }

    fn move_focus(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = FOCUS_ORDER.len() as isize;
        self.focused = (self.focused as isize + delta).rem_euclid(count) as usize;
        cx.notify();
    }

    fn is_focused(&self, control: Control) -> bool {
        FOCUS_ORDER[self.focused] == control
    }
}

/// Writes `edits` into the user settings. A failure is already shown as
/// the file's configuration error, and the controls keep their values.
fn save(edits: &[(&str, serde_json::Value)], cx: &mut gpui::App) {
    let _ = save_user_settings(edits, cx);
}

/// `el` with a `border.focused` ring while `focused`.
fn ring<E: Styled>(el: E, focused: bool, cx: &gpui::App) -> E {
    let ui = &cx.global::<AppState>().theme.ui;
    if focused {
        el.border_1().border_color(color(ui.border_focused))
    } else {
        el
    }
}

/// A rule between blocks of the column.
fn rule(cx: &gpui::App) -> gpui::Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div().w_full().h(px(1.)).bg(color(ui.border_variant))
}

/// A colored span of a sample line: byte range and syntax capture.
type SampleToken = (std::ops::Range<usize>, &'static str);

/// The sample lines of the font card: text, then its colored spans.
const SAMPLE_LINES: [(&str, &[SampleToken]); 2] = [
    ("fn main() {", &[(0..2, "keyword"), (3..7, "function")]),
    (
        "    println!(\"Hello, Lucerna\");",
        &[(4..12, "function"), (13..29, "string")],
    ),
];

impl OnboardingPage {
    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let (text, muted) = (color(ui.text), color(ui.text_muted));
        let keys = cx.global::<AppState>().keymap.label_for(FINISH);
        let finish = ring(
            primary_text_button("onboarding-finish", "Finish", cx),
            self.is_focused(Control::Finish),
            cx,
        )
        .gap(px(2. * GRID))
        .when_some(keys, |b, keys| {
            b.child(
                div()
                    .text_size(px(UI_FONT_SIZE_STATUS))
                    .text_color(muted)
                    .child(keys),
            )
        })
        .on_click(cx.listener(|page, _, window, cx| page.click(Control::Finish, window, cx)));
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(3. * GRID))
            .child(
                svg()
                    .data(icons::LUCERNA_MARK)
                    .flex_none()
                    .size(px(LOGO_SIZE))
                    .text_color(text),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(TITLE_FONT_SIZE))
                            .child("Welcome to Lucerna"),
                    )
                    .child(
                        div()
                            .text_size(px(UI_FONT_SIZE_SMALL))
                            .text_color(muted)
                            .child("Set up the basics. Change them any time."),
                    ),
            )
            .child(finish)
            .into_any_element()
    }

    fn render_theme(&self, mode: ThemeMode, cx: &mut Context<Self>) -> AnyElement {
        let app_state = cx.global::<AppState>();
        let muted = color(app_state.theme.ui.text_muted);
        let (left, right) = match mode {
            ThemeMode::System => {
                let (light, dark) = preview_themes(mode, &app_state.settings);
                (
                    app_state.themes.get_or_default(&light),
                    dark.map(|dark| app_state.themes.get_or_default(&dark)),
                )
            }
            // The theme in use is `workbench.colorTheme`.
            ThemeMode::Light | ThemeMode::Dark => (app_state.theme.clone(), None),
        };
        let name = match &right {
            Some(right) => format!("{} / {}", left.name, right.name),
            None => left.name.clone(),
        };
        let selected = THEME_MODES.iter().position(|m| *m == mode).unwrap_or(0);
        let buttons: Vec<AnyElement> =
            segmented_control("onboarding-theme", &THEME_LABELS, selected, cx)
                .into_iter()
                .zip(THEME_MODES)
                .map(|(button, mode)| {
                    let control = Control::Theme(mode);
                    ring(button, self.is_focused(control), cx)
                        .on_click(
                            cx.listener(move |page, _, window, cx| page.click(control, window, cx)),
                        )
                        .into_any_element()
                })
                .collect();
        let preview = theme_preview("onboarding-preview", &left, right.as_deref(), cx);
        div()
            .flex()
            .flex_col()
            .gap(px(3. * GRID))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4. * GRID))
                    .child(section_header("Theme", cx).flex_1())
                    .child(segmented_group(cx).children(buttons)),
            )
            .child(preview)
            .child(
                div()
                    .text_size(px(UI_FONT_SIZE_SMALL))
                    .text_color(muted)
                    .child(name),
            )
            .into_any_element()
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = &cx.global::<AppState>().settings;
        let family = current_font_family(settings);
        let size = current_font_size(settings);
        let vim: bool = settings.get(VIM_KEY, None).unwrap_or(false);

        let dropdown = ring(
            dropdown_button("onboarding-font-family", family.clone(), cx),
            self.is_focused(Control::FontFamily),
            cx,
        )
        .w(px(DROPDOWN_WIDTH))
        .on_click(cx.listener(|page, _, window, cx| page.click(Control::FontFamily, window, cx)));

        let (value, minus, plus) = number_stepper(
            "onboarding-font-size",
            size,
            size <= FONT_SIZE_MIN,
            size >= FONT_SIZE_MAX,
            cx,
        );
        let minus = ring(minus, self.is_focused(Control::FontSizeMinus), cx).on_click(
            cx.listener(|page, _, window, cx| page.click(Control::FontSizeMinus, window, cx)),
        );
        let plus = ring(plus, self.is_focused(Control::FontSizePlus), cx).on_click(
            cx.listener(|page, _, window, cx| page.click(Control::FontSizePlus, window, cx)),
        );
        let stepper = div()
            .flex()
            .items_center()
            .child(minus)
            .child(value.debug_selector(|| "onboarding-font-size-value".into()))
            .child(plus);

        let vim_switch = ring(
            switch("onboarding-vim", vim, cx),
            self.is_focused(Control::Vim),
            cx,
        )
        .on_click(cx.listener(|page, _, window, cx| page.click(Control::Vim, window, cx)));

        div()
            .flex()
            .flex_col()
            .gap(px(4. * GRID))
            .child(section_header("Editor", cx))
            .child(setting_row("Font Family", None, dropdown, cx))
            .child(setting_row("Font Size", None, stepper, cx))
            .child(setting_row(
                "Vim Mode",
                Some("Enable Vim keybindings (vim.enabled)"),
                vim_switch,
                cx,
            ))
            .child(self.render_font_sample(family, size, cx))
            .into_any_element()
    }

    /// Two lines of code in the editor font and size, in the active
    /// theme's syntax colors.
    fn render_font_sample(&self, family: String, size: u32, cx: &mut Context<Self>) -> AnyElement {
        let theme = &cx.global::<AppState>().theme;
        let foreground = color(theme.ui.editor_foreground);
        let lines = SAMPLE_LINES.iter().map(|(text, tokens)| {
            let highlights: Vec<_> = tokens
                .iter()
                .filter_map(|(range, capture)| {
                    let fill = theme.style_for(capture)?.color?;
                    let style = HighlightStyle {
                        color: Some(color(fill).into()),
                        ..Default::default()
                    };
                    Some((range.clone(), style))
                })
                .collect();
            div()
                .whitespace_nowrap()
                .child(StyledText::new(SharedString::from(*text)).with_highlights(highlights))
        });
        subheader_card(cx)
            .debug_selector(|| "onboarding-font-sample".into())
            .flex_col()
            .items_start()
            .gap(px(GRID))
            .p(px(3. * GRID))
            .overflow_hidden()
            .font_family(SharedString::from(family))
            .text_size(px(size as f32))
            .text_color(foreground)
            .children(lines)
            .into_any_element()
    }

    fn render_open_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let keys = cx.global::<AppState>().keymap.label_for(OPEN_SETTINGS);
        list_button(
            "onboarding-open-settings",
            icons::SETTINGS,
            "Open Settings (JSON)".into(),
            keys,
            self.is_focused(Control::OpenSettings),
            cx,
        )
        .debug_selector(|| "onboarding-open-settings".into())
        .on_click(cx.listener(|page, _, window, cx| page.click(Control::OpenSettings, window, cx)))
        .into_any_element()
    }
}

impl Focusable for OnboardingPage {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for OnboardingPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app_state = cx.global::<AppState>();
        let mode = current_mode(&app_state.settings, &app_state.themes);
        self.drawn_font_size = Some(current_font_size(&app_state.settings));
        let ui = &app_state.theme.ui;
        let (background, text) = (color(ui.editor_background), color(ui.text));
        let title_bar = (self.host == Host::Window)
            .then(|| title_bar::render_plain("Lucerna", self.title_bar_pressed.clone(), ui));
        let warning = self
            .warning
            .clone()
            .map(|text| warning_row("onboarding-warning".into(), text, cx));
        let header = self.render_header(cx);
        let theme = self.render_theme(mode, cx);
        let editor = self.render_editor(cx);
        let open_settings = self.render_open_settings(cx);
        div()
            .id("onboarding")
            .debug_selector(|| "onboarding".into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(background)
            .text_color(text)
            .font_family(UI_FONT_FAMILY)
            .text_size(px(UI_FONT_SIZE))
            .children(title_bar)
            .child(
                div()
                    .id("onboarding-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div().flex().justify_center().p(px(8. * GRID)).child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(6. * GRID))
                                .w_full()
                                .max_w(px(COLUMN_MAX_WIDTH))
                                .children(warning)
                                .child(header)
                                .child(rule(cx))
                                .child(theme)
                                .child(editor)
                                .child(rule(cx))
                                .child(open_settings),
                        ),
                    ),
            )
            .children(
                self.font_picker()
                    .map(|picker| popover_overlay(picker.clone())),
            )
    }
}
