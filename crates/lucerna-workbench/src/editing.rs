//! The editing commands of the code view, with VS Code's ids: cursor
//! movement and selection, typing, deleting, indenting, the clipboard,
//! undo / redo and `cursorUndo` / `cursorRedo`.

use gpui::{Context, Focusable as _, Window};
use lucerna_editor::{Ctx, EditPlan, Motion, SelectionSet};
use lucerna_editor::{commands, comment};

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

/// Runs the edit command `f` in the active view.
fn edit(wb: &mut Workbench, cx: &mut Context<Workbench>, f: fn(Ctx, &SelectionSet) -> EditPlan) {
    with_view(wb, cx, |v, cx| {
        v.unmark();
        v.edit(cx, f);
    });
}

/// Undo (or redo) in the active view; any composition ends first.
fn undo(wb: &mut Workbench, window: &mut Window, cx: &mut Context<Workbench>, undo: bool) {
    if let Some(view) = wb.active_code_view(cx) {
        let focus = view.update(cx, |v, cx| {
            v.undo(undo, cx);
            v.focus_handle(cx)
        });
        window.notify_text_input_changed(&focus);
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

    let edits: [(&'static str, CommandHandler); 9] = [
        ("deleteLeft", |wb, _, _, cx| {
            with_view(wb, cx, |v, cx| {
                v.delete_left(cx);
            })
        }),
        ("deleteRight", |wb, _, _, cx| {
            edit(wb, cx, commands::delete_right)
        }),
        ("deleteWordLeft", |wb, _, _, cx| {
            edit(wb, cx, commands::delete_word_left)
        }),
        ("deleteWordRight", |wb, _, _, cx| {
            edit(wb, cx, commands::delete_word_right)
        }),
        ("deleteAllLeft", |wb, _, _, cx| {
            edit(wb, cx, commands::delete_all_left)
        }),
        ("deleteAllRight", |wb, _, _, cx| {
            edit(wb, cx, commands::delete_all_right)
        }),
        ("tab", |wb, _, _, cx| edit(wb, cx, commands::tab)),
        ("outdent", |wb, _, _, cx| edit(wb, cx, commands::outdent)),
        ("lineBreakInsert", |wb, _, _, cx| {
            edit(wb, cx, commands::line_break_insert)
        }),
    ];
    for (id, handler) in edits {
        commands.register(id, "", handler);
    }
    let comments: [(&'static str, &'static str, CommandHandler); 4] = [
        (
            "editor.action.commentLine",
            "Toggle Line Comment",
            |wb, _, _, cx| edit(wb, cx, comment::toggle_line),
        ),
        (
            "editor.action.addCommentLine",
            "Add Line Comment",
            |wb, _, _, cx| edit(wb, cx, comment::add_line),
        ),
        (
            "editor.action.removeCommentLine",
            "Remove Line Comment",
            |wb, _, _, cx| edit(wb, cx, comment::remove_line),
        ),
        (
            "editor.action.blockComment",
            "Toggle Block Comment",
            |wb, _, _, cx| edit(wb, cx, comment::toggle_block),
        ),
    ];
    for (id, title, handler) in comments {
        commands.register(id, title, handler);
    }
    // VS Code's `type` command: `{"text": "\n"}` is Enter.
    commands.register("type", "", |wb, args, _, cx| {
        let Some(text) = args.and_then(|a| a.get("text")).and_then(|t| t.as_str()) else {
            return;
        };
        let text = text.to_string();
        with_view(wb, cx, |v, cx| {
            v.type_text(&text, cx);
        });
    });
    commands.register("undo", "Undo", |wb, _, window, cx| {
        undo(wb, window, cx, true)
    });
    commands.register("redo", "Redo", |wb, _, window, cx| {
        undo(wb, window, cx, false)
    });
    commands.register("cursorUndo", "Cursor Undo", |wb, _, _, cx| {
        with_view(wb, cx, |v, cx| v.cursor_undo(true, cx))
    });
    commands.register("cursorRedo", "Cursor Redo", |wb, _, _, cx| {
        with_view(wb, cx, |v, cx| v.cursor_undo(false, cx))
    });
    commands.register("editor.action.clipboardCutAction", "Cut", |wb, _, _, cx| {
        with_view(wb, cx, CodeView::cut)
    });
    commands.register(
        "editor.action.clipboardPasteAction",
        "Paste",
        |wb, _, _, cx| with_view(wb, cx, CodeView::paste),
    );
}
