use std::path::{Path, PathBuf};
use std::sync::Arc;

use lucerna_syntax::{
    Associations, ExtensionLanguage, HighlightSpan, LanguageRegistry, ParseOutcome, SyntaxState,
};
use lucerna_text::{Buffer, BufferSnapshot};

fn snapshot(text: &str) -> BufferSnapshot {
    Buffer::from_bytes(text.as_bytes(), false)
        .expect("text decodes")
        .snapshot()
}

fn parsed_in(
    registry: Arc<LanguageRegistry>,
    name: &str,
    text: &str,
) -> (SyntaxState, BufferSnapshot) {
    let language = registry
        .language_by_name(name)
        .unwrap_or_else(|| panic!("no language {name}"));
    let snap = snapshot(text);
    let mut state = SyntaxState::new(language, registry).unwrap();
    assert_eq!(state.parse(&snap, None), ParseOutcome::Done);
    (state, snap)
}

fn parsed(name: &str, text: &str) -> (SyntaxState, BufferSnapshot) {
    parsed_in(Arc::new(LanguageRegistry::bundled()), name, text)
}

fn all_spans(state: &SyntaxState, snap: &BufferSnapshot) -> Vec<HighlightSpan> {
    state.highlights(snap, 0..snap.line_count(), &|_| true)
}

/// The captures of spans covering exactly `range`.
fn captures_at(spans: &[HighlightSpan], range: std::ops::Range<usize>) -> Vec<String> {
    spans
        .iter()
        .filter(|s| s.range == range)
        .map(|s| s.capture.to_string())
        .collect()
}

fn json_wasm() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm/json.wasm")
}

/// An extension language on the JSON WASM grammar with the given queries.
fn json_extension(name: &str, injections: &str) -> ExtensionLanguage {
    json_extension_with(name, "(string) @string", injections)
}

fn json_extension_with(name: &str, highlights: &str, injections: &str) -> ExtensionLanguage {
    let dir =
        std::env::temp_dir().join(format!("lucerna-injection-{}-{}", std::process::id(), name));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("highlights.scm"), highlights).unwrap();
    std::fs::write(dir.join("injections.scm"), injections).unwrap();
    ExtensionLanguage {
        extension_id: format!("test-{name}"),
        name: name.to_string(),
        dir,
        grammar: Some(("json".to_string(), json_wasm())),
        path_suffixes: vec![name.to_lowercase()],
        first_line_pattern: None,
        hidden: false,
    }
}

#[test]
fn html_script_and_style_injected() {
    let text = "<script>let x = 1;</script><style>a{color:red}</style>";
    let (state, snap) = parsed("html", text);
    let spans = all_spans(&state, &snap);
    let let_at = text.find("let").unwrap();
    assert!(
        captures_at(&spans, let_at..let_at + 3)
            .iter()
            .any(|c| c.starts_with("keyword")),
        "{spans:?}"
    );
    let color = text.find("color").unwrap();
    assert!(
        captures_at(&spans, color..color + 5)
            .iter()
            .any(|c| c.starts_with("property")),
        "{spans:?}"
    );
    assert_eq!(state.layer_count(), 2);
}

#[test]
fn markdown_inline_still_highlighted() {
    let text = "# T\n\n**bold**";
    let (state, snap) = parsed("markdown", text);
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
fn markdown_code_fence_injects_rust() {
    let text = "```rust\nfn main() {}\n```";
    let (state, snap) = parsed("markdown", text);
    let spans = all_spans(&state, &snap);
    let at = text.find("fn").unwrap();
    assert!(
        captures_at(&spans, at..at + 2)
            .iter()
            .any(|c| c.starts_with("keyword")),
        "{spans:?}"
    );
}

#[test]
fn markdown_inline_snapshot_matches_phase1() {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.md"),
    )
    .unwrap();
    let (state, snap) = parsed("markdown", &text);
    let spans: Vec<(HighlightSpan, &str)> = all_spans(&state, &snap)
        .into_iter()
        .map(|s| {
            let t = &text[s.range.clone()];
            (s, t)
        })
        .collect();
    insta::assert_debug_snapshot!(spans);
}

