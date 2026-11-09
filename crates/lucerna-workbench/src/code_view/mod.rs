//! The read-only code view: a virtualized, syntax-highlighted view of a
//! [`BufferSnapshot`] with selection, word wrap, folding and a minimap.
//!
//! Only rows on screen (plus a small overscan) are read from the buffer
//! and shaped; shaped rows, highlights and wrap counts are cached per
//! snapshot revision. Large files (`lucerna.largeFile.*`) get no
//! highlighting, folding or minimap.

mod cache;
mod element;
mod layout;
mod minimap;
mod selection;

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use gpui::{
    AnyView, App, ClipboardItem, Context, Entity, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Task, WeakEntity, Window, div,
    prelude::*, px,
};
use lucerna_syntax::{SyntaxState, indent_folds};
use lucerna_text::{Buffer, BufferSnapshot, LoadError, Point, Reload};

pub use layout::{
    DisplayLine, FoldState, MAX_RENDERED_LINE_CHARS, display_lines, display_lines_with_tab_size,
};

use self::cache::Caches;
use self::element::{CodeElement, Frame};
use self::selection::Selection;
use crate::documents::{DocState, Documents};
use crate::editor_groups::{EditorGroupsView, Tab};
use crate::find_widget::FindWidget;
use crate::layout::color;
use crate::tasks::{catch_panic, guarded};
use crate::ui::GRID;
use crate::{AppState, CommandRegistry, Workbench};

/// The status-bar text shown while the active view is a large file.
pub const LARGE_FILE_STATUS: &str = "Large file: highlighting disabled";

const MIN_FONT_SIZE: f32 = 6.;
const MAX_FONT_SIZE: f32 = 40.;

/// The `editor.*` settings a view follows; re-read on every settings
/// change by [`CodeView::reload_config`].
#[derive(Debug, Clone, PartialEq)]
struct Config {
    font_family: SharedString,
    /// `editor.lineHeight`: below 8 a multiple of the font size, else px.
    line_height: f32,
    minimap: bool,
    render_whitespace: bool,
    tab_size: u32,
    max_bytes: u64,
    max_lines: u32,
}

impl Config {
    /// The settings for a view of `language` (a language id such as
    /// `"rust"`), so `"[rust]": {...}` overrides apply.
    fn read(language: Option<&str>, cx: &App) -> (Config, f32, bool) {
        let settings = &cx.global::<AppState>().settings;
        let config = Config {
            font_family: settings
                .get::<String>("editor.fontFamily", language)
                .unwrap_or_else(|| "Menlo".into())
                .into(),
            line_height: settings.get("editor.lineHeight", language).unwrap_or(1.5),
            minimap: settings
                .get("editor.minimap.enabled", language)
                .unwrap_or(true),
            render_whitespace: settings
                .get::<String>("editor.renderWhitespace", language)
                .is_some_and(|w| w == "all"),
            tab_size: settings
                .get::<u32>("editor.tabSize", language)
                .unwrap_or(4)
                .max(1),
            max_bytes: settings
                .get("lucerna.largeFile.maxBytes", language)
                .unwrap_or(20 * 1024 * 1024),
            max_lines: settings
                .get("lucerna.largeFile.maxLines", language)
                .unwrap_or(300_000),
        };
        let font_size = settings
            .get::<f32>("editor.fontSize", language)
            .unwrap_or(13.)
            .clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        let wrap = settings
            .get::<String>("editor.wordWrap", language)
            .is_some_and(|w| w == "on");
        (config, font_size, wrap)
    }
}

/// The scroll position: the top visible buffer row, and how many pixels
/// of that row's display lines are scrolled out above the viewport.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Scroll {
    top_row: u32,
    offset: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    Text,
    Minimap,
}

