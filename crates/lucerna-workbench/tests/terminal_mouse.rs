//! The mouse in the terminal (spec §6.5): selecting, copying, pasting,
//! scrolling, and reports to apps that take the mouse.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    ClipboardItem, Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point,
    ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, point, px,
};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, TerminalView, Workbench};

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

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    draw(cx);
}

/// A workbench with the terminal shown and focused, and its view.
fn with_terminal(
    cx: &mut TestAppContext,
) -> (
    Entity<Workbench>,
    &mut VisualTestContext,
    Entity<TerminalView>,
) {
    let (wb, cx) =
        cx.add_window_view(|window, cx| Workbench::new(PathBuf::from("/tmp/project"), window, cx));
    wb.update(cx, |wb, cx| {
        wb.editor_groups().update(cx, |groups, cx| {
            groups.set_tab_body_factory(Rc::new(CodeView::tab_body), cx)
        })
    });
    draw(cx);
    keys("ctrl-`", cx);
    let view = cx.update(|_, cx| {
        let panel = wb.read(cx).terminal_panel().read(cx);
        let index = panel.active().expect("a terminal tab");
        panel.tabs()[index]
            .view
            .clone()
            .expect("a running terminal")
    });
    (wb, cx, view)
}

fn feed(view: &Entity<TerminalView>, bytes: &[u8], cx: &mut VisualTestContext) {
    view.read_with(cx, |v, _| v.terminal().feed(bytes));
    draw(cx);
}

fn written(view: &Entity<TerminalView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, _| {
        String::from_utf8_lossy(&v.terminal().written()).into_owned()
    })
}

fn selection(view: &Entity<TerminalView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| v.terminal().selection_text())
}

/// The window position of screen cell `row`/`col`, `dx` pixels right of
/// its left edge, half way down.
fn at(
    view: &Entity<TerminalView>,
    row: u16,
    col: u16,
    dx: f32,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    let cell = view
        .read_with(cx, |v, _| v.cell_bounds(row, col))
        .expect("the terminal was laid out");
    point(cell.left() + px(dx), cell.top() + cell.size.height / 2.)
}

fn press(
    pos: Point<Pixels>,
    button: MouseButton,
    mods: Modifiers,
    clicks: usize,
    cx: &mut VisualTestContext,
) {
    cx.simulate_event(MouseDownEvent {
        button,
        position: pos,
        modifiers: mods,
        click_count: clicks,
        first_mouse: false,
    });
    draw(cx);
}

fn release(pos: Point<Pixels>, button: MouseButton, mods: Modifiers, cx: &mut VisualTestContext) {
    cx.simulate_event(MouseUpEvent {
        button,
        position: pos,
        modifiers: mods,
        click_count: 1,
    });
    draw(cx);
}

fn drag(from: Point<Pixels>, to: Point<Pixels>, mods: Modifiers, cx: &mut VisualTestContext) {
    press(from, MouseButton::Left, mods, 1, cx);
    cx.simulate_mouse_move(to, MouseButton::Left, mods);
    draw(cx);
    release(to, MouseButton::Left, mods, cx);
}

fn wheel(pos: Point<Pixels>, lines: f32, mods: Modifiers, cx: &mut VisualTestContext) {
    cx.simulate_event(ScrollEvent {
        position: pos,
        delta: ScrollDelta::Lines(point(0., lines)),
        modifiers: mods,
        ..Default::default()
    });
    draw(cx);
}

fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| cx.read_from_clipboard().and_then(|i| i.text()))
}

fn set_clipboard(text: &str, cx: &mut VisualTestContext) {
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.to_string())));
}

fn eval(wb: &Entity<Workbench>, expr: &str, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        let keys = wb.read(cx).context_keys(window, cx);
        lucerna_settings::parse_when(expr).unwrap().eval(&keys)
    })
}

fn display_offset(view: &Entity<TerminalView>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |v, _| v.terminal().snapshot().display_offset)
}

fn lines(n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\r\n")
        .into_bytes()
}

