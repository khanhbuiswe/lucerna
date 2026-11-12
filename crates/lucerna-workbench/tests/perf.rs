//! Keystroke latency (spec §15), measured headless: one char typed into a
//! 10,000-line Rust file, through the document (edit, syntax edit, every
//! view's update, the synchronous parse attempt), then the next frame's
//! draw. GPUI's test platform does not shape text, so the draw time leaves
//! out glyph shaping. Ignored by default; run in a release build:
//!
//! `cargo test --release -p lucerna-workbench --test perf -- --ignored --nocapture`

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, Workbench};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext) {
    let (keymap, _) = Keymap::load(Keymap::default_bindings(), None);
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
        languages_generation: 0,
        icon_themes: lucerna_theme::IconThemeRegistry::build(&[]).0,
        icons: lucerna_theme::IconThemeRegistry::build(&[])
            .0
            .resolve("Lucide"),
    });
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn active_view(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<CodeView> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("the active tab has a body")
            .downcast::<CodeView>()
            .expect("tab bodies are code views")
    })
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i]
}

fn report(name: &str, mut samples: Vec<Duration>) {
    samples.sort();
    println!(
        "{name}: p50 {:.3} ms, p95 {:.3} ms, max {:.3} ms ({} keystrokes)",
        percentile(&samples, 0.5).as_secs_f64() * 1e3,
        percentile(&samples, 0.95).as_secs_f64() * 1e3,
        samples.last().unwrap().as_secs_f64() * 1e3,
        samples.len()
    );
}

/// A workbench with a 10,000-line Rust file open, parsed, focused, and
/// its cursor on row 5,000.
fn rust_10k(cx: &mut TestAppContext) -> (TempDir, Entity<CodeView>, &mut VisualTestContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let text: String = (0..10_000)
        .map(|i| format!("fn f_{i}(x: u32) -> u32 {{ x + {i} }} // line {i}\n"))
        .collect();
    fs::write(dir.path().join("main.rs"), text).unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("main.rs")), OpenMode::Pinned, cx)
    });
    draw(cx);
    cx.run_until_parked();
    let view = active_view(&wb, cx);
    cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    view.update(cx, |v, cx| {
        v.go_to(
            Point {
                row: 5_000,
                column: 25,
            },
            cx,
        )
    });
    draw(cx);
    cx.run_until_parked();
    assert!(
        view.read_with(cx, |v, _| v.syntax().is_some()),
        "the file is parsed"
    );
    (dir, view, cx)
}

#[gpui::test]
#[ignore = "a measurement: run in a release build with --ignored"]
fn keystroke_latency_10k_line_rust_file(cx: &mut TestAppContext) {
    let (_dir, view, cx) = rust_10k(cx);
    let (mut edit, mut frame, mut total) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..300 {
        let text = if i % 10 == 9 { " " } else { "x" };
        let start = Instant::now();
        view.update(cx, |v, cx| v.type_text(text, cx));
        let typed = Instant::now();
        draw(cx);
        let drawn = Instant::now();
        edit.push(typed - start);
        frame.push(drawn - typed);
        total.push(drawn - start);
        // The background parse, folds and the like land between keys.
        cx.run_until_parked();
    }
    report("edit (document + views + sync parse)", edit);
    report("draw (test platform, no shaping)", frame);
    report("keystroke total", total);
}

#[gpui::test]
#[ignore = "a measurement: run in a release build with --ignored"]
fn keystroke_latency_1000_cursors(cx: &mut TestAppContext) {
    let (_dir, view, cx) = rust_10k(cx);
    let snapshot = view.read_with(cx, |v, _| v.snapshot().clone());
    let selections = (0..1_000u32)
        .map(|i| {
            let at = snapshot.point_to_offset(Point {
                row: i * 10,
                column: 0,
            });
            lucerna_editor::Selection {
                id: u64::from(i),
                anchor: at,
                head: at,
                goal: lucerna_editor::Goal::None,
            }
        })
        .collect();
    let set = lucerna_editor::SelectionSet::from_selections(selections, 0, true);
    view.update(cx, |v, cx| v.set_selections(set, false, cx));
    draw(cx);
    cx.run_until_parked();
    let (mut edit, mut total) = (Vec::new(), Vec::new());
    for _ in 0..30 {
        let start = Instant::now();
        view.update(cx, |v, cx| v.type_text("x", cx));
        edit.push(start.elapsed());
        draw(cx);
        total.push(start.elapsed());
        cx.run_until_parked();
    }
    report("1000 cursors edit", edit);
    report("1000 cursors keystroke total", total);
}

/// Smart typing (spec §15): brackets and quotes that auto-close and type
/// over, on the parsed 10,000-line file. Each round types `("")`, so every
/// key runs the scope check of auto-close or overtype.
#[gpui::test]
#[ignore = "a measurement: run in a release build with --ignored"]
fn keystroke_latency_smart_typing(cx: &mut TestAppContext) {
    let (_dir, view, cx) = rust_10k(cx);
    // After `{`, before a space: pairs auto-close there.
    view.update(cx, |v, cx| {
        v.go_to(
            Point {
                row: 5_000,
                column: 26,
            },
            cx,
        )
    });
    let mut total = Vec::new();
    for i in 0..300 {
        let text = ["(", "\"", "\"", ")"][i % 4];
        let start = Instant::now();
        view.update(cx, |v, cx| v.type_text(text, cx));
        draw(cx);
        total.push(start.elapsed());
        cx.run_until_parked();
    }
    let line = view.read_with(cx, |v, _| v.snapshot().line(5_000).to_string());
    assert!(
        line.contains("(\"\")(\"\")"),
        "auto-closed and typed over: {line}"
    );
    report("smart typing keystroke total", total);
}