pub struct CodeView {
    path: Arc<Path>,
    /// The id of the file's language, for `"[lang]"` settings overrides.
    language_id: Option<&'static str>,
    focus: FocusHandle,
    snapshot: BufferSnapshot,
    /// Bumped by every new snapshot; keys every cache.
    revision: u64,
    /// The last revision a frame was built for.
    painted_revision: u64,
    syntax: Option<Arc<SyntaxState>>,
    large: bool,
    config: Config,
    font_size: f32,
    word_wrap: bool,
    /// `editor.fontSize` and `editor.wordWrap` as last read: a settings
    /// change overrides zoom and the wrap toggle only when these change.
    settings_font_size: f32,
    settings_word_wrap: bool,
    /// Re-reads the config whenever [`AppState`] changes.
    _settings: Subscription,
    scroll: Scroll,
    scroll_x: f32,
    /// Widest line laid out since the text, wrap or font last changed.
    max_line_width: f32,
    selection: Option<Selection>,
    /// Both brackets of the pair next to the cursor, as byte offsets.
    brackets: Option<[usize; 2]>,
    drag: Option<Drag>,
    folds: FoldState,
    /// Every fold range of the current snapshot, sorted by header row.
    fold_ranges: Vec<Range<u32>>,
    _fold_task: Option<Task<()>>,
    caches: Caches,
    /// The last laid-out frame, for hit testing and paging.
    frame: Option<Arc<Frame>>,
    /// A row to center once the viewport size is known.
    pending_center: Option<u32>,
    /// The file's state; errors replace the text with a message.
    doc_state: DocState,
    /// Where the error actions ("Retry", "Open as text") go.
    documents: Option<WeakEntity<Documents>>,
    /// Search matches painted with `search_match_background`: sorted,
    /// non-overlapping byte ranges. Cleared by every new snapshot.
    match_highlights: Arc<[Range<usize>]>,
    /// The index in `match_highlights` drawn with an outline.
    active_match: Option<usize>,
    /// The find widget, kept while hidden so it remembers its query.
    find: Option<Entity<FindWidget>>,
    find_visible: bool,
}

impl CodeView {
    /// An empty view of `path`; [`CodeView::set_buffer`] gives it text.
    pub fn new(path: Arc<Path>, cx: &mut Context<Self>) -> Self {
        // The same lookup `Documents` makes for the file's syntax.
        let language_id = cx
            .global::<AppState>()
            .languages
            .language_for_path(&path)
            .map(|lang| lang.id);
        let (config, font_size, word_wrap) = Config::read(language_id, cx);
        let snapshot = Buffer::from_bytes(b"", false)
            .expect("empty text decodes")
            .snapshot();
        CodeView {
            path,
            language_id,
            focus: cx.focus_handle(),
            snapshot,
            revision: 0,
            painted_revision: 0,
            syntax: None,
            large: false,
            config,
            font_size,
            word_wrap,
            settings_font_size: font_size,
            settings_word_wrap: word_wrap,
            _settings: cx.observe_global::<AppState>(Self::reload_config),
            scroll: Scroll::default(),
            scroll_x: 0.,
            max_line_width: 0.,
            selection: None,
            brackets: None,
            drag: None,
            folds: FoldState::default(),
            fold_ranges: Vec::new(),
            _fold_task: None,
            caches: Caches::default(),
            frame: None,
            pending_center: None,
            doc_state: DocState::Ready,
            documents: None,
            match_highlights: Arc::from([]),
            active_match: None,
            find: None,
            find_visible: false,
        }
    }

    /// A [`crate::TabBodyFactory`] that gives each tab an empty code view.
    pub fn tab_body(tab: &Tab, _: &mut Window, cx: &mut App) -> AnyView {
        let path = tab.path.clone();
        cx.new(|cx| CodeView::new(path, cx)).into()
    }

    pub fn path(&self) -> &Arc<Path> {
        &self.path
    }

    pub fn snapshot(&self) -> &BufferSnapshot {
        &self.snapshot
    }

    /// The parsed syntax shown, if any.
    pub fn syntax(&self) -> Option<&Arc<SyntaxState>> {
        self.syntax.as_ref()
    }

