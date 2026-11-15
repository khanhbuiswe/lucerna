//! Keys while the terminal has focus (spec §6.4): what goes to the shell,
//! what runs a command, ⌃` and the other terminal commands (§6.1, §8.1),
//! and IME input.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Entity, EntityInputHandler, Focusable as _, TestAppContext, VisualTestContext};
use lucerna_settings::{
    Context as WhenContext, ContextValue, Keymap, Layer, Resolution, SettingsStore, parse_keys,
};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, TerminalView, Workbench};

fn init(cx: &mut TestAppContext, user_settings: Option<&str>, user_keymap: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), user_keymap);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    if let Some(json) = user_settings {
        settings.set_layer(Layer::User, json).unwrap();
    }
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
    cx.set_global(AppState {
        settings,
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
        languages_generation: 0,
        icon_themes: lucerna_theme::IconThemeRegistry::build(&[]).0,
        icons: lucerna_theme::IconThemeRegistry::build(&[])
            .0
            .resolve("Lucide"),
    });
    // No real shells: terminals are fakes of the requested size.
    cx.update(|cx| {
        lucerna_workbench::set_default_terminal_spawner(
            Rc::new(|opts: lucerna_terminal::TerminalOptions| {
                Ok(lucerna_terminal::Terminal::fake_with_events(
                    opts.rows,
                    opts.cols,
                    opts.scrollback,
                ))
            }),
            cx,
        )
    });
}

fn open(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut VisualTestContext) {
    let (wb, cx) =
        cx.add_window_view(|window, cx| Workbench::new(PathBuf::from("/tmp/project"), window, cx));
    wb.update(cx, |wb, cx| {
        let body = CodeView::tab_body(wb.settings().clone());
        wb.editor_groups()
            .update(cx, |groups, cx| groups.set_tab_body_factory(body, cx))
    });
    draw(cx);
    (wb, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

fn run(
    wb: &Entity<Workbench>,
    command: &str,
    args: Option<serde_json::Value>,
    cx: &mut VisualTestContext,
) {
    let ran = cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command(command, args.as_ref(), window, cx)
        })
    });
    assert!(ran, "{command} is registered");
    draw(cx);
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    draw(cx);
}

/// Opens `path` with an empty code view.
fn open_file(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new(path)), OpenMode::Pinned, cx)
    });
    draw(cx);
}

fn tab_count(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| wb.read(cx).terminal_panel().read(cx).tab_count())
}

fn active_index(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<usize> {
    cx.update(|_, cx| wb.read(cx).terminal_panel().read(cx).active())
}

fn active_view(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<TerminalView> {
    cx.update(|_, cx| {
        let panel = wb.read(cx).terminal_panel().read(cx);
        let index = panel.active().expect("a terminal tab");
        panel.tabs()[index]
            .view
            .clone()
            .expect("a running terminal")
    })
}

fn written(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<u8> {
    let view = active_view(wb, cx);
    view.read_with(cx, |v, _| v.terminal().written())
}

fn feed(wb: &Entity<Workbench>, bytes: &[u8], cx: &mut VisualTestContext) {
    let view = active_view(wb, cx);
    view.read_with(cx, |v, _| v.terminal().feed(bytes));
    draw(cx);
}

fn terminal_focused(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        let view = {
            let panel = wb.read(cx).terminal_panel().read(cx);
            panel.active().and_then(|i| panel.tabs()[i].view.clone())
        };
        view.is_some_and(|v| v.read(cx).focus_handle(cx).is_focused(window))
    })
}

fn editor_focused(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        let keys = wb.read(cx).context_keys(window, cx);
        lucerna_settings::parse_when("editorFocus")
            .unwrap()
            .eval(&keys)
    })
}

fn panel_visible(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    wb.read_with(cx, |wb, _| wb.panel_state().visible)
}

/// A workbench with the terminal shown and focused.
fn with_terminal(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut VisualTestContext) {
    let (wb, cx) = open(cx);
    keys("ctrl-`", cx);
    assert!(terminal_focused(&wb, cx));
    (wb, cx)
}

#[gpui::test]
fn ctrl_backtick_cycle(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = open(cx);
    assert!(!panel_visible(&wb, cx));

    keys("ctrl-`", cx);
    assert!(panel_visible(&wb, cx));
    assert_eq!(tab_count(&wb, cx), 1, "a terminal is created");
    assert!(terminal_focused(&wb, cx));

    keys("ctrl-`", cx);
    assert!(!panel_visible(&wb, cx));
    assert!(editor_focused(&wb, cx));
    assert_eq!(tab_count(&wb, cx), 1, "hiding keeps the terminal");

    // Shown, but the focus is in the editor area: ⌃` focuses the terminal.
    keys("ctrl-`", cx);
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.focus_editor_area(window, cx)));
    draw(cx);
    assert!(panel_visible(&wb, cx));
    assert!(!terminal_focused(&wb, cx));
    keys("ctrl-`", cx);
    assert!(panel_visible(&wb, cx));
    assert!(terminal_focused(&wb, cx));
    assert_eq!(tab_count(&wb, cx), 1);
}

