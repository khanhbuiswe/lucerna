//! The mouse in a terminal view (spec §6.5): selecting, the right click,
//! the wheel, reports to apps that took the mouse, and the clipboard.

use gpui::{
    App, ClipboardItem, Context, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, ScrollDelta, ScrollEvent, Window,
};
use lucerna_terminal::{
    Modes, MouseAction, MouseButtonKind, Scroll, SelectKind, ViewStamp, alternate_scroll,
    mouse_report,
};

use super::settings::{MouseSettings, PasteWarning, RightClick, mouse_settings};
use super::view::TerminalView;

/// How far the mouse moves before a press becomes a selection drag.
const DRAG_THRESHOLD: f32 = 2.;

/// A button held down over the terminal.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Press {
    /// Reported to the app; motion is reported while it is held.
    Reported {
        button: MouseButtonKind,
        /// The last cell reported, so motion within a cell sends nothing.
        cell: (u16, u16),
    },
    /// Selecting text. A plain press starts a selection only once it moved
    /// [`DRAG_THRESHOLD`]; a double or triple click, or a ⇧-click
    /// extending one, selects at once (`started`).
    Selecting {
        from: Point<Pixels>,
        cell: (u16, u16, bool),
        /// The screen `cell` was read off, so a start deferred past the
        /// threshold still lands on the text pressed.
        stamp: ViewStamp,
        kind: SelectKind,
        started: bool,
    },
}

/// The view's mouse state between events.
#[derive(Debug, Default)]
pub(crate) struct MouseState {
    pub press: Option<Press>,
    /// The last cell a buttonless motion was reported at.
    pub hover_cell: Option<(u16, u16)>,
    /// Wheel lines not yet scrolled (a fraction of a line, or a trackpad's
    /// pixels).
    pub wheel_rest: f32,
}

fn button_kind(button: MouseButton) -> Option<MouseButtonKind> {
    match button {
        MouseButton::Left => Some(MouseButtonKind::Left),
        MouseButton::Middle => Some(MouseButtonKind::Middle),
        MouseButton::Right => Some(MouseButtonKind::Right),
        _ => None,
    }
}

/// Whether a press goes to selection although the app took the mouse: ⇧
/// (as Zed), or ⌥ on macOS with `macOptionClickForcesSelection` (as VS
/// Code).
fn forces_selection(m: &Modifiers, settings: &MouseSettings) -> bool {
    m.shift || (cfg!(target_os = "macos") && m.alt && settings.option_click_forces_selection)
}

impl TerminalView {
    /// Whether a button pressed over the terminal is still held.
    pub(crate) fn mouse_pressed(&self) -> bool {
        self.mouse.press.is_some()
    }

    fn cell_at(&self, pos: Point<Pixels>) -> Option<(u16, u16, bool)> {
        Some(self.last_frame.as_ref()?.geometry.cell_at(pos))
    }

    /// The painted screen the mouse points at.
    fn stamp(&self) -> ViewStamp {
        self.last_frame
            .as_ref()
            .map(|f| f.stamp)
            .unwrap_or_default()
    }

    /// The modes of the painted screen. Read from the last frame, never
    /// from the terminal, so mouse events do not wait on its lock while
    /// the I/O thread parses a flood.
    pub(crate) fn painted_mode(&self) -> Modes {
        self.last_frame.as_ref().map_or(Modes::empty(), |f| f.mode)
    }

    /// The report for a mouse event at `cell`, sent to the app; whether
    /// the app took the mouse.
    fn report(
        &self,
        button: MouseButtonKind,
        action: MouseAction,
        cell: (u16, u16),
        m: &Modifiers,
    ) -> bool {
        let mode = self.painted_mode();
        match mouse_report(
            button, action, cell.0, cell.1, m.shift, m.alt, m.control, mode,
        ) {
            Some(bytes) => {
                self.terminal.input(&bytes);
                true
            }
            None => mode.intersects(Modes::MOUSE_MODE),
        }
    }

