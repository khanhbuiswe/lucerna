//! `highlight_10mb_viewport`: highlight one screen of a 10 MB Rust file.
//! `highlight_markdown_viewport`: highlight one screen in the middle of a
//! Markdown file with inline markup and code fences (under the 1000
//! injected layer cap). The full parse is set up outside the measured loop.
//! `markdown_many_fences`: the same on a Markdown file made almost only of
//! code fences (several hundred injected layers, still under the cap).
//! `html_large_script_injection`: highlight a viewport inside a large HTML
//! `<script>` block (JavaScript injected into HTML).
//! `wasm_json_vs_native`: parse and highlight the same JSON with the bundled
//! native grammar and with the WASM build of it (recorded, no threshold).

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lucerna_syntax::{Language, LanguageRegistry, ParseOutcome, SyntaxState};
use lucerna_text::Buffer;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn source() -> String {
    let mut text = String::with_capacity(11 << 20);
    let mut i = 0u64;
    while text.len() < 10 << 20 {
        text.push_str(&format!(
            "pub fn generated_{i}(a: u64, b: u64) -> u64 {{\n    let c = a.wrapping_mul({i}) ^ b; // line {i}\n    c.rotate_left(7)\n}}\n\n"
        ));
        i += 1;
    }
    text
}

fn highlight_10mb_viewport(c: &mut Criterion) {
    let text = source();
    let snapshot = Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot();
    let language = LanguageRegistry::bundled()
        .language_for_path(Path::new("big.rs"))
        .unwrap();
    let mut state = SyntaxState::new(language, bundled()).unwrap();
    assert!(
        matches!(state.parse(&snapshot, None), ParseOutcome::Done),
        "parse of the 10 MB file did not finish"
    );
    let middle = snapshot.line_count() / 2;
    assert!(
        !state
            .highlights(&snapshot, middle..middle + 60, &|_| true)
            .is_empty(),
        "viewport highlights are empty: the bench would time a no-op"
    );

    let mut group = c.benchmark_group("highlight_10mb_viewport");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function("rows_60", |b| {
        b.iter(|| black_box(state.highlights(&snapshot, middle..middle + 60, &|_| true)));
    });
    group.finish();
}

fn markdown_source() -> String {
    let mut text = String::new();
    for i in 0..150 {
        text.push_str(&format!(
            "## Section {i}\n\nSome *emphasis* and **strong** text with `code` and a [link](https://example.com/{i}).\n\n- item one\n- item **two**\n\n```rust\nfn f{i}() -> u32 {{ {i} }}\n```\n\n"
        ));
    }
    text
}

fn highlight_markdown_viewport(c: &mut Criterion) {
    let text = markdown_source();
    let snapshot = Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot();
    let language = LanguageRegistry::bundled()
        .language_for_path(Path::new("big.md"))
        .unwrap();
    let mut state = SyntaxState::new(language, bundled()).unwrap();
    assert!(matches!(state.parse(&snapshot, None), ParseOutcome::Done));
    let middle = snapshot.line_count() / 2;
    assert!(
        !state
            .highlights(&snapshot, middle..middle + 60, &|_| true)
            .is_empty()
    );

    let mut group = c.benchmark_group("highlight_markdown_viewport");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function("rows_60", |b| {
        b.iter(|| black_box(state.highlights(&snapshot, middle..middle + 60, &|_| true)));
    });
    group.finish();
}

/// Highlight a viewport in the middle of `text`, parsed once outside the loop.
fn viewport_bench(c: &mut Criterion, group_name: &str, file: &str, text: &str) {
    let snapshot = Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot();
    let language = LanguageRegistry::bundled()
        .language_for_path(Path::new(file))
        .unwrap();
    let mut state = SyntaxState::new(language, bundled()).unwrap();
    assert!(matches!(state.parse(&snapshot, None), ParseOutcome::Done));
    let middle = snapshot.line_count() / 2;
    assert!(
        !state
            .highlights(&snapshot, middle..middle + 60, &|_| true)
            .is_empty(),
        "{group_name}: viewport highlights are empty"
    );

    let mut group = c.benchmark_group(group_name);
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function("rows_60", |b| {
        b.iter(|| black_box(state.highlights(&snapshot, middle..middle + 60, &|_| true)));
    });
    group.finish();
}

fn markdown_many_fences(c: &mut Criterion) {
    let mut text = String::new();
    for i in 0..400 {
        text.push_str(&format!(
            "```rust\nfn f{i}() -> u32 {{ {i} }}\n```\n\n```json\n{{\"k{i}\": [1, 2, {i}]}}\n```\n\n"
        ));
    }
    viewport_bench(c, "markdown_many_fences", "fences.md", &text);
}

fn html_large_script_injection(c: &mut Criterion) {
    let mut text = String::from("<!doctype html>\n<html>\n<body>\n<script>\n");
    for i in 0..8000 {
        text.push_str(&format!(
            "function f{i}(a, b) {{ const c = a * {i} + b; return `v${{c}}` + \"s{i}\"; }}\n"
        ));
    }
    text.push_str("</script>\n</body>\n</html>\n");
    viewport_bench(c, "html_large_script_injection", "big.html", &text);
}

fn json_source() -> String {
    let mut text = String::from("[\n");
    for i in 0..4000 {
        text.push_str(&format!(
            "  {{\"id\": {i}, \"name\": \"item {i}\", \"tags\": [\"a\", \"b\"], \"ok\": true, \"v\": {i}.5}},\n"
        ));
    }
    text.push_str("  null\n]\n");
    text
}

/// Parse + highlight the same JSON natively and through the WASM grammar.
fn wasm_json_vs_native(c: &mut Criterion) {
    let text = json_source();
    let snapshot = Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot();
    let native = LanguageRegistry::bundled()
        .language_for_path(Path::new("x.json"))
        .unwrap();
    let wasm_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm/json.wasm");
    let wasm =
        Language::from_wasm_for_test("bench-json", &wasm_path, tree_sitter_json::HIGHLIGHTS_QUERY);

    let mut group = c.benchmark_group("wasm_json_vs_native");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));
    for (name, language) in [("native", native), ("wasm", wasm)] {
        // Grammar load (Cranelift compile for WASM) stays outside the loop.
        let mut warm = SyntaxState::new(language.clone(), bundled()).expect("grammar loads");
        assert!(matches!(warm.parse(&snapshot, None), ParseOutcome::Done));
        group.bench_function(name, |b| {
            b.iter(|| {
                let mut state = SyntaxState::new(language.clone(), bundled()).unwrap();
                assert!(matches!(state.parse(&snapshot, None), ParseOutcome::Done));
                black_box(state.highlights(&snapshot, 0..60, &|_| true))
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    highlight_10mb_viewport,
    highlight_markdown_viewport,
    markdown_many_fences,
    html_large_script_injection,
    wasm_json_vs_native
);
criterion_main!(benches);

/// The registry injected language names resolve in.
fn bundled() -> std::sync::Arc<LanguageRegistry> {
    std::sync::Arc::new(LanguageRegistry::bundled())
}
