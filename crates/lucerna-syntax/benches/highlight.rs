//! `highlight_10mb_viewport`: highlight one screen of a 10 MB Rust file.
//! `highlight_markdown_viewport`: highlight one screen in the middle of a
//! Markdown file with inline markup and code fences (under the 1000
//! injected layer cap). The full parse is set up outside the measured loop.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lucerna_syntax::{LanguageRegistry, ParseOutcome, SyntaxState};
use lucerna_text::Buffer;
use std::path::Path;
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

criterion_group!(
    benches,
    highlight_10mb_viewport,
    highlight_markdown_viewport
);
criterion_main!(benches);

/// The registry injected language names resolve in.
fn bundled() -> std::sync::Arc<LanguageRegistry> {
    std::sync::Arc::new(LanguageRegistry::bundled())
}