#[test]
fn unknown_injected_language_is_skipped() {
    let text = "```nosuchlang\nfoo bar\n```\n";
    let (state, snap) = parsed("markdown", text);
    assert_eq!(state.layer_count(), 0);
    let _ = all_spans(&state, &snap);
}

#[test]
fn combined_injection_makes_one_layer() {
    // The root colours only numbers: string colours come from the layer.
    let spec = json_extension_with(
        "Combo",
        "(number) @number",
        "((string) @injection.content\n (#set! injection.language \"json\")\n (#set! injection.combined))",
    );
    let registry = Arc::new(LanguageRegistry::build(&[spec], Associations::default()));
    let text = r#"["a", "b", "c"]"#;
    let (state, snap) = parsed_in(registry, "combo", text);
    assert_eq!(state.layer_count(), 1);
    let strings: Vec<_> = text.match_indices('"').map(|(i, _)| i).collect();
    let expected: Vec<_> = strings.chunks(2).map(|p| p[0]..p[1] + 1).collect();
    assert_eq!(state.layer_ranges(), vec![expected.clone()]);
    let spans = all_spans(&state, &snap);
    for range in expected {
        assert!(
            captures_at(&spans, range.clone())
                .iter()
                .any(|c| c == "string"),
            "{range:?}: {spans:?}"
        );
    }
}

