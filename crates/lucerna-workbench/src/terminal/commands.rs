//! The terminal's commands (spec §8.1) and which commands a key pressed in
//! the terminal runs instead of going to the shell (spec §6.4).

use std::collections::BTreeSet;

use gpui::{App, KeyDownEvent};
use lucerna_terminal::{KeyInput, Modes, Scroll, key_to_bytes};
use serde_json::Value;

use crate::{AppState, CommandRegistry, Workbench};

const SKIP_SHELL_SETTING: &str = "terminal.integrated.commandsToSkipShell";

/// Commands whose keys the workbench takes even while the terminal has
/// focus: every command of spec §8.1, and the workbench commands listed
/// under it (VS Code's `DEFAULT_COMMANDS_TO_SKIP_SHELL` has the same ones).
pub const DEFAULT_COMMANDS_TO_SKIP_SHELL: &[&str] = &[
    "workbench.action.terminal.toggleTerminal",
    "workbench.action.terminal.new",
    "workbench.action.terminal.focus",
    "workbench.action.terminal.kill",
    "workbench.action.terminal.focusNext",
    "workbench.action.terminal.focusPrevious",
    "workbench.action.terminal.clear",
    "workbench.action.terminal.copySelection",
    "workbench.action.terminal.paste",
    "workbench.action.terminal.selectAll",
    "workbench.action.terminal.clearSelection",
    "workbench.action.terminal.scrollUp",
    "workbench.action.terminal.scrollDown",
    "workbench.action.terminal.scrollUpPage",
    "workbench.action.terminal.scrollDownPage",
    "workbench.action.terminal.scrollToTop",
    "workbench.action.terminal.scrollToBottom",
    "workbench.action.terminal.sendSequence",
    "workbench.action.togglePanel",
    "workbench.action.toggleMaximizedPanel",
    "workbench.action.positionPanelBottom",
    "workbench.action.positionPanelRight",
    "workbench.action.quickOpen",
    "workbench.action.showCommands",
    "workbench.action.findInFiles",
    "workbench.action.toggleSidebarVisibility",
    "workbench.view.explorer",
    "workbench.view.extensions",
    "workbench.action.focusFirstEditorGroup",
    "workbench.action.focusSecondEditorGroup",
    "workbench.action.focusThirdEditorGroup",
    "workbench.action.focusFourthEditorGroup",
    "workbench.action.focusFifthEditorGroup",
    "workbench.action.focusSixthEditorGroup",
    "workbench.action.focusSeventhEditorGroup",
    "workbench.action.focusEighthEditorGroup",
    "workbench.action.quit",
    "workbench.action.gotoLine",
];

/// Whether a key bound to `command` runs it instead of going to the shell:
/// [`DEFAULT_COMMANDS_TO_SKIP_SHELL`] plus the ids in
/// `terminal.integrated.commandsToSkipShell`, minus those written there
/// with a leading `-` (as VS Code).
pub fn skips_shell(command: &str, cx: &App) -> bool {
    let user: Vec<String> = cx
        .global::<AppState>()
        .settings
        .get(SKIP_SHELL_SETTING, None)
        .unwrap_or_default();
    let mut list: BTreeSet<&str> = DEFAULT_COMMANDS_TO_SKIP_SHELL.iter().copied().collect();
    for id in &user {
        match id.strip_prefix('-') {
            Some(removed) => {
                list.remove(removed);
            }
            None => {
                list.insert(id);
            }
        }
    }
    list.contains(command)
}

/// `terminal.integrated.allowChords`: a chord's first key pressed in the
/// terminal waits for the next one (default true, as VS Code).
pub(crate) fn allow_chords(cx: &App) -> bool {
    cx.global::<AppState>()
        .settings
        .get("terminal.integrated.allowChords", None)
        .unwrap_or(true)
}

/// `terminal.integrated.macOptionIsMeta` (default false, as VS Code).
pub(crate) fn option_is_meta(cx: &App) -> bool {
    cx.global::<AppState>()
        .settings
        .get("terminal.integrated.macOptionIsMeta", None)
        .unwrap_or(false)
}

/// The bytes the shell gets for `ev` in `mode`; `None` for a key with no
/// encoding (and for anything with cmd).
pub(crate) fn key_bytes(ev: &KeyDownEvent, mode: Modes, option_is_meta: bool) -> Option<Vec<u8>> {
    let stroke = &ev.keystroke;
    let m = stroke.modifiers;
    let input = KeyInput {
        key: &stroke.key,
        text: stroke.key_char.as_deref(),
        ctrl: m.control,
        alt: m.alt,
        shift: m.shift,
        cmd: m.platform,
    };
    key_to_bytes(&input, mode, option_is_meta)
}