    pub(crate) fn mouse_down(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        // ⌘+click on a link opens it, and nothing else happens: no
        // selection, no report to an app that took the mouse.
        if ev.button == MouseButton::Left
            && super::links::link_modifier(&ev.modifiers)
            && self.link_click(ev.position, cx)
        {
            return;
        }
        let Some(cell) = self.cell_at(ev.position) else {
            return;
        };
        let settings = mouse_settings(cx);
        let mode = self.painted_mode();
        if mode.intersects(Modes::MOUSE_MODE) && !forces_selection(&ev.modifiers, &settings) {
            if let Some(button) = button_kind(ev.button) {
                self.report(button, MouseAction::Press, (cell.0, cell.1), &ev.modifiers);
                self.mouse.press = Some(Press::Reported {
                    button,
                    cell: (cell.0, cell.1),
                });
            }
            return;
        }
        match ev.button {
            MouseButton::Left => self.left_down(ev, cell, &settings),
            MouseButton::Right => self.right_click(cell, &settings, cx),
            _ => {}
        }
        cx.notify();
    }

    fn left_down(&mut self, ev: &MouseDownEvent, cell: (u16, u16, bool), settings: &MouseSettings) {
        let (row, col, right) = cell;
        let stamp = self.stamp();
        let painted_selection = self.last_frame.as_ref().is_some_and(|f| f.has_selection);
        let term = &self.terminal;
        let kind = match ev.click_count {
            n if n >= 3 => SelectKind::Line,
            2 => SelectKind::Word,
            _ if ev.modifiers.alt => SelectKind::Block,
            _ => SelectKind::Simple,
        };
        let started = match kind {
            SelectKind::Word | SelectKind::Line => {
                term.set_word_separators(&settings.word_separators);
                term.select_start_in(stamp, row, col, right, kind);
                true
            }
            _ if ev.modifiers.shift && painted_selection => {
                term.select_update_in(stamp, row, col, right);
                true
            }
            _ => {
                term.clear_selection();
                false
            }
        };
        self.mouse.press = Some(Press::Selecting {
            from: ev.position,
            cell,
            stamp,
            kind,
            started,
        });
    }

    fn right_click(
        &mut self,
        cell: (u16, u16, bool),
        settings: &MouseSettings,
        cx: &mut Context<Self>,
    ) {
        match settings.right_click {
            RightClick::SelectWord => {
                self.terminal.set_word_separators(&settings.word_separators);
                self.terminal.select_start_in(
                    self.stamp(),
                    cell.0,
                    cell.1,
                    cell.2,
                    SelectKind::Word,
                );
                self.selection_made(settings, cx);
            }
            RightClick::CopyPaste => {
                if self.copy_selection(cx) {
                    self.terminal.clear_selection();
                } else {
                    self.paste_from_clipboard(cx);
                }
            }
            RightClick::Paste => self.paste_from_clipboard(cx),
            RightClick::Nothing => {}
        }
    }