    /// The state of the file shown; [`DocState::Ready`] for a view not
    /// made by [`Documents`].
    pub fn doc_state(&self) -> &DocState {
        &self.doc_state
    }

    pub(crate) fn set_doc_state(&mut self, state: DocState, cx: &mut Context<Self>) {
        if self.doc_state != state {
            self.doc_state = state;
            cx.notify();
        }
    }

    pub(crate) fn set_documents(&mut self, documents: WeakEntity<Documents>) {
        self.documents = Some(documents);
    }

    /// Shows `snapshot` from the top, dropping the selection, folds and
    /// syntax. Use [`CodeView::apply_reload`] for a new version of the
    /// same file.
    pub fn set_buffer(&mut self, snapshot: BufferSnapshot, cx: &mut Context<Self>) {
        self.replace_snapshot(snapshot, cx);
        self.syntax = None;
        self.scroll = Scroll::default();
        self.scroll_x = 0.;
        self.selection = None;
        self.brackets = None;
        self.folds.clear();
        self.pending_center = None;
        cx.notify();
    }

    /// Shows a reloaded version of the file, keeping the scroll position,
    /// selection and closed folds on the same lines; a selection touching
    /// a changed row is cleared, as is a fold whose header changed. The
    /// syntax tree is edited to the new text and stays until the next
    /// [`CodeView::set_syntax`], so colours do not flicker off.
    pub fn apply_reload(
        &mut self,
        snapshot: BufferSnapshot,
        reload: Reload,
        cx: &mut Context<Self>,
    ) {
        let rows = snapshot.line_count();
        let adjust = |p: Point| Point {
            row: reload.adjust_row(p.row, rows),
            column: p.column,
        };
        self.scroll = Scroll {
            top_row: reload.adjust_row(self.scroll.top_row, rows),
            offset: 0.,
        };
        let changed = reload.start_row..reload.old_end_row;
        self.selection = self
            .selection
            .filter(|s| {
                let r = s.range();
                r.end.row < changed.start || r.start.row >= changed.end
            })
            .map(|s| Selection {
                anchor: adjust(s.anchor),
                head: adjust(s.head),
            });
        let old = std::mem::replace(&mut self.snapshot, snapshot.clone());
        self.syntax = self
            .syntax
            .as_ref()
            .map(|syntax| Arc::new(syntax.edited(&old, &snapshot, &reload)));
        self.replace_snapshot(snapshot, cx);
        self.folds.remap(&reload, rows);
        self.scroll.top_row = self.folds.prev_visible(self.scroll.top_row);
        self.update_brackets();
        cx.notify();
    }

    /// Sets the parsed syntax of the current snapshot; `None` shows plain
    /// text. Ignored for highlighting while the file is large.
    pub fn set_syntax(&mut self, syntax: Option<Arc<SyntaxState>>, cx: &mut Context<Self>) {
        self.syntax = syntax;
        self.caches.syntax_changed();
        self.update_brackets();
        cx.notify();
    }

    fn replace_snapshot(&mut self, snapshot: BufferSnapshot, cx: &mut Context<Self>) {
        self.match_highlights = Arc::from([]);
        self.active_match = None;
        self.large = snapshot.len_bytes() as u64 > self.config.max_bytes
            || snapshot.line_count() > self.config.max_lines;
        self.snapshot = snapshot;
        self.revision += 1;
        self.max_line_width = 0.;
        self.caches.clear();
        self.fold_ranges.clear();
        self._fold_task = (!self.large).then(|| self.compute_folds(cx));
    }

