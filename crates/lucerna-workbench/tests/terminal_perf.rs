//! Terminal drawing cost (spec §9.2): a 200×60 grid of coloured text,
//! `ls --color` style with the style changing every 3 cells, laid out and
//! shaped by the platform's real text system.
//!
//! It measures `layout_line` over the grid alone, then whole window draws
//! of a window holding only the terminal (render, layout, prepaint with
//! its snapshot and shaping, and paint), both with an unchanged screen and
//! with every row new each frame. The GPU's work is not included.
//!
//! Before measuring, and in every test run, it checks that text drawn from
//! cached glyphs sits on the baseline a shaped line would use.
//!
//! `TestAppContext`'s text system does not shape, and macOS creates the
//! real one only on the main thread, so this test has its own `main` (see
//! `Cargo.toml`). The measurement is skipped unless `--ignored` is given:
//!
//! `cargo test --release -p lucerna-workbench --test terminal_perf -- --ignored`

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, TestDispatcher, VisualTestContext, size};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_terminal::{Rgba8, Terminal, TerminalPalette, layout_line};
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, TerminalView, fonts, terminal_font};

const ROWS: u16 = 60;
const COLS: u16 = 200;
const FRAMES: usize = 200;
/// Spec §9.2: prepaint plus paint of this grid on an M1, release build.
const BUDGET_MS: f64 = 4.0;

fn init(cx: &mut TestAppContext) {
    let (keymap, _) = Keymap::load(Keymap::default_bindings(), None);
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
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
    cx.update(|cx| fonts::register(cx));
}

/// SGR styles cycled every 3 cells: colours, bold, italic, underline,
/// a background and the default.
const STYLES: [&str; 8] = [
    "\x1b[0;31m",
    "\x1b[0;1;32m",
    "\x1b[0;33m",
    "\x1b[0;4;34m",
    "\x1b[0;35;48;5;236m",
    "\x1b[0;3;36m",
    "\x1b[0m",
    "\x1b[0;1;37;44m",
];

/// One full screen; `seed` makes every row's text differ between screens.
fn screen(seed: usize) -> Vec<u8> {
    let mut out = String::from("\x1b[H");
    for row in 0..ROWS as usize {
        if row > 0 {
            out.push_str("\r\n");
        }
        for chunk in 0..(COLS as usize / 3 + 1) {
            let left = COLS as usize - chunk * 3;
            if left == 0 {
                break;
            }
            out.push_str(STYLES[(chunk + row) % STYLES.len()]);
            let n = (seed + row * 7 + chunk) % 1000;
            let text = format!("{n:03}");
            out.push_str(&text[..left.min(3)]);
        }
    }
    out.push_str("\x1b[0m");
    out.into_bytes()
}

