//! One terminal's view. Empty for now: the panel's tabs hold it, and the
//! terminal element draws into it later.

use gpui::{App, Context, FocusHandle, Focusable, IntoElement, Render, Window};

pub struct TerminalView {
    focus: FocusHandle,
}

impl TerminalView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        TerminalView {
            focus: cx.focus_handle(),
        }
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}
