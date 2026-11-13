//! The Markdown preview (spec §5.3): a tab that draws a markdown file's
//! root blocks as the items of a `gpui::list`, following the file's
//! document as it is edited or rewritten on disk (spec §5.4): parses run on
//! a background thread, at most one per [`REPARSE_DELAY`], and keep the
//! text at the top of the view there (moved by the edits above it, or at
//! the same line after a whole rewrite). A preview shown beside an editor of
//! its file scrolls with it ([`sync`], spec §4.5). Its commands, the
//! editor associations and the dynamic preview following the active file
//! are in [`commands`]. Images load from the workspace only, and links open
//! what they point at ([`resolve`], spec §5.6).

mod commands;
mod resolve;
mod sync;

pub use commands::{
    PREVIEW_EDITOR_ID, REFRESH_PREVIEW, SHOW_PREVIEW, SHOW_PREVIEW_TO_SIDE, SHOW_SOURCE,
    TOGGLE_PREVIEW, editor_for,
};
pub(crate) use commands::{associated_editor, follow_active, is_markdown, register_commands};
pub use resolve::{
    ImageContext, ImageSrc, LinkAction, MAX_IMAGE_BYTES, heading_slugs, resolve_image, resolve_link,
};
pub(crate) use sync::ScrollSync;
pub use sync::{item_to_line, line_to_item};

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, ListAlignment, ListOffset, ListState, MouseButton, MouseDownEvent, ParentElement,
    Pixels, Render, RetainAllImageCache, SharedString, Styled, StyledImage, Task, WeakEntity,
    Window, div, img, list, prelude::*, px,
};
use lucerna_text::{BufferSnapshot, LoadError};

use crate::AppState;
use crate::code_view::CodeView;
use crate::documents::{DocState, DocumentEvent, Documents};
use crate::layout::color;
use crate::markdown::{
    ImageLookup, LinkHandler, MarkdownBlocks, MarkdownStyle, image_placeholder,
    image_placeholder_builder, parse_markdown_document, render_block,
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

/// What a [`MarkdownPreview`] tells its scroll-sync links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownPreviewEvent {
    /// The user scrolled the preview (wheel or trackpad); a scroll the
    /// code asked for is not reported.
    Scrolled,
}

/// What a background parse hands back: the blocks and where each of their
/// image URLs resolved to.
struct Parsed {
    blocks: Arc<MarkdownBlocks>,
    images: Arc<HashMap<String, ImageSrc>>,
}

/// A rendered view of a markdown file. It keeps the file's document open
/// (and reloaded from disk) through a passive code view it never draws.
pub struct MarkdownPreview {
    path: Arc<Path>,
    /// Follows the active markdown editor (`MarkdownPreviewDynamic`).
    dynamic: bool,
    /// The workspace root (the file's folder in single-file mode): links
    /// and images starting with `/` start there.
    root: Arc<Path>,
    documents: Entity<Documents>,
    workbench: WeakEntity<Workbench>,
    /// Taken when the preview is clicked or its tab becomes active; the
    /// scroll keys work while it is focused.
    focus_handle: FocusHandle,
    /// Holds the document open; never drawn.
    _source: Entity<CodeView>,
    blocks: Option<Arc<MarkdownBlocks>>,
    /// Where the image URLs of `blocks` resolved to.
    images: Arc<HashMap<String, ImageSrc>>,
    /// The images loaded for this view; dropped with it, replaced by
    /// [`MarkdownPreview::refresh`].
    image_cache: Entity<RetainAllImageCache>,
    /// The heading to scroll to once the blocks are parsed (a link to
    /// `file.md#slug`).
    pending_slug: Option<String>,
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
    /// The source line to show at the top once the blocks are parsed, or
    /// once the top block is measured (its offset into the block needs
    /// the block's height): retried each frame until then.
    pending_line: Option<f32>,
    /// The top line of the view moved through the edits made since the
    /// blocks shown were parsed: where the text at the top now is, which
    /// the next blocks show at the top.
    anchor: Option<f32>,
    /// The height the top block had before the blocks were replaced:
    /// stands in for the new block's until it is measured, so the first
    /// frame after a reparse already shows the old offset into the block.
    stand_in_height: Option<Pixels>,
}

