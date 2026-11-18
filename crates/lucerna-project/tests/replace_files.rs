use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use lucerna_project::replace::ReplacePattern;
use lucerna_project::{
    FileOutcome, ReplaceProgress, ReplaceSpec, SearchQuery, ShownLine, SkipReason, replace_file,
    replace_file_with_hook, replace_files,
};
use lucerna_text::TempFiles;

fn spec(
    pattern: &str,
    replacement: &str,
    is_regex: bool,
    case: bool,
    preserve: bool,
) -> ReplaceSpec {
    ReplaceSpec {
        query: SearchQuery {
            pattern: pattern.into(),
            is_regex,
            case_sensitive: case,
            ..SearchQuery::default()
        },
        pattern: ReplacePattern::parse(replacement, is_regex),
        preserve_case: preserve,
    }
}

fn plain(pattern: &str, replacement: &str) -> ReplaceSpec {
    spec(pattern, replacement, false, true, false)
}

fn shown(n: u32, text: &str) -> ShownLine {
    ShownLine {
        line_number: n,
        text: text.into(),
    }
}

fn run(path: &Path, lines: &[ShownLine], spec: &ReplaceSpec) -> FileOutcome {
    replace_file(path, lines, spec, &TempFiles::default())
}

fn file(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn replaces_only_shown_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(&dir, "a.txt", b"foo\nfoo\nfoo\n");
    let out = run(
        &path,
        &[shown(1, "foo"), shown(3, "foo")],
        &plain("foo", "bar"),
    );
    assert_eq!(out, FileOutcome::Replaced(2));
    assert_eq!(fs::read(&path).unwrap(), b"bar\nfoo\nbar\n");
}

#[test]
fn skips_file_when_a_line_changed() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(&dir, "a.txt", b"foo\nfob\nfoo\n");
    let out = run(
        &path,
        &[shown(1, "foo"), shown(2, "foo")],
        &plain("foo", "bar"),
    );
    assert_eq!(out, FileOutcome::Skipped(SkipReason::ChangedSinceSearch));
    assert_eq!(fs::read(&path).unwrap(), b"foo\nfob\nfoo\n");
}

#[test]
fn skips_file_changed_before_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(&dir, "a.txt", b"foo\n");
    let out = replace_file_with_hook(
        &path,
        &[shown(1, "foo")],
        &plain("foo", "bar"),
        &TempFiles::default(),
        &|| fs::write(&path, b"agent wrote this\n").unwrap(),
    );
    assert_eq!(out, FileOutcome::Skipped(SkipReason::ChangedOnDisk));
    assert_eq!(fs::read(&path).unwrap(), b"agent wrote this\n");
}

#[test]
fn keeps_crlf_bom_and_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let lines = [shown(1, "foo")];

    let crlf = file(&dir, "crlf.txt", b"foo\r\nx\r\n");
    assert_eq!(
        run(&crlf, &lines, &plain("foo", "bar")),
        FileOutcome::Replaced(1)
    );
    assert_eq!(fs::read(&crlf).unwrap(), b"bar\r\nx\r\n");

    let mut utf16 = vec![0xFF, 0xFE];
    for u in "foo\n".encode_utf16() {
        utf16.extend(u.to_le_bytes());
    }
    let u16p = file(&dir, "u16.txt", &utf16);
    assert_eq!(
        run(&u16p, &lines, &plain("foo", "bar")),
        FileOutcome::Replaced(1)
    );
    let mut want = vec![0xFF, 0xFE];
    for u in "bar\n".encode_utf16() {
        want.extend(u.to_le_bytes());
    }
    assert_eq!(fs::read(&u16p).unwrap(), want);

    let latin = file(&dir, "w.txt", b"caf\xe9 foo\r\n");
    let l = [shown(1, "café foo")];
    assert_eq!(
        run(&latin, &l, &plain("foo", "bar")),
        FileOutcome::Replaced(1)
    );
    assert_eq!(fs::read(&latin).unwrap(), b"caf\xe9 bar\r\n");
}

#[test]
fn skips_lossy_and_binary() {
    let dir = tempfile::tempdir().unwrap();
    // UTF-16LE BOM followed by an unpaired high surrogate.
    let lossy_bytes = [0xFF, 0xFE, 0x00, 0xD8, b'a', 0x00];
    let lossy = file(&dir, "l.txt", &lossy_bytes);
    let out = run(&lossy, &[shown(1, "a")], &plain("a", "b"));
    assert_eq!(out, FileOutcome::Skipped(SkipReason::Lossy));
    assert_eq!(fs::read(&lossy).unwrap(), lossy_bytes);

    let bin_bytes = b"foo\0bar\n";
    let bin = file(&dir, "b.bin", bin_bytes);
    let out = run(&bin, &[shown(1, "foo")], &plain("foo", "x"));
    assert_eq!(out, FileOutcome::Skipped(SkipReason::Unreadable));
    assert_eq!(fs::read(&bin).unwrap(), bin_bytes);
}

