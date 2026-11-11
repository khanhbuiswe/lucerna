//! `edit_10k_inserts` and `undo_10k_edits` (spec §15: undo <= 50 ms).

use criterion::{Criterion, criterion_group, criterion_main};
use lucerna_text::Buffer;
use std::sync::Arc;
use std::time::Duration;

fn rope_10k_lines() -> Buffer {
    let text: String = (0..10_000).map(|i| format!("line number {i}\n")).collect();
    Buffer::from_bytes(text.as_bytes(), false).unwrap()
}

fn insert_10k(b: &mut Buffer) {
    let x: Arc<str> = Arc::from("x");
    b.start_transaction(None);
    for i in 0..10_000usize {
        // ASCII text: every offset is a char boundary.
        let at = (i * 7919) % (b.snapshot().len_bytes() + 1);
        b.edit([(at..at, x.clone())]);
    }
    b.end_transaction(None);
}

fn edits(c: &mut Criterion) {
    let mut group = c.benchmark_group("edit");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));
    group.bench_function("insert_10k_chars", |b| {
        b.iter_batched(
            rope_10k_lines,
            |mut buf| insert_10k(&mut buf),
            criterion::BatchSize::LargeInput,
        )
    });
    group.bench_function("undo_10k_edits", |b| {
        b.iter_batched(
            || {
                let mut buf = rope_10k_lines();
                insert_10k(&mut buf);
                buf
            },
            |mut buf| buf.undo().unwrap(),
            criterion::BatchSize::LargeInput,
        )
    });
    group.finish();
}

criterion_group!(benches, edits);
criterion_main!(benches);
