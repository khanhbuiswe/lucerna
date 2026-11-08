use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use lucerna_project::{LineMatch, MAX_RESULTS, SearchEvent, SearchQuery, search};
use tempfile::TempDir;

fn write(root: &Path, rel: &str, content: impl AsRef<[u8]>) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn query(pattern: &str) -> SearchQuery {
    SearchQuery {
        pattern: pattern.into(),
        case_sensitive: true,
        ..Default::default()
    }
}

fn run_with(root: &Path, q: &SearchQuery, settings: &[String]) -> Vec<SearchEvent> {
    let (tx, rx) = crossbeam_channel::unbounded();
    search(root, q, settings, tx, Arc::new(AtomicBool::new(false)));
    rx.try_iter().collect()
}

fn run(root: &Path, q: &SearchQuery) -> Vec<SearchEvent> {
    run_with(root, q, &[])
}

/// Sorted (relative path, matches) pairs.
fn files(root: &Path, events: &[SearchEvent]) -> Vec<(String, Vec<LineMatch>)> {
    let mut out: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            SearchEvent::File { path, matches } => Some((
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                matches.clone(),
            )),
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn names(root: &Path, events: &[SearchEvent]) -> Vec<String> {
    files(root, events).into_iter().map(|f| f.0).collect()
}

fn dones(events: &[SearchEvent]) -> Vec<(usize, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            SearchEvent::Done { total, truncated } => Some((*total, *truncated)),
            _ => None,
        })
        .collect()
}

#[test]
fn finds_literal_with_ranges() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.txt", "one\nfoo.bar and foo.bar\nthree\n");
    let ev = run(dir.path(), &query("foo.bar"));
    let f = files(dir.path(), &ev);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].1.len(), 1);
    let m = &f[0].1[0];
    assert_eq!(m.line_number, 2);
    assert_eq!(m.line_text, "foo.bar and foo.bar");
    assert_eq!(m.ranges, vec![0..7, 12..19]);
    assert_eq!(dones(&ev), vec![(1, false)]);
    // "." is literal: no match for fooXbar.
    write(dir.path(), "b.txt", "fooXbar\n");
    assert_eq!(
        names(dir.path(), &run(dir.path(), &query("foo.bar"))),
        ["a.txt"]
    );
}

#[test]
fn regex_mode() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.txt", "fooXbar\nfoobar\n");
    let mut q = query(r"foo.bar");
    q.is_regex = true;
    let ev = run(dir.path(), &q);
    let f = files(dir.path(), &ev);
    assert_eq!(f[0].1.len(), 1);
    assert_eq!(f[0].1[0].line_number, 1);
}

#[test]
fn case_sensitivity() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.txt", "Hello\nhello\n");
    let mut q = query("hello");
    let f = files(dir.path(), &run(dir.path(), &q));
    assert_eq!(f[0].1.len(), 1);
    assert_eq!(f[0].1[0].line_number, 2);
    q.case_sensitive = false;
    let f = files(dir.path(), &run(dir.path(), &q));
    assert_eq!(f[0].1.len(), 2);
}

#[test]
fn whole_word() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.txt", "login\nlog\nthe log here\n");
    let mut q = query("log");
    q.whole_word = true;
    let f = files(dir.path(), &run(dir.path(), &q));
    let lines: Vec<u32> = f[0].1.iter().map(|m| m.line_number).collect();
    assert_eq!(lines, vec![2, 3]);
}

#[test]
fn include_glob_limits() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "src/deep/a.rs", "needle\n");
    write(dir.path(), "src/b.rs", "needle\n");
    write(dir.path(), "src/c.txt", "needle\n");
    write(dir.path(), "other/d.rs", "needle\n");
    let mut q = query("needle");
    q.include = vec!["src/**/*.rs".into()];
    let ev = run(dir.path(), &q);
    assert_eq!(names(dir.path(), &ev), ["src/b.rs", "src/deep/a.rs"]);
    assert_eq!(dones(&ev), vec![(2, false)]);
}