#[test]
fn missing_file_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(
        &dir.path().join("nope"),
        &[shown(1, "foo")],
        &plain("foo", "x"),
    );
    assert_eq!(out, FileOutcome::Skipped(SkipReason::Missing));
}

#[test]
fn unencodable_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(&dir, "w.txt", b"caf\xe9 foo\r\n");
    let out = run(&path, &[shown(1, "café foo")], &plain("foo", "đường"));
    assert_eq!(
        out,
        FileOutcome::Skipped(SkipReason::Unencodable("windows-1252"))
    );
    assert_eq!(fs::read(&path).unwrap(), b"caf\xe9 foo\r\n");
}

#[test]
fn ignores_matches_past_shown_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let long = format!("{}foo\n", "x".repeat(1000));
    let path = file(&dir, "a.txt", long.as_bytes());
    let out = run(&path, &[shown(1, &"x".repeat(1000))], &plain("foo", "bar"));
    assert_eq!(out, FileOutcome::Replaced(0));
    assert_eq!(fs::read(&path).unwrap(), long.as_bytes());
}

#[test]
fn regex_groups_and_preserve_case() {
    let dir = tempfile::tempdir().unwrap();
    let a = file(&dir, "a.txt", b"user_id\n");
    let s = spec(r"(\w+)_id", "$1Id", true, true, false);
    assert_eq!(
        run(&a, &[shown(1, "user_id")], &s),
        FileOutcome::Replaced(1)
    );
    assert_eq!(fs::read(&a).unwrap(), b"userId\n");

    let b = file(&dir, "b.txt", b"Foo FOO\n");
    let s = spec("foo", "bar", false, false, true);
    assert_eq!(
        run(&b, &[shown(1, "Foo FOO")], &s),
        FileOutcome::Replaced(2)
    );
    assert_eq!(fs::read(&b).unwrap(), b"Bar BAR\n");

    let c = file(&dir, "c.txt", "Đường\n".as_bytes());
    let s = spec("đường", "phố", false, false, true);
    assert_eq!(run(&c, &[shown(1, "Đường")], &s), FileOutcome::Replaced(1));
    assert_eq!(fs::read(&c).unwrap(), "Phố\n".as_bytes());
}

#[test]
fn cross_line_matches_not_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(&dir, "a.txt", b"a\nb");
    let s = spec(r"a\nb", "x", true, true, false);
    assert_eq!(run(&path, &[shown(1, "a")], &s), FileOutcome::Replaced(0));
    assert_eq!(fs::read(&path).unwrap(), b"a\nb");
}

#[test]
fn cancel_stops_remaining_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = file(&dir, "a.txt", b"foo\n");
    let cancel = AtomicBool::new(true);
    let out = replace_files(
        vec![(path.clone(), vec![shown(1, "foo")])],
        &plain("foo", "bar"),
        &TempFiles::default(),
        &cancel,
        &|_| {},
    );
    assert!(out.is_empty());
    assert_eq!(fs::read(&path).unwrap(), b"foo\n");
}

#[test]
fn progress_reports_each_file() {
    let dir = tempfile::tempdir().unwrap();
    let files: Vec<_> = (0..3)
        .map(|i| {
            (
                file(&dir, &format!("{i}.txt"), b"foo\n"),
                vec![shown(1, "foo")],
            )
        })
        .collect();
    let seen = Mutex::new(Vec::new());
    let cancel = AtomicBool::new(false);
    let out = replace_files(
        files,
        &plain("foo", "bar"),
        &TempFiles::default(),
        &cancel,
        &|p: ReplaceProgress| seen.lock().unwrap().push((p.done, p.total)),
    );
    assert_eq!(out.len(), 3);
    assert!(out.iter().all(|(_, o)| *o == FileOutcome::Replaced(1)));
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen.last(), Some(&(3, 3)));
    assert!(!cancel.load(Ordering::SeqCst));
}

#[test]
fn search_output_feeds_the_job_for_windows_1252() {
    use lucerna_project::{SearchEvent, search};
    use std::collections::HashSet;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let path = file(&dir, "w.txt", b"caf\xe9 foo\r\n");
    let s = plain("foo", "bar");
    let (tx, rx) = crossbeam_channel::unbounded();
    search(
        dir.path(),
        &s.query,
        &[],
        None,
        &HashSet::new(),
        &[],
        tx,
        Arc::new(AtomicBool::new(false)),
    );
    let mut lines = Vec::new();
    for event in rx.try_iter() {
        if let SearchEvent::File { excerpts, .. } = event {
            for l in excerpts.iter().flat_map(|e| &e.lines) {
                if !l.ranges.is_empty() {
                    lines.push(shown(l.line_number, &l.line_text));
                }
            }
        }
    }
    assert_eq!(run(&path, &lines, &s), FileOutcome::Replaced(1));
    assert_eq!(fs::read(&path).unwrap(), b"caf\xe9 bar\r\n");
}