const NONE: Modifiers = Modifiers {
    control: false,
    alt: false,
    shift: false,
    platform: false,
    function: false,
};

fn shift() -> Modifiers {
    Modifiers::shift()
}

fn alt() -> Modifiers {
    Modifiers::alt()
}

#[gpui::test]
fn drag_selects_after_2px(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"hello world", cx);
    // Across the middle of a cell, so 1 px would select it if it counted.
    let w = view.read_with(cx, |v, _| v.cell_bounds(0, 0).unwrap().size.width);
    let start = at(&view, 0, 0, f32::from(w) / 2. - 0.5, cx);
    drag(start, start + point(px(1.), px(0.)), NONE, cx);
    assert_eq!(selection(&view, cx), None, "1 px is a click");

    let end = at(&view, 0, 4, 6., cx);
    drag(start, end, NONE, cx);
    assert_eq!(selection(&view, cx).as_deref(), Some("hello"));

    // A click clears it.
    press(start, MouseButton::Left, NONE, 1, cx);
    release(start, MouseButton::Left, NONE, cx);
    assert_eq!(selection(&view, cx), None);
}

#[gpui::test]
fn double_and_triple_click(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"git commit -m done", cx);
    let pos = at(&view, 0, 6, 2., cx);
    press(pos, MouseButton::Left, NONE, 2, cx);
    release(pos, MouseButton::Left, NONE, cx);
    assert_eq!(selection(&view, cx).as_deref(), Some("commit"));
    press(pos, MouseButton::Left, NONE, 3, cx);
    release(pos, MouseButton::Left, NONE, cx);
    assert_eq!(selection(&view, cx).as_deref(), Some("git commit -m done"));
}

#[gpui::test]
fn shift_click_extends_and_alt_drag_selects_block(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"abcdef\r\nghijkl\r\nmnopqr", cx);
    drag(at(&view, 0, 0, 1., cx), at(&view, 0, 1, 6., cx), NONE, cx);
    assert_eq!(selection(&view, cx).as_deref(), Some("ab"));
    let far = at(&view, 1, 2, 6., cx);
    press(far, MouseButton::Left, shift(), 1, cx);
    release(far, MouseButton::Left, shift(), cx);
    assert_eq!(selection(&view, cx).as_deref(), Some("abcdef\nghi"));

    drag(at(&view, 0, 1, 1., cx), at(&view, 2, 3, 6., cx), alt(), cx);
    assert_eq!(selection(&view, cx).as_deref(), Some("bcd\nhij\nnop"));
}

#[gpui::test]
fn cmd_c_copies_and_keeps_selection(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx, view) = with_terminal(cx);
    feed(&view, b"hello world", cx);
    drag(at(&view, 0, 0, 1., cx), at(&view, 0, 4, 6., cx), NONE, cx);
    assert!(eval(&wb, "terminalTextSelected", cx));
    keys("cmd-c", cx);
    assert_eq!(clipboard(cx).as_deref(), Some("hello"));
    assert!(eval(&wb, "terminalTextSelected", cx), "the selection stays");
    assert_eq!(written(&view, cx), "");

    keys("escape", cx);
    assert!(!eval(&wb, "terminalTextSelected", cx), "escape clears it");
}

#[gpui::test]
fn cmd_c_without_selection_does_nothing(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"hello", cx);
    set_clipboard("before", cx);
    keys("cmd-c", cx);
    assert_eq!(written(&view, cx), "");
    assert_eq!(clipboard(cx).as_deref(), Some("before"));
}

#[gpui::test]
fn select_all_command(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx, view) = with_terminal(cx);
    feed(&view, b"one\r\ntwo", cx);
    keys("cmd-a", cx);
    assert!(eval(&wb, "terminalTextSelected", cx));
    assert!(selection(&view, cx).unwrap().starts_with("one\ntwo"));
}