    /// Re-reads the `editor.*` settings and applies what changed:
    /// font, line height, wrap, minimap, whitespace, tab size and the
    /// large-file limits. Font-size zoom and the wrap toggle survive
    /// unless their setting changed, and the top visible row stays. Called on every [`AppState`] change.
    pub fn reload_config(&mut self, cx: &mut Context<Self>) {
        let (config, font_size, word_wrap) = Config::read(self.language_id, cx);
        let old_line_height = self.line_height();
        let mut changed = false;
        let mut wrap_changed = false;
        if font_size != self.settings_font_size {
            self.settings_font_size = font_size;
            self.font_size = font_size;
            changed = true;
        }
        if word_wrap != self.settings_word_wrap {
            self.settings_word_wrap = word_wrap;
            self.word_wrap = word_wrap;
            self.scroll_x = 0.;
            wrap_changed = true;
            changed = true;
        }
        if config != self.config {
            let refold = config.tab_size != self.config.tab_size
                || config.max_bytes != self.config.max_bytes
                || config.max_lines != self.config.max_lines;
            self.config = config;
            if refold {
                self.large = self.snapshot.len_bytes() as u64 > self.config.max_bytes
                    || self.snapshot.line_count() > self.config.max_lines;
                self.folds.clear();
                self.fold_ranges.clear();
                self._fold_task = (!self.large).then(|| self.compute_folds(cx));
            }
            changed = true;
        }
        if changed {
            self.caches.layout_changed();
            // The top buffer row stays; the part of it scrolled out keeps
            // its share of the (maybe new) line height.
            self.scroll.offset = if wrap_changed {
                0.
            } else {
                self.scroll.offset * self.line_height() / old_line_height
            };
            self.max_line_width = 0.;
            cx.notify();
        }
    }

    /// Fold ranges are found once per snapshot, off the main thread.
    fn compute_folds(&self, cx: &mut Context<Self>) -> Task<()> {
        let snapshot = self.snapshot.clone();
        let (revision, tab_size) = (self.revision, self.config.tab_size);
        let folds = cx.background_executor().spawn(async move {
            guarded("folds", || {
                indent_folds(&snapshot, tab_size, 0..snapshot.line_count())
            })
            .unwrap_or_default()
        });
        cx.spawn(async move |this, cx| {
            let folds = folds.await;
            this.update(cx, |view, cx| {
                if view.revision == revision {
                    view.folds.retain_available(&folds);
                    view.fold_ranges = folds;
                    cx.notify();
                }
            })
            .ok();
        })
    }

    /// Bumped by every new snapshot (load or reload).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Paints `ranges` (sorted, non-overlapping byte ranges) with
    /// `search_match_background`, and outlines `active`. Only the ranges
    /// on screen are painted. A new snapshot clears them.
    pub fn set_match_highlights(
        &mut self,
        ranges: impl Into<Arc<[Range<usize>]>>,
        active: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.match_highlights = ranges.into();
        self.active_match = active.filter(|&i| i < self.match_highlights.len());
        cx.notify();
    }

    pub fn match_highlights(&self) -> &[Range<usize>] {
        &self.match_highlights
    }

    pub fn active_match(&self) -> Option<usize> {
        self.active_match
    }

    /// The find widget, while it is shown.
    pub fn find_widget(&self) -> Option<&Entity<FindWidget>> {
        self.find.as_ref().filter(|_| self.find_visible)
    }

    /// The find widget, shown or not.
    pub(crate) fn find_widget_any(&self) -> Option<&Entity<FindWidget>> {
        self.find.as_ref()
    }

    pub(crate) fn show_find_widget(&mut self, find: Entity<FindWidget>, cx: &mut Context<Self>) {
        self.find = Some(find);
        self.find_visible = true;
        cx.notify();
    }

    /// Hides the find widget and clears its highlights.
    pub(crate) fn hide_find_widget(&mut self, cx: &mut Context<Self>) {
        self.find_visible = false;
        self.set_match_highlights(Vec::new(), None, cx);
    }

    /// Whether the file is over the large-file limits.
    pub fn is_large(&self) -> bool {
        self.large
    }

    /// Fold ranges of the current snapshot; empty for large files and
    /// until they have been computed.
    pub fn fold_ranges(&self) -> &[Range<u32>] {
        &self.fold_ranges
    }

    /// Whether the fold starting at `header_row` is closed.
    pub fn is_folded(&self, header_row: u32) -> bool {
        self.folds.is_folded(header_row)
    }

