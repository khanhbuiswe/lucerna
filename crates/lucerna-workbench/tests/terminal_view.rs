//! A terminal tab's view: spawning, drawing the grid, following the
//! panel's size, batching wakeups, theme colours, font and cursor.

use std::cell::{Cell, RefCell};
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    BorrowAppContext, Bounds, Entity, Modifiers, MouseButton, Pixels, TestAppContext,
    VisualTestContext, point, px,
};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_terminal::{Terminal, TerminalOptions};
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, TerminalSpawner, TerminalView, Workbench, terminal_font};

fn init(cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
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
}

type Captured = Rc<RefCell<Vec<TerminalOptions>>>;

/// Records each spawn's options and starts a fake terminal of that size.
fn fake_spawner(captured: Captured) -> TerminalSpawner {
    Rc::new(move |opts: TerminalOptions| {
        captured.borrow_mut().push(opts.clone());
        Ok(Terminal::fake_with_events(
            opts.rows,
            opts.cols,
            opts.scrollback,
        ))
    })
}

fn open(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut VisualTestContext, Captured) {
    let (wb, cx) =
        cx.add_window_view(|window, cx| Workbench::new(PathBuf::from("/tmp/project"), window, cx));
    let captured = Captured::default();
    let spawner = fake_spawner(captured.clone());
    wb.update(cx, |wb, _| wb.set_terminal_spawner(spawner));
    (wb, cx, captured)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn bounds(name: &'static str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds(name)
        .unwrap_or_else(|| panic!("no element {name}"))
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(wb.run_command(command, None, window, cx), "{command}");
        })
    });
    draw(cx);
}

/// The shown tab's terminal view.
fn active_view(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<TerminalView> {
    cx.update(|_, cx| {
        let panel = wb.read(cx).terminal_panel().read(cx);
        let index = panel.active().expect("a tab");
        panel.tabs()[index].view.clone().expect("a terminal view")
    })
}

/// Opens the panel with one terminal and draws it.
fn open_terminal(
    cx: &mut TestAppContext,
) -> (
    Entity<Workbench>,
    &mut VisualTestContext,
    Entity<TerminalView>,
) {
    let (wb, cx, _) = open(cx);
    run(&wb, "workbench.action.terminal.new", cx);
    let view = active_view(&wb, cx);
    settle(cx);
    (wb, cx, view)
}

/// Lets pending events and batch windows run out, then draws.
fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(20));
    cx.run_until_parked();
    draw(cx);
}

fn feed(view: &Entity<TerminalView>, bytes: &[u8], cx: &mut VisualTestContext) {
    view.read_with(cx, |v, _| v.terminal().feed(bytes));
}

fn row_text(view: &Entity<TerminalView>, row: usize, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, _| v.painted_row_text(row))
}

fn cursor(view: &Entity<TerminalView>, cx: &mut VisualTestContext) -> Option<&'static str> {
    view.read_with(cx, |v, _| v.painted_cursor())
}

fn set_user_settings(json: &str, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        cx.update_global::<AppState, _>(|state, _| {
            state.settings.set_layer(Layer::User, json).unwrap();
        })
    });
}

#[gpui::test]
fn new_terminal_spawns_with_root_cwd_and_login_shell(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx, captured) = open(cx);
    run(&wb, "workbench.action.terminal.new", cx);
    let opts = captured.borrow()[0].clone();
    assert_eq!(opts.cwd, PathBuf::from("/tmp/project"));
    if let Some(shell) = std::env::var_os("SHELL").filter(|s| !s.is_empty()) {
        assert_eq!(opts.program, PathBuf::from(shell));
    }
    if cfg!(target_os = "macos") {
        assert!(opts.args.contains(&"-l".to_string()), "{:?}", opts.args);
    }
    assert_eq!(
        opts.env.get("TERM"),
        Some(&Some("xterm-256color".to_string()))
    );
    assert_eq!(
        opts.env.get("TERM_PROGRAM"),
        Some(&Some("lucerna".to_string()))
    );
    assert_eq!(opts.scrollback, 10_000);
    assert!(opts.rows >= 1 && opts.cols >= 1);
    // The tab holds the view, and the panel shows it.
    active_view(&wb, cx);
    assert!(wb.read_with(cx, |wb, _| wb.panel_state().visible));
}

