use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use lucerna_syntax::{HighlightSpan, Language, LanguageRegistry, ParseOutcome, SyntaxState};
use lucerna_text::{Buffer, BufferSnapshot, decode_file, diff_edits};

fn snapshot(text: &str) -> BufferSnapshot {
    Buffer::from_bytes(text.as_bytes(), false)
        .expect("text decodes")
        .snapshot()
}

fn language(path: &str) -> Arc<Language> {
    LanguageRegistry::bundled()
        .language_for_path(Path::new(path))
        .unwrap_or_else(|| panic!("no language for {path}"))
}

fn parsed(path: &str, text: &str) -> (SyntaxState, BufferSnapshot) {
    let snap = snapshot(text);
    let mut state = SyntaxState::new(language(path), bundled()).unwrap();
    assert_eq!(state.parse(&snap, None), ParseOutcome::Done);
    (state, snap)
}

fn id_for(registry: &LanguageRegistry, path: &str) -> Option<std::sync::Arc<str>> {
    registry
        .language_for_path(Path::new(path))
        .map(|l| l.id.clone())
}

#[test]
fn detects_by_extension_and_filename() {
    let registry = LanguageRegistry::bundled();
    let cases = [
        ("src/main.rs", "rust"),
        ("a.ts", "typescript"),
        ("a.mts", "typescript"),
        ("a.cts", "typescript"),
        ("a.tsx", "typescriptreact"),
        ("a.js", "javascript"),
        ("a.mjs", "javascript"),
        ("a.cjs", "javascript"),
        ("a.jsx", "javascript"),
        ("a.py", "python"),
        ("a.pyi", "python"),
        ("a.go", "go"),
        ("a.json", "json"),
        ("a.jsonc", "json"),
        ("proj/tsconfig.json", "json"),
        ("settings.json", "json"),
        ("README.md", "markdown"),
        ("a.markdown", "markdown"),
        ("Cargo.toml", "toml"),
        ("Cargo.lock", "toml"),
        ("ci.yml", "yaml"),
        ("ci.yaml", "yaml"),
        ("run.sh", "shellscript"),
        ("run.bash", "shellscript"),
        ("run.zsh", "shellscript"),
        ("/Users/me/.zshrc", "shellscript"),
        (".bashrc", "shellscript"),
        ("index.html", "html"),
        ("index.htm", "html"),
        ("style.css", "css"),
        ("UPPER.RS", "rust"),
    ];
    for (path, id) in cases {
        assert_eq!(id_for(&registry, path), Some(id.into()), "{path}");
    }
    let rust = registry.language_for_path(Path::new("x.rs")).unwrap();
    assert_eq!(&*rust.name, "Rust");
}

#[test]
fn unknown_extension_none() {
    let registry = LanguageRegistry::bundled();
    for path in ["a.unknownext", "Makefile", "noext", "a.lock"] {
        assert_eq!(id_for(&registry, path), None, "{path}");
    }
}

#[test]
fn rust_keyword_highlighted() {
    let (state, snap) = parsed("main.rs", "fn main() {}");
    let spans = state.highlights(&snap, 0..1, &|_| true);
    assert!(
        spans
            .iter()
            .any(|s| s.range.start == 0 && s.range.end >= 2 && s.capture.starts_with("keyword")),
        "{spans:?}"
    );
}

#[test]
fn highlights_empty_without_tree() {
    let snap = snapshot("fn main() {}");
    let state = SyntaxState::new(language("main.rs"), bundled()).unwrap();
    assert!(state.highlights(&snap, 0..1, &|_| true).is_empty());
}

