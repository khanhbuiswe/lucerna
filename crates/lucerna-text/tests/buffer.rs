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
