use lucerna_git::{Hunk, HunkKind, MAX_DIFF_BYTES, diff_hunks, load_base_text};
use std::time::Instant;

fn lines(n: usize) -> String {
    (1..=n).map(|i| format!("l{i}\n")).collect()
}

#[test]
fn added_modified_deleted_rows() {
    let base = lines(10);
    // add after l2, modify l5, delete l8
    let current = "l1\nl2\nnew\nl3\nl4\nL5\nl6\nl7\nl9\nl10\n";
    assert_eq!(
        diff_hunks(&base, current),
        vec![
            Hunk {
                kind: HunkKind::Added,
                rows: 2..3,
                base_rows: 2..2
            },
            Hunk {
                kind: HunkKind::Modified,
                rows: 5..6,
                base_rows: 4..5
            },
            Hunk {
                kind: HunkKind::Deleted,
                rows: 8..8,
                base_rows: 7..8
            },
        ]
    );
}

#[test]
fn deletion_at_end_and_start() {
    let base = lines(3);
    assert_eq!(
        diff_hunks(&base, "l1\nl2\n"),
        vec![Hunk {
            kind: HunkKind::Deleted,
            rows: 2..2,
            base_rows: 2..3
        }]
    );
    assert_eq!(
        diff_hunks(&base, "l2\nl3\n"),
        vec![Hunk {
            kind: HunkKind::Deleted,
            rows: 0..0,
            base_rows: 0..1
        }]
    );
    assert!(diff_hunks(&base, &base).is_empty());
}

#[test]
fn crlf_base_equals_lf_buffer() {
    let base = load_base_text(b"a\r\nb\r\n").unwrap();
    assert!(diff_hunks(&base, "a\nb\n").is_empty());
}

#[test]
fn binary_and_large_have_no_base() {
    assert_eq!(load_base_text(b"ab\0cd"), None);
    let big = vec![b'a'; MAX_DIFF_BYTES as usize + 1];
    assert_eq!(load_base_text(&big), None);
    assert_eq!(load_base_text(b"ok\n").as_deref(), Some("ok\n"));
}

#[test]
#[ignore]
fn ten_thousand_lines_under_20ms() {
    let base = lines(10_000);
    let mut current = base.clone();
    current = current
        .replace("l5000\n", "changed\n")
        .replace("l9000\n", "");
    current.insert_str(0, "head\n");
    let start = Instant::now();
    let h = diff_hunks(&base, &current);
    let elapsed = start.elapsed();
    eprintln!("diff_hunks 10k lines: {elapsed:?}");
    assert_eq!(h.len(), 3); // head, changed, deleted
    assert!(elapsed.as_millis() < 20, "{elapsed:?}");
}
