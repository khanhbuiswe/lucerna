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

/// `text` parsed as Markdown, then edited by replacing `at` with `with`:
/// the old state, the edited state, the new text, and a fresh parse of it.
fn markdown_edited(
    text: &str,
    at: std::ops::Range<usize>,
    with: &str,
) -> (SyntaxState, SyntaxState, BufferSnapshot, SyntaxState) {
    let registry = Arc::new(LanguageRegistry::bundled());
    let mut buffer = Buffer::from_bytes(text.as_bytes(), false).unwrap();
    let old = buffer.snapshot();
    let language = registry.language_by_name("markdown").unwrap();
    let mut state = SyntaxState::new(language, registry.clone()).unwrap();
    assert_eq!(state.parse(&old, None), ParseOutcome::Done);
    let patch = buffer.edit([(at, Arc::from(with))]);
    let new = buffer.snapshot();
    let edited = state.edited(&old, &new, &patch);
    let (fresh, _) = parsed_in(
        registry,
        "markdown",
        &new.text_for_range(0..new.len_bytes()),
    );
    (state, edited, new, fresh)
}

const MD: &str = "# T\n\n```rust\nfn main() {}\n```\n\nsee `code` and **bold**\n";

/// Final review, finding 4: an edit keeps the injected layers (a fenced
/// code block, inline code), shifted with the text, until the next parse.
#[test]
fn edited_keeps_children_shifted() {
    let (state, edited, new, fresh) = markdown_edited(MD, 0..0, "\n\n");
    assert!(edited.tree().is_some());
    assert_eq!(edited.layer_count(), state.layer_count());
    let shifted: Vec<Vec<_>> = state
        .layer_ranges()
        .into_iter()
        .map(|l| l.into_iter().map(|r| r.start + 2..r.end + 2).collect())
        .collect();
    let mut got = edited.layer_ranges();
    let mut want = shifted;
    got.sort_by_key(|l| l.first().map(|r| r.start));
    want.sort_by_key(|l| l.first().map(|r| r.start));
    assert_eq!(got, want);
    // The same colours as a fresh parse of the new text.
    assert_eq!(all_spans(&edited, &new), all_spans(&fresh, &new));
    let text = new.text_for_range(0..new.len_bytes());
    let at = text.find("fn").unwrap();
    assert!(
        captures_at(&all_spans(&edited, &new), at..at + 2)
            .iter()
            .any(|c| c.starts_with("keyword")),
    );
}

/// Typing inside an injected region keeps its colours; a reparse from the
/// edited state (which reuses the layer trees) matches a fresh parse.
#[test]
fn typing_inside_a_layer_keeps_it() {
    let at = MD.find("main").unwrap() + 2;
    let (state, edited, new, fresh) = markdown_edited(MD, at..at, "x");
    assert_eq!(edited.layer_count(), state.layer_count());
    let text = new.text_for_range(0..new.len_bytes());
    let fn_at = text.find("fn").unwrap();
    let spans = all_spans(&edited, &new);
    assert!(
        captures_at(&spans, fn_at..fn_at + 2)
            .iter()
            .any(|c| c.starts_with("keyword")),
        "{spans:?}"
    );
    let code = text.find("`code`").unwrap();
    assert!(
        spans
            .iter()
            .any(|s| s.range.start >= code && s.range.end <= code + 6),
        "inline code keeps its span: {spans:?}"
    );
    let mut reparsed = edited.fork();
    assert_eq!(reparsed.reparse(&new, None, None), ParseOutcome::Done);
    assert_eq!(reparsed.layer_count(), fresh.layer_count());
    assert_eq!(all_spans(&reparsed, &new), all_spans(&fresh, &new));
}

/// Deleting a whole injected region drops its layer.
#[test]
fn deleting_a_layer_drops_it() {
    let start = MD.find("```rust").unwrap();
    let end = MD.find("```\n\n").unwrap() + 4;
    let (state, edited, new, fresh) = markdown_edited(MD, start..end, "");
    assert!(edited.layer_count() < state.layer_count());
    let mut reparsed = edited.fork();
    assert_eq!(reparsed.reparse(&new, None, None), ParseOutcome::Done);
    assert_eq!(all_spans(&reparsed, &new), all_spans(&fresh, &new));
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

/// A layer build that runs out of budget keeps the layers built so far and
/// returns `TimedOut`; a parse without a budget then builds them all. The
/// child limit stands in for the clock, so this does not depend on timing.
#[test]
fn budget_bounds_child_layers() {
    let text = "```rust\nfn main() {}\n```\n\nsome *inline* text\n\n".repeat(400);
    let (full, snap) = parsed("markdown", &text);
    let all = full.layer_count();
    assert_eq!(all, 800);
    let registry = Arc::new(LanguageRegistry::bundled());
    let markdown = registry.language_by_name("markdown").unwrap();
    let mut state = SyntaxState::new(markdown, registry).unwrap();
    assert_eq!(
        state.parse_with_child_limit(&snap, 300),
        ParseOutcome::TimedOut
    );
    assert!(state.tree().is_some());
    assert!(state.layer_count() > 0);
    assert!(state.layer_count() < all);
    assert_eq!(state.layer_count(), 300);
    assert_eq!(state.parse(&snap, None), ParseOutcome::Done);
    assert_eq!(state.layer_count(), all);
}
