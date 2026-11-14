//! Links in the terminal (spec §6.7): ⌘ (ctrl off macOS) + hover underlines
//! a link, ⌘+click opens files in the editor, folders in the Explorer and
//! URLs in the browser.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Entity, Focusable, InputEvent, Modifiers, ModifiersChangedEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, TestAppContext, VisualTestContext, point, px,
};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point as TextPoint;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, TerminalView, Workbench};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext, user_settings: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
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

/// A project with `src/lib.rs` (five lines), `src/nested/`, `a.txt` and
/// `sub/a.txt`.
fn project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src/nested")).unwrap();
    std::fs::write(root.join("src/nested/x.rs"), "\n").unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n",
    )
    .unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), "root\n").unwrap();
    std::fs::write(root.join("sub/a.txt"), "sub\n").unwrap();
    dir
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

/// A workbench on `root` (scanned) with the terminal shown and focused.
fn with_terminal<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (
    Entity<Workbench>,
    &'a mut VisualTestContext,
    Entity<TerminalView>,
) {
    let root = root.to_path_buf();
    let (wb, cx) = cx.add_window_view(move |window, cx| Workbench::new(root, window, cx));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !wb.read_with(cx, |wb, _| wb.worktree().snapshot().scan_complete()) {
        assert!(Instant::now() < deadline, "worktree scan did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::ScanComplete, cx);
    });
    draw(cx);
    cx.simulate_keystrokes("ctrl-`");
    draw(cx);
    let view = cx.update(|_, cx| {
        let panel = wb.read(cx).terminal_panel().read(cx);
        let index = panel.active().expect("a terminal tab");
        panel.tabs()[index]
            .view
            .clone()
            .expect("a running terminal")
    });
    // Tests run in parallel on a debug build: 1 ms is too tight to be sure.
    view.update(cx, |v, _| {
        v.set_link_scan_budget(Duration::from_millis(200))
    });
    (wb, cx, view)
}

fn feed(view: &Entity<TerminalView>, text: &str, cx: &mut VisualTestContext) {
    view.read_with(cx, |v, _| v.terminal().feed(text.as_bytes()));
    draw(cx);
}

/// The middle of screen cell `row`/`col`.
fn at(
    view: &Entity<TerminalView>,
    row: u16,
    col: u16,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    view.read_with(cx, |v, _| v.cell_bounds(row, col))
        .expect("the terminal was laid out")
        .center()
}

/// The link modifier: ⌘ on macOS, ctrl elsewhere.
fn link_mods() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::command()
    } else {
        Modifiers::control()
    }
}

fn hover(pos: Point<Pixels>, mods: Modifiers, cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(pos, None, mods);
    draw(cx);
}

fn click(pos: Point<Pixels>, mods: Modifiers, cx: &mut VisualTestContext) {
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: pos,
        modifiers: mods,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: pos,
        modifiers: mods,
        click_count: 1,
    });
    draw(cx);
}

fn underlined(cx: &mut VisualTestContext) -> bool {
    cx.debug_bounds("terminal-link-hover").is_some()
}

fn active_tab(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<PathBuf> {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        Some(groups.model().active_tab()?.path()?.to_path_buf())
    })
}

