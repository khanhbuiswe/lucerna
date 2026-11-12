//! The editing commands of the code view, with VS Code's ids: cursor
//! movement and selection.

use gpui::Context;
use lucerna_editor::Motion;

use crate::code_view::CodeView;
use crate::{CommandHandler, CommandRegistry, Workbench};

/// Runs `f` on the code view of the active tab, if it shows one.
fn with_view(
    wb: &mut Workbench,
    cx: &mut Context<Workbench>,
    f: impl FnOnce(&mut CodeView, &mut Context<CodeView>),
) {
    if let Some(view) = wb.active_code_view(cx) {
        view.update(cx, f);
    }
}

fn move_cursor(wb: &mut Workbench, cx: &mut Context<Workbench>, motion: Motion, extend: bool) {
    with_view(wb, cx, |v, cx| v.move_cursor(motion, extend, cx));
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    // Not in the command palette, as in VS Code: they are for keys.
    // A handler for `motion`; `extend` is the `…Select` variant.
    macro_rules! mv {
        ($motion:ident, $extend:expr) => {
            |wb, _, _, cx| move_cursor(wb, cx, Motion::$motion, $extend)
        };
    }
    let moves: [(&'static str, CommandHandler); 26] = [
        ("cursorLeft", mv!(Left, false)),
        ("cursorLeftSelect", mv!(Left, true)),
        ("cursorRight", mv!(Right, false)),
        ("cursorRightSelect", mv!(Right, true)),
        ("cursorUp", mv!(Up, false)),
        ("cursorUpSelect", mv!(Up, true)),
        ("cursorDown", mv!(Down, false)),
        ("cursorDownSelect", mv!(Down, true)),
        ("cursorWordLeft", mv!(WordStartLeft, false)),
        ("cursorWordLeftSelect", mv!(WordStartLeft, true)),
        ("cursorWordEndRight", mv!(WordEndRight, false)),
        ("cursorWordEndRightSelect", mv!(WordEndRight, true)),
        ("cursorHome", mv!(Home, false)),
        ("cursorHomeSelect", mv!(Home, true)),
        ("cursorEnd", mv!(End, false)),
        ("cursorEndSelect", mv!(End, true)),
        ("cursorLineStart", mv!(LineStart, false)),
        ("cursorLineEnd", mv!(LineEnd, false)),
        ("cursorTop", mv!(Top, false)),
        ("cursorTopSelect", mv!(Top, true)),
        ("cursorBottom", mv!(Bottom, false)),
        ("cursorBottomSelect", mv!(Bottom, true)),
        ("cursorPageUp", mv!(PageUp, false)),
        ("cursorPageUpSelect", mv!(PageUp, true)),
        ("cursorPageDown", mv!(PageDown, false)),
        ("cursorPageDownSelect", mv!(PageDown, true)),
    ];
    for (id, handler) in moves {
        commands.register(id, "", handler);
    }
    commands.register("editor.action.selectAll", "Select All", |wb, _, _, cx| {
        with_view(wb, cx, CodeView::select_all)
    });
}
