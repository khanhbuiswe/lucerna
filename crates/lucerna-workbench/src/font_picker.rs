//! The font family picker the onboarding page opens from its Font Family
//! dropdown: Quick Open's list over the installed font families, each row
//! drawn in its own font. Arrow keys only move the selection (there is no
//! live preview); Enter confirms and Escape dismisses without a choice.

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, KeyDownEvent, Render, ScrollHandle, SharedString, Styled, Task, Window,
    prelude::*,
};

use crate::quick_open::picker::{
    fuzzy_filter, highlighted, picker_input_row, picker_list, picker_row, wrap_index,
};
use crate::text_input::TextInput;
use crate::ui::popover;

/// The bundled editor font, always the first row.
const DEFAULT_FONT: &str = "Lilex";

/// What the host does as the picker is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontPickerEvent {
    /// Enter (or a click) chose this family.
    Confirmed(String),
    Dismissed,
}

pub struct FontPicker {
    input: Entity<TextInput>,
    names: Vec<String>,
    /// Matching names: index into `names` and matched char positions.
    rows: Vec<(usize, Vec<u32>)>,
    selected: usize,
    scroll: ScrollHandle,
}

impl EventEmitter<FontPickerEvent> for FontPicker {}

/// The installed font family names, listed on a background thread.
pub fn font_names(cx: &App) -> Task<Vec<String>> {
    let text_system = cx.text_system().clone();
    cx.background_spawn(async move { text_system.all_font_names() })
}

impl FontPicker {
    /// Lists "Lilex" first, then `names` without duplicates in
    /// case-insensitive alphabetical order, with `current` selected.
    pub fn new(
        names: Vec<String>,
        current: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let this = cx.weak_entity();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder("Select a font family");
            input.on_change(move |text, _, cx| {
                this.update(cx, |picker, cx| picker.query_changed(text, cx))
                    .ok();
            });
            input
        });
        let mut rest: Vec<String> = names.into_iter().filter(|n| n != DEFAULT_FONT).collect();
        rest.sort_by_cached_key(|n| (n.to_lowercase(), n.clone()));
        rest.dedup();
        let names: Vec<String> = std::iter::once(DEFAULT_FONT.to_string())
            .chain(rest)
            .collect();
        let rows = fuzzy_filter(names.iter().map(String::as_str), "");
        let selected = names.iter().position(|n| n == current).unwrap_or(0);
        let scroll = ScrollHandle::new();
        scroll.scroll_to_item(selected);
        FontPicker {
            input,
            names,
            rows,
            selected,
            scroll,
        }
    }

    pub fn input(&self) -> &Entity<TextInput> {
        &self.input
    }

    /// The family names listed, in order.
    pub fn row_texts(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|(i, _)| self.names[*i].clone())
            .collect()
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    fn query_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        self.rows = fuzzy_filter(self.names.iter().map(String::as_str), text.trim());
        self.select(0, cx);
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.scroll.scroll_to_item(index);
        cx.notify();
    }

    fn confirm(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(&(name, _)) = self.rows.get(index) {
            cx.emit(FontPickerEvent::Confirmed(self.names[name].clone()));
        }
    }

    fn key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.shift || m.platform {
            return;
        }
        match ev.keystroke.key.as_str() {
            "up" => self.select(wrap_index(self.selected, self.rows.len(), -1), cx),
            "down" => self.select(wrap_index(self.selected, self.rows.len(), 1), cx),
            "enter" => self.confirm(self.selected, cx),
            "escape" => cx.emit(FontPickerEvent::Dismissed),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let (name, positions) = &self.rows[index];
        let name = &self.names[*name];
        picker_row("font-picker-row", index, index == self.selected, cx)
            .on_click(cx.listener(move |picker, _, _, cx| picker.confirm(index, cx)))
            .font_family(SharedString::from(name.clone()))
            .child(highlighted(name.clone().into(), positions, cx))
            .into_any_element()
    }
}

impl Focusable for FontPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for FontPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = (0..self.rows.len())
            .map(|i| self.render_row(i, cx))
            .collect();
        popover(cx)
            .id("font-picker")
            .occlude()
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(FontPickerEvent::Dismissed)))
            .child(picker_input_row(&self.input, cx))
            .when(!rows.is_empty(), |p| {
                p.child(picker_list("font-picker-list", &self.scroll).children(rows))
            })
    }
}
