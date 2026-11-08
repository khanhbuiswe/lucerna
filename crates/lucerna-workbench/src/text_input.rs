//! A single-line text input for Quick Open, the find widget and the
//! Search view.
//!
//! Editing keys are handled in `on_key_down` (the workbench does not use
//! GPUI's keymap); typed characters arrive through the platform input
//! handler, which [`EntityInputHandler`] implements with marked-text
//! support so IME composition (Vietnamese Telex/VNI, CJK) works.
//!
//! Offsets are UTF-8 byte offsets into [`TextInput::text`]; the input
//! handler converts to and from the UTF-16 offsets the platform uses.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InspectorElementId,
    IntoElement, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, Render, ShapedLine, SharedString, Style, TextAlign, TextRun,
    UTF16Selection, UnderlineStyle, Window, div, fill, point, prelude::*, px, relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::AppState;
use crate::layout::color;

/// The GPUI key context of every text input; the workbench reports
/// `textInputFocus` while the focused element is inside one.
pub const TEXT_INPUT_KEY_CONTEXT: &str = "TextInput";

/// Called with the new text after every change, deferred to the end of
/// the current update so it may update the input's owner.
type OnChange = Rc<dyn Fn(&str, &mut Window, &mut App)>;

pub struct TextInput {
    focus: FocusHandle,
    text: String,
    placeholder: SharedString,
    /// Byte range; empty when it is just the caret.
    selected: Range<usize>,
    /// The caret is at `selected.start` rather than `selected.end`.
    reversed: bool,
    /// Byte range of the IME composition in progress.
    marked: Option<Range<usize>>,
    /// A mouse drag is extending the selection.
    selecting: bool,
    /// The line and bounds of the last paint, for mouse hit-testing and
    /// IME candidate-window placement.
    last_line: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    on_change: Option<OnChange>,
}

