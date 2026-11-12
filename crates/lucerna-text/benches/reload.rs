//! `reload_10mb`: reloading a 10 MB buffer whose tail changed: decode, line
//! diff and apply the edits, as `Documents` does.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lucerna_text::{Buffer, decode_file, diff_edits};
use std::time::Duration;

fn source(extra_tail: bool) -> Vec<u8> {
    let mut text = String::with_capacity(11 << 20);
    let mut i = 0u64;
    while text.len() < 10 << 20 {
        text.push_str(&format!(
            "pub fn generated_{i}(a: u64) -> u64 {{ a ^ {i} }}\n"
        ));
        i += 1;
    }
    if extra_tail {
        text.push_str("// changed on disk\n");
    }
    text.into_bytes()
}

fn reload_10mb(c: &mut Criterion) {
    let before = source(false);
    let after = source(true);
    let mut group = c.benchmark_group("reload_10mb");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function("reload", |b| {
        b.iter_batched(
            || Buffer::from_bytes(&before, false).unwrap(),
            |mut buffer| {
                let decoded = decode_file(&after, false).unwrap();
                let edits = diff_edits(&buffer.snapshot(), &decoded.text);
                black_box(buffer.edit(edits))
            },
            criterion::BatchSize::LargeInput,
        );
    });
    group.finish();
}

criterion_group!(benches, reload_10mb);
criterion_main!(benches);