#[gpui::test]
fn renders_fed_text(cx: &mut TestAppContext) {
    init(cx);
    let (_wb, cx, view) = open_terminal(cx);
    feed(&view, b"hello", cx);
    settle(cx);
    assert!(
        row_text(&view, 0, cx).contains("hello"),
        "{:?}",
        row_text(&view, 0, cx)
    );
    assert!(cx.debug_bounds("terminal-view").is_some());
}

#[gpui::test]
fn plain_wide_and_combining_text_keep_their_columns(cx: &mut TestAppContext) {
    // Plain runs are drawn from looked-up glyphs, the rest shaped: both
    // land on their own cells.
    init(cx);
    let (_wb, cx, view) = open_terminal(cx);
    feed(
        &view,
        "ab \x1b[1m日本\x1b[0m e\u{301}x \x1b[4mok\x1b[0m".as_bytes(),
        cx,
    );
    settle(cx);
    assert_eq!(row_text(&view, 0, cx).trim_end(), "ab 日本 e\u{301}x ok");
}

#[gpui::test]
fn title_names_the_tab(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx, view) = open_terminal(cx);
    feed(&view, b"\x1b]0;claude\x07", cx);
    settle(cx);
    let title = cx.update(|_, cx| {
        wb.read(cx).terminal_panel().read(cx).tabs()[0]
            .title
            .clone()
    });
    assert_eq!(title.as_ref(), "claude");
}

#[gpui::test]
fn resize_follows_panel(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx, view) = open_terminal(cx);
    assert_eq!(
        wb.read_with(cx, |wb, _| wb.panel_state().bottom_height),
        320.
    );
    let before = view.read_with(cx, |v, _| v.grid_size());

    // Drag the sash up 160 px: 320 → 480.
    let sash = bounds("terminal-panel-sash", cx).center();
    let p = bounds("terminal-panel", cx);
    let none = Modifiers::none();
    cx.simulate_mouse_down(sash, MouseButton::Left, none);
    cx.simulate_mouse_move(sash - point(px(0.), px(10.)), MouseButton::Left, none);
    let to = point(sash.x, p.bottom() - px(480.));
    cx.simulate_mouse_move(to, MouseButton::Left, none);
    cx.simulate_mouse_up(to, MouseButton::Left, none);
    draw(cx);
    assert_eq!(
        wb.read_with(cx, |wb, _| wb.panel_state().bottom_height),
        480.
    );

    let after = view.read_with(cx, |v, _| v.grid_size());
    let size = cx.update(|_, cx| terminal_font(wb.read(cx).settings().read(cx).view(cx), cx).size);
    let expected = (160. / (f32::from(size) * 1.2)).floor() as i32;
    let grew = after.0 as i32 - before.0 as i32;
    assert!(
        (grew - expected).abs() <= 1,
        "grew {grew}, expected {expected}"
    );
    assert_eq!(after.1, before.1, "the width did not change");
    let snap = view.read_with(cx, |v, _| v.terminal().snapshot());
    assert_eq!((snap.rows, snap.cols), after);
}

#[gpui::test]
fn zero_bounds_do_not_resize_to_zero(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx, view) = open_terminal(cx);
    let check = |cx: &mut VisualTestContext| {
        let (rows, cols) = view.read_with(cx, |v, _| v.grid_size());
        assert!(rows >= 1 && cols >= 1, "{rows}×{cols}");
        let snap = view.read_with(cx, |v, _| v.terminal().snapshot());
        assert!(snap.rows >= 1 && snap.cols >= 1);
    };
    check(cx);
    run(&wb, "workbench.action.togglePanel", cx);
    check(cx);
    run(&wb, "workbench.action.togglePanel", cx);
    check(cx);
    // A tiny window leaves the panel almost no room.
    cx.simulate_resize(gpui::size(px(60.), px(60.)));
    draw(cx);
    check(cx);
}

/// Counts the view's notifications, each of which asks for a frame.
fn count_notifies(
    view: &Entity<TerminalView>,
    cx: &mut VisualTestContext,
) -> (Rc<Cell<usize>>, gpui::Subscription) {
    let count = Rc::new(Cell::new(0));
    let sub = cx.update(|_, cx| {
        let count = count.clone();
        cx.observe(view, move |_, _| count.set(count.get() + 1))
    });
    (count, sub)
}

fn frames(view: &Entity<TerminalView>, cx: &mut VisualTestContext) -> u64 {
    view.read_with(cx, |v, _| v.frames_drawn())
}

