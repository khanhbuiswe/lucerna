//! A terminal's life (spec §6.1–§6.3, §6.6, §6.8, §8.2): the settings it
//! starts with, Restricted Mode, exit, bell, titles, quitting and the
//! panel restored with the window.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{BorrowAppContext, Entity, Focusable, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_terminal::{ProcessInfo, Terminal, TerminalEvent, TerminalOptions};
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, TabState, TerminalView, Workbench};
use tempfile::TempDir;

/// What the fake spawner saw: each spawn's options, and the sender of
/// each terminal's events, so a test can play the terminal's side.
#[derive(Default)]
struct Fakes {
    options: Vec<TerminalOptions>,
    events: Vec<UnboundedSender<TerminalEvent>>,
}

type Shared = Rc<RefCell<Fakes>>;

fn init(user: &str, cx: &mut TestAppContext) -> Shared {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
    let mut settings = SettingsStore::with_defaults();
    settings
        .set_layer(Layer::User, user)
        .expect("valid user settings");
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
    let fakes = Shared::default();
    let spawner_fakes = fakes.clone();
    cx.update(|cx| {
        lucerna_workbench::set_default_terminal_spawner(
            Rc::new(move |opts: TerminalOptions| {
                let (tx, rx) = unbounded();
                let mut fakes = spawner_fakes.borrow_mut();
                fakes.events.push(tx);
                fakes.options.push(opts.clone());
                Ok((Terminal::fake(opts.rows, opts.cols, opts.scrollback), rx))
            }),
            cx,
        )
    });
    fakes
}

/// The settings key suffix for this platform.
fn os() -> &'static str {
    if cfg!(target_os = "macos") {
        "osx"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

fn open<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let root = root.to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    settle(cx);
    (wb, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// Lets events, batch windows and polls run out, then draws.
fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(20));
    cx.run_until_parked();
    draw(cx);
    cx.run_until_parked();
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(wb.run_command(command, None, window, cx), "{command}");
        })
    });
    settle(cx);
}

fn new_terminal(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<TerminalView> {
    run(wb, "workbench.action.terminal.new", cx);
    active_view(wb, cx)
}

fn active_view(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<TerminalView> {
    cx.update(|_, cx| {
        let panel = wb.read(cx).terminal_panel().read(cx);
        let index = panel.active().expect("a tab");
        panel.tabs()[index].view.clone().expect("a terminal view")
    })
}

fn tab_count(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| wb.read(cx).terminal_panel().read(cx).tab_count())
}

fn tab_state(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> TabState {
    cx.update(|_, cx| wb.read(cx).terminal_panel().read(cx).tabs()[0].state)
}

fn tab_title(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        wb.read(cx).terminal_panel().read(cx).tabs()[0]
            .title
            .to_string()
    })
}

/// Sends `event` from the last terminal started, as its I/O thread would.
fn emit(fakes: &Shared, event: TerminalEvent, cx: &mut VisualTestContext) {
    let tx = fakes.borrow().events.last().expect("a terminal").clone();
    tx.unbounded_send(event).unwrap();
    settle(cx);
}

fn last_options(fakes: &Shared) -> TerminalOptions {
    fakes.borrow().options.last().expect("a spawn").clone()
}

fn set_user(json: &str, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        cx.update_global::<AppState, _>(|state, _| {
            state.settings.set_layer(Layer::User, json).unwrap();
        })
    });
    settle(cx);
}

fn screen(view: &Entity<TerminalView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, _| {
        let rows = v.grid_size().0 as usize;
        (0..rows).map(|r| v.painted_row_text(r)).collect()
    })
}

fn screen_contains(view: &Entity<TerminalView>, text: &str, cx: &mut VisualTestContext) -> bool {
    screen(view, cx).iter().any(|row| row.contains(text))
}