impl MarkdownPreview {
    pub fn new(
        path: Arc<Path>,
        dynamic: bool,
        root: Arc<Path>,
        documents: Entity<Documents>,
        workbench: WeakEntity<Workbench>,
        cx: &mut Context<Self>,
    ) -> Self {
        let source = documents.update(cx, |d, cx| d.open_preview(path.clone(), cx));
        // Every change to the documents (an edit, a reload, the first
        // load, a deletion) is checked against the revision shown.
        cx.observe(&documents, |preview, _, cx| preview.documents_changed(cx))
            .detach();
        // A document dropped while the preview shows it (the close
        // prompt's Don't Save) is opened again from disk; each edit moves
        // the anchor with the text above it.
        cx.subscribe(
            &documents,
            |preview, _, event: &DocumentEvent, cx| match event {
                DocumentEvent::Closed { .. } => preview.reopen_if_dropped(cx),
                DocumentEvent::Edited {
                    path,
                    old,
                    snapshot,
                    patch,
                    ..
                } if *path == preview.documents.read(cx).key(&preview.path) => {
                    let line = preview.anchor.unwrap_or_else(|| preview.top_line());
                    preview.anchor = Some(sync::map_line(patch, old, snapshot, line));
                }
                _ => {}
            },
        )
        .detach();
        let list_state = ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW));
        // Runs inside the list's own update: read it once that is over.
        let this = cx.weak_entity();
        list_state.set_scroll_handler(move |_, _, cx| {
            let this = this.clone();
            cx.defer(move |cx| {
                this.update(cx, |preview, cx| preview.user_scrolled(cx))
                    .ok();
            });
        });
        let mut preview = MarkdownPreview {
            path,
            dynamic,
            root,
            documents,
            workbench,
            focus_handle: cx.focus_handle(),
            _source: source,
            blocks: None,
            images: Arc::default(),
            image_cache: RetainAllImageCache::new(cx),
            pending_slug: None,
            revision: None,
            requested: None,
            pending: None,
            epoch: 0,
            parses_started: 0,
            too_large: false,
            state: None,
            list_state,
            pending_line: None,
            anchor: None,
            stand_in_height: None,
        };
        preview.documents_changed(cx);
        preview
    }

    /// The images loaded for this view (for tests).
    pub fn image_cache(&self) -> &Entity<RetainAllImageCache> {
        &self.image_cache
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
        self.images = Arc::default();
        // The old file's images go with it: a dynamic preview following
        // files would otherwise keep every image it has shown.
        self.image_cache = RetainAllImageCache::new(cx);
        self.pending_slug = None;
        self.revision = None;
        self.requested = None;
        self.pending = None;
        self.epoch += 1;
        self.too_large = false;
        self.state = None;
        self.pending_line = None;
        self.anchor = None;
        self.stand_in_height = None;
        self.list_state.reset(0);
        self.documents_changed(cx);
    }

    /// Opens the document again when it was dropped under the preview: the
    /// new document starts its revisions over, so the old ones are
    /// forgotten (the blocks stay shown until the new text is parsed).
    fn reopen_if_dropped(&mut self, cx: &mut Context<Self>) {
        if self.documents.read(cx).state(&self.path).is_some() {
            return;
        }
        let path = self.path.clone();
        self._source = self.documents.update(cx, |d, cx| d.open_preview(path, cx));
        self.revision = None;
        self.requested = None;
        self.pending = None;
        self.epoch += 1;
        self.anchor = None;
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

    /// The source line (0-based, fractional) at the top of the view: the
    /// top block's lines, scaled by how far into the block the view is
    /// scrolled ([`line_to_item`]). A line still waiting to be shown
    /// counts as shown.
    pub fn top_line(&self) -> f32 {
        if let Some(line) = self.pending_line {
            return line;
        }
        let Some(blocks) = &self.blocks else {
            return 0.;
        };
        let top = self.list_state.logical_scroll_top();
        let height = self
            .list_state
            .bounds_for_item(top.item_ix)
            .map_or(0., |b| f32::from(b.size.height));
        let fraction = if height > 0. {
            f32::from(top.offset_in_item) / height
        } else {
            0.
        };
        item_to_line(
            blocks.root_lines(),
            blocks.line_count(),
            top.item_ix,
            fraction,
        )
    }

    /// Scrolls source line `line` (0-based, fractional) to the top: its
    /// block, offset by the line's fraction of the block's height. Before
    /// the blocks are parsed, or while the block is not measured, the
    /// line waits and is shown on a later frame.
    pub fn scroll_to_line(&mut self, line: f32, cx: &mut Context<Self>) {
        self.anchor = None;
        self.stand_in_height = None;
        self.show_line(line, cx);
    }

    /// Scrolls `line` to the top, now or once it can be placed.
    fn show_line(&mut self, line: f32, cx: &mut Context<Self>) {
        self.place_line(line);
        cx.notify();
    }

    /// Scrolls `line` to the top, or keeps it waiting for a later frame.
    fn place_line(&mut self, line: f32) {
        if self.apply_line(line) {
            self.pending_line = None;
            self.stand_in_height = None;
        } else {
            self.pending_line = Some(line);
        }
    }

    /// Scrolls `line` to the top; `false` when it could not be placed
    /// exactly yet (no blocks, or its block not measured: then the stand-in
    /// height, if any, places it for now).
    fn apply_line(&self, line: f32) -> bool {
        let Some(blocks) = &self.blocks else {
            return false;
        };
        let (item_ix, fraction) = line_to_item(blocks.root_lines(), blocks.line_count(), line);
        self.list_state.scroll_to(ListOffset {
            item_ix,
            offset_in_item: px(0.),
        });
        if fraction == 0. {
            return true;
        }
        let (height, measured) = match self.list_state.bounds_for_item(item_ix) {
            Some(bounds) => (bounds.size.height, true),
            None => match self.stand_in_height {
                Some(height) => (height, false),
                None => return false,
            },
        };
        self.list_state.scroll_to(ListOffset {
            item_ix,
            offset_in_item: height * fraction,
        });
        measured
    }

    /// Scrolls the heading with GitHub slug `slug` to the top; before the
    /// blocks are parsed, once they are. A slug no heading has does
    /// nothing.
    pub fn scroll_to_slug(&mut self, slug: &str, cx: &mut Context<Self>) {
        if self.blocks.is_none() {
            self.pending_slug = Some(slug.to_owned());
            return;
        }
        if let Some(line) = self.slug_line(slug) {
            self.scroll_to_line(line, cx);
        }
    }

    /// The first source line of the heading with slug `slug`, matched
    /// ignoring case (slugs are lower case; `#Install` finds "Install").
    fn slug_line(&self, slug: &str) -> Option<f32> {
        let blocks = self.blocks.as_ref()?;
        let slug = slug.to_lowercase();
        let (_, ix) = heading_slugs(blocks)
            .into_iter()
            .find(|(s, _)| *s == slug)?;
        Some(blocks.root_lines().get(ix)?.start as f32)
    }

    /// The user scrolled: a line still waiting gives way, and the links
    /// hear of it.
    fn user_scrolled(&mut self, cx: &mut Context<Self>) {
        self.pending_line = None;
        self.pending_slug = None;
        self.anchor = None;
        self.stand_in_height = None;
        cx.emit(MarkdownPreviewEvent::Scrolled);
    }

    /// The scroll keys (spec §4.2): ↑/↓ a line of text, PageUp/PageDown
    /// and Space/Shift+Space a page, Home/End and ⌘↑/⌘↓ to the start or
    /// the end. Other keys go on to the parent.
    fn key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let stroke = &ev.keystroke;
        let mods = stroke.modifiers;
        if mods.control || mods.alt || mods.function {
            return;
        }
        let style = Self::style(cx);
        let line = px(style.text_size * style.line_height);
        let viewport = self.list_state.viewport_bounds().size.height;
        let page = (viewport - line).max(line);
        enum Move {
            By(Pixels),
            Start,
            End,
        }
        let to = match (stroke.key.as_str(), mods.platform, mods.shift) {
            ("up", false, false) => Move::By(-line),
            ("down", false, false) => Move::By(line),
            ("pageup", false, false) | ("space", false, true) => Move::By(-page),
            ("pagedown", false, false) | ("space", false, false) => Move::By(page),
            ("home", false, false) | ("up", true, false) => Move::Start,
            ("end", false, false) | ("down", true, false) => Move::End,
            _ => return,
        };
        match to {
            Move::By(distance) => self.list_state.scroll_by(distance),
            Move::Start => self.list_state.scroll_to(ListOffset::default()),
            Move::End => self.list_state.scroll_to_end(),
        }
        cx.stop_propagation();
        self.user_scrolled(cx);
        cx.notify();
    }

    /// The number of parses started so far (for tests).
    pub fn parses_started(&self) -> u64 {
        self.parses_started
    }

    /// Parses the document again now, without waiting for a change, and
    /// drops the loaded images, so images changed on disk load again
    /// (`markdown.preview.refresh`, spec §5.4).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.image_cache = RetainAllImageCache::new(cx);
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
        let images = ImageContext::for_file(&self.path, &self.root);
        let parse = cx.background_spawn(async move { parse_snapshot(snapshot, &images) });
        cx.spawn(async move |preview, cx| {
            let parsed = parse.await;
            preview
                .update(cx, |preview, cx| {
                    if preview.epoch == epoch {
                        preview.show(revision, Some(parsed), cx);
                    }
                })
                .ok();
        })
        .detach();
    }

    /// Shows the blocks parsed from `revision` (`None`: the file is too
    /// large), unless newer ones are shown, keeping the text at the top of
    /// the view (the anchor), within its block too; or showing the heading
    /// a link asked for.
    fn show(&mut self, revision: u64, parsed: Option<Parsed>, cx: &mut Context<Self>) {
        if self.revision.is_some_and(|shown| shown > revision) {
            return;
        }
        let mut top = self.anchor.unwrap_or_else(|| self.top_line());
        // `reset` forgets every height: keep the top block's for the
        // first frame.
        let top_ix = self.list_state.logical_scroll_top().item_ix;
        if let Some(bounds) = self.list_state.bounds_for_item(top_ix) {
            self.stand_in_height = Some(bounds.size.height);
        }
        // Edits made since this text was read move the anchor on.
        if self.documents.read(cx).revision_of(&self.path) == Some(revision) {
            self.anchor = None;
        }
        self.revision = Some(revision);
        self.too_large = parsed.is_none();
        let (blocks, images) =
            parsed.map_or((None, Arc::default()), |p| (Some(p.blocks), p.images));
        self.blocks = blocks;
        self.images = images;
        let count = self.blocks.as_ref().map_or(0, |b| b.blocks().len());
        self.list_state.reset(count);
        if let Some(line) = self
            .pending_slug
            .take()
            .and_then(|slug| self.slug_line(&slug))
        {
            top = line;
            self.anchor = None;
            self.stand_in_height = None;
        }
        self.show_line(top, cx);
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

    /// Draws the images of the blocks shown: local files through `img()`
    /// with the view's cache, as wide as they are up to the column's
    /// width; the others as their alt text.
    fn image_lookup(&self) -> ImageLookup {
        let (images, cache) = (self.images.clone(), self.image_cache.clone());
        Rc::new(move |url, alt, n, cx| match images.get(url) {
            Some(ImageSrc::Local(path)) => img(path.clone())
                .debug_selector(move || format!("md-image-{n}"))
                .image_cache(&cache)
                .max_w_full()
                .with_fallback(image_placeholder_builder(alt, None, n, cx))
                .into_any_element(),
            Some(ImageSrc::Blocked("too-large")) => {
                image_placeholder(alt, Some("Ảnh quá lớn"), n, cx)
            }
            _ => image_placeholder(alt, None, n, cx),
        })
    }

    /// Clicked links (spec §5.6), resolved against the previewed file:
    /// see [`commands::follow_link`].
    fn link_handler(&self, cx: &Context<Self>) -> LinkHandler {
        let (workbench, preview) = (self.workbench.clone(), cx.weak_entity());
        let root = self.root.clone();
        let base_dir: Arc<Path> = Arc::from(self.path.parent().unwrap_or(&self.root));
        Rc::new(move |url, window, cx| {
            let action = resolve_link(url, &base_dir, &root);
            let (workbench, preview) = (workbench.clone(), preview.clone());
            // Not inside the click: the link may replace this tab.
            window.defer(cx, move |window, cx| {
                workbench
                    .update(cx, |wb, cx| {
                        commands::follow_link(wb, preview, action, window, cx)
                    })
                    .ok();
            });
        })
    }

    /// "File quá lớn để xem trước" with a button to the source.
    fn render_too_large(&self, cx: &mut Context<Self>) -> AnyElement {
        let workbench = self.workbench.clone();
        let button = text_button("md-preview-show-source", "Show Source", cx).on_click(
            move |_, window, cx| {
                // Not while the preview's own click is being handled: the
                // command replaces its tab.
                let workbench = workbench.clone();
                window.defer(cx, move |window, cx| {
                    workbench
                        .update(cx, |wb, cx| wb.run_command(SHOW_SOURCE, None, window, cx))
                        .ok();
                });
            },
        );
        notice("md-preview-too-large", "File quá lớn để xem trước", cx)
            .child(button)
            .into_any_element()
    }
}