fn palette() -> TerminalPalette {
    let c = |r, g, b| Rgba8::rgb(r, g, b);
    TerminalPalette {
        ansi: [
            c(0x00, 0x00, 0x00),
            c(0xcd, 0x31, 0x31),
            c(0x0d, 0xbc, 0x79),
            c(0xe5, 0xe5, 0x10),
            c(0x24, 0x72, 0xc8),
            c(0xbc, 0x3f, 0xbc),
            c(0x11, 0xa8, 0xcd),
            c(0xe5, 0xe5, 0xe5),
            c(0x66, 0x66, 0x66),
            c(0xf1, 0x4c, 0x4c),
            c(0x23, 0xd1, 0x8b),
            c(0xf5, 0xf5, 0x43),
            c(0x3b, 0x8e, 0xea),
            c(0xd6, 0x70, 0xd6),
            c(0x29, 0xb8, 0xdb),
            c(0xe5, 0xe5, 0xe5),
        ],
        dim: [c(0x80, 0x80, 0x80); 8],
        foreground: c(0xcc, 0xcc, 0xcc),
        bright_foreground: c(0xff, 0xff, 0xff),
        dim_foreground: c(0x88, 0x88, 0x88),
        background: c(0x1e, 0x1e, 0x1e),
        cursor: c(0xff, 0xff, 0xff),
        selection: c(0x26, 0x4f, 0x78),
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// p50, p95 and max in ms.
fn stats(mut samples: Vec<Duration>) -> (f64, f64, f64) {
    samples.sort();
    let at = |p: f64| ms(samples[((samples.len() as f64 - 1.) * p).round() as usize]);
    (at(0.5), at(0.95), ms(*samples.last().unwrap()))
}

fn report(name: &str, samples: Vec<Duration>) -> f64 {
    let (p50, p95, max) = stats(samples);
    println!("{name}: p50 {p50:.3} ms, p95 {p95:.3} ms, max {max:.3} ms ({FRAMES} frames)");
    p95
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn feed(view: &Entity<TerminalView>, bytes: &[u8], cx: &mut VisualTestContext) {
    view.read_with(cx, |v, _| v.terminal().feed(bytes));
}

/// Plain text's baseline matches `ShapedLine::paint`'s: centred in the
/// cell by the font's ascent and (positive) descent.
fn check_baseline(view: &Entity<TerminalView>, cx: &mut VisualTestContext) {
    let (baseline, cell) = view.read_with(cx, |v, _| (v.painted_baseline(), v.cell_bounds(0, 0)));
    let (baseline, cell) = (
        baseline.expect("a drawn frame"),
        cell.expect("a drawn cell"),
    );
    let layout = cx.update(|window, cx| {
        let font = terminal_font(cx);
        let runs = [gpui::TextRun {
            len: 1,
            font: gpui::Font {
                weight: font.weight,
                ..gpui::font(font.family.clone())
            },
            color: gpui::Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        window
            .text_system()
            .layout_line("m", font.size, &runs, None)
    });
    assert!(
        layout.descent >= gpui::px(0.),
        "a line's descent is positive"
    );
    let expected = (cell.size.height - layout.ascent - layout.descent) / 2. + layout.ascent;
    let diff = (f32::from(baseline) - f32::from(expected)).abs();
    assert!(
        diff < 0.01,
        "plain text baseline {baseline:?}, a shaped line's {expected:?}"
    );
    println!("terminal_perf: baseline matches shaped text: ok");
}

fn main() {
    let measure = std::env::args().any(|a| a == "--ignored" || a == "--include-ignored");
    let text_system = gpui_platform::current_platform(true).text_system();
    let mut app = TestAppContext::build_with_text_system(TestDispatcher::new(0), None, text_system);
    init(&mut app);

    let (view, cx) = app.add_window_view(|window, cx| {
        let (terminal, events) = Terminal::fake_with_events(ROWS, COLS, 10_000);
        TerminalView::new(terminal, events, window, cx)
    });
    // An M1's display, and a window just big enough for 200×60.
    cx.simulate_scale_factor_change(2.0);
    draw(cx);
    check_baseline(&view, cx);
    if !measure {
        println!("terminal_perf: measurement skipped (pass --ignored in a release build)");
        return;
    }
    if cfg!(debug_assertions) {
        println!("terminal_perf: debug build, the numbers mean little");
    }
    let cell = view
        .read_with(cx, |v, _| v.cell_bounds(0, 0))
        .expect("a drawn cell");
    let (cw, ch) = (f32::from(cell.size.width), f32::from(cell.size.height));
    cx.simulate_resize(size(
        gpui::px(cw * (COLS as f32 + 1.) + cw / 2.),
        gpui::px(ch * ROWS as f32 + ch / 2.),
    ));
    feed(&view, &screen(0), cx);
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(10));
    cx.run_until_parked();
    draw(cx);
    let grid = view.read_with(cx, |v, _| v.grid_size());
    assert_eq!(grid, (ROWS, COLS), "the window holds a 200×60 grid");
    let row0 = view.read_with(cx, |v, _| v.painted_row_text(0));
    assert_eq!(row0.trim_end().chars().count(), COLS as usize, "{row0}");
    println!("cell {cw:.2}×{ch:.2} px at 2×, grid {}×{}", grid.1, grid.0);

    // `layout_line` alone over the grid, as prepaint calls it.
    let snapshot = view.read_with(cx, |v, _| v.terminal().snapshot());
    let pal = palette();
    let mut runs = 0;
    for row in &snapshot.cells {
        runs += layout_line(row, &pal, 4.5, false).runs.len();
    }
    let mut layout = Vec::with_capacity(FRAMES);
    for _ in 0..FRAMES {
        let start = Instant::now();
        for row in &snapshot.cells {
            std::hint::black_box(layout_line(row, &pal, 4.5, false));
        }
        layout.push(start.elapsed());
    }
    println!("{runs} text runs on screen");
    report("layout_line, 60 rows", layout);

    // Snapshot of the grid alone.
    let mut snap = Vec::with_capacity(FRAMES);
    for _ in 0..FRAMES {
        let start = Instant::now();
        std::hint::black_box(view.read_with(cx, |v, _| v.terminal().snapshot()));
        snap.push(start.elapsed());
    }
    report("snapshot, 200×60", snap);

    // Unchanged screen: GPUI's line cache holds last frame's shapes.
    for _ in 0..10 {
        draw(cx);
    }
    let mut same = Vec::with_capacity(FRAMES);
    for _ in 0..FRAMES {
        let start = Instant::now();
        draw(cx);
        same.push(start.elapsed());
    }
    let same_p95 = report("window draw, unchanged screen", same);

    // Every row new each frame (a flood): every run is shaped afresh.
    let mut fresh = Vec::with_capacity(FRAMES);
    for i in 0..FRAMES {
        feed(&view, &screen(i * 13 + 1), cx);
        let start = Instant::now();
        draw(cx);
        fresh.push(start.elapsed());
        if i % 20 == 0 {
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(10));
            cx.run_until_parked();
        }
    }
    let fresh_p95 = report("window draw, every row new", fresh);

    let frames = view.read_with(cx, |v, _| v.frames_drawn());
    assert!(
        frames as usize >= 2 * FRAMES,
        "the terminal painted each draw"
    );
    println!(
        "budget {BUDGET_MS} ms: unchanged p95 {same_p95:.3} ms, every row new p95 {fresh_p95:.3} ms"
    );
}