/// `<root>/.lucerna/settings.json`, which a window reads when it is made.
fn write_workspace_settings(root: &Path, text: &str) {
    std::fs::create_dir_all(root.join(".lucerna")).unwrap();
    std::fs::write(root.join(".lucerna").join("settings.json"), text).unwrap();
}

fn env_of(opts: &TerminalOptions, key: &str) -> Option<Option<String>> {
    opts.env.get(key).cloned()
}

#[gpui::test]
fn untrusted_ignores_workspace_shell_settings(cx: &mut TestAppContext) {
    let root = TempDir::new().unwrap();
    std::fs::create_dir(root.path().join("ws")).unwrap();
    let os = os();
    let user = format!(r#"{{ "terminal.integrated.env.{os}": {{ "FROM_USER": "1" }} }}"#);
    let workspace = format!(
        r#"{{
            "terminal.integrated.cwd": "ws",
            "terminal.integrated.env.{os}": {{ "FROM_WS": "1" }},
            "terminal.integrated.defaultProfile.{os}": "evil",
            "terminal.integrated.profiles.{os}": {{ "evil": {{ "path": "/tmp/evil" }} }}
        }}"#
    );
    let fakes = init(&user, cx);
    write_workspace_settings(root.path(), &workspace);
    {
        let (wb, cx) = open(root.path(), cx);
        assert!(wb.read_with(cx, |wb, cx| wb.is_restricted(cx)));
        new_terminal(&wb, cx);
        let opts = last_options(&fakes);
        assert_eq!(opts.cwd, root.path());
        assert_eq!(env_of(&opts, "FROM_USER"), Some(Some("1".into())));
        assert_eq!(env_of(&opts, "FROM_WS"), None);
        assert_ne!(opts.program, PathBuf::from("/tmp/evil"));
    }

    // A trusted window uses them.
    let trusted = format!(
        r#"{{ "session.trustAllWorktrees": true,
              "terminal.integrated.env.{os}": {{ "FROM_USER": "1" }} }}"#
    );
    cx.update_global::<AppState, _>(|state, _| {
        state.settings.set_layer(Layer::User, &trusted).unwrap();
    });
    let (wb, cx) = open(root.path(), cx);
    assert!(!wb.read_with(cx, |wb, cx| wb.is_restricted(cx)));
    new_terminal(&wb, cx);
    let opts = last_options(&fakes);
    assert_eq!(opts.cwd, root.path().join("ws"));
    assert_eq!(env_of(&opts, "FROM_USER"), Some(Some("1".into())));
    assert_eq!(env_of(&opts, "FROM_WS"), Some(Some("1".into())));
    assert_eq!(opts.program, PathBuf::from("/tmp/evil"));
}

#[gpui::test]
fn untrusted_opens_without_trust_modal(cx: &mut TestAppContext) {
    init("{}", cx);
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    assert!(wb.read_with(cx, |wb, cx| wb.is_restricted(cx)));
    cx.simulate_keystrokes("ctrl-`");
    settle(cx);
    assert!(wb.read_with(cx, |wb, _| wb.trust_modal().is_none()));
    assert_eq!(tab_count(&wb, cx), 1);
}

#[gpui::test]
fn zero_exit_closes_tab(cx: &mut TestAppContext) {
    let fakes = init("{}", cx);
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    let first = new_terminal(&wb, cx);
    new_terminal(&wb, cx);
    // The focused tab exits: the tab shown next takes the focus.
    emit(&fakes, TerminalEvent::Exited(Some(0)), cx);
    assert_eq!(tab_count(&wb, cx), 1);
    assert_eq!(active_view(&wb, cx), first);
    assert!(
        cx.update(|window, cx| first.read(cx).focus_handle(cx).is_focused(window)),
        "the next terminal has the focus"
    );

    let tx = fakes.borrow().events[0].clone();
    tx.unbounded_send(TerminalEvent::Exited(Some(0))).unwrap();
    settle(cx);
    assert_eq!(tab_count(&wb, cx), 0);
    // The last tab closed: the panel hides (`hideOnLastClosed`) and the
    // focus goes back to the editor area.
    assert!(!wb.read_with(cx, |wb, _| wb.panel_state().visible));
    let editor_focus = cx.update(|window, cx| {
        let keys = wb.read(cx).context_keys(window, cx);
        lucerna_settings::parse_when("editorFocus")
            .unwrap()
            .eval(&keys)
    });
    assert!(editor_focus, "the editor area has the focus");
}