#[test]
fn every_bundled_language_parses_fixture() {
    let registry = LanguageRegistry::bundled();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut covered = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            continue; // e.g. `wasm/`: grammar fixtures, not sample sources
        }
        let lang = registry
            .language_for_path(&path)
            .unwrap_or_else(|| panic!("no language for fixture {}", path.display()));
        let text = std::fs::read_to_string(&path).unwrap();
        let snap = snapshot(&text);
        let mut state = SyntaxState::new(lang.clone(), bundled()).unwrap();
        assert_eq!(state.parse(&snap, None), ParseOutcome::Done, "{}", lang.id);
        let root = state.tree().expect("tree after Done").root_node();
        assert!(
            !root.has_error(),
            "{} fixture has a parse error: {}",
            lang.id,
            root.to_sexp()
        );
        let spans = state.highlights(&snap, 0..snap.line_count(), &|_| true);
        assert!(!spans.is_empty(), "{} gave no spans", lang.id);
        covered.push(lang.id.clone());
    }
    covered.sort_unstable();
    // Hidden languages (e.g. `markdown_inline`) are only injected.
    let mut all: Vec<_> = registry
        .languages()
        .iter()
        .filter(|l| !l.hidden())
        .map(|l| l.id.clone())
        .collect();
    all.sort_unstable();
    assert_eq!(
        covered, all,
        "every bundled language needs exactly one fixture"
    );
}

#[test]
fn markdown_inline_emphasis_highlighted() {
    let text = "# T\n\n**bold**";
    let (state, snap) = parsed("a.md", text);
    let strong = text.find("**bold**").unwrap()..text.len();
    let bold = strong.start + 2..strong.end - 2;
    let spans = state.highlights(&snap, 0..snap.line_count(), &|_| true);
    // Only the inline layer highlights inside the paragraph `**bold**`.
    assert!(
        spans.iter().any(|s| s.range.start < bold.end
            && s.range.end > bold.start
            && s.range.start >= strong.start
            && s.range.end <= strong.end),
        "{spans:?}"
    );
}

#[test]
fn highlights_limited_to_rows() {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.rs"),
    )
    .unwrap();
    let (state, snap) = parsed("sample.rs", &text);
    let rows = 5..9;
    let spans = state.highlights(&snap, rows.clone(), &|_| true);
    assert!(!spans.is_empty());
    let first = snap.point_to_offset(lucerna_text::Point {
        row: rows.start,
        column: 0,
    });
    let end = snap.point_to_offset(lucerna_text::Point {
        row: rows.end,
        column: 0,
    });
    for s in &spans {
        let row = snap.offset_to_point(s.range.start).row;
        assert!(
            row < rows.end,
            "span starts after last requested row: {s:?}"
        );
        assert!(
            s.range.start >= first && s.range.end <= end,
            "{s:?} outside {first}..{end}"
        );
    }
}

#[test]
fn minified_line_spans_capped() {
    let unit = "var a=[1,\"s\",{b:2}];";
    let text = unit.repeat(5 * 1024 * 1024 / unit.len());
    assert!(text.len() > 5_000_000 && !text.contains('\n'));
    let (state, snap) = parsed("min.js", &text);
    let spans = state.highlights(&snap, 0..1, &|_| true);
    assert!(!spans.is_empty());
    assert!(
        spans.iter().all(|s| s.range.end <= 10_000),
        "a span passes byte 10000"
    );
}

#[test]
fn timeout_keeps_old_tree() {
    let unit = "fn f(x: u32) -> u32 { let y = x * 2; if y > 3 { y } else { x } }\n";
    let text = unit.repeat(40_000);
    let (mut state, snap) = parsed("big.rs", &text);
    let before = state.highlights(&snap, 0..10, &|_| true);
    assert!(!before.is_empty());
    assert_eq!(
        state.parse(&snap, Some(Duration::ZERO)),
        ParseOutcome::TimedOut
    );
    assert_eq!(state.highlights(&snap, 0..10, &|_| true), before);
    // The parser is usable again after a cancelled parse.
    assert_eq!(state.parse(&snap, None), ParseOutcome::Done);
}