    pub(crate) fn mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(cell) = self.cell_at(ev.position) else {
            return;
        };
        match self.mouse.press {
            Some(Press::Reported { button, cell: last }) => {
                if (cell.0, cell.1) != last {
                    self.report(button, MouseAction::Motion, (cell.0, cell.1), &ev.modifiers);
                    self.mouse.press = Some(Press::Reported {
                        button,
                        cell: (cell.0, cell.1),
                    });
                }
            }
            Some(Press::Selecting {
                from,
                cell: start,
                stamp,
                kind,
                started,
            }) => {
                if !started {
                    let moved = ev.position - from;
                    let distance = f32::from(moved.x).hypot(f32::from(moved.y));
                    if distance < DRAG_THRESHOLD {
                        return;
                    }
                    self.terminal
                        .select_start_in(stamp, start.0, start.1, start.2, kind);
                    self.mouse.press = Some(Press::Selecting {
                        from,
                        cell: start,
                        stamp,
                        kind,
                        started: true,
                    });
                }
                self.terminal
                    .select_update_in(self.stamp(), cell.0, cell.1, cell.2);
                cx.notify();
            }
            None => {
                self.link_mouse_move(ev.position, &ev.modifiers, cx);
                let here = (cell.0, cell.1);
                if self.mouse.hover_cell != Some(here)
                    && self.painted_mode().contains(Modes::MOUSE_MOTION)
                {
                    self.mouse.hover_cell = Some(here);
                    self.report(
                        MouseButtonKind::None,
                        MouseAction::Motion,
                        here,
                        &ev.modifiers,
                    );
                }
            }
        }
    }

    pub(crate) fn mouse_up(&mut self, ev: &MouseUpEvent, cx: &mut Context<Self>) {
        match self.mouse.press.take() {
            Some(Press::Reported { button, cell }) => {
                let at = self.cell_at(ev.position).map_or(cell, |c| (c.0, c.1));
                self.report(button, MouseAction::Release, at, &ev.modifiers);
            }
            Some(Press::Selecting { started: true, .. }) => {
                let settings = mouse_settings(cx);
                self.selection_made(&settings, cx);
                cx.notify();
            }
            _ => {}
        }
    }

    /// A selection was just made: `copyOnSelection` copies it, and on
    /// Linux it becomes the primary selection.
    fn selection_made(&mut self, settings: &MouseSettings, cx: &mut Context<Self>) {
        if settings.copy_on_selection {
            self.copy_selection(cx);
        }
        #[cfg(target_os = "linux")]
        if let Some(text) = self.terminal.selection_text() {
            cx.write_to_primary(ClipboardItem::new_string(text));
        }
    }

    /// The wheel: reported to an app that took the mouse (unless ⇧),
    /// arrow keys on the alternate screen, otherwise the scrollback, by
    /// `mouseWheelScrollSensitivity` (`fastScrollSensitivity` with ⌥).
    pub(crate) fn scroll_wheel(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let Some(frame) = self.last_frame.clone() else {
            return;
        };
        let settings = mouse_settings(cx);
        let delta = match ev.delta {
            ScrollDelta::Lines(d) => d.y,
            ScrollDelta::Pixels(d) => f32::from(d.y) / f32::from(frame.geometry.cell_h),
        };
        let factor = if ev.modifiers.alt {
            settings.fast_scroll_sensitivity
        } else {
            settings.wheel_sensitivity
        };
        // A change of direction drops what was left of the other one.
        if self.mouse.wheel_rest * delta < 0. {
            self.mouse.wheel_rest = 0.;
        }
        let total = self.mouse.wheel_rest + delta * factor;
        let lines = total.trunc();
        self.mouse.wheel_rest = total - lines;
        let lines = lines as i32;
        if lines == 0 {
            return;
        }
        let mode = frame.mode;
        if mode.intersects(Modes::MOUSE_MODE) && !ev.modifiers.shift {
            let (row, col, _) = frame.geometry.cell_at(ev.position);
            let button = if lines > 0 {
                MouseButtonKind::WheelUp
            } else {
                MouseButtonKind::WheelDown
            };
            for _ in 0..lines.unsigned_abs() {
                self.report(button, MouseAction::Press, (row, col), &ev.modifiers);
            }
        } else if mode.contains(Modes::ALT_SCREEN) {
            if let Some(bytes) = alternate_scroll(-lines, mode) {
                self.terminal.input(&bytes);
            }
        } else {
            self.terminal.scroll(Scroll::Lines(lines));
            cx.notify();
        }
    }

    /// Puts the selection on the clipboard and keeps it selected; whether
    /// there was one.
    pub(crate) fn copy_selection(&mut self, cx: &mut App) -> bool {
        let Some(text) = self.terminal.selection_text() else {
            return false;
        };
        #[cfg(target_os = "linux")]
        cx.write_to_primary(ClipboardItem::new_string(text.clone()));
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    /// Pastes the clipboard's text, asking first when it is several lines
    /// (see [`TerminalView::request_paste`]).
    pub(crate) fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.request_paste(text, cx);
        }
    }

    /// Pastes `text`, or first shows the multi-line warning: with
    /// `enableMultiLinePasteWarning` `auto`, when the text has a newline
    /// and the app has not turned on bracketed paste.
    pub(crate) fn request_paste(&mut self, text: String, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        let multi_line = text.contains('\n') || text.contains('\r');
        let warn = multi_line
            && match mouse_settings(cx).paste_warning {
                PasteWarning::Never => false,
                PasteWarning::Always => true,
                PasteWarning::Auto => !self.terminal.mode().contains(Modes::BRACKETED_PASTE),
            };
        if warn {
            self.pending_paste = Some(text);
            cx.notify();
        } else {
            self.paste_text(&text, cx);
        }
    }

    /// Answers the multi-line paste warning.
    pub(crate) fn resolve_paste(&mut self, confirmed: bool, cx: &mut Context<Self>) {
        if let Some(text) = self.pending_paste.take() {
            if confirmed {
                self.paste_text(&text, cx);
            }
            cx.notify();
        }
    }

    /// A key pressed while the paste warning shows: enter pastes, escape
    /// cancels, and other keys are dropped so they do not reach the shell
    /// behind it. False when there is no warning.
    pub(crate) fn paste_warning_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if self.pending_paste.is_none() {
            return false;
        }
        match key {
            "enter" => self.resolve_paste(true, cx),
            "escape" => self.resolve_paste(false, cx),
            _ => {}
        }
        true
    }
}
