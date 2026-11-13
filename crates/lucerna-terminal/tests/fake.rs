//! Parser-level behaviour through `Terminal::fake`, with no PTY.

use lucerna_terminal::{
    CellFlags, CursorShape, Modes, NamedColor, Scroll, Snapshot, TermColor, Terminal,
};

fn row_text(snap: &Snapshot, row: usize) -> String {
    snap.cells[row].iter().map(|c| c.ch).collect()
}

fn written(term: &Terminal) -> String {
    String::from_utf8(term.written()).unwrap()
}

#[test]
fn feed_draws_text_and_cursor() {
    let term = Terminal::fake(4, 10, 100);
    term.feed(b"ab");
    let snap = term.snapshot();
    assert_eq!((snap.rows, snap.cols), (4, 10));
    assert_eq!(snap.cells.len(), 4);
    assert!(snap.cells.iter().all(|r| r.len() == 10));
    assert_eq!(row_text(&snap, 0), "ab        ");
    let cursor = snap.cursor.unwrap();
    assert_eq!(
        (cursor.row, cursor.col, cursor.shape),
        (0, 2, CursorShape::Block)
    );
}

#[test]
fn hidden_cursor_is_none() {
    let term = Terminal::fake(4, 10, 100);
    term.feed(b"\x1b[?25l");
    assert!(term.snapshot().cursor.is_none());
}

#[test]
fn cursor_shape_follows_decscusr() {
    let term = Terminal::fake(4, 10, 100);
    term.feed(b"\x1b[6 q");
    assert_eq!(term.snapshot().cursor.unwrap().shape, CursorShape::Bar);
    term.feed(b"\x1b[4 q");
    assert_eq!(
        term.snapshot().cursor.unwrap().shape,
        CursorShape::Underline
    );
}

#[test]
fn modes_follow_private_modes() {
    let term = Terminal::fake(4, 10, 100);
    assert!(!term.snapshot().mode.contains(Modes::ALT_SCREEN));
    term.feed(b"\x1b[?1049h\x1b[?2004h\x1b[?1h\x1b[?1006h\x1b[?1000h\x1b[?1004h\x1b=");
    let mode = term.snapshot().mode;
    for m in [
        Modes::ALT_SCREEN,
        Modes::BRACKETED_PASTE,
        Modes::APP_CURSOR,
        Modes::SGR_MOUSE,
        Modes::MOUSE_REPORT_CLICK,
        Modes::FOCUS_IN_OUT,
        Modes::APP_KEYPAD,
    ] {
        assert!(mode.contains(m), "{m:?} missing from {mode:?}");
    }
    assert!(mode.intersects(Modes::MOUSE_MODE));
    assert!(!mode.contains(Modes::MOUSE_DRAG));
}

#[test]
fn cell_colors_and_flags() {
    let term = Terminal::fake(2, 10, 100);
    term.feed(b"\x1b[1;3;31;48;2;1;2;3mX\x1b[0;38;5;200;9mY");
    let snap = term.snapshot();
    let x = &snap.cells[0][0];
    assert_eq!(x.ch, 'X');
    assert_eq!(x.fg, TermColor::Named(NamedColor::Red));
    assert_eq!(x.bg, TermColor::Rgb(1, 2, 3));
    assert!(x.flags.contains(CellFlags::BOLD | CellFlags::ITALIC));
    let y = &snap.cells[0][1];
    assert_eq!(y.fg, TermColor::Indexed(200));
    assert_eq!(y.bg, TermColor::Named(NamedColor::Background));
    assert!(y.flags.contains(CellFlags::STRIKEOUT));
    assert!(!y.flags.contains(CellFlags::BOLD));
}

#[test]
fn wide_and_combining_cells() {
    let term = Terminal::fake(2, 10, 100);
    term.feed("你e\u{301}".as_bytes());
    let snap = term.snapshot();
    let row = &snap.cells[0];
    assert_eq!(row[0].ch, '你');
    assert!(row[0].flags.contains(CellFlags::WIDE));
    assert!(row[1].flags.contains(CellFlags::WIDE_SPACER));
    assert_eq!(row[2].ch, 'e');
    assert_eq!(row[2].zerowidth.as_deref(), Some(&['\u{301}'][..]));
}

#[test]
fn osc8_hyperlink_is_kept() {
    let term = Terminal::fake(2, 10, 100);
    term.feed(b"\x1b]8;;https://a.b/\x1b\\ln\x1b]8;;\x1b\\x");
    let snap = term.snapshot();
    assert_eq!(snap.cells[0][0].hyperlink.as_deref(), Some("https://a.b/"));
    assert_eq!(snap.cells[0][1].hyperlink.as_deref(), Some("https://a.b/"));
    assert_eq!(snap.cells[0][2].hyperlink, None);
}

