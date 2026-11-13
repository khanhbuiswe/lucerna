//! The Markdown preview (spec §5.3): a tab that draws a markdown file's
//! root blocks as the items of a `gpui::list`, following the file's
//! document as it is edited or rewritten on disk (spec §5.4): parses run on
//! a background thread, at most one per [`REPARSE_DELAY`], and keep the
//! source line at the top of the view.

mod sync;

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, Entity, IntoElement, ListAlignment, ListOffset, ListState,
    ParentElement, Render, SharedString, Styled, Task, WeakEntity, Window, div, list, prelude::*,
    px,
};
use lucerna_text::{BufferSnapshot, LoadError};

use crate::AppState;
use crate::code_view::CodeView;
use crate::documents::{DocState, Documents};
use crate::layout::color;
use crate::markdown::{
    LinkHandler, MarkdownBlocks, MarkdownStyle, open_markdown_link, parse_markdown_document,
    render_block,
};
use crate::ui::{GRID, UI_FONT_FAMILY, text_button};
use crate::workbench::Workbench;

/// Files larger than this are not parsed: the preview offers the source
/// instead.
pub const MAX_PREVIEW_BYTES: usize = 5 * 1024 * 1024;

/// How long after a change to the text the preview parses it again; the
/// changes in between are taken in by that one parse.
pub const REPARSE_DELAY: Duration = Duration::from_millis(200);

/// The widest the content column gets, in px.
const CONTENT_MAX_WIDTH: f32 = 800.;
/// Space left and right of the content, in px.
const PADDING_X: f32 = 32.;
/// Space above the first block and below the last, in px.
const PADDING_Y: f32 = 24.;
/// How far past the viewport the list lays out blocks, in px.
const LIST_OVERDRAW: f32 = 50. * GRID;

/// The command the too-large notice runs.
const SHOW_SOURCE: &str = "markdown.showSource";

/// A rendered view of a markdown file. It keeps the file's document open
/// (and reloaded from disk) through a passive code view it never draws.
pub struct MarkdownPreview {
    path: Arc<Path>,
    /// Follows the active markdown editor (`MarkdownPreviewDynamic`).
    dynamic: bool,
    documents: Entity<Documents>,
    workbench: WeakEntity<Workbench>,
    /// Holds the document open; never drawn.
    _source: Entity<CodeView>,
    blocks: Option<Arc<MarkdownBlocks>>,
    /// The document revision `blocks` (or `too_large`) was made from.
    revision: Option<u64>,
    /// The revision of the last text handed to a parse, shown or not.
    requested: Option<u64>,
    /// The timer of the next parse, while one is waiting.
    pending: Option<Task<()>>,
    /// Bumped when the previewed file changes, so parses of the old file
    /// are dropped.
    epoch: u64,
    /// Parses started, for tests.
    parses_started: u64,
    /// The file is over [`MAX_PREVIEW_BYTES`]; `blocks` is `None`.
    too_large: bool,
    /// The document's state when last checked.
    state: Option<DocState>,
    list_state: ListState,
}

impl MarkdownPreview {
    pub fn new(
        path: Arc<Path>,
        dynamic: bool,
        documents: Entity<Documents>,
        workbench: WeakEntity<Workbench>,
        cx: &mut Context<Self>,
    ) -> Self {
        let source = documents.update(cx, |d, cx| d.open_preview(path.clone(), cx));
        // Every change to the documents (an edit, a reload, the first
        // load, a deletion) is checked against the revision shown.
        cx.observe(&documents, |preview, _, cx| preview.documents_changed(cx))
            .detach();
        let mut preview = MarkdownPreview {
            path,
            dynamic,
            documents,
            workbench,
            _source: source,
            blocks: None,
            revision: None,
            requested: None,
            pending: None,
            epoch: 0,
            parses_started: 0,
            too_large: false,
            state: None,
            list_state: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
        };
        preview.documents_changed(cx);
        preview
    }

    /// The previewed file.
    pub fn path(&self) -> &Arc<Path> {
        &self.path
    }

    /// Whether this is the preview that follows the active editor.
    pub fn is_dynamic(&self) -> bool {
        self.dynamic
    }