#[gpui::test]
fn copy_on_selection_copies_on_mouse_up(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "terminal.integrated.copyOnSelection": true }"#),
    );
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"hello world", cx);
    set_clipboard("before", cx);
    drag(at(&view, 0, 6, 1., cx), at(&view, 0, 10, 6., cx), NONE, cx);
    assert_eq!(clipboard(cx).as_deref(), Some("world"));
}

#[gpui::test]
fn osc52_store_writes_clipboard(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    // "copied" in base64.
    feed(&view, b"\x1b]52;c;Y29waWVk\x07", cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(20));
    draw(cx);
    assert_eq!(clipboard(cx).as_deref(), Some("copied"));
}

#[gpui::test]
fn paste_bracketed(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"\x1b[?2004h", cx);
    set_clipboard("a\nb", cx);
    keys("cmd-v", cx);
    assert_eq!(written(&view, cx), "\x1b[200~a\nb\x1b[201~");
    assert!(cx.debug_bounds("terminal-paste-warning").is_none());
}

#[gpui::test]
fn multiline_paste_warns_once_without_bracketed_mode(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    set_clipboard("a\nb", cx);
    keys("cmd-v", cx);
    assert!(cx.debug_bounds("terminal-paste-warning").is_some());
    assert_eq!(written(&view, cx), "", "nothing until confirmed");

    let confirm = cx
        .debug_bounds("terminal-paste-confirm")
        .expect("a paste button");
    cx.simulate_click(confirm.center(), NONE);
    draw(cx);
    assert_eq!(written(&view, cx), "a\rb");
    assert!(cx.debug_bounds("terminal-paste-warning").is_none());

    // Cancel drops the paste.
    keys("cmd-v", cx);
    let cancel = cx
        .debug_bounds("terminal-paste-cancel")
        .expect("a cancel button");
    cx.simulate_click(cancel.center(), NONE);
    draw(cx);
    assert_eq!(written(&view, cx), "a\rb");
    assert!(cx.debug_bounds("terminal-paste-warning").is_none());

    // One line: no warning.
    set_clipboard("ls", cx);
    keys("cmd-v", cx);
    assert!(cx.debug_bounds("terminal-paste-warning").is_none());
    assert_eq!(written(&view, cx), "a\rbls");
}

#[gpui::test]
fn multiline_paste_warning_never(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "terminal.integrated.enableMultiLinePasteWarning": "never" }"#),
    );
    let (_, cx, view) = with_terminal(cx);
    set_clipboard("a\nb", cx);
    keys("cmd-v", cx);
    assert!(cx.debug_bounds("terminal-paste-warning").is_none());
    assert_eq!(written(&view, cx), "a\rb");
}

#[gpui::test]
fn mouse_mode_reports_click(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"\x1b[?1000h\x1b[?1006h", cx);
    let pos = at(&view, 2, 3, 2., cx);
    press(pos, MouseButton::Left, NONE, 1, cx);
    release(pos, MouseButton::Left, NONE, cx);
    assert_eq!(written(&view, cx), "\x1b[<0;4;3M\x1b[<0;4;3m");
    assert_eq!(selection(&view, cx), None);
}

#[gpui::test]
fn mouse_mode_reports_drag_and_wheel(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"\x1b[?1002h\x1b[?1006h", cx);
    drag(at(&view, 0, 0, 2., cx), at(&view, 0, 5, 2., cx), NONE, cx);
    assert_eq!(written(&view, cx), "\x1b[<0;1;1M\x1b[<32;6;1M\x1b[<0;6;1m");
    assert_eq!(selection(&view, cx), None);
    let before = written(&view, cx).len();
    wheel(at(&view, 1, 1, 2., cx), 2., NONE, cx);
    assert_eq!(&written(&view, cx)[before..], "\x1b[<64;2;2M\x1b[<64;2;2M");
}

#[gpui::test]
fn shift_click_selects_in_mouse_mode(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"hello world\x1b[?1000h\x1b[?1006h", cx);
    drag(
        at(&view, 0, 0, 1., cx),
        at(&view, 0, 4, 6., cx),
        shift(),
        cx,
    );
    assert_eq!(written(&view, cx), "");
    assert_eq!(selection(&view, cx).as_deref(), Some("hello"));
}