#[gpui::test]
fn ctrl_shift_backtick_new_terminal(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = with_terminal(cx);
    assert_eq!(tab_count(&wb, cx), 1);
    keys("ctrl-shift-`", cx);
    assert_eq!(tab_count(&wb, cx), 2);
    assert_eq!(active_index(&wb, cx), Some(1));
    assert!(terminal_focused(&wb, cx));
    assert!(written(&wb, cx).is_empty());
}

#[gpui::test]
fn typing_goes_to_pty(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = with_terminal(cx);
    keys("l s enter", cx);
    // Printable keys arrive once, through the input handler.
    assert_eq!(written(&wb, cx), b"ls\r");
}

#[gpui::test]
fn typing_scrolls_to_bottom(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = with_terminal(cx);
    feed(&wb, &b"line\r\n".repeat(100), cx);
    run(&wb, "workbench.action.terminal.scrollToTop", None, cx);
    let view = active_view(&wb, cx);
    assert!(view.read_with(cx, |v, _| v.terminal().snapshot().display_offset) > 0);
    keys("x", cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.terminal().snapshot().display_offset),
        0
    );
    keys("left", cx);
    assert_eq!(written(&wb, cx), b"x\x1b[D");
}

#[gpui::test]
fn ctrl_c_goes_to_pty(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = with_terminal(cx);
    keys("ctrl-c", cx);
    assert_eq!(written(&wb, cx), b"\x03");
    // ctrl+r opens recent folders elsewhere; in the terminal it is the
    // shell's history search.
    keys("ctrl-r", cx);
    assert_eq!(written(&wb, cx), b"\x03\x12");
}

#[gpui::test]
fn cmd_keys_never_reach_pty(cx: &mut TestAppContext) {
    let unbind = r#"[{ "key": "cmd+j", "command": "-workbench.action.togglePanel" }]"#;
    init(cx, None, Some(unbind));
    let (wb, cx) = with_terminal(cx);
    let sidebar = wb.read_with(cx, |wb, _| wb.sidebar_visible());
    keys("cmd-b", cx);
    assert_ne!(wb.read_with(cx, |wb, _| wb.sidebar_visible()), sidebar);
    assert!(written(&wb, cx).is_empty());
    keys("cmd-j", cx);
    assert!(written(&wb, cx).is_empty());
    assert!(panel_visible(&wb, cx));
    keys("cmd-u", cx);
    assert!(written(&wb, cx).is_empty());
}

#[gpui::test]
fn cmd_j_toggles_panel(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = with_terminal(cx);
    keys("cmd-j", cx);
    assert!(!panel_visible(&wb, cx));
    keys("cmd-j", cx);
    assert!(panel_visible(&wb, cx));
}

#[gpui::test]
fn cmd_w_closes_editor_not_terminal(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = open(cx);
    open_file(&wb, "a.rs", cx);
    keys("ctrl-`", cx);
    assert!(terminal_focused(&wb, cx));
    keys("cmd-w", cx);
    let editor_tab = cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .active_tab()
            .is_some()
    });
    assert!(!editor_tab, "⌘W closed the editor tab");
    assert_eq!(tab_count(&wb, cx), 1);
    assert!(written(&wb, cx).is_empty());
}

#[gpui::test]
fn vim_does_not_take_terminal_keys(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "vim.enabled": true }"#), None);
    let (wb, cx) = open(cx);
    open_file(&wb, "a.rs", cx);
    keys("ctrl-`", cx);
    assert!(terminal_focused(&wb, cx));
    keys("j", cx);
    assert_eq!(written(&wb, cx), b"j");
    keys("escape", cx);
    assert_eq!(written(&wb, cx), b"j\x1b");
}

#[gpui::test]
fn skip_shell_minus_prefix(cx: &mut TestAppContext) {
    let settings =
        r#"{ "terminal.integrated.commandsToSkipShell": ["-workbench.action.terminal.new"] }"#;
    init(cx, Some(settings), None);
    let (wb, cx) = with_terminal(cx);
    keys("ctrl-shift-`", cx);
    assert!(!written(&wb, cx).is_empty(), "the key went to the shell");
    assert_eq!(tab_count(&wb, cx), 1);
    // ⌃` is still in the list.
    keys("ctrl-`", cx);
    assert!(!panel_visible(&wb, cx));
}

