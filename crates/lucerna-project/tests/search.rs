use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use lucerna_project::{Excerpt, LineMatch, MAX_RESULTS, SearchEvent, SearchQuery, search};
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
    search(root, q, settings, &[], tx, Arc::new(AtomicBool::new(false)));
    rx.try_iter().collect()
}

/// Searches with the given unsaved buffers `(path, text)` of open files.
fn run_buffers(root: &Path, q: &SearchQuery, buffers: &[(Arc<Path>, String)]) -> Vec<SearchEvent> {
    let (tx, rx) = crossbeam_channel::unbounded();
    search(root, q, &[], buffers, tx, Arc::new(AtomicBool::new(false)));
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
            SearchEvent::File { path, excerpts } => Some((
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                excerpts
                    .iter()
                    .flat_map(|e| &e.lines)
                    .filter(|l| !l.ranges.is_empty())
                    .cloned()
                    .collect(),
            )),
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Sorted (relative path, excerpts as line-number lists) pairs.
fn excerpts(root: &Path, events: &[SearchEvent]) -> Vec<(String, Vec<Vec<u32>>)> {
    let mut out: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            SearchEvent::File { path, excerpts } => Some((
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                excerpts
                    .iter()
                    .map(|e: &Excerpt| e.lines.iter().map(|l| l.line_number).collect())
                    .collect(),
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
        search(&root, &q, &[], &[], tx, flag);
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
    search(
        dir.path(),
        &q,
        &[],
        &[],
        tx,
        Arc::new(AtomicBool::new(true)),
    );
    assert!(rx.try_iter().next().is_none());
}

/// A file of `n` lines `l1..ln` where the lines in `hits` read `needle`.
fn numbered(n: u32, hits: &[u32]) -> String {
    (1..=n)
        .map(|i| {
            if hits.contains(&i) {
                format!("needle {i}\n")
            } else {
                format!("l{i}\n")
            }
        })
        .collect()
}

fn ctx_query(context_lines: u32) -> SearchQuery {
    SearchQuery {
        context_lines,
        ..query("needle")
    }
}

fn groups(n: u32, hits: &[u32], ctx: u32) -> Vec<Vec<u32>> {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.txt", numbered(n, hits));
    let ev = run(dir.path(), &ctx_query(ctx));
    excerpts(dir.path(), &ev).remove(0).1
}

#[test]
fn context_zero_gives_one_line_excerpts() {
    assert_eq!(groups(9, &[2, 5], 0), vec![vec![2], vec![5]]);
}

#[test]
fn context_lines_surround_match() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.txt", numbered(9, &[5]));
    let ev = run(dir.path(), &ctx_query(1));
    assert_eq!(excerpts(dir.path(), &ev)[0].1, vec![vec![4, 5, 6]]);
    let SearchEvent::File { excerpts, .. } = &ev[0] else {
        panic!()
    };
    let lines = &excerpts[0].lines;
    assert!(lines[0].ranges.is_empty() && lines[2].ranges.is_empty());
    assert!(!lines[1].ranges.is_empty());
    assert_eq!(excerpts[0].match_count(), 1);
}

#[test]
fn context_clipped_at_file_edges() {
    assert_eq!(groups(9, &[1, 9], 2), vec![vec![1, 2, 3], vec![7, 8, 9]]);
}

#[test]
fn touching_context_merges() {
    assert_eq!(groups(9, &[2, 4], 1), vec![vec![1, 2, 3, 4, 5]]);
}

#[test]
fn adjacent_after_gap_merges() {
    assert_eq!(groups(9, &[2, 5], 1), vec![vec![1, 2, 3, 4, 5, 6]]);
}

#[test]
fn separate_groups_split() {
    assert_eq!(groups(10, &[2, 8], 1), vec![vec![1, 2, 3], vec![7, 8, 9]]);
}

#[test]
fn cap_counts_only_match_lines() {
    let dir = TempDir::new().unwrap();
    let hits: Vec<u32> = (1..=(MAX_RESULTS as u32 + 5)).map(|i| i * 3).collect();
    write(
        dir.path(),
        "a.txt",
        numbered(MAX_RESULTS as u32 * 3 + 30, &hits),
    );
    let ev = run(dir.path(), &ctx_query(1));
    assert_eq!(dones(&ev), vec![(MAX_RESULTS, true)]);
    let matches: usize = ev
        .iter()
        .map(|e| match e {
            SearchEvent::File { excerpts, .. } => excerpts.iter().map(Excerpt::match_count).sum(),
            _ => 0,
        })
        .sum();
    assert_eq!(matches, MAX_RESULTS);
}

#[test]
fn context_lines_are_cut_like_matches() {
    let dir = TempDir::new().unwrap();
    write(
        dir.path(),
        "a.txt",
        format!("{}\nneedle\n", "x".repeat(1500)),
    );
    let ev = run(dir.path(), &ctx_query(1));
    let SearchEvent::File { excerpts, .. } = &ev[0] else {
        panic!()
    };
    assert_eq!(excerpts[0].lines[0].line_text.chars().count(), 1000);
    assert!(excerpts[0].lines[0].ranges.is_empty());
}

#[test]
fn match_past_the_cut_still_counts_as_match() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.txt", format!("{}needle\n", "-".repeat(1500)));
    let ev = run(dir.path(), &ctx_query(1));
    let SearchEvent::File { excerpts, .. } = &ev[0] else {
        panic!()
    };
    assert_eq!(excerpts[0].match_count(), 1);
    assert_eq!(dones(&ev), vec![(1, false)]);
}

/// T5-6: an open file with unsaved changes is searched on its buffer text,
/// not on the disk, so its line numbers are the buffer's (as VS Code).
#[test]
fn unsaved_buffer_replaces_the_disk_text() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.rs", numbered(60, &[50]));
    write(dir.path(), "b.rs", "needle on disk\n");
    // Ten unsaved lines inserted at the top of `a.rs`.
    let buffer = format!("{}{}", "new\n".repeat(10), numbered(60, &[50]));
    let buffers = [(Arc::from(dir.path().join("a.rs")), buffer)];
    let lines: Vec<(String, Vec<u32>)> = files(
        dir.path(),
        &run_buffers(dir.path(), &query("needle"), &buffers),
    )
    .into_iter()
    .map(|(path, lines)| (path, lines.iter().map(|l| l.line_number).collect()))
    .collect();
    assert_eq!(
        lines,
        vec![("a.rs".into(), vec![60]), ("b.rs".into(), vec![1])],
        "a.rs from its buffer, b.rs from the disk"
    );
}

