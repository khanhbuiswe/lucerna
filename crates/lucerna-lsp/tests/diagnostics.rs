use std::path::{Path, PathBuf};

use lsp_types::{
    CodeDescription, DiagnosticSeverity, DiagnosticTag, NumberOrString, Position,
    PublishDiagnosticsParams, Range,
};
use lucerna_lsp::ServerKey;
use lucerna_lsp::diagnostics::{Diagnostic, DiagnosticStore, Scope, Severity};
use lucerna_lsp::position::PositionEncoding::{Utf8, Utf16};
use lucerna_text::{Buffer, BufferSnapshot, Point};

fn snap(text: &str) -> BufferSnapshot {
    Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot()
}

fn key(server: &'static str) -> ServerKey {
    ServerKey {
        root: Path::new("/proj").into(),
        server,
    }
}

fn uri(path: &Path) -> lsp_types::Uri {
    lucerna_lsp::position::path_to_uri(path)
}

fn lsp_diag(
    (sl, sc): (u32, u32),
    (el, ec): (u32, u32),
    severity: Option<DiagnosticSeverity>,
    message: &str,
) -> lsp_types::Diagnostic {
    lsp_types::Diagnostic {
        range: Range::new(Position::new(sl, sc), Position::new(el, ec)),
        severity,
        message: message.to_owned(),
        ..Default::default()
    }
}

fn params(
    path: &Path,
    version: Option<i32>,
    diagnostics: Vec<lsp_types::Diagnostic>,
) -> PublishDiagnosticsParams {
    PublishDiagnosticsParams {
        uri: uri(path),
        diagnostics,
        version,
    }
}

fn pt(row: u32, column: u32) -> Point {
    Point { row, column }
}

const ERR: Option<DiagnosticSeverity> = Some(DiagnosticSeverity::ERROR);
const WARN: Option<DiagnosticSeverity> = Some(DiagnosticSeverity::WARNING);
const HINT: Option<DiagnosticSeverity> = Some(DiagnosticSeverity::HINT);

#[test]
fn stale_version_dropped() {
    let mut store = DiagnosticStore::default();
    let path = Path::new("/proj/a.rs");
    let s = snap("let x = 1;\n");
    // The editor is at version 3; the server answers for version 2.
    let dropped = store.publish(
        &key("ra"),
        params(path, Some(2), vec![lsp_diag((0, 0), (0, 3), ERR, "old")]),
        Utf16,
        Some((&s, 3)),
    );
    assert_eq!(dropped, None);
    assert!(store.for_file(path).is_empty());
    assert_eq!(store.summary(), (0, 0));

    let kept = store.publish(
        &key("ra"),
        params(path, Some(3), vec![lsp_diag((0, 0), (0, 3), ERR, "new")]),
        Utf16,
        Some((&s, 3)),
    );
    assert_eq!(kept, Some(path.to_path_buf()));
    assert_eq!(store.for_file(path)[0].message, "new");

    // No version: always accepted.
    let kept = store.publish(
        &key("ra"),
        params(
            path,
            None,
            vec![lsp_diag((0, 0), (0, 3), ERR, "unversioned")],
        ),
        Utf16,
        Some((&s, 9)),
    );
    assert!(kept.is_some());
    assert_eq!(store.for_file(path)[0].message, "unversioned");
}

#[test]
fn out_of_range_clamped() {
    let mut store = DiagnosticStore::default();
    let path = Path::new("/proj/a.rs");
    let s = snap("ab\ncde");
    store.publish(
        &key("ra"),
        params(path, None, vec![lsp_diag((0, 1), (99, 0), ERR, "x")]),
        Utf16,
        Some((&s, 0)),
    );
    let d = &store.for_file(path)[0];
    assert_eq!(d.range, pt(0, 1)..pt(1, 3));
}