/// Parses the whole text of `snapshot` and resolves its image URLs
/// against `images` (which reads the disk); runs on a background thread.
fn parse_snapshot(snapshot: BufferSnapshot, images: &ImageContext) -> Parsed {
    let source = snapshot.text_for_range(0..snapshot.len_bytes());
    let blocks = parse_markdown_document(&source, None);
    let images = resolve::resolve_images(&blocks, images);
    Parsed {
        blocks: Arc::new(blocks),
        images: Arc::new(images),
    }
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

impl EventEmitter<MarkdownPreviewEvent> for MarkdownPreview {}

impl Focusable for MarkdownPreview {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MarkdownPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let root = div()
            .id("markdown-preview")
            .debug_selector(|| "markdown-preview".into())
            .track_focus(&self.focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|preview, _, window, cx| window.focus(&preview.focus_handle, cx)),
            )
            .on_key_down(cx.listener(Self::key_down))
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
        // The line waiting for its block's height: the last frame
        // measured the block (now at the top) unless it is still loading.
        if let Some(line) = self.pending_line {
            self.place_line(line);
            if self.pending_line.is_some() {
                window.request_animation_frame();
            }
        }
        let mut style = Self::style(cx);
        style.images = Some(self.image_lookup());
        let on_link = self.link_handler(cx);
        let last = blocks.blocks().len().saturating_sub(1);
        let (workbench, preview) = (self.workbench.clone(), cx.entity_id());
        let items = list(self.list_state.clone(), move |ix, _, cx| {
            let block = render_block(&blocks, ix, &style, on_link.clone(), cx);
            let line = blocks.root_lines().get(ix).map_or(0, |lines| lines.start);
            let workbench = workbench.clone();
            // Each item spans the list, so the whole page scrolls; its
            // content sits in the centred column.
            div()
                .debug_selector(move || format!("md-block-{ix}"))
                // A double-click opens the source at the block's first
                // line (`markdown.preview.doubleClickToSwitchToEditor`).
                .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
                    if ev.click_count != 2 {
                        return;
                    }
                    cx.stop_propagation();
                    let workbench = workbench.clone();
                    window.defer(cx, move |window, cx| {
                        workbench
                            .update(cx, |wb, cx| {
                                commands::double_clicked(wb, preview, line, window, cx)
                            })
                            .ok();
                    });
                })
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
