//! The markdown component hover popovers use, rendered in a test window.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Context, IntoElement, Render, TestAppContext, VisualTestContext, Window};
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
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

struct Host(Arc<MarkdownBlocks>);

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_markdown(&self.0, cx)
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