/// The active editor's cursor once it reached `expected` (the file loads
/// on a background thread), or after a few seconds.
fn cursor(wb: &Entity<Workbench>, expected: TextPoint, cx: &mut VisualTestContext) -> TextPoint {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        draw(cx);
        let view = cx.update(|_, cx| {
            wb.read(cx)
                .editor_groups()
                .read(cx)
                .active_body()
                .and_then(|b| b.downcast::<CodeView>().ok())
                .expect("a code view")
        });
        let at = view.read_with(cx, |v, _| v.cursor());
        if at == expected || Instant::now() > deadline {
            return at;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[gpui::test]
fn hover_needs_cmd(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (_wb, cx, view) = with_terminal(dir.path(), cx);
    feed(&view, "see https://example.com/x now", cx);
    let pos = at(&view, 0, 10, cx);

    hover(pos, Modifiers::none(), cx);
    assert!(!underlined(cx), "no underline without the link modifier");
    assert!(cx.debug_bounds("terminal-link-tooltip").is_none());

    hover(pos + point(px(1.), px(0.)), link_mods(), cx);
    assert!(underlined(cx), "the URL is underlined while ⌘ is held");
    assert!(cx.debug_bounds("terminal-link-tooltip").is_some());

    // Releasing the modifier clears it.
    cx.simulate_modifiers_change(Modifiers::none());
    draw(cx);
    assert!(!underlined(cx));
    assert!(cx.debug_bounds("terminal-link-tooltip").is_none());
}

#[gpui::test]
fn cmd_click_opens_file_at_line(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (wb, cx, view) = with_terminal(dir.path(), cx);
    feed(&view, "error at src/lib.rs:3:2 here", cx);
    let pos = at(&view, 0, 12, cx);

    hover(pos, link_mods(), cx);
    assert!(underlined(cx), "an existing file is a link once resolved");

    click(pos, link_mods(), cx);
    let tab = active_tab(&wb, cx).expect("a tab opened");
    assert_eq!(tab.file_name().unwrap(), "lib.rs");
    let expected = TextPoint { row: 2, column: 1 };
    assert_eq!(cursor(&wb, expected, cx), expected);
    assert!(
        view.read_with(cx, |v, _| v.terminal().selection_text())
            .is_none(),
        "⌘+click on a link does not select"
    );
}

#[gpui::test]
fn cmd_click_without_hover_opens_file(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (wb, cx, view) = with_terminal(dir.path(), cx);
    feed(&view, "src/lib.rs:2", cx);
    click(at(&view, 0, 3, cx), link_mods(), cx);
    let tab = active_tab(&wb, cx).expect("a tab opened");
    assert_eq!(tab.file_name().unwrap(), "lib.rs");
    let expected = TextPoint { row: 1, column: 0 };
    assert_eq!(cursor(&wb, expected, cx), expected);
}

#[gpui::test]
fn relative_to_terminal_cwd_first(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (wb, cx, view) = with_terminal(dir.path(), cx);
    let sub = dir.path().join("sub");
    view.update(cx, |v, _| v.set_cwd_now(Some(sub.clone())));
    feed(&view, "cat a.txt", cx);
    click(at(&view, 0, 5, cx), link_mods(), cx);
    assert_eq!(active_tab(&wb, cx), Some(sub.join("a.txt")));
}

#[gpui::test]
fn missing_file_is_not_a_link(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (wb, cx, view) = with_terminal(dir.path(), cx);
    feed(&view, "see nope/missing.rs:4 and e.g. this", cx);
    let pos = at(&view, 0, 8, cx);
    hover(pos, link_mods(), cx);
    assert!(!underlined(cx), "a path that does not exist is not a link");
    click(pos, link_mods(), cx);
    assert_eq!(active_tab(&wb, cx), None);

    // `e.g.` matches the loose path pattern but is no file either.
    let pos = at(&view, 0, 27, cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    hover(pos, link_mods(), cx);
    assert!(!underlined(cx));
}

#[gpui::test]
fn url_opens_only_allowed_schemes(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (_wb, cx, view) = with_terminal(dir.path(), cx);
    feed(&view, "go https://example.com/a now", cx);
    click(at(&view, 0, 6, cx), link_mods(), cx);
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/a"));

    // An OSC 8 link with a scheme outside `allowedLinkSchemes`.
    feed(&view, "\r\n\x1b]8;;ftp://x\x1b\\ftplink\x1b]8;;\x1b\\", cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    hover(at(&view, 1, 2, cx), link_mods(), cx);
    assert!(!underlined(cx), "a scheme that will not open is no link");
    click(at(&view, 1, 2, cx), link_mods(), cx);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.com/a"),
        "ftp:// does not open"
    );
}

#[gpui::test]
fn osc8_needs_cmd_click(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (_wb, cx, view) = with_terminal(dir.path(), cx);
    feed(
        &view,
        "\x1b]8;;https://example.com/osc\x1b\\click me\x1b]8;;\x1b\\",
        cx,
    );
    let pos = at(&view, 0, 2, cx);
    click(pos, Modifiers::none(), cx);
    assert_eq!(cx.opened_url(), None, "a plain click opens nothing");
    click(pos, link_mods(), cx);
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/osc"));
}

#[gpui::test]
fn file_url_opens_in_editor(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (wb, cx, view) = with_terminal(dir.path(), cx);
    let file = dir.path().join("a.txt");
    feed(&view, &format!("file://{}", file.display()), cx);
    click(at(&view, 0, 3, cx), link_mods(), cx);
    assert_eq!(active_tab(&wb, cx), Some(file));
    assert_eq!(cx.opened_url(), None, "file:// opens in the editor");
}

#[gpui::test]
fn dir_in_workspace_reveals_in_explorer(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (wb, cx, view) = with_terminal(dir.path(), cx);
    let visible = |wb: &Entity<Workbench>, cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            wb.read(cx)
                .explorer()
                .read(cx)
                .model()
                .visible_rows()
                .iter()
                .map(|r| r.path.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
    };
    assert!(!visible(&wb, cx).contains(&"src/nested".to_string()));
    feed(&view, "ls src/nested/", cx);
    click(at(&view, 0, 6, cx), link_mods(), cx);
    assert!(
        visible(&wb, cx).contains(&"src/nested".to_string()),
        "the folder's parents are expanded: {:?}",
        visible(&wb, cx)
    );
    assert!(wb.read_with(cx, |wb, _| wb.sidebar_visible()));
    assert_eq!(active_tab(&wb, cx), None, "a folder opens no tab");
}

#[gpui::test]
fn enable_file_links_off(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "terminal.integrated.enableFileLinks": "off" }"#),
    );
    let dir = project();
    let (wb, cx, view) = with_terminal(dir.path(), cx);
    feed(&view, "src/lib.rs:3 https://example.com/b", cx);
    let path_pos = at(&view, 0, 3, cx);
    hover(path_pos, link_mods(), cx);
    assert!(!underlined(cx), "no path links");
    click(path_pos, link_mods(), cx);
    assert_eq!(active_tab(&wb, cx), None);

    click(at(&view, 0, 20, cx), link_mods(), cx);
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/b"));
}

#[gpui::test]
fn hover_throttled(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (_wb, cx, view) = with_terminal(dir.path(), cx);
    feed(&view, "https://example.com/a-rather-long-path-here", cx);
    let scans = |view: &Entity<TerminalView>, cx: &mut VisualTestContext| {
        view.read_with(cx, |v, _| v.link_scans())
    };
    let before = scans(&view, cx);
    // 20 moves, far enough apart to beat the 5 px rule, in no time at all.
    for i in 0..20u16 {
        let pos = at(&view, 0, 2 + (i % 2) * 20, cx);
        cx.simulate_mouse_move(pos, None, link_mods());
    }
    draw(cx);
    assert!(
        scans(&view, cx) - before <= 1,
        "{} scans",
        scans(&view, cx) - before
    );
    // The last position is still scanned once the 100 ms are over.
    cx.executor().advance_clock(Duration::from_millis(120));
    draw(cx);
    assert!(scans(&view, cx) - before <= 2);
    assert!(underlined(cx));
}

#[gpui::test]
fn column_counts_characters_on_utf8_lines(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    // `ê` is two bytes: character 7 (`=`) starts at byte 7, not 6.
    std::fs::write(dir.path().join("src/u.rs"), "let ê = 1;\n").unwrap();
    let (wb, cx, view) = with_terminal(dir.path(), cx);
    feed(&view, "src/u.rs:1:7\r\nsrc/u.rs:1:9", cx);

    // Not loaded yet: the jump waits for the text.
    click(at(&view, 0, 2, cx), link_mods(), cx);
    let expected = TextPoint { row: 0, column: 7 };
    assert_eq!(cursor(&wb, expected, cx), expected);

    // Loaded: the jump applies at once.
    click(at(&view, 1, 2, cx), link_mods(), cx);
    let expected = TextPoint { row: 0, column: 9 };
    assert_eq!(cursor(&wb, expected, cx), expected);
}

#[gpui::test]
fn releasing_cmd_clears_link_when_unfocused(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (wb, cx, view) = with_terminal(dir.path(), cx);
    feed(&view, "https://example.com/a-rather-long-path-here", cx);
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.focus_editor_area(window, cx)));
    draw(cx);
    let focused = cx.update(|window, cx| view.read(cx).focus_handle(cx).is_focused(window));
    assert!(!focused, "the terminal is not focused");

    hover(at(&view, 0, 2, cx), link_mods(), cx);
    assert!(underlined(cx));
    // A second move inside the throttle leaves a trailing scan pending.
    let far = at(&view, 0, 30, cx);
    cx.simulate_mouse_move(far, None, link_mods());
    let scans = view.read_with(cx, |v, _| v.link_scans());

    cx.simulate_modifiers_change(Modifiers::none());
    draw(cx);
    assert!(!underlined(cx), "releasing ⌘ clears the link");
    cx.executor().advance_clock(Duration::from_millis(200));
    draw(cx);
    assert!(!underlined(cx));
    assert_eq!(
        view.read_with(cx, |v, _| v.link_scans()),
        scans,
        "the pending trailing scan was cancelled"
    );
}