#[test]
fn depth_limit_stops_at_four() {
    let spec = json_extension(
        "Selfinj",
        "((string) @injection.content (#set! injection.language \"selfinj\"))",
    );
    let registry = Arc::new(LanguageRegistry::build(&[spec], Associations::default()));
    let (state, snap) = parsed_in(registry, "selfinj", r#""a""#);
    assert_eq!(state.max_layer_depth(), 4);
    assert!(state.layer_count() <= 4);
    let _ = all_spans(&state, &snap);
}

#[test]
fn layer_cap_1000() {
    let text = "```rust\nfn main() {}\n```\n\n".repeat(1500);
    let (state, _) = parsed("markdown", &text);
    assert_eq!(state.layer_count(), 1000);
}

#[test]
fn edited_drops_children_until_reparse() {
    let old_text = "# T\n\n```rust\nfn main() {}\n```\n";
    let new_text = "# T2\n\n```rust\nfn main() {}\n```\n";
    let registry = Arc::new(LanguageRegistry::bundled());
    let mut buffer = Buffer::from_bytes(old_text.as_bytes(), false).unwrap();
    let old = buffer.snapshot();
    let language = registry.language_by_name("markdown").unwrap();
    let mut state = SyntaxState::new(language, registry).unwrap();
    assert_eq!(state.parse(&old, None), ParseOutcome::Done);
    assert!(state.layer_count() > 0);
    let reload = buffer.reload(new_text.as_bytes()).unwrap();
    let new = buffer.snapshot();
    let mut edited = state.edited(&old, &new, &reload);
    assert!(edited.tree().is_some());
    assert_eq!(edited.layer_count(), 0);
    assert_eq!(edited.parse(&new, None), ParseOutcome::Done);
    assert_eq!(edited.layer_count(), state.layer_count());
}

/// With a budget (the sync attempt on the UI thread), an injected WASM
/// grammar that is not loaded yet is not loaded there: the parse times out
/// so the caller finishes it in the background, where it loads.
#[test]
fn unloaded_wasm_child_defers_to_unbudgeted_parse() {
    let spec = json_extension("Deferred", "");
    let registry = Arc::new(LanguageRegistry::build(&[spec], Associations::default()));
    let child = registry.language_by_name("deferred").unwrap();
    assert!(!child.grammar_ready());
    let markdown = registry.language_by_name("markdown").unwrap();
    let snap = snapshot("```deferred\n[\"a\"]\n```\n");
    let mut state = SyntaxState::new(markdown, registry).unwrap();
    assert_eq!(
        state.parse(&snap, Some(std::time::Duration::from_secs(60))),
        ParseOutcome::TimedOut
    );
    assert!(state.tree().is_some());
    assert_eq!(state.layer_count(), 0);
    assert!(!child.grammar_ready());
    assert_eq!(state.parse(&snap, None), ParseOutcome::Done);
    assert_eq!(state.layer_count(), 1);
}

/// Markdown's own inline layers do not count toward the 1000-layer cap:
/// the paragraphs after 1500 fences keep their inline colours, while the
/// fences past the first 1000 stay uninjected.
#[test]
fn markdown_inline_layers_are_not_capped() {
    let fences = "```rust\nfn main() {}\n```\n\n".repeat(1500);
    let paragraphs = "plain *em* text\n\n".repeat(1200);
    let text = format!("{fences}{paragraphs}");
    let (state, snap) = parsed("markdown", &text);
    assert_eq!(state.layer_count(), 1000 + 1200);
    let spans = all_spans(&state, &snap);
    let last_em = text.rfind("*em*").unwrap();
    assert!(
        spans
            .iter()
            .any(|s| s.range.start >= last_em && s.range.end <= last_em + 4),
        "the last paragraph has inline spans"
    );
    let last_fn = text.rfind("fn main").unwrap();
    assert!(captures_at(&spans, last_fn..last_fn + 2).is_empty());
}

/// A fence tag of multi-byte chars longer than the name limit must not be
/// cut mid-char.
#[test]
fn long_multibyte_fence_language_does_not_panic() {
    let text = format!("```{}\nx\n```\n", "€".repeat(50));
    let (state, snap) = parsed("markdown", &text);
    assert_eq!(state.layer_count(), 0);
    let _ = all_spans(&state, &snap);
}

#[test]
fn unknown_predicate_disables_injection_pattern() {
    let spec = json_extension(
        "Unkpred",
        "((string) @injection.content (#set! injection.language \"json\") (#zed-only? @injection.content))",
    );
    let registry = Arc::new(LanguageRegistry::build(&[spec], Associations::default()));
    let (state, _) = parsed_in(registry, "unkpred", r#"["a"]"#);
    assert_eq!(state.layer_count(), 0);
}

/// Non-ASCII text before and inside injected regions: spans land on the
/// right bytes.
#[test]
fn multibyte_text_highlights_on_char_boundaries() {
    let text = "é€ intro *ém* ü\n\n```rust\n// ü\nfn ñ() {}\n```\n";
    let (state, snap) = parsed("markdown", text);
    let spans = all_spans(&state, &snap);
    for s in &spans {
        assert!(
            text.is_char_boundary(s.range.start) && text.is_char_boundary(s.range.end),
            "{s:?}"
        );
    }
    let at = text.find("fn").unwrap();
    assert!(
        captures_at(&spans, at..at + 2)
            .iter()
            .any(|c| c.starts_with("keyword")),
        "{spans:?}"
    );
    let em = text.find("*ém*").unwrap();
    assert!(
        spans
            .iter()
            .any(|s| s.range.start >= em && s.range.end <= em + "*ém*".len()),
        "{spans:?}"
    );
}

/// The sync budget covers child layers too: with a budget that fits the
/// root parse but not every child, the parse stops with some layers built,
/// and a parse without a budget then builds them all.
#[test]
fn budget_bounds_child_layers() {
    let text = "```rust\nfn main() {}\n```\n\nsome *inline* text\n\n".repeat(400);
    let (full, snap) = parsed("markdown", &text);
    let all = full.layer_count();
    assert_eq!(all, 800);
    let registry = Arc::new(LanguageRegistry::bundled());
    let markdown = registry.language_by_name("markdown").unwrap();
    let mut partial = None;
    let mut budget = std::time::Duration::from_micros(20);
    while budget < std::time::Duration::from_secs(10) {
        let mut state = SyntaxState::new(markdown.clone(), registry.clone()).unwrap();
        let outcome = state.parse(&snap, Some(budget));
        if outcome == ParseOutcome::Done {
            break;
        }
        if state.tree().is_some() && state.layer_count() < all {
            partial = Some(state);
            break;
        }
        budget = budget * 3 / 2;
    }
    let mut state = partial.expect("some budget parses the root but not every child");
    assert!(state.layer_count() < all);
    assert_eq!(state.parse(&snap, None), ParseOutcome::Done);
    assert_eq!(state.layer_count(), all);
}