#[test]
fn conversion_of_fields() {
    let mut store = DiagnosticStore::default();
    let path = Path::new("/proj/a.rs");
    let s = snap("abc\n");
    let mut d = lsp_diag((0, 0), (0, 1), None, "m");
    d.source = Some("rustc".into());
    d.code = Some(NumberOrString::Number(308));
    d.code_description = Some(CodeDescription {
        href: "https://example.com/E0308".parse().unwrap(),
    });
    d.tags = Some(vec![DiagnosticTag::UNNECESSARY, DiagnosticTag::DEPRECATED]);
    let mut e = lsp_diag((0, 2), (0, 3), WARN, "w");
    e.code = Some(NumberOrString::String("W1".into()));
    store.publish(
        &key("ra"),
        params(path, None, vec![e, d]),
        Utf16,
        Some((&s, 0)),
    );
    assert_eq!(
        store.for_file(path),
        vec![
            Diagnostic {
                range: pt(0, 0)..pt(0, 1),
                severity: Severity::Error,
                message: "m".into(),
                source: Some("rustc".into()),
                code: Some("308".into()),
                code_href: Some("https://example.com/E0308".into()),
                unnecessary: true,
                deprecated: true,
            },
            Diagnostic {
                range: pt(0, 2)..pt(0, 3),
                severity: Severity::Warning,
                message: "w".into(),
                source: None,
                code: Some("W1".into()),
                code_href: None,
                unnecessary: false,
                deprecated: false,
            },
        ]
    );
    assert!(Severity::Error < Severity::Warning && Severity::Information < Severity::Hint);
}

#[test]
fn summary_counts_unopened() {
    let mut store = DiagnosticStore::default();
    let open = Path::new("/proj/open.rs");
    let s = snap("abc\n");
    store.publish(
        &key("ra"),
        params(
            open,
            None,
            vec![
                lsp_diag((0, 0), (0, 1), ERR, "e"),
                lsp_diag((0, 1), (0, 2), HINT, "h"),
            ],
        ),
        Utf16,
        Some((&s, 0)),
    );
    // Unopened (and nonexistent) file: counted without reading it.
    store.publish(
        &key("ra"),
        params(
            Path::new("/proj/closed.rs"),
            None,
            vec![
                lsp_diag((5, 0), (5, 1), WARN, "w"),
                lsp_diag((6, 0), (6, 1), None, "missing severity is an error"),
            ],
        ),
        Utf16,
        None,
    );
    assert_eq!(store.summary(), (2, 1));
    // Publishing an empty list clears the file.
    store.publish(
        &key("ra"),
        params(Path::new("/proj/closed.rs"), None, vec![]),
        Utf16,
        None,
    );
    assert_eq!(store.summary(), (1, 0));
}

#[test]
fn next_skips_hint_wraps_across_files() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.rs");
    let b = dir.path().join("b.rs");
    std::fs::write(&a, "aaaa\nbbbb\ncccc\n").unwrap();
    std::fs::write(&b, "xxxx\nyyyy\n").unwrap();
    let mut store = DiagnosticStore::default();
    let k = key("ra");
    store.publish(
        &k,
        params(
            &a,
            None,
            vec![
                lsp_diag((0, 1), (0, 2), ERR, "a1"),
                lsp_diag((1, 0), (1, 1), HINT, "a-hint"),
                lsp_diag((2, 0), (2, 1), WARN, "a3"),
            ],
        ),
        Utf16,
        None,
    );
    store.publish(
        &k,
        params(&b, None, vec![lsp_diag((1, 2), (1, 3), WARN, "b2")]),
        Utf16,
        None,
    );
    fn step(
        store: &DiagnosticStore,
        from: (&Path, Point),
        forward: bool,
        scope: Scope,
    ) -> Option<(String, String)> {
        store.next(from, forward, scope).map(|(p, d)| {
            (
                p.file_name().unwrap().to_str().unwrap().to_owned(),
                d.message,
            )
        })
    }

    // Forward across files, skipping the hint and the origin itself.
    assert_eq!(
        step(&store, (&a, pt(0, 1)), true, Scope::Files),
        Some(("a.rs".into(), "a3".into()))
    );
    assert_eq!(
        step(&store, (&a, pt(2, 0)), true, Scope::Files),
        Some(("b.rs".into(), "b2".into()))
    );
    // Wraps to the first file.
    assert_eq!(
        step(&store, (&b, pt(1, 2)), true, Scope::Files),
        Some(("a.rs".into(), "a1".into()))
    );
    // Backward, with wrap.
    assert_eq!(
        step(&store, (&b, pt(1, 2)), false, Scope::Files),
        Some(("a.rs".into(), "a3".into()))
    );
    assert_eq!(
        step(&store, (&a, pt(0, 1)), false, Scope::Files),
        Some(("b.rs".into(), "b2".into()))
    );
    // File scope stays in the file and wraps inside it.
    assert_eq!(
        step(&store, (&a, pt(2, 0)), true, Scope::File),
        Some(("a.rs".into(), "a1".into()))
    );
    assert_eq!(
        step(&store, (&a, pt(0, 0)), false, Scope::File),
        Some(("a.rs".into(), "a3".into()))
    );
    // A position between entries.
    assert_eq!(
        step(&store, (&a, pt(1, 3)), true, Scope::File),
        Some(("a.rs".into(), "a3".into()))
    );
    // Nothing but the origin: nothing to go to.
    store.clear_server(&k);
    store.publish(
        &k,
        params(&a, None, vec![lsp_diag((0, 1), (0, 2), ERR, "only")]),
        Utf16,
        None,
    );
    assert_eq!(step(&store, (&a, pt(0, 1)), true, Scope::File), None);
    assert_eq!(step(&store, (&b, pt(0, 0)), true, Scope::File), None);
}