impl TextInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        TextInput {
            focus: cx.focus_handle(),
            text: String::new(),
            placeholder: SharedString::default(),
            selected: 0..0,
            reversed: false,
            marked: None,
            selecting: false,
            last_line: None,
            last_bounds: None,
            on_change: None,
        }
    }

    /// Muted text shown while the input is empty.
    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>) {
        self.placeholder = placeholder.into();
    }

    /// Calls `f` with the new text after every change, typed or set.
    pub fn on_change(&mut self, f: impl Fn(&str, &mut Window, &mut App) + 'static) {
        self.on_change = Some(Rc::new(f));
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the text and puts the caret at the end.
    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.text = sanitize(text);
        self.selected = self.text.len()..self.text.len();
        self.reversed = false;
        self.marked = None;
        self.changed(window, cx);
    }

    /// The selection as a byte range; empty when it is just the caret.
    pub fn selected_range(&self) -> Range<usize> {
        self.selected.clone()
    }

    fn cursor(&self) -> usize {
        if self.reversed {
            self.selected.start
        } else {
            self.selected.end
        }
    }

    fn move_to(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = offset..offset;
        self.reversed = false;
        window.notify_text_selection_changed(&self.focus);
        cx.notify();
    }

    /// Moves the caret end of the selection, keeping the other end.
    fn select_to(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        let anchor = if self.reversed {
            self.selected.end
        } else {
            self.selected.start
        };
        self.reversed = offset < anchor;
        self.selected = offset.min(anchor)..offset.max(anchor);
        window.notify_text_selection_changed(&self.focus);
        cx.notify();
    }

    fn prev_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .take_while(|&i| i < offset)
            .last()
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|&i| i > offset)
            .unwrap_or(self.text.len())
    }

    /// Replaces `range` with `new_text` and puts the caret after it.
    fn replace(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_text = sanitize(new_text);
        if range.is_empty() && new_text.is_empty() && self.marked.is_none() {
            return;
        }
        self.text.replace_range(range.clone(), &new_text);
        let caret = range.start + new_text.len();
        self.selected = caret..caret;
        self.reversed = false;
        self.marked = None;
        self.changed(window, cx);
    }

    fn changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.notify_text_input_changed(&self.focus);
        cx.notify();
        if let Some(f) = self.on_change.clone() {
            let text = self.text.clone();
            cx.defer_in(window, move |_, window, cx| f(&text, window, cx));
        }
    }

    fn key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.function {
            return;
        }
        let (cmd, shift) = (m.platform, m.shift);
        let cursor = self.cursor();
        match ev.keystroke.key.as_str() {
            "left" | "right" | "home" | "end" => {
                let target = match ev.keystroke.key.as_str() {
                    "home" => 0,
                    "end" => self.text.len(),
                    "left" if cmd => 0,
                    "right" if cmd => self.text.len(),
                    // Without shift, an arrow collapses a selection to its edge.
                    "left" if !shift && !self.selected.is_empty() => self.selected.start,
                    "right" if !shift && !self.selected.is_empty() => self.selected.end,
                    "left" => self.prev_boundary(cursor),
                    _ => self.next_boundary(cursor),
                };
                if shift {
                    self.select_to(target, window, cx);
                } else {
                    self.move_to(target, window, cx);
                }
            }
            "backspace" | "delete" if !cmd => {
                let range = if !self.selected.is_empty() {
                    self.selected.clone()
                } else if ev.keystroke.key == "backspace" {
                    self.prev_boundary(cursor)..cursor
                } else {
                    cursor..self.next_boundary(cursor)
                };
                self.replace(range, "", window, cx);
            }
            "a" if cmd => {
                self.selected = 0..self.text.len();
                self.reversed = false;
                window.notify_text_selection_changed(&self.focus);
                cx.notify();
            }
            "c" | "x" if cmd => {
                if self.selected.is_empty() {
                    return;
                }
                let selected = self.text[self.selected.clone()].to_string();
                cx.write_to_clipboard(ClipboardItem::new_string(selected));
                if ev.keystroke.key == "x" {
                    self.replace(self.selected.clone(), "", window, cx);
                }
            }
            "v" if cmd => {
                if let Some(pasted) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    self.replace(self.selected.clone(), &pasted, window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.selecting = true;
        let offset = self.offset_for_position(ev.position);
        if ev.modifiers.shift {
            self.select_to(offset, window, cx);
        } else {
            self.move_to(offset, window, cx);
        }
    }

    fn mouse_move(&mut self, ev: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.selecting && ev.pressed_button == Some(MouseButton::Left) {
            let offset = self.offset_for_position(ev.position);
            self.select_to(offset, window, cx);
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        match (&self.last_line, &self.last_bounds) {
            (Some(line), Some(bounds)) if line.text == self.text.as_str() => {
                line.closest_index_for_x(position.x - bounds.left())
            }
            _ => self.text.len(),
        }
    }

    /// Byte offset for a UTF-16 offset, clamped to the text.
    fn offset_from_utf16(&self, utf16: usize) -> usize {
        let mut units = 0;
        for (i, ch) in self.text.char_indices() {
            if units >= utf16 {
                return i;
            }
            units += ch.len_utf16();
        }
        self.text.len()
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.text[..offset.min(self.text.len())]
            .chars()
            .map(char::len_utf16)
            .sum()
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let start = self.offset_from_utf16(range.start);
        start..self.offset_from_utf16(range.end).max(start)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }
}

/// Keeps the input on one line: newlines become spaces and other control
/// characters (a typed tab or return) are dropped.
fn sanitize(text: &str) -> String {
    text.replace("\r\n", " ")
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect()
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        *adjusted_range = Some(self.range_to_utf16(&range));
        Some(self.text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }

    /// Commits text: typed characters and the end of an IME composition.
    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        // A typed Return or Tab inserts nothing; pasted newlines (in
        // `key_down`) still become spaces.
        let new_text = if new_text.chars().all(char::is_control) {
            ""
        } else {
            new_text
        };
        self.replace(range, new_text, window, cx);
    }

    /// Shows an IME composition in progress, underlined, in place of the
    /// marked text (or the selection when nothing is marked yet).
    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|r| self.range_from_utf16(&r))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        let new_text = sanitize(new_text);
        self.text.replace_range(range.clone(), &new_text);
        let end = range.start + new_text.len();
        self.marked = (!new_text.is_empty()).then_some(range.start..end);
        // The new selection is relative to the inserted text, in UTF-16.
        self.selected = match new_selected_range_utf16 {
            Some(sel) => {
                let base = self.offset_to_utf16(range.start);
                let len = self.offset_to_utf16(end) - base;
                let sel = base + sel.start.min(len)..base + sel.end.min(len);
                self.range_from_utf16(&sel)
            }
            None => end..end,
        };
        self.reversed = false;
        self.changed(window, cx);
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = self.range_from_utf16(&range_utf16);
        self.reversed = false;
        window.notify_text_selection_changed(&self.focus);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_line.as_ref()?;
        if line.text != self.text.as_str() {
            return None;
        }
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(range.start), bounds.top()),
            point(bounds.left() + line.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let local = self.last_bounds?.localize(&position)?;
        let line = self.last_line.as_ref()?;
        if line.text != self.text.as_str() {
            return None;
        }
        Some(self.offset_to_utf16(line.closest_index_for_x(local.x)))
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.offset_to_utf16(self.text.len()))
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("text-input")
            .key_context(TEXT_INPUT_KEY_CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .w_full()
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .child(LineElement { input: cx.entity() })
    }
}