#[gpui::test]
fn wakeup_triggers_one_redraw(cx: &mut TestAppContext) {
    init(cx);
    let (_wb, cx, view) = open_terminal(cx);
    // A test window draws itself whenever a view asks for a frame, so
    // `frames_drawn` counts the frames the terminal caused.
    let start = frames(&view, cx);
    feed(&view, b"a", cx);
    cx.run_until_parked();
    assert_eq!(
        frames(&view, cx),
        start + 1,
        "the first wakeup draws at once"
    );

    // Its 4 ms window is open: ten more wakeups make one frame at its end.
    let before = frames(&view, cx);
    for i in 0..10 {
        feed(&view, format!("{i}").as_bytes(), cx);
    }
    cx.run_until_parked();
    assert_eq!(frames(&view, cx), before, "nothing until the window ends");
    cx.executor().advance_clock(Duration::from_millis(4));
    cx.run_until_parked();
    assert_eq!(frames(&view, cx), before + 1);
    assert!(row_text(&view, 0, cx).contains("a0123456789"));
}

#[gpui::test]
fn idle_draws_nothing(cx: &mut TestAppContext) {
    init(cx);
    let (_wb, cx, view) = open_terminal(cx);
    let (notifies, _sub) = count_notifies(&view, cx);
    let before = frames(&view, cx);
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(notifies.get(), 0, "an idle terminal asks for no frame");
    assert_eq!(frames(&view, cx), before);
}

#[gpui::test]
fn idle_prompt_draws_nothing_for_ten_seconds(cx: &mut TestAppContext) {
    // Spec §9.2: a terminal at its prompt draws 0 frames a second.
    init(cx);
    let (_wb, cx, view) = open_terminal(cx);
    feed(&view, b"~/project $ ", cx);
    settle(cx);
    let (notifies, _sub) = count_notifies(&view, cx);
    let before = frames(&view, cx);
    for _ in 0..100 {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }
    assert_eq!(notifies.get(), 0, "an idle prompt asks for no frame");
    assert_eq!(frames(&view, cx), before, "and draws none");
}

#[gpui::test]
fn hidden_busy_panel_draws_nothing(cx: &mut TestAppContext) {
    // Spec §9.1: a hidden panel is not drawn, but its output is parsed.
    init(cx);
    let (wb, cx, view) = open_terminal(cx);
    run(&wb, "workbench.action.togglePanel", cx);
    assert!(cx.debug_bounds("terminal-panel").is_none(), "panel hidden");
    let before = frames(&view, cx);
    for i in 0..500 {
        feed(&view, format!("line {i}\r\n").as_bytes(), cx);
        if i % 10 == 9 {
            cx.executor().advance_clock(Duration::from_millis(4));
            cx.run_until_parked();
            draw(cx);
        }
    }
    settle(cx);
    assert_eq!(frames(&view, cx), before, "a hidden terminal is not drawn");
    let snapshot = view.read_with(cx, |v, _| v.terminal().snapshot());
    let text: String = snapshot
        .cells
        .iter()
        .map(|row| row.iter().map(|c| c.ch).collect::<String>() + "\n")
        .collect();
    assert!(text.contains("line 499"), "output still parsed: {text}");
}

#[gpui::test]
fn theme_change_recolors(cx: &mut TestAppContext) {
    init(cx);
    let (_wb, cx, view) = open_terminal(cx);
    let before = view.read_with(cx, |v, _| v.painted_background()).unwrap();
    // The workbench follows the system appearance, so switch to whichever
    // Lucerna theme is not showing.
    let new_bg = cx.update(|_, cx| {
        cx.update_global::<AppState, _>(|state, _| {
            let other = if state.theme.name == "Lucerna Light" {
                "Lucerna Dark"
            } else {
                "Lucerna Light"
            };
            let other = state.themes.get_or_default(other);
            state.set_theme(other);
            state.theme.terminal.background
        })
    });
    draw(cx);
    let after = view.read_with(cx, |v, _| v.painted_background()).unwrap();
    assert_ne!(before, after);
    let expected = gpui::Rgba {
        r: new_bg.r,
        g: new_bg.g,
        b: new_bg.b,
        a: new_bg.a,
    };
    assert_eq!(after, expected);
}