    /// Folds or unfolds the fold starting at `header_row`.
    pub fn toggle_fold(&mut self, header_row: u32, cx: &mut Context<Self>) {
        self.folds.toggle(header_row, &self.fold_ranges);
        self.scroll.top_row = self.folds.prev_visible(self.scroll.top_row);
        cx.notify();
    }

    /// The rows whose highlights are cached, if any were requested.
    pub fn cached_highlight_rows(&self) -> Option<Range<u32>> {
        self.caches.highlight_rows()
    }

    /// The topmost visible buffer row.
    pub fn scroll_top_row(&self) -> u32 {
        self.scroll.top_row
    }

    /// Scrolls `row` to the top, clamped to the last row.
    pub fn set_scroll_top_row(&mut self, row: u32, cx: &mut Context<Self>) {
        let last = self.snapshot.line_count().saturating_sub(1);
        self.scroll = Scroll {
            top_row: self.folds.prev_visible(row.min(last)),
            offset: 0.,
        };
        self.pending_center = None;
        cx.notify();
    }

    /// How many whole rows fit in the viewport; 0 before the first paint.
    pub fn visible_rows(&self) -> u32 {
        self.frame.as_ref().map_or(0, |f| f.visible_rows())
    }

    /// The selected range, start before end; `None` when nothing is
    /// selected.
    pub fn selection(&self) -> Option<Range<Point>> {
        self.selection.filter(|s| !s.is_empty()).map(|s| s.range())
    }

    /// The cursor: the moving end of the selection.
    pub fn cursor(&self) -> Point {
        self.selection
            .map_or(Point { row: 0, column: 0 }, |s| s.head)
    }

    /// Selects `range`; its end becomes the cursor.
    pub fn select(&mut self, range: Range<Point>, cx: &mut Context<Self>) {
        self.set_selection(
            Selection {
                anchor: range.start,
                head: range.end,
            },
            cx,
        );
    }

    fn set_selection(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.selection = Some(selection);
        self.update_brackets();
        cx.notify();
    }

    /// The selected text, with ends snapped to char boundaries.
    pub fn copy_selection(&self) -> Option<String> {
        let range = self.selection()?;
        Some(selection::text_for(&self.snapshot, range))
    }

    /// Finds the pair of the bracket next to the cursor; uses the syntax
    /// tree when there is one, so brackets in strings are skipped.
    fn update_brackets(&mut self) {
        self.brackets = self.selection.and_then(|s| {
            let at = selection::boundary_offset(&self.snapshot, s.head);
            // On the UI thread: a panic is logged and shows no pair.
            let matching = |offset| {
                catch_panic("brackets", || match &self.syntax {
                    Some(syntax) if syntax.tree().is_some() => {
                        syntax.matching_bracket(&self.snapshot, offset)
                    }
                    _ => lucerna_syntax::matching_bracket(&self.snapshot, offset),
                })
                .ok()
                .flatten()
                .filter(|&at| at < self.snapshot.len_bytes())
            };
            let partner = matching(at)?;
            Some([matching(partner)?, partner])
        });
    }

    /// `editor.tabSize` for this view's language.
    pub fn tab_size(&self) -> u32 {
        self.config.tab_size
    }

    pub fn word_wrap(&self) -> bool {
        self.word_wrap
    }

    pub fn toggle_word_wrap(&mut self, cx: &mut Context<Self>) {
        self.word_wrap = !self.word_wrap;
        self.scroll.offset = 0.;
        self.scroll_x = 0.;
        self.max_line_width = 0.;
        cx.notify();
    }

    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    /// Changes the font size by `delta`, clamped to 6–40.
    pub fn zoom(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.font_size = (self.font_size + delta).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        self.scroll.offset = 0.;
        self.max_line_width = 0.;
        cx.notify();
    }

