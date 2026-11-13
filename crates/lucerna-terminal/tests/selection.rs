//! Selection on a fake terminal (spec §6.5): words, logical lines, blocks,
//! select all, and a selection that follows output scrolling the screen.

use lucerna_terminal::{Scroll, SelectKind, Terminal};

/// VS Code's `terminal.integrated.wordSeparators` default.
const VS_CODE_SEPARATORS: &str = " ()[]{}',\"`─‘’“”|";

#[test]
fn word_selection_uses_separators() {
    let t = Terminal::fake(4, 20, 100);
    t.feed(b"foo/bar baz");
    // `/` is not one of VS Code's separators: the word is the whole path.
    t.select_start(0, 5, false, SelectKind::Word);
    assert_eq!(t.selection_text().as_deref(), Some("foo/bar"));

    t.set_word_separators(&format!("{VS_CODE_SEPARATORS}/"));
    t.select_start(0, 5, false, SelectKind::Word);
    assert_eq!(t.selection_text().as_deref(), Some("bar"));

    t.select_start(0, 9, false, SelectKind::Word);
    assert_eq!(t.selection_text().as_deref(), Some("baz"));
}

#[test]
fn default_separators_are_vs_code_ones() {
    let t = Terminal::fake(4, 30, 100);
    t.feed(b"(alpha)'beta'\"gamma\"|delta");
    t.select_start(0, 2, false, SelectKind::Word);
    assert_eq!(t.selection_text().as_deref(), Some("alpha"));
    t.select_start(0, 9, false, SelectKind::Word);
    assert_eq!(t.selection_text().as_deref(), Some("beta"));
    t.select_start(0, 22, false, SelectKind::Word);
    assert_eq!(t.selection_text().as_deref(), Some("delta"));
}

#[test]
fn line_selection_joins_wrapped_rows() {
    let t = Terminal::fake(4, 10, 100);
    t.feed(b"abcdefghijklmno\r\nxyz");
    // Clicked on the wrapped second row: the whole logical line.
    t.select_start(1, 2, false, SelectKind::Line);
    assert_eq!(t.selection_text().as_deref(), Some("abcdefghijklmno"));
    t.select_start(2, 0, false, SelectKind::Line);
    assert_eq!(t.selection_text().as_deref(), Some("xyz"));
}

#[test]
fn simple_selection_follows_the_drag() {
    let t = Terminal::fake(4, 20, 100);
    t.feed(b"hello world\r\nsecond line");
    t.select_start(0, 6, false, SelectKind::Simple);
    assert!(!t.has_selection(), "a press alone selects nothing");
    t.select_update(1, 5, true);
    assert!(t.has_selection());
    assert_eq!(t.selection_text().as_deref(), Some("world\nsecond"));
}

#[test]
fn block_selection_takes_columns() {
    let t = Terminal::fake(4, 20, 100);
    t.feed(b"abcdef\r\nghijkl\r\nmnopqr");
    t.select_start(0, 1, false, SelectKind::Block);
    t.select_update(2, 3, true);
    assert_eq!(t.selection_text().as_deref(), Some("bcd\nhij\nnop"));
}

#[test]
fn select_all_and_clear() {
    let t = Terminal::fake(3, 10, 100);
    t.feed(b"one\r\ntwo");
    t.select_all();
    let text = t.selection_text().expect("everything is selected");
    assert!(text.starts_with("one\ntwo"), "{text:?}");
    t.clear_selection();
    assert!(!t.has_selection());
    assert_eq!(t.selection_text(), None);
}

#[test]
fn selection_survives_scrolling_output() {
    let t = Terminal::fake(3, 10, 100);
    t.feed(b"foo bar");
    t.select_start(0, 1, false, SelectKind::Word);
    assert_eq!(t.selection_text().as_deref(), Some("foo"));
    t.feed(b"\r\n1\r\n2\r\n3\r\n4");
    assert_eq!(t.selection_text().as_deref(), Some("foo"));
    let snap = t.snapshot();
    let sel = snap.selection.expect("still selected");
    assert!(sel.start.line < 0, "moved into history: {sel:?}");
}

#[test]
fn selection_in_scrolled_view_uses_screen_rows() {
    let t = Terminal::fake(3, 10, 100);
    t.feed(b"zero\r\none\r\ntwo\r\nthree\r\nfour");
    t.scroll(Scroll::Lines(1));
    // The screen now shows `one`, `two`, `three`.
    t.select_start(0, 0, false, SelectKind::Line);
    assert_eq!(t.selection_text().as_deref(), Some("one"));
}