#[gpui::test]
fn nonzero_exit_keeps_tab(cx: &mut TestAppContext) {
    let fakes = init("{}", cx);
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    let view = new_terminal(&wb, cx);
    view.read_with(cx, |v, _| v.terminal().feed(b"$ false"));
    emit(&fakes, TerminalEvent::Exited(Some(2)), cx);
    assert_eq!(tab_count(&wb, cx), 1);
    assert_eq!(tab_state(&wb, cx), TabState::Failed);
    let rows = screen(&view, cx);
    let last = rows
        .iter()
        .rposition(|r| !r.trim().is_empty())
        .expect("text on screen");
    assert!(
        rows[last].contains("[Process exited with code 2]"),
        "{rows:?}"
    );

    // The next key closes the tab and goes nowhere.
    cx.simulate_keystrokes("a");
    settle(cx);
    assert_eq!(tab_count(&wb, cx), 0);
    assert!(view.read_with(cx, |v, _| v.terminal().written().is_empty()));
}

#[gpui::test]
fn signal_exit_keeps_tab(cx: &mut TestAppContext) {
    let fakes = init("{}", cx);
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    let view = new_terminal(&wb, cx);
    emit(&fakes, TerminalEvent::Exited(None), cx);
    assert_eq!(tab_count(&wb, cx), 1);
    assert_eq!(tab_state(&wb, cx), TabState::Failed);
    assert!(screen_contains(&view, "[Process exited]", cx));
}