#[test]
fn rust_fixture_spans() {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.rs"),
    )
    .unwrap();
    let (state, snap) = parsed("sample.rs", &text);
    let spans: Vec<(HighlightSpan, &str)> = state
        .highlights(&snap, 0..snap.line_count(), &|_| true)
        .into_iter()
        .map(|s| {
            let t = &text[s.range.clone()];
            (s, t)
        })
        .collect();
    insta::assert_debug_snapshot!(spans);
}

/// Until a reload is parsed again, the old tree is edited to the new text:
/// rows above the change keep their colours, and a shorter text never
/// sends the old tree's ranges past its end.
#[test]
fn edited_state_highlights_the_new_text_until_reparsed() {
    let old_text = "fn a() {}\nfn b() { let x = \"long string here\"; }\n";
    let new_text = "fn a() {}\n";
    let mut buffer = Buffer::from_bytes(old_text.as_bytes(), false).unwrap();
    let old = buffer.snapshot();
    let mut state = SyntaxState::new(language("a.rs"), bundled()).unwrap();
    assert_eq!(state.parse(&old, None), ParseOutcome::Done);
    let decoded = decode_file(new_text.as_bytes(), false).unwrap();
    let patch = buffer.edit(diff_edits(&old, &decoded.text));
    let new = buffer.snapshot();

    let edited = state.edited(&old, &new, &patch);
    assert!(edited.tree().is_some());
    let spans = edited.highlights(&new, 0..new.line_count(), &|_| true);
    assert!(
        spans
            .iter()
            .any(|s| s.range == (0..2) && &*s.capture == "keyword"),
        "row 0 keeps its colours: {spans:?}"
    );
    assert!(spans.iter().all(|s| s.range.end <= new.len_bytes()));
    // Brackets of the edited tree stay inside the new text too.
    if let Some(at) = edited.matching_bracket(&new, 7) {
        assert!(at <= new.len_bytes());
    }
}

/// Reloads `old_text` as `new_text` and highlights the edited state.
fn highlights_after_reload(old_text: &str, new_text: &str) -> (Vec<HighlightSpan>, usize) {
    let mut buffer = Buffer::from_bytes(old_text.as_bytes(), false).unwrap();
    let old = buffer.snapshot();
    let mut state = SyntaxState::new(language("a.rs"), bundled()).unwrap();
    assert_eq!(state.parse(&old, None), ParseOutcome::Done);
    let decoded = decode_file(new_text.as_bytes(), false).unwrap();
    let patch = buffer.edit(diff_edits(&old, &decoded.text));
    let new = buffer.snapshot();
    let edited = state.edited(&old, &new, &patch);
    let spans = edited.highlights(&new, 0..new.line_count(), &|_| true);
    (spans, new.len_bytes())
}

/// R30: a reload with no changed rows but another byte length (CRLF to LF,
/// a removed final newline) must still give tree-sitter a valid edit.
#[test]
fn eol_only_reload_keeps_the_tree_usable() {
    let lf = "fn a() {}\nfn b() {}\n";
    let crlf = lf.replace('\n', "\r\n");
    for (old_text, new_text) in [
        (crlf.as_str(), lf),
        (lf, crlf.as_str()),
        (lf, "fn a() {}\nfn b() {}"),
        ("fn a() {}\nfn b() {}", lf),
    ] {
        let (spans, len) = highlights_after_reload(old_text, new_text);
        assert!(!spans.is_empty(), "{old_text:?} -> {new_text:?}");
        assert!(spans.iter().all(|s| s.range.end <= len));
    }
}