#[gpui::test]
fn click_survives_a_plain_move(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (wb, cx, view) = with_terminal(dir.path(), cx);
    feed(&view, "src/lib.rs:2", cx);
    let pos = at(&view, 0, 3, cx);
    // ⌘+click starts a resolve; the mouse moves on without ⌘ before it
    // ends (all in one turn, so no background work runs in between).
    let away = pos + point(px(40.), px(0.));
    cx.update(|window, cx| {
        let mods = link_mods();
        let events = [
            MouseDownEvent {
                button: MouseButton::Left,
                position: pos,
                modifiers: mods,
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            MouseUpEvent {
                button: MouseButton::Left,
                position: pos,
                modifiers: mods,
                click_count: 1,
            }
            .to_platform_input(),
            MouseMoveEvent {
                position: away,
                modifiers: Modifiers::none(),
                pressed_button: None,
            }
            .to_platform_input(),
            ModifiersChangedEvent {
                modifiers: Modifiers::none(),
                capslock: Default::default(),
            }
            .to_platform_input(),
        ];
        for event in events {
            window.dispatch_event(event, cx);
        }
    });
    draw(cx);
    let tab = active_tab(&wb, cx).expect("the clicked file opened");
    assert_eq!(tab.file_name().unwrap(), "lib.rs");
}
