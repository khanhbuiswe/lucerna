//! Typing (spec §15): one char into a 10,000-line file, and one char at
//! 1,000 cursors (target <= 16 ms). Each builds the plan with
//! `commands::insert` and applies it to the buffer, as the workbench does.

use std::sync::Arc;
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use lucerna_editor::{Ctx, EditConfig, EditKind, Goal, Selection, SelectionSet, commands};
use lucerna_text::Buffer;

/// 10,000 lines of Rust-like text.
fn buffer_10k_lines() -> Buffer {
    let text: String = (0..10_000)
        .map(|i| format!("    let value_{i} = compute({i}, \"text\"); // line {i}\n"))
        .collect();
    Buffer::from_bytes(text.as_bytes(), false).unwrap()
}

fn config() -> EditConfig {
    EditConfig {
        tab_size: 4,
        insert_spaces: true,
        keep_indent: true,
        trim_auto_whitespace: true,
        word_separators: Arc::from("`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?"),
        empty_selection_clipboard: true,
    }
}

/// A cursor at the end of every `step`-th line, `count` of them.
fn cursors(buffer: &Buffer, count: u32, step: u32) -> SelectionSet {
    let snapshot = buffer.snapshot();
    let selections = (0..count)
        .map(|i| {
            let row = i * step;
            let at = snapshot.point_to_offset(lucerna_text::Point {
                row,
                column: snapshot.line(row).len() as u32,
            });
            Selection {
                id: u64::from(i),
                anchor: at,
                head: at,
                goal: Goal::None,
            }
        })
        .collect();
    SelectionSet::from_selections(selections, 0, true)
}

/// Builds the plan for typing `x` at `set` and applies it.
fn type_x(buffer: &mut Buffer, set: &SelectionSet, config: &EditConfig) {
    let snapshot = buffer.snapshot();
    let ctx = Ctx {
        snapshot: &snapshot,
        config,
    };
    let plan = commands::insert(ctx, set, "x", EditKind::TypingOther);
    buffer.start_transaction(None);
    buffer.edit(plan.edits);
    buffer.end_transaction(None);
}

fn typing(c: &mut Criterion) {
    let config = config();
    let mut group = c.benchmark_group("typing");
    group.measurement_time(Duration::from_secs(5));
    group.bench_function("insert_char_10k_lines", |b| {
        b.iter_batched(
            || {
                let buffer = buffer_10k_lines();
                let set = cursors(&buffer, 1, 5_000);
                (buffer, set)
            },
            |(mut buffer, set)| type_x(&mut buffer, &set, &config),
            BatchSize::LargeInput,
        )
    });
    group.bench_function("insert_char_1000_cursors", |b| {
        b.iter_batched(
            || {
                let buffer = buffer_10k_lines();
                let set = cursors(&buffer, 1_000, 10);
                (buffer, set)
            },
            |(mut buffer, set)| type_x(&mut buffer, &set, &config),
            BatchSize::LargeInput,
        )
    });
    group.finish();
}

criterion_group!(benches, typing);
criterion_main!(benches);