#[gpui::test]
fn skip_shell_adds_command(cx: &mut TestAppContext) {
    // ctrl+r is not in the default list; adding its command runs it.
    let settings =
        r#"{ "terminal.integrated.commandsToSkipShell": ["workbench.action.openRecent"] }"#;
    init(cx, Some(settings), None);
    let (wb, cx) = with_terminal(cx);
    keys("ctrl-r", cx);
    assert!(written(&wb, cx).is_empty());
}

#[gpui::test]
fn cmd_k_clears(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = with_terminal(cx);
    feed(&wb, &b"line\r\n".repeat(100), cx);
    let view = active_view(&wb, cx);
    assert!(view.read_with(cx, |v, _| v.terminal().snapshot().history_len) > 0);
    keys("cmd-k", cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.terminal().snapshot().history_len),
        0
    );
    assert!(written(&wb, cx).is_empty());
    assert!(
        wb.read_with(cx, |wb, _| wb.pending_chord_label()).is_none(),
        "⌘K clears instead of starting a chord"
    );
}

#[gpui::test]
fn cmd_left_sends_ctrl_a_outside_alt_screen(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = with_terminal(cx);
    keys("cmd-left", cx);
    assert_eq!(written(&wb, cx), b"\x01");
    keys("cmd-right cmd-backspace alt-left alt-right", cx);
    assert_eq!(written(&wb, cx), b"\x01\x05\x15\x1bb\x1bf");

    feed(&wb, b"\x1b[?1049h", cx);
    let before = written(&wb, cx);
    keys("cmd-left", cx);
    assert_eq!(
        written(&wb, cx),
        before,
        "⌘← in the alt screen sends nothing"
    );
    // ⌥← is encoded for the app instead.
    keys("alt-left", cx);
    assert_eq!(&written(&wb, cx)[before.len()..], b"\x1b[1;3D");
}

#[gpui::test]
fn focus_next_previous(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = with_terminal(cx);
    keys("ctrl-shift-` ctrl-shift-`", cx);
    assert_eq!(active_index(&wb, cx), Some(2));
    keys("cmd-shift-]", cx);
    assert_eq!(active_index(&wb, cx), Some(0), "wraps around");
    assert!(terminal_focused(&wb, cx));
    keys("cmd-shift-]", cx);
    assert_eq!(active_index(&wb, cx), Some(1));
    keys("cmd-shift-[ cmd-shift-[", cx);
    assert_eq!(active_index(&wb, cx), Some(2));
    assert!(terminal_focused(&wb, cx));
}

#[gpui::test]
fn kill_closes_active_tab(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = with_terminal(cx);
    keys("ctrl-shift-`", cx);
    assert_eq!(tab_count(&wb, cx), 2);
    let killed = active_view(&wb, cx).downgrade();
    run(&wb, "workbench.action.terminal.kill", None, cx);
    assert_eq!(tab_count(&wb, cx), 1);
    assert!(killed.upgrade().is_none(), "the terminal is dropped");
    assert!(
        terminal_focused(&wb, cx),
        "focus moves to the remaining tab"
    );
    run(&wb, "workbench.action.terminal.kill", None, cx);
    assert_eq!(tab_count(&wb, cx), 0);
    assert!(!panel_visible(&wb, cx));
}

#[gpui::test]
fn focus_command_shows_and_focuses(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = open(cx);
    run(&wb, "workbench.action.terminal.focus", None, cx);
    assert!(panel_visible(&wb, cx));
    assert!(terminal_focused(&wb, cx));
}

#[gpui::test]
fn linux_copy_paste_bindings(_cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none());
    let ctx = |os: &str| {
        let mut ctx = WhenContext::default();
        ctx.set("terminalFocus", ContextValue::Bool(true));
        ctx.set("terminalTextSelected", ContextValue::Bool(true));
        for key in ["isMac", "isLinux", "isWindows"] {
            ctx.set(key, ContextValue::Bool(key == os));
        }
        ctx
    };
    let command = |keys: &str, os: &str| match keymap.resolve(&parse_keys(keys).unwrap(), &ctx(os))
    {
        Resolution::Command { command, .. } => Some(command),
        _ => None,
    };
    for os in ["isLinux", "isWindows"] {
        assert_eq!(
            command("ctrl+shift+c", os).as_deref(),
            Some("workbench.action.terminal.copySelection")
        );
        assert_eq!(
            command("ctrl+shift+v", os).as_deref(),
            Some("workbench.action.terminal.paste")
        );
    }
    assert_eq!(command("ctrl+shift+c", "isMac"), None);
    assert_eq!(command("ctrl+shift+v", "isMac"), None);
    // ⌃` stays on every platform.
    assert_eq!(
        command("ctrl+`", "isLinux").as_deref(),
        Some("workbench.action.terminal.toggleTerminal")
    );
}