#[test]
fn resolve_open_converts_raw_utf16() {
    let mut store = DiagnosticStore::default();
    let path = Path::new("/nonexistent/emoji.rs");
    // "😀" is 4 UTF-8 bytes and 2 UTF-16 units; the squiggle covers `b`.
    let text = "a😀b = 1;\n";
    store.publish(
        &key("ra"),
        params(path, None, vec![lsp_diag((0, 3), (0, 4), ERR, "bad b")]),
        Utf16,
        None,
    );
    // Unopened and unreadable: nothing to convert yet.
    assert!(store.for_file(path).is_empty());
    assert_eq!(store.summary(), (1, 0));

    store.resolve_open(path, &snap(text));
    let d = &store.for_file(path)[0];
    assert_eq!(d.range, pt(0, 5)..pt(0, 6));
    assert_eq!(
        &text[d.range.start.column as usize..d.range.end.column as usize],
        "b"
    );

    // A UTF-8 server keeps byte columns.
    let path8 = Path::new("/nonexistent/u8.rs");
    store.publish(
        &key("clangd"),
        params(path8, None, vec![lsp_diag((0, 5), (0, 6), ERR, "b")]),
        Utf8,
        None,
    );
    store.resolve_open(path8, &snap(text));
    assert_eq!(store.for_file(path8)[0].range, pt(0, 5)..pt(0, 6));
}

#[test]
fn unopened_file_read_from_disk_when_needed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("emoji.rs");
    std::fs::write(&path, "a😀b\n").unwrap();
    let mut store = DiagnosticStore::default();
    store.publish(
        &key("ra"),
        params(&path, None, vec![lsp_diag((0, 3), (0, 4), ERR, "b")]),
        Utf16,
        None,
    );
    assert_eq!(store.for_file(&path)[0].range, pt(0, 5)..pt(0, 6));
}

#[test]
fn multiple_servers_merge_sorted() {
    let mut store = DiagnosticStore::default();
    let path = Path::new("/proj/a.rs");
    let s = snap("abcdef\n");
    store.publish(
        &key("a"),
        params(path, None, vec![lsp_diag((0, 4), (0, 5), ERR, "late")]),
        Utf16,
        Some((&s, 0)),
    );
    store.publish(
        &key("b"),
        params(path, None, vec![lsp_diag((0, 1), (0, 2), WARN, "early")]),
        Utf16,
        Some((&s, 0)),
    );
    let msgs: Vec<_> = store
        .for_file(path)
        .into_iter()
        .map(|d| d.message)
        .collect();
    assert_eq!(msgs, ["early", "late"]);
}

#[test]
fn clear_server_returns_paths() {
    let mut store = DiagnosticStore::default();
    let s = snap("abc\n");
    let a = Path::new("/proj/a.rs");
    let b = Path::new("/proj/b.rs");
    store.publish(
        &key("x"),
        params(b, None, vec![lsp_diag((0, 0), (0, 1), ERR, "1")]),
        Utf16,
        Some((&s, 0)),
    );
    store.publish(
        &key("x"),
        params(a, None, vec![lsp_diag((0, 0), (0, 1), ERR, "2")]),
        Utf16,
        Some((&s, 0)),
    );
    store.publish(
        &key("y"),
        params(a, None, vec![lsp_diag((0, 0), (0, 1), WARN, "3")]),
        Utf16,
        Some((&s, 0)),
    );
    let mut cleared = store.clear_server(&key("x"));
    cleared.sort();
    assert_eq!(cleared, vec![a.to_path_buf(), b.to_path_buf()]);
    assert_eq!(store.for_file(a).len(), 1);
    assert!(store.for_file(b).is_empty());
    assert_eq!(store.summary(), (0, 1));
    assert!(store.clear_server(&key("x")).is_empty());
}

