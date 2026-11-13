//! The per-block markdown renderer with the preview style: tables, task
//! lists and images.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Context, IntoElement, ParentElement, Render, Styled, TestAppContext, VisualTestContext, Window,
    div, px,
};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::AppState;
use lucerna_workbench::markdown::{
    ImageLookup, MarkdownBlocks, MarkdownStyle, parse_markdown_document, render_block,
};

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

struct Host {
    blocks: Arc<MarkdownBlocks>,
    images: Option<ImageLookup>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = MarkdownStyle::preview(14., 1.6, self.images.clone());
        div().flex().flex_col().w(px(400.)).children(
            (0..self.blocks.blocks().len())
                .map(|root| render_block(&self.blocks, root, &style, Rc::new(|_, _, _| {}), cx)),
        )
    }
}

fn host<'a>(
    source: &str,
    images: Option<ImageLookup>,
    cx: &'a mut TestAppContext,
) -> &'a mut VisualTestContext {
    init(cx);
    let blocks = Arc::new(parse_markdown_document(source, None));
    let (_host, cx) = cx.add_window_view(move |_, _| Host { blocks, images });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx
}

#[gpui::test]
fn table_renders_cells(cx: &mut TestAppContext) {
    let cx = host("| a | b |\n|--|--|\n| 1 | 2 |\n", None, cx);
    for cell in [
        "md-table-cell-0-0",
        "md-table-cell-0-1",
        "md-table-cell-1-0",
        "md-table-cell-1-1",
    ] {
        assert!(cx.debug_bounds(cell).is_some(), "{cell}");
    }
    assert!(cx.debug_bounds("md-table-cell-2-0").is_none());
}

#[gpui::test]
fn task_list_draws_checkboxes(cx: &mut TestAppContext) {
    let cx = host("- [x] a\n- [ ] b\n", None, cx);
    assert!(cx.debug_bounds("md-task-0").is_some());
    assert!(cx.debug_bounds("md-task-1").is_some());
    assert!(cx.debug_bounds("md-task-2").is_none());
}

#[gpui::test]
fn image_without_context_shows_alt(cx: &mut TestAppContext) {
    let cx = host("![a cat](cat.png)\n", None, cx);
    assert!(cx.debug_bounds("md-image-alt-0").is_some());
}

#[gpui::test]
fn inline_image_in_link_goes_through_lookup(cx: &mut TestAppContext) {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = seen.clone();
    let lookup: ImageLookup = Rc::new(move |url, alt, n, _| {
        log.borrow_mut().push((url.to_owned(), alt.to_owned(), n));
        div().size(px(10.)).into_any_element()
    });
    let _cx = host(
        "[![b](x.svg)](https://a.dev) text ![c](y.png)\n\n![d](z.png)\n",
        Some(lookup),
        cx,
    );
    let seen = seen.borrow();
    for (i, (url, alt)) in [("x.svg", "b"), ("y.png", "c"), ("z.png", "d")]
        .iter()
        .enumerate()
    {
        assert!(
            seen.iter().any(|s| s.0 == *url && s.1 == *alt && s.2 == i),
            "{seen:?}"
        );
    }
}

#[gpui::test]
fn table_rows_align_when_cells_wrap(cx: &mut TestAppContext) {
    let long = "word ".repeat(60);
    let source = format!("| a | b |\n|--|--|\n| {long} | short |\n");
    let cx = host(&source, None, cx);
    let a = cx.debug_bounds("md-table-cell-1-0").expect("cell 1-0");
    let b = cx.debug_bounds("md-table-cell-1-1").expect("cell 1-1");
    // The long cell wrapped (one line is about 35 px).
    assert!(a.size.height > px(100.), "{:?}", a.size);
    assert_eq!(a.origin.y, b.origin.y);
    assert_eq!(a.size.height, b.size.height);
}