#[gpui::test]
fn platform_context_key(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = open(cx);
    let is = cx.update(|window, cx| {
        let keys = wb.read(cx).context_keys(window, cx);
        ["isMac", "isLinux", "isWindows"]
            .map(|k| lucerna_settings::parse_when(k).unwrap().eval(&keys))
    });
    assert_eq!(
        is,
        [
            cfg!(target_os = "macos"),
            cfg!(target_os = "linux"),
            cfg!(target_os = "windows")
        ]
    );
}

#[gpui::test]
fn terminal_context_keys(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = with_terminal(cx);
    let eval = |expr: &str, cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            let keys = wb.read(cx).context_keys(window, cx);
            lucerna_settings::parse_when(expr).unwrap().eval(&keys)
        })
    };
    assert!(eval("terminalFocus", cx));
    assert!(!eval("terminalTextSelected", cx));
    assert!(!eval("terminalAltBufferActive", cx));
    feed(&wb, b"\x1b[?1049h", cx);
    assert!(eval("terminalAltBufferActive", cx));
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.focus_editor_area(window, cx)));
    draw(cx);
    assert!(!eval("terminalFocus", cx));
}

#[gpui::test]
fn send_sequence_args(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = with_terminal(cx);
    run(
        &wb,
        "workbench.action.terminal.sendSequence",
        Some(serde_json::json!({ "text": "\u{1b}\r" })),
        cx,
    );
    assert_eq!(written(&wb, cx), b"\x1b\r");
}

#[gpui::test]
fn ime_commit_writes_utf8(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = with_terminal(cx);
    let view = active_view(&wb, cx);
    view.update_in(cx, |v, window, cx| {
        v.replace_and_mark_text_in_range(None, "vie", None, window, cx)
    });
    draw(cx);
    assert!(written(&wb, cx).is_empty(), "marked text is not written");
    assert_eq!(
        view.read_with(cx, |v, _| v.marked_text().map(str::to_string)),
        Some("vie".into())
    );
    let marked = view.update_in(cx, |v, window, cx| v.marked_text_range(window, cx));
    assert_eq!(marked, Some(0..3));
    // The candidate window sits at the cursor cell.
    let bounds = view.update_in(cx, |v, window, cx| {
        v.bounds_for_range(0..0, gpui::Bounds::default(), window, cx)
    });
    let bounds = bounds.expect("cursor cell bounds");
    assert!(bounds.size.width > gpui::px(0.) && bounds.size.height > gpui::px(0.));

    view.update_in(cx, |v, window, cx| {
        v.replace_text_in_range(None, "việt", window, cx)
    });
    draw(cx);
    assert_eq!(written(&wb, cx), "việt".as_bytes());
    assert_eq!(
        view.read_with(cx, |v, _| v.marked_text().map(str::to_string)),
        None
    );
}

#[gpui::test]
fn status_bar_button_toggles_terminal(cx: &mut TestAppContext) {
    init(cx, None, None);
    let (wb, cx) = open(cx);
    let button = cx.debug_bounds("panel-toggle-terminal").expect("button");
    cx.simulate_click(button.center(), gpui::Modifiers::none());
    draw(cx);
    assert!(panel_visible(&wb, cx));
    assert!(terminal_focused(&wb, cx));

    // Focus elsewhere: the button focuses the terminal, as ⌃` does.
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.focus_editor_area(window, cx)));
    draw(cx);
    cx.simulate_click(button.center(), gpui::Modifiers::none());
    draw(cx);
    assert!(panel_visible(&wb, cx));
    assert!(terminal_focused(&wb, cx));
    // The click keeps the focus in the terminal, so the panel hides.
    cx.simulate_click(button.center(), gpui::Modifiers::none());
    draw(cx);
    assert!(!panel_visible(&wb, cx));
    assert!(editor_focused(&wb, cx));
}

#[gpui::test]
fn completed_chord_runs_its_command(cx: &mut TestAppContext) {
    // Neither key has ⌘, and selectTheme is not in the skip-shell list:
    // once the chord is pending, its last key still belongs to the keymap.
    let chord = r#"[{ "key": "ctrl+k ctrl+t", "command": "workbench.action.selectTheme" }]"#;
    init(cx, None, Some(chord));
    let (wb, cx) = with_terminal(cx);
    keys("ctrl-k", cx);
    assert!(wb.read_with(cx, |wb, _| wb.pending_chord_label()).is_some());
    keys("ctrl-t", cx);
    assert!(
        written(&wb, cx).is_empty(),
        "no key of the chord reaches the shell"
    );
    assert!(
        cx.update(|_, cx| wb.read(cx).theme_picker().is_some()),
        "the chord's command ran"
    );
}