/// Review Focus 2: text only in the buffer is found, text only on the disk
/// of that file is not.
#[test]
fn text_only_in_the_buffer_is_found() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.rs", "old needle\n");
    let buffers = [(
        Arc::from(dir.path().join("a.rs")),
        "fresh\nunsaved haystack\n".to_owned(),
    )];
    let found = files(
        dir.path(),
        &run_buffers(dir.path(), &query("haystack"), &buffers),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, "a.rs");
    assert_eq!(found[0].1[0].line_number, 2);
    assert_eq!(found[0].1[0].line_text, "unsaved haystack");
    assert_eq!(found[0].1[0].ranges, vec![8..16]);
    let ev = run_buffers(dir.path(), &query("needle"), &buffers);
    assert!(files(dir.path(), &ev).is_empty(), "{ev:?}");
    assert_eq!(dones(&ev), vec![(0, false)]);
}

/// A buffer still obeys the include and exclude filters.
#[test]
fn unsaved_buffer_obeys_the_filters() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.rs", "x\n");
    let buffers = [(Arc::from(dir.path().join("a.rs")), "needle\n".to_owned())];
    let mut q = query("needle");
    q.exclude = vec!["*.rs".into()];
    assert!(files(dir.path(), &run_buffers(dir.path(), &q, &buffers)).is_empty());
}