    /// Previews another file: lets the old document go, opens the new one
    /// and parses it now.
    pub fn set_path(&mut self, path: Arc<Path>, cx: &mut Context<Self>) {
        if path == self.path {
            return;
        }
        self._source = self
            .documents
            .update(cx, |d, cx| d.open_preview(path.clone(), cx));
        self.path = path;
        self.blocks = None;
        self.revision = None;
        self.requested = None;
        self.pending = None;
        self.epoch += 1;
        self.too_large = false;
        self.state = None;
        self.list_state.reset(0);
        self.documents_changed(cx);
    }

    /// The parsed blocks shown; `None` before the first load and for a
    /// file over [`MAX_PREVIEW_BYTES`].
    pub fn blocks(&self) -> Option<&Arc<MarkdownBlocks>> {
        self.blocks.as_ref()
    }

    pub fn list_state(&self) -> &ListState {
        &self.list_state
    }

    /// The source line (0-based) of the block at the top of the view.
    pub fn top_line(&self) -> f32 {
        let top = self.list_state.logical_scroll_top().item_ix;
        self.blocks
            .as_ref()
            .and_then(|b| b.root_lines().get(top))
            .map_or(0., |lines| lines.start as f32)
    }

    /// Scrolls the block holding source line `line` (0-based) to the top.
    pub fn scroll_to_line(&mut self, line: f32, cx: &mut Context<Self>) {
        let Some(blocks) = &self.blocks else {
            return;
        };
        // The last block starting at or before the line: a line between
        // blocks (a blank one) belongs to the block above.
        let item_ix = blocks
            .root_lines()
            .partition_point(|lines| lines.start as f32 <= line)
            .saturating_sub(1);
        self.list_state.scroll_to(ListOffset {
            item_ix,
            offset_in_item: px(0.),
        });
        cx.notify();
    }

    /// The number of parses started so far (for tests).
    pub fn parses_started(&self) -> u64 {
        self.parses_started
    }

    /// Parses the document again now, without waiting for a change.
    // Task 8: also drops the view's image cache, so changed images load
    // again (`markdown.preview.refresh`, spec §5.4).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        self.parse_now(cx);
    }

    /// Redraws when the document's state changed; when its text changed,
    /// parses it now the first time, later once [`REPARSE_DELAY`] has
    /// passed (a change while that wait runs joins it).
    fn documents_changed(&mut self, cx: &mut Context<Self>) {
        let docs = self.documents.read(cx);
        let state = docs.state(&self.path).cloned();
        let revision = docs.revision_of(&self.path);
        if state != self.state {
            self.state = state;
            cx.notify();
        }
        if revision.is_none() || revision == self.requested || self.pending.is_some() {
            return;
        }
        if self.requested.is_none() {
            self.parse_now(cx);
            return;
        }
        self.pending = Some(cx.spawn(async move |preview, cx| {
            cx.background_executor().timer(REPARSE_DELAY).await;
            preview
                .update(cx, |preview, cx| {
                    preview.pending = None;
                    if preview.documents.read(cx).revision_of(&preview.path) != preview.requested {
                        preview.parse_now(cx);
                    }
                })
                .ok();
        }));
    }

    /// Parses the document's current text on a background thread; a file
    /// over [`MAX_PREVIEW_BYTES`] shows the notice instead.
    fn parse_now(&mut self, cx: &mut Context<Self>) {
        let docs = self.documents.read(cx);
        let (Some(revision), Some(snapshot)) =
            (docs.revision_of(&self.path), docs.snapshot_of(&self.path))
        else {
            return;
        };
        self.requested = Some(revision);
        if snapshot.len_bytes() > MAX_PREVIEW_BYTES {
            self.show(revision, None, cx);
            return;
        }
        self.parses_started += 1;
        let epoch = self.epoch;
        let parse = cx.background_spawn(async move { parse_snapshot(snapshot) });
        cx.spawn(async move |preview, cx| {
            let blocks = parse.await;
            preview
                .update(cx, |preview, cx| {
                    if preview.epoch == epoch {
                        preview.show(revision, Some(Arc::new(blocks)), cx);
                    }
                })
                .ok();
        })
        .detach();
    }

    /// Shows the blocks parsed from `revision` (`None`: the file is too
    /// large), unless newer ones are shown, keeping the source line at the
    /// top of the view.
    fn show(&mut self, revision: u64, blocks: Option<Arc<MarkdownBlocks>>, cx: &mut Context<Self>) {
        if self.revision.is_some_and(|shown| shown > revision) {
            return;
        }
        let top = self.top_line();
        self.revision = Some(revision);
        self.too_large = blocks.is_none();
        self.blocks = blocks;
        let count = self.blocks.as_ref().map_or(0, |b| b.blocks().len());
        self.list_state.reset(count);
        self.scroll_to_line(top, cx);
        cx.notify();
    }

    /// The preview's style, from the `markdown.preview.*` settings.
    fn style(cx: &App) -> MarkdownStyle {
        let settings = &cx.global::<AppState>().settings;
        let font_size = settings
            .get::<f32>("markdown.preview.fontSize", None)
            .unwrap_or(14.);
        let line_height = settings
            .get::<f32>("markdown.preview.lineHeight", None)
            .unwrap_or(1.6);
        let family = settings
            .get::<String>("markdown.preview.fontFamily", None)
            .filter(|f| !f.trim().is_empty())
            .map_or_else(|| SharedString::from(UI_FONT_FAMILY), SharedString::from);
        let mut style = MarkdownStyle::preview(font_size, line_height, None);
        style.font_family = family;
        style
    }

    /// Clicked links: as in hover popovers.
    fn link_handler(&self) -> LinkHandler {
        let workbench = self.workbench.clone();
        Rc::new(move |url, _, cx| {
            workbench
                .update(cx, |wb, cx| open_markdown_link(url, wb, cx))
                .ok();
        })
    }

    /// "File quá lớn để xem trước" with a button to the source.
    fn render_too_large(&self, cx: &mut Context<Self>) -> AnyElement {
        let workbench = self.workbench.clone();
        let button = text_button("md-preview-show-source", "Show Source", cx).on_click(
            move |_, window, cx| {
                workbench
                    .update(cx, |wb, cx| wb.run_command(SHOW_SOURCE, None, window, cx))
                    .ok();
            },
        );
        notice("md-preview-too-large", "File quá lớn để xem trước", cx)
            .child(button)
            .into_any_element()
    }
}