/// Shapes and paints the text, selection, composition underline and caret,
/// and registers the input handler while the input is focused.
struct LineElement {
    input: Entity<TextInput>,
}

struct Painted {
    line: ShapedLine,
    selection: Option<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl IntoElement for LineElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for LineElement {
    type RequestLayoutState = ();
    type PrepaintState = Painted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Painted {
        let ui = &cx.global::<AppState>().theme.ui;
        let input = self.input.read(cx);
        let style = window.text_style();
        let (shown, text_color) = if input.text.is_empty() {
            (input.placeholder.clone(), color(ui.text_muted))
        } else {
            (SharedString::from(input.text.clone()), color(ui.text))
        };
        let run = |len: usize, underline: bool| TextRun {
            len,
            font: style.font(),
            color: text_color.into(),
            background_color: None,
            underline: underline.then(|| UnderlineStyle {
                color: Some(text_color.into()),
                thickness: px(1.),
                wavy: false,
            }),
            strikethrough: None,
        };
        let runs = match &input.marked {
            Some(m) if !input.text.is_empty() => [
                run(m.start, false),
                run(m.len(), true),
                run(shown.len() - m.end, false),
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect(),
            _ => vec![run(shown.len(), false)],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(shown, font_size, &runs, None);

        let x = |offset: usize| {
            if input.text.is_empty() {
                px(0.)
            } else {
                line.x_for_index(offset)
            }
        };
        let (selection, caret) = if input.selected.is_empty() {
            let caret = fill(
                Bounds::new(
                    point(bounds.left() + x(input.cursor()), bounds.top()),
                    size(px(1.5), bounds.size.height),
                ),
                color(ui.text),
            );
            (None, Some(caret))
        } else {
            let selection = fill(
                Bounds::from_corners(
                    point(bounds.left() + x(input.selected.start), bounds.top()),
                    point(bounds.left() + x(input.selected.end), bounds.bottom()),
                ),
                color(ui.selection),
            );
            (Some(selection), None)
        };
        Painted {
            line,
            selection,
            caret,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        painted: &mut Painted,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(selection) = painted.selection.take() {
            window.paint_quad(selection);
        }
        let line = painted.line.clone();
        if let Err(err) = line.paint(
            bounds.origin,
            window.line_height(),
            TextAlign::Left,
            None,
            window,
            cx,
        ) {
            tracing::warn!(%err, "could not paint the text input");
        }
        if focus.is_focused(window)
            && let Some(caret) = painted.caret.take()
        {
            window.paint_quad(caret);
        }
        self.input.update(cx, |input, _| {
            // The placeholder's layout must not be used for hit-testing.
            input.last_line = (!input.text.is_empty()).then_some(line);
            input.last_bounds = Some(bounds);
        });
    }
}