#[gpui::test]
fn option_click_selects_when_setting_on(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "terminal.integrated.macOptionClickForcesSelection": true }"#),
    );
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"hello world\x1b[?1000h\x1b[?1006h", cx);
    drag(at(&view, 0, 0, 1., cx), at(&view, 0, 4, 6., cx), alt(), cx);
    if cfg!(target_os = "macos") {
        assert_eq!(written(&view, cx), "");
        assert!(selection(&view, cx).is_some());
    }
}

#[gpui::test]
fn option_click_reported_when_setting_off(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"\x1b[?1000h\x1b[?1006h", cx);
    let pos = at(&view, 0, 0, 2., cx);
    press(pos, MouseButton::Left, alt(), 1, cx);
    release(pos, MouseButton::Left, alt(), cx);
    assert_eq!(written(&view, cx), "\x1b[<8;1;1M\x1b[<8;1;1m");
}

#[gpui::test]
fn wheel_scrolls_history(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, &lines(100), cx);
    let pos = at(&view, 0, 0, 1., cx);
    wheel(pos, 3., NONE, cx);
    assert_eq!(display_offset(&view, cx), 3);
    wheel(pos, -1., NONE, cx);
    assert_eq!(display_offset(&view, cx), 2);
    // ⌥ scrolls fast: fastScrollSensitivity 5.
    wheel(pos, 1., alt(), cx);
    assert_eq!(display_offset(&view, cx), 7);
    assert_eq!(written(&view, cx), "");
}

#[gpui::test]
fn wheel_sensitivity_setting(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "terminal.integrated.mouseWheelScrollSensitivity": 2 }"#),
    );
    let (_, cx, view) = with_terminal(cx);
    feed(&view, &lines(100), cx);
    wheel(at(&view, 0, 0, 1., cx), 3., NONE, cx);
    assert_eq!(display_offset(&view, cx), 6);
}

#[gpui::test]
fn wheel_in_alt_screen_sends_arrows(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"\x1b[?1049h", cx);
    let pos = at(&view, 0, 0, 1., cx);
    wheel(pos, 3., NONE, cx);
    assert_eq!(written(&view, cx), "\x1b[A\x1b[A\x1b[A");
    wheel(pos, -1., NONE, cx);
    assert_eq!(written(&view, cx), "\x1b[A\x1b[A\x1b[A\x1b[B");
    assert_eq!(display_offset(&view, cx), 0);
}

#[gpui::test]
fn typing_scrolls_to_bottom(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, &lines(100), cx);
    wheel(at(&view, 0, 0, 1., cx), 5., NONE, cx);
    assert_eq!(display_offset(&view, cx), 5);
    keys("x", cx);
    assert_eq!(display_offset(&view, cx), 0);
}

#[gpui::test]
fn output_while_scrolled_keeps_view(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, &lines(100), cx);
    wheel(at(&view, 0, 0, 1., cx), 10., NONE, cx);
    let before: Vec<String> =
        view.read_with(cx, |v, _| (0..3).map(|r| v.painted_row_text(r)).collect());
    feed(&view, b"\r\nmore 1\r\nmore 2\r\nmore 3", cx);
    let after: Vec<String> =
        view.read_with(cx, |v, _| (0..3).map(|r| v.painted_row_text(r)).collect());
    assert_eq!(before, after);
    assert!(display_offset(&view, cx) > 10);
}

#[gpui::test]
fn right_click_select_word_default_macos(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"cargo test --all", cx);
    set_clipboard("before", cx);
    let pos = at(&view, 0, 7, 2., cx);
    press(pos, MouseButton::Right, NONE, 1, cx);
    release(pos, MouseButton::Right, NONE, cx);
    if cfg!(target_os = "windows") {
        // copyPaste: nothing selected, so it pastes.
        assert_eq!(written(&view, cx), "before");
    } else {
        // selectWord on macOS; `default` acts as selectWord on Linux.
        assert_eq!(selection(&view, cx).as_deref(), Some("test"));
        assert_eq!(written(&view, cx), "");
    }
}

