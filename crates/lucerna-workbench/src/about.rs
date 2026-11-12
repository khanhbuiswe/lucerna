//! Help: About (`workbench.action.showAboutDialog`): the app's version in
//! a prompt, with a button that copies it.

use gpui::{App, ClipboardItem, PromptLevel, Window};

use crate::CommandRegistry;

/// What the prompt says and Copy copies.
const ABOUT_TEXT: &str = concat!("Lucerna ", env!("CARGO_PKG_VERSION"));
const ANSWERS: [&str; 2] = ["OK", "Copy"];

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.showAboutDialog",
        "Help: About",
        |_, _, window, cx| show(window, cx),
    );
}

/// Shows "Lucerna ‹version›" with OK and Copy; Copy puts the same text on
/// the clipboard.
pub(crate) fn show(window: &mut Window, cx: &mut App) {
    let answer = window.prompt(PromptLevel::Info, ABOUT_TEXT, None, &ANSWERS[..], cx);
    cx.spawn(async move |cx| {
        if answer.await == Ok(1) {
            cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(ABOUT_TEXT.into())));
        }
    })
    .detach();
}
