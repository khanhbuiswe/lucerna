//! The markdown component hover popovers use, rendered in a test window;
//! and LSP hover (spec §7.2): timing, hiding, the diagnostic block.

mod support;

use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, Styled, TestAppContext,
    VisualTestContext, Window, div, px,
};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::markdown::{
    MarkdownBlocks, open_markdown_link, parse_markdown, render_markdown,
};
use lucerna_workbench::{AppState, Workbench};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
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
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

struct Host(Arc<MarkdownBlocks>);

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().flex_col().items_start().child(
            div()
                .debug_selector(|| "md-root".to_owned())
                .w(px(320.))
                .child(render_markdown(&self.0, cx)),
        )
    }
}

const DOC: &str = "# Title\n\nSome *text* with `code` and [a link](https://zed.dev).\n\n\
```rust\nfn main() {\n    let x = 1;\n}\n```\n\n```nonsense\nplain\n```\n\n\
- one\n- two\n\n> quote\n\n---\n";

fn host<'a>(
    source: &str,
    fallback: Option<&str>,
    cx: &'a mut TestAppContext,
) -> (Arc<MarkdownBlocks>, &'a mut VisualTestContext) {
    init(cx);
    let blocks = Arc::new(parse_markdown(source, fallback));
    let for_host = blocks.clone();
    let (_host, cx) = cx.add_window_view(move |_, _| Host(for_host));
    draw(cx);
    (blocks, cx)
}

#[gpui::test]
fn renders_without_panic_and_highlights_rust_block(cx: &mut TestAppContext) {
    let (blocks, cx) = host(DOC, None, cx);
    assert!(cx.debug_bounds("md-code-0").is_some(), "no md-code-0");
    assert!(cx.debug_bounds("md-code-1").is_some(), "no md-code-1");
    assert!(cx.debug_bounds("md-code-2").is_none());
    cx.update(|_, cx| {
        let (text, rust) = blocks.code_runs(0, cx).unwrap();
        assert_eq!(text, "fn main() {\n    let x = 1;\n}");
        assert!(rust.len() > 1, "{} runs", rust.len());
        assert_eq!(rust.iter().map(|r| r.len).sum::<usize>(), text.len());
        // Cached: the second call hands out the same runs.
        assert!(Arc::ptr_eq(&rust, &blocks.code_runs(0, cx).unwrap().1));
        // Unknown language and no fallback: one plain run.
        assert_eq!(blocks.code_runs(1, cx).unwrap().1.len(), 1);
        assert!(blocks.code_runs(2, cx).is_none());
    });
}

#[gpui::test]
fn hover_layout_unchanged(cx: &mut TestAppContext) {
    let (_blocks, cx) = host(DOC, None, cx);
    let bounds = cx.debug_bounds("md-root").expect("md-root");
    // Measured at eecc032, before the per-block renderer: 280 px at 320 px wide.
    assert_eq!(bounds.size.height, px(280.));
}

#[gpui::test]
fn fence_without_language_uses_the_file_language(cx: &mut TestAppContext) {
    let (blocks, cx) = host("```\nfn main() {}\n```", Some("rust"), cx);
    assert!(cx.debug_bounds("md-code-0").is_some());
    cx.update(|_, cx| assert!(blocks.code_runs(0, cx).unwrap().1.len() > 1));
}

#[gpui::test]
fn fence_without_any_language_is_plain(cx: &mut TestAppContext) {
    let (blocks, cx) = host("```\nfn main() {}\n```", None, cx);
    cx.update(|_, cx| assert_eq!(blocks.code_runs(0, cx).unwrap().1.len(), 1));
}

#[gpui::test]
fn file_link_opens_the_file_at_the_line(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let text: String = (1..=30).map(|i| format!("line number {i}\n")).collect();
    std::fs::write(dir.path().join("a.txt"), text).unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(move |window, cx| Workbench::new(root, window, cx));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !wb.read_with(cx, |wb, _| wb.worktree().snapshot().scan_complete()) {
        assert!(Instant::now() < deadline, "worktree scan did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::ScanComplete, cx)
    });

    let url = format!("file://{}#L10,5", dir.path().join("a.txt").display());
    wb.update(cx, |wb, cx| open_markdown_link(&url, wb, cx));
    let cursor = |cx: &mut VisualTestContext| {
        draw(cx);
        cx.update(|_, cx| {
            let body = wb.read(cx).editor_groups().read(cx).active_body()?;
            Some(body.downcast::<CodeView>().ok()?.read(cx).cursor())
        })
    };
    // The file loads on a background thread.
    let deadline = Instant::now() + Duration::from_secs(10);
    while cursor(cx) != Some(Point { row: 9, column: 4 }) {
        assert!(Instant::now() < deadline, "cursor: {:?}", cursor(cx));
        std::thread::sleep(Duration::from_millis(10));
    }

    // Anything else is ignored.
    wb.update(cx, |wb, cx| open_markdown_link("command:rm", wb, cx));
}