/// Past the cap the edited state shows plain text rather than editing
/// thousands of times.
#[test]
fn mass_edit_patch_drops_the_tree() {
    let text = "fn a() {}\n".repeat(1200);
    let mut buffer = Buffer::from_bytes(text.as_bytes(), false).unwrap();
    let old = buffer.snapshot();
    let mut state = SyntaxState::new(language("a.rs"), bundled()).unwrap();
    assert_eq!(state.parse(&old, None), ParseOutcome::Done);
    let patch = buffer.edit((0..1200).map(|i| (i * 10..i * 10, "//".into())));
    assert!(patch.edits().len() > 1000);
    let new = buffer.snapshot();
    let edited = state.edited(&old, &new, &patch);
    assert!(edited.tree().is_none());
    assert!(
        edited
            .highlights(&new, 0..new.line_count(), &|_| true)
            .is_empty()
    );
}

/// The registry injected language names resolve in.
fn bundled() -> std::sync::Arc<LanguageRegistry> {
    std::sync::Arc::new(LanguageRegistry::bundled())
}

/// An insert at the top shifts the old tree: `b` is a function name on
/// row 2 before any reparse.
#[test]
fn edited_tree_shifts_highlights_after_insert() {
    let mut buffer = Buffer::from_bytes(b"fn a() {}\nfn b() {}", false).unwrap();
    let old = buffer.snapshot();
    let mut state = SyntaxState::new(language("a.rs"), bundled()).unwrap();
    assert_eq!(state.parse(&old, None), ParseOutcome::Done);
    let patch = buffer.edit([(0..0, "// x\n".into())]);
    let new = buffer.snapshot();
    let edited = state.edited(&old, &new, &patch);
    let b = new.len_bytes() - "b() {}".len();
    let spans = edited.highlights(&new, 2..3, &|_| true);
    assert!(
        spans
            .iter()
            .any(|s| s.range == (b..b + 1) && s.capture.starts_with("function")),
        "`b` is a function name on row 2: {spans:?}"
    );
}

/// The id of the last top-level node of `state`'s tree.
fn last_item_id(state: &SyntaxState) -> usize {
    let root = state.tree().expect("parsed").root_node();
    root.child(root.child_count() - 1).expect("an item").id()
}

/// A keystroke near the top of a 10k-line file reparses incrementally:
/// the items below the edit keep their nodes (tree-sitter reuses them),
/// where a from-scratch parse builds new ones.
#[test]
fn reparse_reuses_untouched_subtrees() {
    let text = "fn f(x: u32) -> u32 { x }\n".repeat(10_000);
    let mut buffer = Buffer::from_bytes(text.as_bytes(), false).unwrap();
    let old = buffer.snapshot();
    let mut state = SyntaxState::new(language("a.rs"), bundled()).unwrap();
    assert_eq!(state.parse(&old, None), ParseOutcome::Done);
    let before = last_item_id(&state);
    let patch = buffer.edit([(3..3, "g".into())]);
    let new = buffer.snapshot();

    let mut edited = state.edited(&old, &new, &patch).fork();
    assert_eq!(edited.reparse(&new, None, None), ParseOutcome::Done);
    let tree = edited.tree().unwrap();
    assert!(!tree.root_node().has_changes(), "the edit was parsed");
    assert_eq!(last_item_id(&edited), before, "reused, not rebuilt");

    let (full, _) = parsed("a.rs", &new.text_for_range(0..new.len_bytes()));
    assert_ne!(last_item_id(&full), before, "a full parse rebuilds it");
    assert_eq!(
        tree.root_node().to_sexp(),
        full.tree().unwrap().root_node().to_sexp(),
        "the same tree as a full parse"
    );
}

/// A set cancel flag stops a parse like a spent budget: the old tree stays.
#[test]
fn cancelled_reparse_keeps_the_old_tree() {
    let text = "fn f() {}\n".repeat(1000);
    let (mut state, snap) = parsed("a.rs", &text);
    let before = last_item_id(&state);
    let cancel = std::sync::atomic::AtomicBool::new(true);
    assert_eq!(
        state.reparse(&snap, None, Some(&cancel)),
        ParseOutcome::TimedOut
    );
    assert_eq!(last_item_id(&state), before);
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        state.reparse(&snap, None, Some(&cancel)),
        ParseOutcome::Done
    );
}