/// Reads like the store does, remembering which files were read.
fn counting_reader(
    reads: &std::cell::RefCell<Vec<String>>,
) -> impl Fn(&Path) -> Option<BufferSnapshot> + '_ {
    move |path| {
        reads
            .borrow_mut()
            .push(path.file_name().unwrap().to_str().unwrap().to_owned());
        Buffer::load(path).ok().map(|buffer| buffer.snapshot())
    }
}

#[test]
fn files_scope_reads_only_what_it_needs() {
    let dir = tempfile::tempdir().unwrap();
    let path = |name: &str| dir.path().join(name);
    let mut store = DiagnosticStore::default();
    // a: hints only. b: a warning. c: an error. d: hints only. e: would
    // fail to read (it does not exist) and sits after the hit.
    for (name, severity) in [
        ("a.rs", HINT),
        ("b.rs", WARN),
        ("c.rs", ERR),
        ("d.rs", HINT),
    ] {
        std::fs::write(path(name), "xxxx\nyyyy\n").unwrap();
        store.publish(
            &key("ra"),
            params(
                &path(name),
                None,
                vec![lsp_diag((1, 0), (1, 1), severity, name)],
            ),
            Utf16,
            None,
        );
    }
    store.publish(
        &key("ra"),
        params(
            &path("e.rs"),
            None,
            vec![lsp_diag((0, 0), (0, 1), ERR, "e")],
        ),
        Utf16,
        None,
    );
    let reads = std::cell::RefCell::new(Vec::new());
    let reader = counting_reader(&reads);
    let name = |hit: Option<(PathBuf, Diagnostic)>| hit.map(|(_, d)| d.message);

    // From a (hints only: not read), forward: b is the first with a hit.
    let hit = store.next_with_reader((&path("a.rs"), pt(0, 0)), true, Scope::Files, &reader);
    assert_eq!(name(hit), Some("b.rs".into()));
    assert_eq!(reads.take(), ["b.rs"]);

    // From b forward: c. Neither d nor e is read.
    let hit = store.next_with_reader((&path("b.rs"), pt(1, 0)), true, Scope::Files, &reader);
    assert_eq!(name(hit), Some("c.rs".into()));
    assert_eq!(reads.take(), ["b.rs", "c.rs"]);

    // Backward from c: b. Neither a nor d is read.
    let hit = store.next_with_reader((&path("c.rs"), pt(1, 0)), false, Scope::Files, &reader);
    assert_eq!(name(hit), Some("b.rs".into()));
    assert_eq!(reads.take(), ["c.rs", "b.rs"]);

    // Wrapping forward from the last file with a hit.
    let hit = store.next_with_reader((&path("e.rs"), pt(0, 0)), true, Scope::Files, &reader);
    assert_eq!(name(hit), Some("b.rs".into()));
    assert_eq!(reads.take(), ["e.rs", "b.rs"]);
}

#[test]
fn same_start_diagnostics_have_a_fixed_order() {
    let a = Path::new("/nonexistent/a.rs");
    let snapshot = snap("0123456789\nabcdef\n");
    let diags = || {
        vec![
            lsp_diag((0, 2), (0, 6), WARN, "zz"),
            lsp_diag((0, 2), (0, 4), WARN, "b"),
            lsp_diag((0, 2), (0, 4), ERR, "z"),
            lsp_diag((0, 2), (0, 4), ERR, "a"),
            lsp_diag((1, 0), (1, 1), ERR, "later"),
        ]
    };
    let order = |split: bool| {
        let mut store = DiagnosticStore::default();
        let all = diags();
        let (first, second) = if split {
            (all[..2].to_vec(), all[2..].to_vec())
        } else {
            (all, Vec::new())
        };
        // Two servers: the order does not depend on who published.
        for (server, list) in [("ra", first), ("lint", second)] {
            store.publish(
                &key(server),
                params(a, Some(0), list),
                Utf16,
                Some((&snapshot, 0)),
            );
        }
        let messages: Vec<String> = store.for_file(a).into_iter().map(|d| d.message).collect();
        (store, messages)
    };
    let expected = ["a", "z", "b", "zz", "later"];
    let (store, messages) = order(false);
    assert_eq!(messages, expected);
    assert_eq!(order(true).1, expected);

    // Forward from before the group lands on its first; backward from
    // after it on its last.
    let step = |from, forward| {
        store
            .next((a, from), forward, Scope::File)
            .unwrap()
            .1
            .message
    };
    assert_eq!(step(pt(0, 0), true), "a");
    assert_eq!(step(pt(1, 0), false), "zz");
    // Wrapping lands on the same ends.
    assert_eq!(step(pt(1, 0), true), "a");
    assert_eq!(step(pt(0, 2), false), "later");
}