#[gpui::test]
fn deeply_nested_markdown_renders_without_overflowing(cx: &mut TestAppContext) {
    let quotes = format!("{}x\n\n```rust\nfn a() {{}}\n```", "> ".repeat(49_000));
    let (_blocks, _cx) = host(&quotes, None, cx);
}

#[gpui::test]
fn deeply_nested_list_renders_without_overflowing(cx: &mut TestAppContext) {
    let (_blocks, _cx) = host(&format!("{}x", "- ".repeat(49_000)), None, cx);
}

// ---- LSP hover ----

mod lsp_hover {
    use super::*;
    use gpui::{
        BorrowAppContext, Bounds, Entity, Modifiers, Pixels, ScrollDelta, ScrollEvent, point, px,
    };
    use lucerna_lsp::position::path_to_uri;
    use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
    use lucerna_settings::parse_when;
    use serde_json::{Value, json};
    use support::lsp::{TRUSTED, active_view, count, lsp_workbench_with, open, sent, settle};

    const MAIN: &str = "fn main() {\n    let x = 1;\n}\n";
    const HOVER: &str = "textDocument/hover";
    const CANCEL: &str = "$/cancelRequest";

    fn rust_project() -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), MAIN).unwrap();
        dir
    }

    fn uri(dir: &TempDir, name: &str) -> String {
        path_to_uri(&dir.path().join("src").join(name))
            .as_str()
            .to_owned()
    }

    fn range(line: u32, start: u32, end: u32) -> Value {
        json!({ "start": { "line": line, "character": start },
                "end": { "line": line, "character": end } })
    }

    /// Answers every hover with markdown about `main` over (0, 3..7).
    fn answering(fake: FakeServer) -> FakeServer {
        fake.on_request(HOVER, |_| {
            Ok(json!({
                "contents": { "kind": "markdown",
                              "value": "```rust\nfn main()\n```\n\nThe entry point." },
                "range": range(0, 3, 7),
            }))
        })
    }

    fn setup<'a>(
        settings: &str,
        fake: FakeServer,
        cx: &'a mut TestAppContext,
    ) -> (
        TempDir,
        FakeServer,
        Entity<Workbench>,
        Entity<CodeView>,
        &'a mut VisualTestContext,
    ) {
        let dir = rust_project();
        let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
        let (wb, cx) = lsp_workbench_with(dir.path(), launcher, settings, cx);
        let view = open(&wb, "src/main.rs", cx);
        (dir, fake, wb, view, cx)
    }

    /// Lets `ms` of virtual time pass, then draws.
    fn advance(ms: u64, cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(ms));
        cx.run_until_parked();
        super::draw(cx);
    }

    /// Moves the mouse over the char at `(row, column)` of `view`.
    fn hover_at(view: &Entity<CodeView>, row: u32, column: u32, cx: &mut VisualTestContext) {
        super::draw(cx);
        let at = view
            .read_with(cx, |v, _| v.char_position(Point { row, column }))
            .expect("the char is on screen");
        cx.simulate_mouse_move(at, None, Modifiers::none());
    }

    fn move_to(at: gpui::Point<Pixels>, cx: &mut VisualTestContext) {
        cx.simulate_mouse_move(at, None, Modifiers::none());
    }

    fn popover(cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
        super::draw(cx);
        cx.debug_bounds("hover-popover")
    }

    fn visible(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> bool {
        let shown = view.read_with(cx, |v, _| v.hover().is_visible());
        assert_eq!(shown, popover(cx).is_some(), "state and drawing agree");
        shown
    }

    fn markdown(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Option<String> {
        view.read_with(cx, |v, _| v.hover().info().map(|i| i.markdown.clone()))
    }

    fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            wb.update(cx, |wb, cx| {
                assert!(wb.run_command(command, None, window, cx), "{command}");
            })
        });
        cx.run_until_parked();
        super::draw(cx);
    }

    fn holds(wb: &Entity<Workbench>, when: &str, cx: &mut VisualTestContext) -> bool {
        let ctx = cx.update(|window, cx| wb.read(cx).context_keys(window, cx));
        parse_when(when).unwrap().eval(&ctx)
    }

    /// A spot in the editor that is not over any char: far right of row 2.
    fn blank(cx: &mut VisualTestContext) -> gpui::Point<Pixels> {
        let area = cx.debug_bounds("code-text-area").expect("code area drawn");
        point(area.right() - px(40.), area.top() + px(60.))
    }

    #[gpui::test]
    fn hover_appears_after_delay(cx: &mut TestAppContext) {
        let (dir, fake, _wb, view, cx) = setup(TRUSTED, answering(FakeServer::new()), cx);
        hover_at(&view, 0, 4, cx);
        advance(149, cx);
        assert_eq!(count(&fake, HOVER), 0, "no request before half the delay");
        advance(2, cx);
        let requests = sent(&fake, HOVER);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["textDocument"]["uri"], uri(&dir, "main.rs"));
        assert_eq!(
            requests[0]["position"],
            json!({ "line": 0, "character": 4 })
        );
        // The answer is in, but the popover waits for the full delay.
        assert!(!visible(&view, cx));
        advance(148, cx);
        assert!(!visible(&view, cx));
        advance(2, cx);
        assert!(visible(&view, cx));
        assert!(markdown(&view, cx).unwrap().contains("The entry point."));
        assert!(cx.debug_bounds("md-code-0").is_some(), "code block drawn");
        let highlight = view.read_with(cx, |v, _| v.hover().highlight_range());
        assert_eq!(
            highlight,
            Some(Point { row: 0, column: 3 }..Point { row: 0, column: 7 })
        );

        // Moving inside the shown range keeps it and asks nothing new.
        hover_at(&view, 0, 6, cx);
        advance(1000, cx);
        assert!(visible(&view, cx));
        assert_eq!(count(&fake, HOVER), 1);
    }

    #[gpui::test]
    fn moving_away_cancels_request(cx: &mut TestAppContext) {
        let fake = FakeServer::new().hang_on(HOVER);
        let (_dir, fake, _wb, view, cx) = setup(TRUSTED, fake, cx);
        hover_at(&view, 0, 4, cx);
        advance(151, cx);
        assert_eq!(count(&fake, HOVER), 1);
        assert_eq!(count(&fake, CANCEL), 0);
        // Still in the same word: the request stays.
        hover_at(&view, 0, 5, cx);
        advance(1, cx);
        assert_eq!(count(&fake, CANCEL), 0);
        // Another word before the popover showed: dropped and cancelled.
        hover_at(&view, 1, 8, cx);
        advance(1, cx);
        assert_eq!(count(&fake, CANCEL), 1);
        // Off the text: the next request is dropped too, nothing shows.
        advance(151, cx);
        assert_eq!(count(&fake, HOVER), 2);
        move_to(blank(cx), cx);
        advance(1000, cx);
        assert_eq!(count(&fake, CANCEL), 2);
        assert!(!visible(&view, cx));
    }

    #[gpui::test]
    fn sticky_keeps_open_when_entering_popover(cx: &mut TestAppContext) {
        let (_dir, _fake, _wb, view, cx) = setup(TRUSTED, answering(FakeServer::new()), cx);
        hover_at(&view, 0, 4, cx);
        advance(300, cx);
        let bounds = popover(cx).expect("popover shown");
        move_to(bounds.center(), cx);
        advance(1000, cx);
        assert!(visible(&view, cx), "kept while the mouse is in it");
        // Leaving the popover hides it after the hiding delay.
        move_to(blank(cx), cx);
        advance(299, cx);
        assert!(visible(&view, cx));
        advance(2, cx);
        assert!(!visible(&view, cx));
    }

    #[gpui::test]
    fn hides_after_hiding_delay(cx: &mut TestAppContext) {
        let (_dir, _fake, wb, view, cx) = setup(TRUSTED, answering(FakeServer::new()), cx);
        hover_at(&view, 0, 4, cx);
        advance(300, cx);
        assert!(visible(&view, cx));
        assert!(holds(&wb, "lspHoverVisible", cx));
        move_to(blank(cx), cx);
        advance(299, cx);
        assert!(visible(&view, cx));
        advance(2, cx);
        assert!(!visible(&view, cx));
        assert!(!holds(&wb, "lspHoverVisible", cx));

        // Scrolling hides at once.
        hover_at(&view, 0, 4, cx);
        advance(300, cx);
        assert!(visible(&view, cx));
        let area = cx.debug_bounds("code-text-area").unwrap();
        cx.simulate_event(ScrollEvent {
            position: area.center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(-10.))),
            ..Default::default()
        });
        super::draw(cx);
        assert!(!visible(&view, cx));
    }

    #[gpui::test]
    fn not_sticky_hides_when_leaving_range(cx: &mut TestAppContext) {
        let settings = r#"{ "session.trustAllWorktrees": true, "editor.hover.sticky": false }"#;
        let (_dir, _fake, _wb, view, cx) = setup(settings, answering(FakeServer::new()), cx);
        hover_at(&view, 0, 4, cx);
        advance(300, cx);
        assert!(visible(&view, cx));
        move_to(blank(cx), cx);
        advance(0, cx);
        assert!(!visible(&view, cx));
    }

    #[gpui::test]
    fn show_hover_command_no_delay(cx: &mut TestAppContext) {
        let (_dir, fake, wb, view, cx) = setup(TRUSTED, answering(FakeServer::new()), cx);
        view.update(cx, |v, cx| v.go_to(Point { row: 0, column: 4 }, cx));
        run(&wb, "editor.action.showHover", cx);
        assert_eq!(count(&fake, HOVER), 1);
        assert!(visible(&view, cx), "shown without advancing the clock");

        // Escape hides it.
        wb.update_in(cx, |wb, window, cx| wb.focus_editor_area(window, cx));
        cx.simulate_keystrokes("escape");
        assert!(!visible(&view, cx));

        // ⌘K ⌘I shows it again.
        cx.simulate_keystrokes("cmd-k cmd-i");
        cx.run_until_parked();
        assert!(visible(&view, cx));
    }

    #[gpui::test]
    fn diagnostic_block_above_info(cx: &mut TestAppContext) {
        let (dir, fake, _wb, view, cx) = setup(TRUSTED, answering(FakeServer::new()), cx);
        fake.notify(
            "textDocument/publishDiagnostics",
            json!({
                "uri": uri(&dir, "main.rs"),
                "diagnostics": [
                    { "range": range(0, 0, 11), "severity": 2, "message": "wide" },
                    { "range": range(0, 3, 7), "severity": 1, "message": "boom",
                      "source": "rustc", "code": "E0601",
                      "codeDescription": { "href": "https://doc.rust-lang.org/error_codes/E0601.html" } },
                    { "range": range(1, 8, 9), "severity": 1, "message": "elsewhere" },
                ],
            }),
        );
        settle(cx);
        hover_at(&view, 0, 4, cx);
        advance(300, cx);
        assert!(visible(&view, cx));
        let messages: Vec<String> = view.read_with(cx, |v, _| {
            v.hover()
                .diagnostics()
                .iter()
                .map(|d| d.message.clone())
                .collect()
        });
        assert_eq!(
            messages,
            ["boom", "wide"],
            "narrowest first, others left out"
        );
        let first = cx
            .debug_bounds("hover-diagnostic-0")
            .expect("diagnostic block");
        let second = cx.debug_bounds("hover-diagnostic-1").expect("second block");
        let source = cx
            .debug_bounds("hover-diagnostic-source-0")
            .expect("source(code) line");
        let info = cx.debug_bounds("hover-markdown").expect("hover info");
        assert!(first.bottom() <= second.top());
        assert!(source.bottom() <= first.bottom());
        assert!(cx.debug_bounds("hover-diagnostic-source-1").is_none());
        assert_eq!(
            info.top() - second.bottom(),
            px(6.),
            "6 px below the blocks"
        );
    }

    #[gpui::test]
    fn f8_shows_diagnostic_hover_at_once(cx: &mut TestAppContext) {
        let fake = FakeServer::new().hang_on(HOVER);
        let (dir, _fake, wb, view, cx) = setup(TRUSTED, fake.clone(), cx);
        fake.notify(
            "textDocument/publishDiagnostics",
            json!({
                "uri": uri(&dir, "main.rs"),
                "diagnostics": [{ "range": range(1, 8, 9), "severity": 2, "message": "unused" }],
            }),
        );
        settle(cx);
        run(&wb, "editor.action.marker.nextInFiles", cx);
        assert!(visible(&view, cx));
        let messages: Vec<String> = view.read_with(cx, |v, _| {
            v.hover()
                .diagnostics()
                .iter()
                .map(|d| d.message.clone())
                .collect()
        });
        assert_eq!(messages, ["unused"]);
    }

    #[gpui::test]
    fn f8_into_unopened_file_shows_diagnostic_hover(cx: &mut TestAppContext) {
        let (dir, fake, wb, _view, cx) = setup(TRUSTED, FakeServer::new().hang_on(HOVER), cx);
        fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}\n").unwrap();
        fake.notify(
            "textDocument/publishDiagnostics",
            json!({
                "uri": uri(&dir, "lib.rs"),
                "diagnostics": [{ "range": range(0, 7, 8), "severity": 1, "message": "lib error" }],
            }),
        );
        settle(cx);
        run(&wb, "editor.action.marker.nextInFiles", cx);
        settle(cx);
        let view = active_view(&wb, cx);
        assert!(view.read_with(cx, |v, _| v.path().ends_with("lib.rs")));
        assert!(visible(&view, cx));
        let messages: Vec<String> = view.read_with(cx, |v, _| {
            v.hover()
                .diagnostics()
                .iter()
                .map(|d| d.message.clone())
                .collect()
        });
        assert_eq!(messages, ["lib error"]);
    }

    #[gpui::test]
    fn disabled_by_setting(cx: &mut TestAppContext) {
        let settings = r#"{ "session.trustAllWorktrees": true, "editor.hover.enabled": false }"#;
        let (_dir, fake, wb, view, cx) = setup(settings, answering(FakeServer::new()), cx);
        hover_at(&view, 0, 4, cx);
        advance(1000, cx);
        assert_eq!(count(&fake, HOVER), 0);
        assert!(!visible(&view, cx));
        // The command still shows it, as in VS Code.
        view.update(cx, |v, cx| v.go_to(Point { row: 0, column: 4 }, cx));
        run(&wb, "editor.action.showHover", cx);
        assert!(visible(&view, cx));
    }

    #[gpui::test]
    fn popover_size_and_placement(cx: &mut TestAppContext) {
        let long = format!(
            "```rust\nlet s = \"{}\";\n```\n\nShort text.",
            "y".repeat(500)
        );
        let fake = FakeServer::new().on_request(HOVER, move |_| {
            Ok(json!({ "contents": { "kind": "markdown", "value": long.clone() } }))
        });
        let (dir, _fake, wb, _view, cx) = setup(TRUSTED, fake, cx);
        let text: String = (0..40).map(|i| format!("let v{i} = {i};\n")).collect();
        fs::write(dir.path().join("src/many.rs"), text).unwrap();
        let view = open(&wb, "src/many.rs", cx);
        let area = cx.debug_bounds("code-text-area").unwrap();
        let line = view.read_with(cx, |v, _| {
            let a = v.char_position(Point { row: 0, column: 4 }).unwrap();
            let b = v.char_position(Point { row: 1, column: 4 }).unwrap();
            b.y - a.y
        });

        // Row 0: no room above, so below the line.
        hover_at(&view, 0, 4, cx);
        advance(300, cx);
        let below = popover(cx).expect("popover");
        let row0 = view.read_with(cx, |v, _| {
            v.char_position(Point { row: 0, column: 4 }).unwrap()
        });
        assert!(below.top() >= row0.y + line / 2., "below row 0");
        assert!(
            below.right() <= area.right() - px(8.) + px(0.5),
            "8 px from the right edge"
        );
        assert!(
            below.size.width <= area.size.width / 2. + px(0.5),
            "at most half the editor"
        );
        assert!(
            below.size.height <= line * 16. + px(0.5),
            "at most 16 lines"
        );
        let code = cx.debug_bounds("md-code-0").expect("code block");
        assert!(
            code.size.width <= below.size.width,
            "the code line scrolls inside"
        );

        // A lower row has room above it.
        move_to(blank(cx), cx);
        advance(1000, cx);
        hover_at(&view, 20, 4, cx);
        advance(300, cx);
        let above = popover(cx).expect("popover");
        let row20 = view.read_with(cx, |v, _| {
            v.char_position(Point { row: 20, column: 4 }).unwrap()
        });
        assert!(
            above.bottom() <= row20.y - line / 2. + px(0.5),
            "above row 20"
        );
    }

    #[gpui::test]
    fn crossing_adjacent_row_into_popover_keeps_the_hover(cx: &mut TestAppContext) {
        let (_dir, fake, _wb, view, cx) = setup(TRUSTED, answering(FakeServer::new()), cx);
        hover_at(&view, 0, 4, cx);
        advance(300, cx);
        let before = markdown(&view, cx).expect("shown");
        let bounds = popover(cx).expect("popover below row 0");
        // The strip between row 0 and the popover is row 1's chars.
        let row1 = view
            .read_with(cx, |v, _| v.char_position(Point { row: 1, column: 4 }))
            .unwrap();
        let strip = point(row1.x, bounds.top() - px(2.));
        move_to(strip, cx);
        move_to(bounds.center(), cx);
        advance(1000, cx);
        assert!(visible(&view, cx));
        assert_eq!(markdown(&view, cx), Some(before));
        assert_eq!(count(&fake, HOVER), 1, "no request for the crossed row");
    }

    #[gpui::test]
    fn key_bound_to_a_command_hides_the_hover(cx: &mut TestAppContext) {
        let (_dir, _fake, wb, view, cx) = setup(TRUSTED, answering(FakeServer::new()), cx);
        cx.update(|_, cx| {
            cx.update_global::<AppState, _>(|state, _| {
                let user = r#"[{ "key": "down", "command": "editor.action.toggleWordWrap" }]"#;
                state.keymap = lucerna_settings::Keymap::load(
                    lucerna_settings::Keymap::default_bindings(),
                    Some(user),
                )
                .0;
            })
        });
        wb.update_in(cx, |wb, window, cx| wb.focus_editor_area(window, cx));
        hover_at(&view, 0, 4, cx);
        advance(300, cx);
        assert!(visible(&view, cx));
        cx.simulate_keystrokes("down");
        assert!(view.read_with(cx, |v, _| v.word_wrap()), "the command ran");
        assert!(!visible(&view, cx));
    }

    #[gpui::test]
    fn vim_visual_escape_hides_the_hover(cx: &mut TestAppContext) {
        let settings = r#"{ "session.trustAllWorktrees": true, "vim.enabled": true }"#;
        let (_dir, _fake, wb, view, cx) = setup(settings, answering(FakeServer::new()), cx);
        wb.update_in(cx, |wb, window, cx| wb.focus_editor_area(window, cx));
        cx.simulate_keystrokes("v");
        super::draw(cx);
        assert!(holds(&wb, "vim.mode == 'Visual'", cx), "in visual mode");
        hover_at(&view, 0, 4, cx);
        advance(300, cx);
        assert!(visible(&view, cx));
        cx.simulate_keystrokes("escape");
        assert!(
            holds(&wb, "vim.mode == 'Normal'", cx),
            "vim took the Escape"
        );
        assert!(!visible(&view, cx));
    }

    #[gpui::test]
    fn new_text_hides_the_hover(cx: &mut TestAppContext) {
        let (_dir, _fake, _wb, view, cx) = setup(TRUSTED, answering(FakeServer::new()), cx);
        hover_at(&view, 0, 4, cx);
        advance(300, cx);
        assert!(visible(&view, cx));
        let snapshot = lucerna_text::Buffer::from_bytes(b"other text\n", false)
            .unwrap()
            .snapshot();
        view.update(cx, |v, cx| v.set_buffer(snapshot, cx));
        assert!(!visible(&view, cx));
    }

    #[gpui::test]
    fn switching_tabs_by_command_hides_the_old_hover(cx: &mut TestAppContext) {
        let (dir, _fake, wb, main, cx) = setup(TRUSTED, answering(FakeServer::new()), cx);
        fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}\n").unwrap();
        wb.update_in(cx, |wb, window, cx| wb.focus_editor_area(window, cx));
        let lib = dir.path().join("src/lib.rs");
        wb.update(cx, |wb, cx| {
            wb.jump_to(
                Arc::from(lib.as_path()),
                Point { row: 0, column: 0 },
                lucerna_workbench::OpenMode::Pinned,
                cx,
            )
        });
        settle(cx);
        let lib_view = active_view(&wb, cx);
        assert_ne!(lib_view, main);
        hover_at(&lib_view, 0, 8, cx);
        advance(300, cx);
        assert!(visible(&lib_view, cx));
        run(&wb, "workbench.action.navigateBack", cx);
        settle(cx);
        assert_eq!(active_view(&wb, cx), main);
        assert!(
            !lib_view.read_with(cx, |v, _| v.hover().is_visible()),
            "the hidden tab's hover is gone"
        );
    }
}