#[gpui::test]
fn right_click_copy_paste_setting(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "terminal.integrated.rightClickBehavior": "copyPaste" }"#),
    );
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"hello world", cx);
    drag(at(&view, 0, 0, 1., cx), at(&view, 0, 4, 6., cx), NONE, cx);
    let pos = at(&view, 0, 8, 2., cx);
    press(pos, MouseButton::Right, NONE, 1, cx);
    release(pos, MouseButton::Right, NONE, cx);
    assert_eq!(clipboard(cx).as_deref(), Some("hello"), "copies");
    assert_eq!(selection(&view, cx), None, "and clears");
    press(pos, MouseButton::Right, NONE, 1, cx);
    release(pos, MouseButton::Right, NONE, cx);
    assert_eq!(written(&view, cx), "hello", "then pastes");
}

#[gpui::test]
fn scroll_commands_disabled_in_alt_screen(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, &lines(100), cx);
    keys("pageup", cx);
    assert!(
        display_offset(&view, cx) > 0,
        "pageup scrolls the main screen"
    );
    assert_eq!(written(&view, cx), "");
    keys("cmd-end", cx);
    assert_eq!(display_offset(&view, cx), 0);

    feed(&view, b"\x1b[?1049h", cx);
    keys("pageup", cx);
    assert_eq!(written(&view, cx), "\x1b[5~");
    assert_eq!(display_offset(&view, cx), 0);
}

#[gpui::test]
fn selection_is_painted(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    feed(&view, b"hello world", cx);
    assert_eq!(view.read_with(cx, |v, _| v.painted_selection_rects()), 0);
    drag(at(&view, 0, 0, 1., cx), at(&view, 0, 4, 6., cx), NONE, cx);
    assert_eq!(view.read_with(cx, |v, _| v.painted_selection_rects()), 1);
}

/// Output that scrolls the screen between the painted frame and the drag
/// (the view follows the bottom) must not move the selection off the text
/// that was under the mouse.
#[gpui::test]
fn drag_selects_painted_text_while_output_scrolls(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    let rows = view.read_with(cx, |v, _| v.grid_size().0);
    // Fill the screen so new lines scroll it, with `target` on row 1.
    let mut text = b"first\r\ntarget word".to_vec();
    for i in 2..rows {
        text.extend_from_slice(format!("\r\nfill {i}").as_bytes());
    }
    feed(&view, &text, cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.painted_row_text(1)),
        "target word"
    );
    let from = at(&view, 1, 0, 1., cx);
    let to = at(&view, 1, 5, 6., cx);
    press(from, MouseButton::Left, NONE, 1, cx);
    // Three lines arrive before the next frame is painted.
    view.read_with(cx, |v, _| v.terminal().feed(b"\r\nnew 1\r\nnew 2\r\nnew 3"));
    cx.simulate_mouse_move(to, MouseButton::Left, NONE);
    release(to, MouseButton::Left, NONE, cx);
    assert_eq!(selection(&view, cx).as_deref(), Some("target"));
}

/// The mouse reads the terminal's modes from the painted frame, so a hover
/// never waits on the terminal lock: a mode the app set after the last
/// frame takes effect once it is drawn.
#[gpui::test]
fn hover_reads_mode_from_painted_frame(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, cx, view) = with_terminal(cx);
    view.read_with(cx, |v, _| v.terminal().feed(b"\x1b[?1003h\x1b[?1006h"));
    let pos = at(&view, 0, 1, 2., cx);
    cx.simulate_mouse_move(pos, None, NONE);
    assert_eq!(written(&view, cx), "", "not painted yet");
    draw(cx);
    let pos = at(&view, 0, 2, 2., cx);
    cx.simulate_mouse_move(pos, None, NONE);
    assert_eq!(written(&view, cx), "\x1b[<35;3;1M");
}