/// Scrolls the active terminal; the scroll commands only work on the main
/// screen, as in VS Code.
fn scroll(wb: &mut Workbench, to: Scroll, cx: &mut gpui::Context<Workbench>) {
    let Some(view) = wb.active_terminal_view(cx) else {
        return;
    };
    view.update(cx, |view, cx| {
        if !view.terminal().mode().contains(Modes::ALT_SCREEN) {
            view.terminal().scroll(to);
            cx.notify();
        }
    });
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.terminal.toggleTerminal",
        "View: Toggle Terminal",
        |wb, _, window, cx| wb.toggle_terminal(window, cx),
    );
    commands.register(
        "workbench.action.terminal.new",
        "Terminal: Create New Terminal",
        |wb, _, window, cx| wb.new_terminal_tab(window, cx),
    );
    commands.register(
        "workbench.action.terminal.focus",
        "Terminal: Focus Terminal",
        |wb, _, window, cx| wb.show_panel(window, cx),
    );
    commands.register(
        "workbench.action.terminal.kill",
        "Terminal: Kill the Active Terminal Instance",
        |wb, _, window, cx| wb.kill_terminal(window, cx),
    );
    commands.register(
        "workbench.action.terminal.focusNext",
        "Terminal: Focus Next Terminal",
        |wb, _, window, cx| wb.focus_terminal_by(1, window, cx),
    );
    commands.register(
        "workbench.action.terminal.focusPrevious",
        "Terminal: Focus Previous Terminal",
        |wb, _, window, cx| wb.focus_terminal_by(-1, window, cx),
    );
    commands.register(
        "workbench.action.terminal.clear",
        "Terminal: Clear",
        |wb, _, _, cx| {
            if let Some(view) = wb.active_terminal_view(cx) {
                view.update(cx, |view, cx| {
                    view.terminal().clear();
                    cx.notify();
                });
            }
        },
    );
    // Selection arrives with the mouse handling (Task 10); until then
    // there is never a selection, so these do nothing.
    commands.register(
        "workbench.action.terminal.copySelection",
        "Terminal: Copy Selection",
        |_, _, _, _| {},
    );
    commands.register(
        "workbench.action.terminal.selectAll",
        "Terminal: Select All",
        |_, _, _, _| {},
    );
    commands.register(
        "workbench.action.terminal.clearSelection",
        "Terminal: Clear Selection",
        |_, _, _, _| {},
    );
    commands.register(
        "workbench.action.terminal.paste",
        "Terminal: Paste into Active Terminal",
        |wb, _, _, cx| {
            let text = cx.read_from_clipboard().and_then(|item| item.text());
            if let (Some(view), Some(text)) = (wb.active_terminal_view(cx), text) {
                view.update(cx, |view, cx| view.paste_text(&text, cx));
            }
        },
    );
    commands.register(
        "workbench.action.terminal.scrollUp",
        "Terminal: Scroll Up (Line)",
        |wb, _, _, cx| scroll(wb, Scroll::Lines(1), cx),
    );
    commands.register(
        "workbench.action.terminal.scrollDown",
        "Terminal: Scroll Down (Line)",
        |wb, _, _, cx| scroll(wb, Scroll::Lines(-1), cx),
    );
    commands.register(
        "workbench.action.terminal.scrollUpPage",
        "Terminal: Scroll Up (Page)",
        |wb, _, _, cx| scroll(wb, Scroll::PageUp, cx),
    );
    commands.register(
        "workbench.action.terminal.scrollDownPage",
        "Terminal: Scroll Down (Page)",
        |wb, _, _, cx| scroll(wb, Scroll::PageDown, cx),
    );
    commands.register(
        "workbench.action.terminal.scrollToTop",
        "Terminal: Scroll to Top",
        |wb, _, _, cx| scroll(wb, Scroll::Top, cx),
    );
    commands.register(
        "workbench.action.terminal.scrollToBottom",
        "Terminal: Scroll to Bottom",
        |wb, _, _, cx| scroll(wb, Scroll::Bottom, cx),
    );
    commands.register(
        "workbench.action.terminal.sendSequence",
        "Terminal: Send Custom Sequence to Terminal",
        |wb, args, _, cx| {
            let text = args.and_then(|a| a.get("text")).and_then(Value::as_str);
            if let (Some(view), Some(text)) = (wb.active_terminal_view(cx), text) {
                view.update(cx, |view, cx| view.write_input(text.as_bytes(), cx));
            }
        },
    );
}