fn zoom_in(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    let ran = cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("editor.action.fontZoomIn", None, window, cx)
        })
    });
    assert!(ran);
}

#[gpui::test]
fn terminal_follows_zoom(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx, _view) = open_terminal(cx);
    set_user_settings(r#"{ "editor.fontSize": 14 }"#, cx);
    zoom_in(&wb, cx);
    let font = cx.update(|_, cx| terminal_font(wb.read(cx).settings().read(cx).view(cx), cx));
    assert_eq!(font.size, px(15.));
}

#[gpui::test]
fn terminal_with_own_size_ignores_zoom(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx, _view) = open_terminal(cx);
    set_user_settings(r#"{ "terminal.integrated.fontSize": 12 }"#, cx);
    zoom_in(&wb, cx);
    let font = cx.update(|_, cx| terminal_font(wb.read(cx).settings().read(cx).view(cx), cx));
    assert_eq!(font.size, px(12.));
}

#[gpui::test]
fn font_falls_back_to_editor(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx, _view) = open_terminal(cx);
    set_user_settings(
        r#"{ "editor.fontSize": 17, "editor.fontFamily": "Menlo" }"#,
        cx,
    );
    let font = cx.update(|_, cx| terminal_font(wb.read(cx).settings().read(cx).view(cx), cx));
    assert_eq!(font.size, px(17.));
    assert_eq!(font.family.as_ref(), "Menlo");
    assert_eq!(font.line_height, 1.2);
    assert_eq!(font.letter_spacing, 0.);
    assert_eq!(font.weight, gpui::FontWeight::NORMAL);
    assert_eq!(font.bold_weight, gpui::FontWeight::BOLD);

    set_user_settings(
        r#"{ "editor.fontSize": 17, "terminal.integrated.fontSize": 11,
             "terminal.integrated.fontFamily": "Courier", "terminal.integrated.lineHeight": 1.5,
             "terminal.integrated.fontWeight": "300", "terminal.integrated.fontWeightBold": 800 }"#,
        cx,
    );
    let font = cx.update(|_, cx| terminal_font(wb.read(cx).settings().read(cx).view(cx), cx));
    assert_eq!(font.size, px(11.));
    assert_eq!(font.family.as_ref(), "Courier");
    assert_eq!(font.line_height, 1.5);
    assert_eq!(font.weight, gpui::FontWeight(300.));
    assert_eq!(font.bold_weight, gpui::FontWeight(800.));
}

#[gpui::test]
fn app_cursor_shape_is_drawn(cx: &mut TestAppContext) {
    init(cx);
    let (_wb, cx, view) = open_terminal(cx);
    assert_eq!(cursor(&view, cx), Some("terminal-cursor-block"));
    // DECSCUSR 6: steady bar, until the app resets it.
    feed(&view, b"\x1b[6 q", cx);
    settle(cx);
    assert_eq!(cursor(&view, cx), Some("terminal-cursor-bar"));
    feed(&view, b"\x1b[4 q", cx);
    settle(cx);
    assert_eq!(cursor(&view, cx), Some("terminal-cursor-underline"));
    feed(&view, b"\x1b[0 q", cx);
    settle(cx);
    assert_eq!(cursor(&view, cx), Some("terminal-cursor-block"));
    // DECTCEM off hides it.
    feed(&view, b"\x1b[?25l", cx);
    settle(cx);
    assert_eq!(cursor(&view, cx), None);
}

#[gpui::test]
fn inactive_cursor_is_outline(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx, view) = open_terminal(cx);
    assert_eq!(cursor(&view, cx), Some("terminal-cursor-block"));
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.focus_editor_area(window, cx)));
    draw(cx);
    assert_eq!(cursor(&view, cx), Some("terminal-cursor-outline"));
}

#[gpui::test]
fn spawn_failure_shows_the_error(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) =
        cx.add_window_view(|window, cx| Workbench::new(PathBuf::from("/tmp/project"), window, cx));
    let failing: TerminalSpawner = Rc::new(|_| Err(io::Error::other("no such shell")));
    wb.update(cx, |wb, _| wb.set_terminal_spawner(failing));
    run(&wb, "workbench.action.terminal.new", cx);
    let tabs = cx.update(|_, cx| wb.read(cx).terminal_panel().read(cx).tab_count());
    assert_eq!(tabs, 1, "the tab stays open");
    assert!(cx.debug_bounds("terminal-error").is_some());
}