#[test]
fn query_exclude_applied() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.rs", "needle\n");
    write(dir.path(), "gen/b.rs", "needle\n");
    let mut q = query("needle");
    q.include = vec!["*.rs".into()];
    q.exclude = vec!["gen/**".into()];
    assert_eq!(names(dir.path(), &run(dir.path(), &q)), ["a.rs"]);
}

#[test]
fn settings_exclude_applied() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.txt", "needle\n");
    write(dir.path(), "node_modules/b.txt", "needle\n");
    let ev = run_with(dir.path(), &query("needle"), &["**/node_modules".into()]);
    assert_eq!(names(dir.path(), &ev), ["a.txt"]);
}

#[test]
fn gitignored_skipped() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), ".gitignore", "target/\n");
    write(dir.path(), "a.txt", "needle\n");
    write(dir.path(), "target/b.txt", "needle\n");
    let ev = run(dir.path(), &query("needle"));
    assert_eq!(names(dir.path(), &ev), ["a.txt"]);
}

#[test]
fn binary_skipped() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.txt", "needle\n");
    write(dir.path(), "b.bin", b"needle\0\x01\x02 needle\n");
    let ev = run(dir.path(), &query("needle"));
    assert_eq!(names(dir.path(), &ev), ["a.txt"]);
}

#[test]
fn long_lines_are_truncated() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.txt", format!("needle{}\n", "x".repeat(3000)));
    let ev = run(dir.path(), &query("needle"));
    let m = &files(dir.path(), &ev)[0].1[0];
    assert_eq!(m.line_text.chars().count(), 1000);
    assert_eq!(m.ranges, vec![0..6]);
}

#[test]
fn cap_truncates() {
    let dir = TempDir::new().unwrap();
    for i in 0..5 {
        write(dir.path(), &format!("f{i}.txt"), "m\n".repeat(5000));
    }
    let ev = run(dir.path(), &query("m"));
    let sent: usize = files(dir.path(), &ev).iter().map(|f| f.1.len()).sum();
    assert_eq!(sent, MAX_RESULTS);
    assert_eq!(dones(&ev), vec![(MAX_RESULTS, true)]);
}

#[test]
fn invalid_regex_reports_error() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.txt", "x\n");
    let mut q = query("(unclosed");
    q.is_regex = true;
    let ev = run(dir.path(), &q);
    assert!(matches!(ev[0], SearchEvent::Error(_)));
    assert_eq!(dones(&ev), vec![(0, false)]);
    assert_eq!(ev.len(), 2);
}

#[test]
fn cancel_stops_without_done() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.txt", "needle\n");
    let (tx, rx) = crossbeam_channel::unbounded();
    search(
        dir.path(),
        &query("needle"),
        &[],
        tx,
        Arc::new(AtomicBool::new(true)),
    );
    let ev: Vec<_> = rx.try_iter().collect();
    assert!(dones(&ev).is_empty(), "{ev:?}");
}

#[test]
fn cancel_during_walk_sends_no_done() {
    let dir = TempDir::new().unwrap();
    for i in 0..300 {
        write(
            dir.path(),
            &format!("d{}/f{i}.txt", i % 10),
            "needle\n".repeat(2000),
        );
    }
    let (tx, rx) = crossbeam_channel::bounded(1);
    let cancel = Arc::new(AtomicBool::new(false));
    let root = dir.path().to_path_buf();
    let flag = cancel.clone();
    let handle = std::thread::spawn(move || {
        let mut q = query("needle");
        q.case_sensitive = true;
        search(&root, &q, &[], tx, flag);
    });
    // Wait for the search to be in progress, then cancel.
    let first = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
    assert!(matches!(first, SearchEvent::File { .. }));
    cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    // The channel holds one event, so the walk cannot have finished before
    // the flag was set. Draining lets blocked senders return.
    let rest: Vec<_> = rx.iter().collect();
    handle.join().unwrap();
    assert!(dones(&rest).is_empty(), "Done after cancel");
}

#[test]
fn invalid_regex_with_cancel_sends_nothing() {
    let dir = TempDir::new().unwrap();
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut q = query("(unclosed");
    q.is_regex = true;
    search(dir.path(), &q, &[], tx, Arc::new(AtomicBool::new(true)));
    assert!(rx.try_iter().next().is_none());
}
