//! `highlight_10mb_viewport`: highlight one screen of a 10 MB Rust file.
//! The full parse is set up outside the measured loop.

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
    let mut state = SyntaxState::new(language);
    assert!(
        matches!(state.parse(&snapshot, None), ParseOutcome::Done),
        "parse of the 10 MB file did not finish"
    );
    let middle = snapshot.line_count() / 2;
    assert!(
        !state.highlights(&snapshot, middle..middle + 60).is_empty(),
        "viewport highlights are empty: the bench would time a no-op"
    );

    let mut group = c.benchmark_group("highlight_10mb_viewport");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function("rows_60", |b| {
        b.iter(|| black_box(state.highlights(&snapshot, middle..middle + 60)));
    });
    group.finish();
}

criterion_group!(benches, highlight_10mb_viewport);
criterion_main!(benches);