    /// Puts the cursor at `p` and scrolls its row to the middle of the
    /// viewport, opening any fold that hides it.
    pub fn go_to(&mut self, p: Point, cx: &mut Context<Self>) {
        let last = self.snapshot.line_count().saturating_sub(1);
        let p = Point {
            row: p.row.min(last),
            column: p.column,
        };
        while let Some(header) = self.folds.hiding_header(p.row) {
            self.folds.toggle(header, &self.fold_ranges);
        }
        self.set_selection(Selection::cursor(p), cx);
        match self.visible_rows() {
            0 => {
                self.scroll = Scroll {
                    top_row: p.row,
                    offset: 0.,
                };
                self.pending_center = Some(p.row);
            }
            visible => self.center(p.row, visible),
        }
        cx.notify();
    }

    fn center(&mut self, row: u32, visible_rows: u32) {
        self.scroll = Scroll {
            top_row: self
                .folds
                .prev_visible(row.saturating_sub(visible_rows / 2)),
            offset: 0.,
        };
        self.pending_center = None;
    }

    fn line_height(&self) -> f32 {
        let lh = self.config.line_height;
        if lh < 8. { lh * self.font_size } else { lh }.max(1.)
    }

    /// Display lines `row` takes at the current wrap width.
    fn wrap_count(&mut self, row: u32) -> u32 {
        let wrap = self.wrap_chars();
        self.caches.wrap_count(
            &self.snapshot,
            self.revision,
            row,
            wrap,
            self.config.tab_size,
        )
    }

    fn wrap_chars(&self) -> Option<u32> {
        if !self.word_wrap {
            return None;
        }
        self.frame.as_ref().map(|f| f.wrap_chars).unwrap_or(None)
    }

    /// Scrolls down by `dy` pixels (up when negative), walking only the
    /// rows passed over.
    fn scroll_by(&mut self, dy: f32) {
        let lh = self.line_height();
        let line_count = self.snapshot.line_count();
        let mut scroll = self.scroll;
        scroll.offset += dy;
        while scroll.offset < 0. {
            if scroll.top_row == 0 {
                scroll.offset = 0.;
                break;
            }
            scroll.top_row = self.folds.prev_visible(scroll.top_row - 1);
            scroll.offset += self.wrap_count(scroll.top_row) as f32 * lh;
        }
        loop {
            let height = self.wrap_count(scroll.top_row) as f32 * lh;
            let next = self.folds.next_visible(scroll.top_row + 1);
            if next >= line_count {
                scroll.offset = scroll.offset.clamp(0., (height - lh).max(0.));
                break;
            }
            if scroll.offset < height {
                break;
            }
            scroll.offset -= height;
            scroll.top_row = next;
        }
        self.scroll = scroll;
        self.pending_center = None;
    }
}

/// Runs `f` on the code view in the active tab, if it shows one.
fn with_active_view(
    wb: &mut Workbench,
    cx: &mut Context<Workbench>,
    f: impl FnOnce(&mut CodeView, &mut Context<CodeView>),
) {
    let view = wb.editor_groups().read(cx).active_body();
    if let Some(view) = view.and_then(|v| v.downcast::<CodeView>().ok()) {
        view.update(cx, f);
    }
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "editor.action.toggleWordWrap",
        "View: Toggle Word Wrap",
        |wb, _, _, cx| with_active_view(wb, cx, CodeView::toggle_word_wrap),
    );
    commands.register(
        "editor.action.fontZoomIn",
        "Editor Font Zoom In",
        |wb, _, _, cx| with_active_view(wb, cx, |v, cx| v.zoom(1., cx)),
    );
    commands.register(
        "editor.action.fontZoomOut",
        "Editor Font Zoom Out",
        |wb, _, _, cx| with_active_view(wb, cx, |v, cx| v.zoom(-1., cx)),
    );
    commands.register(
        "editor.action.clipboardCopyAction",
        "Copy",
        |wb, _, _, cx| {
            with_active_view(wb, cx, |v, cx| {
                if let Some(text) = v.copy_selection() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            })
        },
    );
}