/// Parses the whole text of `snapshot`; runs on a background thread.
fn parse_snapshot(snapshot: BufferSnapshot) -> MarkdownBlocks {
    let source = snapshot.text_for_range(0..snapshot.len_bytes());
    parse_markdown_document(&source, None)
}

/// A centred, muted message, as the code view shows load errors.
fn notice(selector: &'static str, message: impl Into<SharedString>, cx: &App) -> gpui::Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .debug_selector(move || selector.into())
        .flex()
        .flex_col()
        .items_center()
        .gap(px(3. * GRID))
        .max_w(px(480.))
        .text_color(color(ui.text_muted))
        .child(message.into())
}

/// What the code view says for a file it could not load.
fn load_error_message(err: &LoadError) -> String {
    match err {
        LoadError::Binary => "Binary file".into(),
        LoadError::TooLarge { .. } => "File is larger than 512 MB".into(),
        err => err.to_string(),
    }
}

impl Render for MarkdownPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let root = div()
            .id("markdown-preview")
            .debug_selector(|| "markdown-preview".into())
            .size_full()
            .bg(color(ui.editor_background));
        if let Some(DocState::Error(err)) = &self.state {
            let message = load_error_message(err);
            return root.flex().justify_center().items_center().child(notice(
                "md-preview-error",
                message,
                cx,
            ));
        }
        if self.too_large {
            return root
                .flex()
                .justify_center()
                .items_center()
                .child(self.render_too_large(cx));
        }
        let Some(blocks) = self.blocks.clone() else {
            // Still loading: an empty page, no spinner.
            return root;
        };
        let style = Self::style(cx);
        let on_link = self.link_handler();
        let last = blocks.blocks().len().saturating_sub(1);
        let items = list(self.list_state.clone(), move |ix, _, cx| {
            let block = render_block(&blocks, ix, &style, on_link.clone(), cx);
            // Each item spans the list, so the whole page scrolls; its
            // content sits in the centred column.
            div()
                .w_full()
                .flex()
                .justify_center()
                .px(px(PADDING_X))
                .when(ix == 0, |d| d.pt(px(PADDING_Y)))
                .pb(px(if ix == last {
                    PADDING_Y
                } else {
                    style.block_gap
                }))
                .child(
                    div()
                        .w_full()
                        .max_w(px(CONTENT_MAX_WIDTH))
                        .min_w_0()
                        .child(block),
                )
                .into_any_element()
        })
        .size_full();
        root.child(items)
    }
}
