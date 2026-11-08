//! Pure display layout of the code view: truncation, wrapping, folding and
//! row clamping, with no GPUI involved.

use std::time::{Duration, Instant};

use lucerna_text::{Buffer, BufferSnapshot};
use lucerna_workbench::code_view::{
    FoldState, MAX_RENDERED_LINE_CHARS, display_lines, display_lines_with_tab_size,
};

fn snapshot(text: &str) -> BufferSnapshot {
    Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot()
}

#[test]
fn long_line_truncated_at_10000() {
    let long = "a".repeat(MAX_RENDERED_LINE_CHARS + 500);
    let s = snapshot(&format!("{long}\nshort\n"));
    let lines = display_lines(&s, &FoldState::default(), None, 0..2);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].truncated);
    assert_eq!(
        lines[0].text,
        format!("{}…", "a".repeat(MAX_RENDERED_LINE_CHARS))
    );
    assert!(!lines[1].truncated);
    assert_eq!(lines[1].text, "short");

    // Exactly the limit is not truncated, and multi-byte chars count once.
    let exact = "é".repeat(MAX_RENDERED_LINE_CHARS);
    let s = snapshot(&format!("{exact}\r\n"));
    let lines = display_lines(&s, &FoldState::default(), None, 0..1);
    assert!(!lines[0].truncated);
    assert_eq!(lines[0].text, exact);
}

#[test]
fn minified_5mb_single_line_layout_fast() {
    let line = "var a=1;".repeat(5 * 1024 * 1024 / 8);
    let s = snapshot(&line);
    // 5 ms in release; debug builds get ten times that.
    let budget = if cfg!(debug_assertions) {
        Duration::from_millis(50)
    } else {
        Duration::from_millis(5)
    };
    let start = Instant::now();
    let lines = display_lines(&s, &FoldState::default(), None, 0..1);
    let elapsed = start.elapsed();
    assert!(elapsed < budget, "took {elapsed:?}");
    assert_eq!(lines.len(), 1);
    assert!(lines[0].truncated);
    assert_eq!(lines[0].text.chars().count(), MAX_RENDERED_LINE_CHARS + 1);
}

#[test]
fn wrap_splits_lines() {
    let s = snapshot("abcdefghij\nxy\n\n");
    let lines = display_lines(&s, &FoldState::default(), Some(4), 0..3);
    let got: Vec<_> = lines
        .iter()
        .map(|l| (l.buffer_row, l.text.as_str(), l.wrap_index))
        .collect();
    assert_eq!(
        got,
        vec![
            (0, "abcd", 0),
            (0, "efgh", 1),
            (0, "ij", 2),
            (1, "xy", 0),
            (2, "", 0),
        ]
    );
    // Wrapping counts chars, not bytes.
    let s = snapshot("ééééé");
    let lines = display_lines(&s, &FoldState::default(), Some(2), 0..1);
    let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, vec!["éé", "éé", "é"]);
}

#[test]
fn fold_hides_rows() {
    let s = snapshot("fn a() {\n    one\n    two\n}\nnext\n");
    let available = [0..3, 5..7];
    let mut folds = FoldState::default();
    folds.toggle(0, &available);
    assert!(!folds.is_hidden(0));
    assert!(folds.is_hidden(1));
    assert!(folds.is_hidden(2));
    assert!(!folds.is_hidden(3));
    let rows: Vec<_> = display_lines(&s, &folds, None, 0..5)
        .iter()
        .map(|l| l.buffer_row)
        .collect();
    assert_eq!(rows, vec![0, 3, 4]);

    // Toggling again unfolds.
    folds.toggle(0, &available);
    assert!(!folds.is_hidden(1));
}

#[test]
fn fold_toggle_ignores_non_header() {
    let available = vec![0..3, 5..8];
    let mut folds = FoldState::default();
    folds.toggle(1, &available);
    folds.toggle(4, &available);
    for row in 0..10 {
        assert!(!folds.is_hidden(row), "row {row}");
    }
}

#[test]
fn rows_beyond_end_clamped() {
    let s = snapshot("one\ntwo\n");
    let lines = display_lines(&s, &FoldState::default(), None, 1..50);
    let got: Vec<_> = lines
        .iter()
        .map(|l| (l.buffer_row, l.text.as_str()))
        .collect();
    assert_eq!(got, vec![(1, "two")]);
    assert!(display_lines(&s, &FoldState::default(), None, 7..9).is_empty());
}

/// Display columns of a wrapped piece: tabs advance to the next multiple
/// of `tab_size` from the piece start, CJK chars take two columns.
fn piece_columns(text: &str, tab_size: u32) -> u32 {
    text.chars().fold(0, |col, c| match c {
        '\t' => col + tab_size - col % tab_size,
        c if ('\u{3040}'..='\u{9fff}').contains(&c) => col + 2,
        _ => col + 1,
    })
}

fn wrapped(text: &str, wrap: u32, tab_size: u32) -> Vec<String> {
    let s = snapshot(text);
    display_lines_with_tab_size(&s, &FoldState::default(), Some(wrap), tab_size, 0..1)
        .into_iter()
        .map(|l| l.text)
        .collect()
}

#[test]
fn wrap_counts_tabs_as_display_columns() {
    // Two tabs reach the wrap width of 8 on their own; by chars (6) the
    // line would fit and overflow by 4 columns when drawn.
    let pieces = wrapped("\t\tabcd", 8, 4);
    assert_eq!(pieces, vec!["\t\t", "abcd"]);
    for piece in &pieces {
        assert!(piece_columns(piece, 4) <= 8, "{piece:?}");
    }
    // Exactly the width fits in one piece.
    assert_eq!(wrapped("\tabcd", 8, 4), vec!["\tabcd"]);
}

#[test]
fn wrap_counts_mixed_tabs_and_spaces() {
    let pieces = wrapped("  \tx  \ty  \tz", 6, 4);
    assert_eq!(pieces.concat(), "  \tx  \ty  \tz");
    assert!(pieces.len() > 1);
    for piece in &pieces {
        assert!(piece_columns(piece, 4) <= 6, "{piece:?}");
    }
}

#[test]
fn wrap_counts_wide_chars_as_two_columns() {
    assert_eq!(wrapped("日本語", 4, 4), vec!["日本", "語"]);
}