#[gpui::test]
fn cwd_setting_relative_to_root(cx: &mut TestAppContext) {
    let root = TempDir::new().unwrap();
    std::fs::create_dir(root.path().join("sub")).unwrap();
    let fakes = init(r#"{ "terminal.integrated.cwd": "sub" }"#, cx);
    let (wb, cx) = open(root.path(), cx);
    new_terminal(&wb, cx);
    assert_eq!(last_options(&fakes).cwd, root.path().join("sub"));

    // An absolute one is used as it is.
    let abs = root.path().join("sub");
    set_user(
        &serde_json::json!({ "terminal.integrated.cwd": abs }).to_string(),
        cx,
    );
    new_terminal(&wb, cx);
    assert_eq!(last_options(&fakes).cwd, abs);

    // A missing folder: home, and a warning in the terminal.
    set_user(r#"{ "terminal.integrated.cwd": "missing" }"#, cx);
    let view = new_terminal(&wb, cx);
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
    assert_eq!(last_options(&fakes).cwd, home);
    assert!(
        screen_contains(&view, "missing", cx),
        "{:?}",
        screen(&view, cx)
    );
}

#[gpui::test]
fn profile_selected(cx: &mut TestAppContext) {
    let os = os();
    let user = format!(
        r#"{{
            "terminal.integrated.defaultProfile.{os}": "fish",
            "terminal.integrated.profiles.{os}": {{
                "fish": {{ "path": "/opt/fish", "args": ["-i"], "env": {{ "FISHY": "1", "GONE": null }} }}
            }},
            "terminal.integrated.env.{os}": {{ "GONE": "x", "KEPT": "2", "DROPPED": null }}
        }}"#
    );
    let fakes = init(&user, cx);
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    new_terminal(&wb, cx);
    let opts = last_options(&fakes);
    assert_eq!(opts.program, PathBuf::from("/opt/fish"));
    assert_eq!(opts.args, vec!["-i".to_string()]);
    assert_eq!(env_of(&opts, "FISHY"), Some(Some("1".into())));
    assert_eq!(env_of(&opts, "KEPT"), Some(Some("2".into())));
    // `null` removes a variable, from either layer.
    assert_eq!(env_of(&opts, "GONE"), Some(None));
    assert_eq!(env_of(&opts, "DROPPED"), Some(None));
    assert_eq!(env_of(&opts, "TERM"), Some(Some("xterm-256color".into())));

    // A profile name with no entry falls back to the default shell.
    set_user(
        &format!(r#"{{ "terminal.integrated.defaultProfile.{os}": "nope" }}"#),
        cx,
    );
    new_terminal(&wb, cx);
    assert_ne!(last_options(&fakes).program, PathBuf::from("/opt/fish"));
}

#[gpui::test]
fn scrollback_setting_clamped(cx: &mut TestAppContext) {
    let fakes = init(r#"{ "terminal.integrated.scrollback": 500000 }"#, cx);
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    let view = new_terminal(&wb, cx);
    assert_eq!(last_options(&fakes).scrollback, 100_000);

    // A change applies to the open terminal.
    set_user(r#"{ "terminal.integrated.scrollback": 10 }"#, cx);
    view.read_with(cx, |v, _| v.terminal().feed("x\r\n".repeat(200).as_bytes()));
    let history = view.read_with(cx, |v, _| v.terminal().snapshot().history_len);
    assert_eq!(history, 10);
}

#[gpui::test]
fn font_setting_applies_live(cx: &mut TestAppContext) {
    let fakes = init(r#"{ "terminal.integrated.fontSize": 10 }"#, cx);
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    let view = new_terminal(&wb, cx);
    let height = |cx: &mut VisualTestContext| {
        view.read_with(cx, |v, _| v.cell_bounds(0, 0).expect("drawn").size.height)
    };
    let before = height(cx);
    set_user(r#"{ "terminal.integrated.fontSize": 20 }"#, cx);
    let after = height(cx);
    assert!(after > before, "{before:?} -> {after:?}");
    assert_eq!(fakes.borrow().options.len(), 1, "no respawn");
}

#[gpui::test]
fn cursor_style_setting(cx: &mut TestAppContext) {
    init(r#"{ "terminal.integrated.cursorStyle": "line" }"#, cx);
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    let view = new_terminal(&wb, cx);
    let cursor = |cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.painted_cursor());
    assert_eq!(cursor(cx), Some("terminal-cursor-bar"));
    // An app still picks its own (DECSCUSR 4), and resets to the setting.
    view.read_with(cx, |v, _| v.terminal().feed(b"\x1b[4 q"));
    settle(cx);
    assert_eq!(cursor(cx), Some("terminal-cursor-underline"));
    view.read_with(cx, |v, _| v.terminal().feed(b"\x1b[0 q"));
    settle(cx);
    assert_eq!(cursor(cx), Some("terminal-cursor-bar"));
    // Live, and the unfocused shape follows `cursorStyleInactive`.
    set_user(
        r#"{ "terminal.integrated.cursorStyle": "underline",
             "terminal.integrated.cursorStyleInactive": "none" }"#,
        cx,
    );
    assert_eq!(cursor(cx), Some("terminal-cursor-underline"));
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.focus_editor_area(window, cx)));
    settle(cx);
    assert_eq!(cursor(cx), None);
    set_user(
        r#"{ "terminal.integrated.cursorStyleInactive": "block" }"#,
        cx,
    );
    assert_eq!(cursor(cx), Some("terminal-cursor-block"));
    set_user("{}", cx);
    assert_eq!(cursor(cx), Some("terminal-cursor-outline"));
}

#[gpui::test]
fn bell_marks_tab_until_key(cx: &mut TestAppContext) {
    let fakes = init("{}", cx);
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    new_terminal(&wb, cx);
    emit(&fakes, TerminalEvent::Bell, cx);
    assert_eq!(tab_state(&wb, cx), TabState::Normal, "enableBell is off");

    set_user(r#"{ "terminal.integrated.enableBell": true }"#, cx);
    emit(&fakes, TerminalEvent::Bell, cx);
    assert_eq!(tab_state(&wb, cx), TabState::Bell);
    cx.simulate_keystrokes("a");
    settle(cx);
    assert_eq!(tab_state(&wb, cx), TabState::Normal);
}

#[gpui::test]
fn title_from_osc_then_process(cx: &mut TestAppContext) {
    let fakes = init("{}", cx);
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    new_terminal(&wb, cx);
    emit(&fakes, TerminalEvent::Title(Some("✳ Fix bug".into())), cx);
    emit(&fakes, TerminalEvent::Wakeup, cx);
    cx.executor().advance_clock(Duration::from_millis(600));
    settle(cx);
    assert_eq!(tab_title(&wb, cx), "✳ Fix bug");

    // The poll sees the process that set it: the title stays. Another
    // process in the foreground: the old title goes, its name shows.
    let view = active_view(&wb, cx);
    let process = |pid: u32, name: &str| ProcessInfo {
        pid,
        name: name.into(),
        argv: vec![name.into()],
        cwd: None,
    };
    view.update(cx, |v, cx| {
        v.set_foreground_process(Some(process(10, "claude")), cx)
    });
    settle(cx);
    assert_eq!(tab_title(&wb, cx), "✳ Fix bug");
    view.update(cx, |v, cx| {
        v.set_foreground_process(Some(process(20, "vim")), cx)
    });
    settle(cx);
    assert_eq!(tab_title(&wb, cx), "vim");
    // The same title again, from the new process, names the tab again.
    emit(&fakes, TerminalEvent::Title(Some("✳ Fix bug".into())), cx);
    assert_eq!(tab_title(&wb, cx), "✳ Fix bug");

    emit(&fakes, TerminalEvent::Title(None), cx);
    assert_eq!(tab_title(&wb, cx), "vim");
    view.update(cx, |v, cx| v.set_foreground_process(None, cx));
    settle(cx);
    assert_eq!(tab_title(&wb, cx), "Terminal");
}

fn confirm_quit(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> gpui::Task<bool> {
    let quit = cx.update(|window, cx| wb.update(cx, |wb, cx| wb.confirm_quit(window, cx)));
    cx.run_until_parked();
    quit
}

fn quit_answer(quit: gpui::Task<bool>, cx: &mut VisualTestContext) -> bool {
    use futures::FutureExt as _;
    settle(cx);
    quit.now_or_never().expect("the quit prompt was answered")
}

#[gpui::test]
fn confirm_on_exit_never_by_default(cx: &mut TestAppContext) {
    init("{}", cx);
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    let view = new_terminal(&wb, cx);
    let quit = confirm_quit(&wb, cx);
    assert!(!cx.has_pending_prompt());
    assert!(quit_answer(quit, cx));

    // `hasChildProcesses`: only while something else than the shell runs.
    set_user(
        r#"{ "terminal.integrated.confirmOnExit": "hasChildProcesses" }"#,
        cx,
    );
    let quit = confirm_quit(&wb, cx);
    assert!(!cx.has_pending_prompt());
    assert!(quit_answer(quit, cx));
    let claude = ProcessInfo {
        pid: 4242,
        name: "claude".into(),
        argv: vec!["claude".into()],
        cwd: None,
    };
    view.update(cx, |v, cx| v.set_foreground_process(Some(claude), cx));
    let quit = confirm_quit(&wb, cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    assert!(!quit_answer(quit, cx));

    // `always`: whenever a terminal is open.
    view.update(cx, |v, cx| v.set_foreground_process(None, cx));
    set_user(r#"{ "terminal.integrated.confirmOnExit": "always" }"#, cx);
    let quit = confirm_quit(&wb, cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Terminate");
    assert!(quit_answer(quit, cx));
}

#[gpui::test]
fn panel_reopens_with_fresh_terminal(cx: &mut TestAppContext) {
    let fakes = init("{}", cx);
    let (root, data) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    fn open_saved<'a>(
        root: &Path,
        data: &Path,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        let (root, data) = (root.to_path_buf(), data.to_path_buf());
        cx.add_window_view(|window, cx| Workbench::with_data_dir(root, data, window, cx))
    }
    {
        let (wb, cx) = open_saved(root.path(), data.path(), cx);
        run(&wb, "workbench.action.togglePanel", cx);
        wb.update(cx, |wb, cx| wb.save_workspace_state(cx));
    }
    let spawned = fakes.borrow().options.len();
    let (wb, cx) = open_saved(root.path(), data.path(), cx);
    settle(cx);
    assert!(wb.read_with(cx, |wb, _| wb.panel_state().visible));
    assert_eq!(tab_count(&wb, cx), 1);
    assert_eq!(fakes.borrow().options.len(), spawned + 1, "a new terminal");
}

fn view_focused(view: &Entity<TerminalView>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| view.read(cx).focus_handle(cx).is_focused(window))
}

fn assert_types_into(view: &Entity<TerminalView>, what: &str, cx: &mut VisualTestContext) {
    assert!(view_focused(view, cx), "{what}: the terminal has the focus");
    assert!(
        view.read_with(cx, |v, _| v.input_handler_installed()),
        "{what}: the input handler (IME) is installed"
    );
}

fn click_at(name: &'static str, cx: &mut VisualTestContext) {
    let at = cx
        .debug_bounds(name)
        .unwrap_or_else(|| panic!("no element {name}"))
        .center();
    cx.simulate_click(at, gpui::Modifiers::none());
    settle(cx);
}

#[gpui::test]
fn clicks_in_the_tab_bar_keep_the_terminal_focused(cx: &mut TestAppContext) {
    init("{}", cx);
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    let first = new_terminal(&wb, cx);
    let second = new_terminal(&wb, cx);
    assert_types_into(&second, "a new terminal", cx);

    // Clicking a tab focuses its terminal, not the panel frame.
    click_at("terminal-tab-0", cx);
    assert_types_into(&first, "after a tab click", cx);
    click_at("terminal-tab-1", cx);
    assert_types_into(&second, "after another tab click", cx);

    // × on a background tab: the shown terminal keeps the focus.
    new_terminal(&wb, cx);
    click_at("terminal-tab-1", cx);
    let at = cx.debug_bounds("terminal-tab-2").unwrap().center();
    cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
    settle(cx);
    click_at("terminal-tab-close-2", cx);
    assert_eq!(tab_count(&wb, cx), 2);
    assert_types_into(&second, "after × on a background tab", cx);

    // × on the shown tab: the tab shown next takes the focus.
    click_at("terminal-tab-close-1", cx);
    assert_eq!(tab_count(&wb, cx), 1);
    assert_types_into(&first, "after × on the shown tab", cx);

    // Maximize, then restore.
    click_at("terminal-maximize", cx);
    assert!(wb.read_with(cx, |wb, _| wb.panel_state().maximized));
    assert_types_into(&first, "after maximize", cx);
    click_at("terminal-maximize", cx);
    assert!(!wb.read_with(cx, |wb, _| wb.panel_state().maximized));
    assert_types_into(&first, "after restore", cx);
}

#[gpui::test]
fn middle_click_close_keeps_the_terminal_focused(cx: &mut TestAppContext) {
    init("{}", cx);
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    let first = new_terminal(&wb, cx);
    new_terminal(&wb, cx);
    let at = cx.debug_bounds("terminal-tab-1").unwrap().center();
    cx.simulate_mouse_down(at, gpui::MouseButton::Middle, gpui::Modifiers::none());
    cx.simulate_mouse_up(at, gpui::MouseButton::Middle, gpui::Modifiers::none());
    settle(cx);
    assert_eq!(tab_count(&wb, cx), 1);
    assert_types_into(&first, "after a middle-click close", cx);
}

/// The reply to an OSC 11 query for `background`, as xterm words it.
fn osc11_reply(background: lucerna_theme::Rgba) -> String {
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    let (r, g, b) = (byte(background.r), byte(background.g), byte(background.b));
    format!("\x1b]11;rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}\x07")
}

#[gpui::test]
fn background_query_answers_with_the_theme(cx: &mut TestAppContext) {
    init("{}", cx);
    let light = cx.update(|cx| {
        let state = cx.global::<AppState>();
        state.themes.get_or_default("Lucerna Light")
    });
    cx.update_global::<AppState, _>(|state, _| state.set_theme(light.clone()));
    let (wb, cx) = open(Path::new("/tmp/project"), cx);
    let view = new_terminal(&wb, cx);
    view.read_with(cx, |v, _| v.terminal().feed(b"\x1b]11;?\x07"));
    let written = view.read_with(cx, |v, _| v.terminal().written());
    let light_reply = osc11_reply(light.terminal.background);
    assert_eq!(String::from_utf8(written).unwrap(), light_reply);

    // A theme change answers the next query with the new background.
    let dark = cx.update(|_, cx| {
        cx.global::<AppState>()
            .themes
            .get_or_default("Lucerna Dark")
    });
    cx.update(|_, cx| cx.update_global::<AppState, _>(|state, _| state.set_theme(dark.clone())));
    settle(cx);
    view.read_with(cx, |v, _| v.terminal().feed(b"\x1b]11;?\x07"));
    let written = view.read_with(cx, |v, _| v.terminal().written());
    assert_eq!(
        String::from_utf8(written).unwrap(),
        light_reply + &osc11_reply(dark.terminal.background)
    );
}

#[gpui::test]
fn restored_terminal_in_a_trusted_folder_uses_workspace_settings(cx: &mut TestAppContext) {
    let root = TempDir::new().unwrap();
    let data = TempDir::new().unwrap();
    std::fs::create_dir(root.path().join("ws")).unwrap();
    let fakes = init("{}", cx);
    write_workspace_settings(root.path(), r#"{ "terminal.integrated.cwd": "ws" }"#);
    let mut store = lucerna_workbench::TrustStore::load(data.path());
    store.trust(root.path(), false).unwrap();
    let state: lucerna_workbench::WorkspaceState = serde_json::from_value(serde_json::json!({
        "version": 1,
        "sidebar_visible": true,
        "sidebar_width": 260.0,
        "sidebar_view": "explorer",
        "groups": { "groups": [], "active_group": 0 },
        "scroll": {},
        "panel": { "visible": true, "position": "bottom", "bottom_height": 320.0,
                   "right_width": 640.0, "maximized": true },
    }))
    .unwrap();
    let state_file = lucerna_workbench::state_path(data.path(), root.path());
    lucerna_workbench::save_state(&state_file, &state).unwrap();

    let (r, d) = (root.path().to_path_buf(), data.path().to_path_buf());
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::with_data_dir(r, d, window, cx));
    settle(cx);
    assert!(!wb.read_with(cx, |wb, cx| wb.is_restricted(cx)));
    assert_eq!(tab_count(&wb, cx), 1, "the restored panel has one terminal");
    assert_eq!(fakes.borrow().options.len(), 1);
    assert_eq!(last_options(&fakes).cwd, root.path().join("ws"));
    // Restored maximized, the panel's focus went to the terminal once it
    // started.
    let view = active_view(&wb, cx);
    assert_types_into(&view, "the restored terminal", cx);
}