#[test]
fn input_is_written_to_the_pty() {
    let term = Terminal::fake(2, 10, 100);
    term.input(b"ls\r");
    assert_eq!(written(&term), "ls\r");
}

#[test]
fn paste_maps_newlines_to_cr() {
    let term = Terminal::fake(2, 10, 100);
    term.paste("a\r\nb\nc");
    assert_eq!(written(&term), "a\rb\rc");
}

#[test]
fn paste_is_bracketed_and_strips_escape() {
    let term = Terminal::fake(2, 10, 100);
    term.feed(b"\x1b[?2004h");
    term.paste("a\x1b[201~b");
    assert_eq!(written(&term), "\x1b[200~a[201~b\x1b[201~");
}

#[test]
fn device_status_report_is_answered() {
    let term = Terminal::fake(4, 10, 100);
    term.feed(b"\x1b[2;3H\x1b[6n");
    assert_eq!(written(&term), "\x1b[2;3R");
}

#[test]
fn text_area_size_request_is_answered() {
    let term = Terminal::fake(4, 10, 100);
    term.resize(6, 20, 9, 18);
    term.feed(b"\x1b[14t");
    assert_eq!(written(&term), format!("\x1b[4;{};{}t", 6 * 18, 20 * 9));
}

#[test]
fn color_request_is_answered() {
    let term = Terminal::fake(4, 10, 100);
    term.feed(b"\x1b]11;?\x07");
    let w = written(&term);
    assert!(
        w.starts_with("\x1b]11;rgb:") && w.ends_with('\x07'),
        "{w:?}"
    );
}

#[test]
fn clipboard_load_is_refused() {
    let term = Terminal::fake(4, 10, 100);
    term.feed(b"\x1b]52;c;?\x07");
    assert_eq!(written(&term), "");
}

#[test]
fn scroll_moves_the_viewport() {
    let term = Terminal::fake(5, 10, 100);
    for i in 0..20 {
        term.feed(format!("{i}\r\n").as_bytes());
    }
    let snap = term.snapshot();
    assert_eq!((snap.display_offset, snap.history_len), (0, 16));
    assert!(snap.cursor.is_some());
    term.scroll(Scroll::Lines(3));
    let snap = term.snapshot();
    assert_eq!(snap.display_offset, 3);
    assert!(snap.cursor.is_none(), "cursor is below the viewport");
    term.scroll(Scroll::Top);
    let snap = term.snapshot();
    assert_eq!(snap.display_offset, 16);
    assert_eq!(row_text(&snap, 0).trim_end(), "0");
    term.scroll(Scroll::PageDown);
    assert_eq!(term.snapshot().display_offset, 11);
    term.scroll(Scroll::Bottom);
    assert_eq!(term.snapshot().display_offset, 0);
}

#[test]
fn clear_keeps_the_cursor_line() {
    let term = Terminal::fake(5, 10, 100);
    for i in 0..10 {
        term.feed(format!("{i}\r\n").as_bytes());
    }
    term.feed(b"$ ab");
    term.clear();
    let snap = term.snapshot();
    assert_eq!(snap.history_len, 0);
    assert_eq!(row_text(&snap, 0).trim_end(), "$ ab");
    for r in 1..5 {
        assert_eq!(row_text(&snap, r).trim_end(), "", "row {r}");
    }
    let cursor = snap.cursor.unwrap();
    assert_eq!((cursor.row, cursor.col), (0, 4));
}

#[test]
fn resize_reflows_rows_and_cols() {
    let term = Terminal::fake(4, 10, 100);
    term.resize(6, 12, 8, 16);
    let snap = term.snapshot();
    assert_eq!((snap.rows, snap.cols), (6, 12));
    assert_eq!(snap.cells.len(), 6);
    assert!(snap.cells.iter().all(|r| r.len() == 12));
}

#[test]
fn fake_has_no_child() {
    let term = Terminal::fake(4, 10, 100);
    assert_eq!(term.child_pid(), None);
    #[cfg(unix)]
    assert_eq!(term.pty_fd(), None);
}

#[test]
fn fake_with_events_reports_wakeup_and_title() {
    use lucerna_terminal::TerminalEvent;
    let (term, mut events) = Terminal::fake_with_events(4, 10, 100);
    term.feed(b"\x1b]0;build\x07hi");
    let mut got = Vec::new();
    while let Ok(ev) = events.try_recv() {
        got.push(ev);
    }
    assert!(
        got.contains(&TerminalEvent::Title(Some("build".into()))),
        "{got:?}"
    );
    assert_eq!(got.last(), Some(&TerminalEvent::Wakeup), "{got:?}");
}
