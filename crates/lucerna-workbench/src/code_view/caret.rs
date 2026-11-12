//! The caret's look (`editor.cursorStyle`) and blink
//! (`editor.cursorBlinking`): 500 ms on, 500 ms off while the view has
//! focus, back on at once after any selection change or edit.

use std::time::Duration;

use gpui::{Context, Task};

use super::CodeView;

/// Each half of a blink.
pub(crate) const BLINK_INTERVAL: Duration = Duration::from_millis(500);

/// How the caret is drawn, from `editor.cursorStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaretStyle {
    /// A bar before the char, this many pixels wide.
    Line(u8),
    /// The char's cell, filled.
    Block,
    /// The char's cell, outlined.
    BlockOutline,
    /// A bar under the char, this many pixels high.
    Underline(u8),
}

impl CaretStyle {
    /// VS Code's `editor.cursorStyle` values; anything else is `line`.
    pub(crate) fn from_setting(value: &str) -> CaretStyle {
        match value {
            "block" => CaretStyle::Block,
            "block-outline" => CaretStyle::BlockOutline,
            "underline" => CaretStyle::Underline(2),
            "underline-thin" => CaretStyle::Underline(1),
            "line-thin" => CaretStyle::Line(1),
            _ => CaretStyle::Line(2),
        }
    }
}

/// The blink state. The timer runs only while the view has focus.
#[derive(Default)]
pub(crate) struct Caret {
    /// In the off half of a blink.
    hidden: bool,
    /// The running blink; dropping it stops the blink.
    task: Option<Task<()>>,
}

impl Caret {
    pub(crate) fn visible(&self) -> bool {
        !self.hidden
    }

    /// Starts blinking if `focused`, `blinking` and not already running;
    /// stops (caret on) when not focused.
    pub(crate) fn ensure(&mut self, focused: bool, blinking: bool, cx: &mut Context<CodeView>) {
        if !focused || !blinking {
            self.stop();
        } else if self.task.is_none() {
            self.task = Some(blink(cx));
        }
    }

    /// Shows the caret now and starts the blink over, if it runs.
    pub(crate) fn reset(&mut self, cx: &mut Context<CodeView>) {
        self.hidden = false;
        if self.task.is_some() {
            self.task = Some(blink(cx));
        }
    }

    pub(crate) fn stop(&mut self) {
        self.hidden = false;
        self.task = None;
    }
}

fn blink(cx: &mut Context<CodeView>) -> Task<()> {
    cx.spawn(async move |view, cx| {
        loop {
            cx.background_executor().timer(BLINK_INTERVAL).await;
            let toggled = view.update(cx, |view, cx| {
                view.caret.hidden = !view.caret.hidden;
                cx.notify();
            });
            if toggled.is_err() {
                break;
            }
        }
    })
}