impl Focusable for CodeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CodeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.painted_revision != self.revision {
            self.painted_revision = self.revision;
            tracing::debug!(
                path = %self.path.display(),
                revision = self.revision,
                "first render of revision"
            );
        }
        let ui = &cx.global::<AppState>().theme.ui;
        match &self.doc_state {
            DocState::Ready | DocState::Deleted => {}
            DocState::Loading => {
                return div()
                    .id("code-view")
                    .track_focus(&self.focus)
                    .size_full()
                    .bg(color(ui.editor_background));
            }
            DocState::Error(err) => return self.render_error(err.clone(), cx),
        }
        let show_minimap = self.config.minimap && !self.large;
        let view = cx.entity();
        div()
            .id("code-view")
            .track_focus(&self.focus)
            .flex()
            .flex_row()
            .size_full()
            .bg(color(ui.editor_background))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(CodeElement::new(view.clone()))
                    .children(self.find_widget().cloned().map(|find| {
                        div()
                            .absolute()
                            .top(px(GRID))
                            .right(px(4. * GRID))
                            .child(find)
                    })),
            )
            .when(show_minimap, |d| d.child(minimap::render(view)))
    }
}

/// What the error block offers for each load error.
enum ErrorAction {
    Retry,
    OpenAsText,
    None,
}

impl CodeView {
    /// A centred, muted message with at most one action button.
    fn render_error(&self, err: LoadError, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let ui = &cx.global::<AppState>().theme.ui;
        let (message, action) = match err {
            LoadError::Binary => ("Binary file".to_string(), ErrorAction::OpenAsText),
            LoadError::TooLarge { .. } => ("File is larger than 512 MB".into(), ErrorAction::None),
            err @ (LoadError::NotFound | LoadError::PermissionDenied | LoadError::Io(_)) => {
                (err.to_string(), ErrorAction::Retry)
            }
        };
        let hover = color(ui.ghost_element_hover);
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .px(px(2. * GRID))
                .py(px(GRID))
                .rounded(px(GRID))
                .border_1()
                .border_color(color(ui.border_variant))
                .text_color(color(ui.text))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(label)
        };
        let path = self.path.clone();
        let documents = self.documents.clone();
        let button = match action {
            ErrorAction::Retry => Some(button("doc-retry", "Retry").on_click(move |_, _, cx| {
                if let Some(docs) = documents.as_ref().and_then(WeakEntity::upgrade) {
                    docs.update(cx, |docs, cx| docs.retry(&path, cx));
                }
            })),
            ErrorAction::OpenAsText => Some(button("doc-open-as-text", "Open as text").on_click(
                move |_, _, cx| {
                    if let Some(docs) = documents.as_ref().and_then(WeakEntity::upgrade) {
                        docs.update(cx, |docs, cx| docs.open_as_text(&path, cx));
                    }
                },
            )),
            ErrorAction::None => None,
        };
        div()
            .id("code-view")
            .track_focus(&self.focus)
            .flex()
            .size_full()
            .justify_center()
            .items_center()
            .bg(color(ui.editor_background))
            .child(
                div()
                    .debug_selector(|| "doc-error".into())
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(3. * GRID))
                    .max_w(px(480.))
                    .text_color(color(ui.text_muted))
                    .child(message)
                    .children(button),
            )
    }
}

/// The status-bar notice for a large file in the active tab.
pub(crate) struct LargeFileStatus {
    groups: Entity<EditorGroupsView>,
}

impl LargeFileStatus {
    pub(crate) fn new(groups: Entity<EditorGroupsView>) -> Self {
        LargeFileStatus { groups }
    }
}

impl Render for LargeFileStatus {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let large = self
            .groups
            .read(cx)
            .active_body()
            .and_then(|v| v.downcast::<CodeView>().ok())
            .is_some_and(|v| v.read(cx).is_large());
        div().when(large, |d| {
            d.debug_selector(|| "large-file-status".into())
                .child(LARGE_FILE_STATUS)
        })
    }
}

/// Converts a theme color to a GPUI fill color.
fn hsla(c: lucerna_theme::Rgba) -> gpui::Hsla {
    color(c).into()
}
