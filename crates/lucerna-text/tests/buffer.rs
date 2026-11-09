use lucerna_text::{Buffer, LineEnding, LoadError, MAX_OPEN_BYTES, Point, check_size};

fn buf(s: &str) -> Buffer {
    Buffer::from_bytes(s.as_bytes(), false).unwrap()
}

#[test]
fn utf8_lines() {
    let s = buf("a\nb\n").snapshot();
    assert_eq!(s.line_count(), 2);
    assert_eq!(s.line(1), "b");
}

#[test]
fn empty_file_one_line() {
    let s = buf("").snapshot();
    assert_eq!(s.line_count(), 1);
    assert_eq!(s.line(0), "");
}

#[test]
fn crlf_stripped() {
    let b = buf("a\r\nb");
    assert_eq!(b.snapshot().line(0), "a");
    assert_eq!(b.line_ending(), LineEnding::CrLf);
}

#[test]
fn mixed_endings_and_no_trailing_newline() {
    let s = buf("a\r\nb\nc").snapshot();
    assert_eq!(s.line_count(), 3);
    for i in 0..3 {
        assert!(!s.line(i).contains('\r'));
    }
    assert_eq!(s.line(2), "c");
}

#[test]
fn point_offset_roundtrip_multibyte() {
    let text = "xin chào\nđ";
    let s = buf(text).snapshot();
    assert_eq!(s.len_bytes(), text.len());
    for (offset, _) in text.char_indices().chain([(text.len(), ' ')]) {
        let p = s.offset_to_point(offset);
        assert_eq!(s.point_to_offset(p), offset, "offset {offset}");
    }
    assert_eq!(s.offset_to_point(text.len()), Point { row: 1, column: 2 });
    assert_eq!(s.text_for_range(4..9), "chào");
}

#[test]
fn binary_detected() {
    assert_eq!(
        Buffer::from_bytes(b"ab\0cd", false).err(),
        Some(LoadError::Binary)
    );
    assert!(Buffer::from_bytes(b"ab\0cd", true).is_ok());
}

#[test]
fn utf16_bom_decoded() {
    let b = Buffer::from_bytes(b"\xff\xfeh\0i\0", false).unwrap();
    assert_eq!(b.snapshot().line(0), "hi");
    assert_eq!(b.encoding(), encoding_rs::UTF_16LE);
}

#[test]
fn latin1_guessed() {
    let b = Buffer::from_bytes(b"caf\xe9 cr\xe8me", false).unwrap();
    assert!(b.snapshot().line(0).contains("café"));
}

#[test]
fn too_large() {
    assert_eq!(check_size(MAX_OPEN_BYTES), Ok(()));
    assert_eq!(
        check_size(MAX_OPEN_BYTES + 1),
        Err(LoadError::TooLarge {
            bytes: MAX_OPEN_BYTES + 1
        })
    );
}

#[test]
fn chunks_cover_range() {
    let s = buf("hello\nworld").snapshot();
    assert_eq!(s.chunks_in_range(3..9).collect::<String>(), "lo\nwor");
}

#[test]
fn is_large_by_bytes_or_lines() {
    let b = buf("a\nb\nc\n");
    assert!(b.is_large(5, 100));
    assert!(b.is_large(100, 2));
    assert!(!b.is_large(100, 3));
}

#[test]
fn load_reports_not_found() {
    let err = Buffer::load(std::path::Path::new("/nonexistent/lucerna/x")).err();
    assert_eq!(err, Some(LoadError::NotFound));
}

fn numbered(n: usize) -> String {
    (0..n).map(|i| format!("line{i}\n")).collect()
}

#[test]
fn reload_insert_above_shifts_row() {
    let mut b = buf(&numbered(10));
    let mut new_lines: Vec<String> = (0..10).map(|i| format!("line{i}")).collect();
    for k in 0..3 {
        new_lines.insert(2 + k, format!("new{k}"));
    }
    let new = new_lines.join("\n") + "\n";
    let r = b.reload(new.as_bytes()).unwrap();
    assert_eq!((r.start_row, r.old_end_row, r.new_end_row), (2, 2, 5));
    assert_eq!(b.snapshot().line_count(), 13);
    assert_eq!(r.adjust_row(7, 13), 10);
    assert_eq!(r.adjust_row(1, 13), 1);
}

#[test]
fn reload_truncate_to_empty_clamps() {
    let mut b = buf(&numbered(100));
    let r = b.reload(b"").unwrap();
    assert_eq!(r.adjust_row(80, 1), 0);
}

#[test]
fn reload_edit_inside_changed_region() {
    let mut b = buf(&numbered(10));
    let new = numbered(10).replace("line4\nline5\n", "x\ny\nz\n");
    let r = b.reload(new.as_bytes()).unwrap();
    assert_eq!((r.start_row, r.old_end_row, r.new_end_row), (4, 6, 7));
    assert_eq!(r.adjust_row(5, 11), 4);
    assert_eq!(r.adjust_row(6, 11), 7);
}

#[test]
fn reload_keeps_force_text() {
    let mut b = Buffer::from_bytes(b"a\0b", true).unwrap();
    assert!(b.reload(b"a\0c").is_ok());
    let mut plain = buf("a");
    assert_eq!(plain.reload(b"a\0c").err(), Some(LoadError::Binary));
}

#[test]
fn crlf_line_counts() {
    assert_eq!(buf("a\r\n").snapshot().line_count(), 1);
    let s = buf("a\r\nb\r\nc\r\n").snapshot();
    assert_eq!(s.line_count(), 3);
    assert_eq!(s.line(2), "c");
}

#[test]
fn long_crlf_line_spans_chunks() {
    let long = "x".repeat(10_000);
    let s = buf(&format!("{long}\r\nend\r\n")).snapshot();
    let line = s.line(0);
    assert!(!line.contains('\r'));
    assert_eq!(line.len(), 10_000);
    assert_eq!(s.line(1), "end");
}

#[test]
fn reload_shrink_below_scroll_clamps_to_last_row() {
    let mut b = buf(&numbered(100));
    let r = b.reload(numbered(10).as_bytes()).unwrap();
    assert_eq!(r.adjust_row(80, 10), 9);
}

#[test]
fn reload_with_crlf_content() {
    let mut b = buf("a\r\nb\r\nc\r\nd\r\n");
    let r = b.reload(b"a\r\nb\r\nX\r\nY\r\nd\r\n").unwrap();
    assert_eq!((r.start_row, r.old_end_row, r.new_end_row), (2, 3, 4));
    assert_eq!(b.line_ending(), LineEnding::CrLf);
}

#[test]
fn char_column_counts_chars_before_the_byte_column() {
    // `é` is 2 bytes, `😀` is 4.
    let b = Buffer::from_bytes("x\naé😀b\r\nlast".as_bytes(), false).unwrap();
    let s = b.snapshot();
    let col = |row, column| s.char_column(Point { row, column });
    assert_eq!(col(1, 0), 0);
    assert_eq!(col(1, 1), 1, "after `a`");
    assert_eq!(col(1, 3), 2, "after `é`");
    assert_eq!(col(1, 2), 1, "inside `é` rounds down");
    assert_eq!(col(1, 7), 3, "after `😀`");
    assert_eq!(col(1, 5), 2, "inside `😀` rounds down");
    assert_eq!(col(1, 8), 4, "end of line");
    assert_eq!(col(1, 99), 4, "past the end clamps before the CRLF");
    assert_eq!(col(2, 4), 4, "last line without newline");
    assert_eq!(col(9, 3), 0, "out-of-range row");
}
