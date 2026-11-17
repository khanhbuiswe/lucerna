//! Typing (spec §15): one char into a 10,000-line file (target <= 8 ms),
//! and one char at 1,000 cursors (target <= 16 ms). Each builds the plan
//! with `commands::insert` (or `commands::newline`) and applies it to the
//! buffer, as the workbench does. The `rust_*` cases run on a parsed
//! 10,000-line Rust file with the bundled language's editing data, so
//! auto-close, overtype and closer indent see real scopes.

use std::sync::Arc;
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use lucerna_editor::{
    AutoClosing, AutoClosingEdit, AutoIndent, AutoSurround, Ctx, EditConfig, EditKind, EditPlan,
    Goal, MultiCursorPaste, Selection, SelectionSet, commands,
};
use lucerna_syntax::{Language, LanguageRegistry, ParseOutcome, SyntaxState};
use lucerna_text::{Buffer, Point};

/// 10,000 lines of Rust-like text.
fn buffer_10k_lines() -> Buffer {
    let text: String = (0..10_000)
        .map(|i| format!("    let value_{i} = compute({i}, \"text\"); // line {i}\n"))
        .collect();
    Buffer::from_bytes(text.as_bytes(), false).unwrap()
}

/// 10,000 lines of Rust: 2,500 functions of four lines each.
fn rust_text_10k_lines() -> String {
    (0..2_500)
        .map(|i| {
            format!(
                "fn f_{i}(x: u32) -> u32 {{\n    let value = compute(x, \"text\", {i});\n    value + 1\n}}\n"
            )
        })
        .collect()
}

/// The text parsed once; each run gets a fresh buffer of the same text.
struct Parsed {
    text: String,
    language: Arc<Language>,
    syntax: SyntaxState,
}

fn parsed_rust() -> Parsed {
    let text = rust_text_10k_lines();
    let registry = Arc::new(LanguageRegistry::bundled());
    let language = registry.language_by_name("Rust").unwrap();
    let mut syntax = SyntaxState::new(language.clone(), registry).unwrap();
    let buffer = Buffer::from_bytes(text.as_bytes(), false).unwrap();
    assert_eq!(syntax.parse(&buffer.snapshot(), None), ParseOutcome::Done);
    Parsed {
        text,
        language,
        syntax,
    }
}

/// A cursor at the end of each of `rows`.
fn cursors_at_rows(buffer: &Buffer, rows: impl Iterator<Item = u32>) -> SelectionSet {
    let snapshot = buffer.snapshot();
    let selections = rows
        .enumerate()
        .map(|(i, row)| {
            let at = snapshot.point_to_offset(Point {
                row,
                column: snapshot.line(row).len() as u32,
            });
            Selection {
                id: i as u64,
                anchor: at,
                head: at,
                goal: Goal::None,
            }
        })
        .collect();
    SelectionSet::from_selections(selections, 0, true)
}

/// Builds a plan on `parsed` with `build` and applies it to `buffer`.
fn apply_parsed(
    buffer: &mut Buffer,
    parsed: &Parsed,
    config: &EditConfig,
    build: impl FnOnce(Ctx) -> EditPlan,
) {
    let snapshot = buffer.snapshot();
    let ctx = Ctx {
        snapshot: &snapshot,
        config,
        language: Some(parsed.language.editing()),
        syntax: Some(&parsed.syntax),
    };
    let plan = build(ctx);
    buffer.start_transaction(None);
    buffer.edit(plan.edits);
    buffer.end_transaction(None);
}

fn config() -> EditConfig {
    EditConfig {
        tab_size: 4,
        insert_spaces: true,
        auto_indent: AutoIndent::Full,
        auto_closing_brackets: AutoClosing::LanguageDefined,
        auto_closing_quotes: AutoClosing::LanguageDefined,
        auto_closing_overtype: AutoClosingEdit::Auto,
        auto_closing_delete: AutoClosingEdit::Auto,
        auto_surround: AutoSurround::LanguageDefined,
        trim_auto_whitespace: true,
        word_separators: Arc::from("`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?"),
        empty_selection_clipboard: true,
        multi_cursor_paste: MultiCursorPaste::Spread,
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
        language: None,
        syntax: None,
    };
    let plan = commands::insert(ctx, set, "x", EditKind::TypingOther, &[]);
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
    let parsed = parsed_rust();
    let fresh = || Buffer::from_bytes(parsed.text.as_bytes(), false).unwrap();
    // Row 4k + 1 is `    let value = compute(x, "text", k);`, in code; the
    // 1,000 cursors sit on every other function's.
    let let_rows = |count: u32| (0..count).map(|i| i * 8 + 1);
    {
        // The quote case measures an auto-close, not a plain insert.
        let buffer = fresh();
        let set = cursors_at_rows(&buffer, std::iter::once(5_001));
        let snapshot = buffer.snapshot();
        let ctx = Ctx {
            snapshot: &snapshot,
            config: &config,
            language: Some(parsed.language.editing()),
            syntax: Some(&parsed.syntax),
        };
        let plan = commands::insert(ctx, &set, "\"", EditKind::TypingOther, &[]);
        assert_eq!(plan.autoclose.len(), 1, "the quote auto-closes");
    }
    let cases: [(&str, &str, u32); 3] = [
        ("rust_quote_10k_lines", "\"", 1),
        ("rust_close_paren_10k_lines", ")", 1),
        ("rust_close_paren_1000_cursors", ")", 1_000),
    ];
    for (name, ch, count) in cases {
        group.bench_function(name, |b| {
            b.iter_batched(
                || {
                    let buffer = fresh();
                    let set = if count == 1 {
                        cursors_at_rows(&buffer, std::iter::once(5_001))
                    } else {
                        cursors_at_rows(&buffer, let_rows(count))
                    };
                    (buffer, set)
                },
                |(mut buffer, set)| {
                    apply_parsed(&mut buffer, &parsed, &config, |ctx| {
                        commands::insert(ctx, &set, ch, EditKind::TypingOther, &[])
                    })
                },
                BatchSize::LargeInput,
            )
        });
    }
    // Row 4k is `fn f_k(x: u32) -> u32 {`.
    group.bench_function("rust_enter_after_brace_10k_lines", |b| {
        b.iter_batched(
            || {
                let buffer = fresh();
                let set = cursors_at_rows(&buffer, std::iter::once(5_000));
                (buffer, set)
            },
            |(mut buffer, set)| {
                apply_parsed(&mut buffer, &parsed, &config, |ctx| {
                    commands::newline(ctx, &set)
                })
            },
            BatchSize::LargeInput,
        )
    });
    group.finish();
}

criterion_group!(benches, typing);
criterion_main!(benches);
