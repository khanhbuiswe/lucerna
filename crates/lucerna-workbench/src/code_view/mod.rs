//! The read-only code view: a virtualized, syntax-highlighted view of a
//! [`BufferSnapshot`] with selection, word wrap, folding and a minimap.
//!
//! Only rows on screen (plus a small overscan) are read from the buffer
//! and shaped; shaped rows, highlights and wrap counts are cached per
//! snapshot revision. Large files (`lucerna.largeFile.*`) get no
//! highlighting, folding or minimap.

mod cache;
mod diagnostics;
mod element;
mod layout;
mod link;
mod minimap;
mod selection;

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use gpui::{
    AnyView, App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Task, WeakEntity,
    Window, div, prelude::*, px,
};
use lucerna_lsp::diagnostics::Diagnostic;
use lucerna_syntax::{SyntaxState, indent_folds};
use lucerna_text::{Buffer, BufferSnapshot, LoadError, Point, Reload};

pub use diagnostics::{DiagnosticRows, Mark, MarkStyle};
pub use layout::{
    DisplayLine, FoldState, MAX_RENDERED_LINE_CHARS, display_lines, display_lines_with_tab_size,
};
pub(crate) use selection::word_at;

use self::cache::Caches;
use self::element::{CodeElement, Frame};
use self::selection::Selection;
use crate::documents::{DocState, Documents};
use crate::editor_groups::{EditorGroupsView, Tab};
use crate::find_widget::FindWidget;
use crate::goto::GotoMessage;
use crate::hover::{HoverConfig, HoverController};
use crate::language_servers::LanguageServers;
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
    /// `editor.cursorSurroundingLines`: rows kept between the cursor and
    /// the viewport edges by [`CodeView::set_cursor`].
    cursor_surrounding_lines: u32,
    /// `lucerna.diagnostics.gutterIcons`.
    gutter_icons: bool,
    /// `lucerna.diagnostics.inline.enabled`.
    inline_diagnostics: bool,
    hover: HoverConfig,
}

impl Config {
    /// The settings for a view of `language` (a language id such as
    /// `"rust"`), so `"[rust]": {...}` overrides apply.
    fn read(language: Option<&str>, cx: &App) -> (Config, f32, bool) {
        let settings = &cx.global::<AppState>().settings;
        let config = Config {
            font_family: settings
                .get::<String>("editor.fontFamily", language)
                .unwrap_or_else(|| crate::fonts::EDITOR_FONT_FAMILY.into())
                .into(),
            line_height: settings.get("editor.lineHeight", language).unwrap_or(1.6),
            minimap: settings
                .get("editor.minimap.enabled", language)
                .unwrap_or(false),
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
            cursor_surrounding_lines: settings
                .get("editor.cursorSurroundingLines", language)
                .unwrap_or(0),
            gutter_icons: settings
                .get("lucerna.diagnostics.gutterIcons", language)
                .unwrap_or(true),
            inline_diagnostics: settings
                .get("lucerna.diagnostics.inline.enabled", language)
                .unwrap_or(false),
            hover: HoverConfig::read(settings, language),
        };
        let font_size = settings
            .get::<f32>("editor.fontSize", language)
            .unwrap_or(14.)
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

/// What [`CodeView::fold_at`] does to the fold it finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldOp {
    Toggle,
    Close,
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    /// Fills the cell of the char under the cursor.
    Block,
}

/// A cursor painted over the text (vim's normal-mode cursor); the view
/// paints none by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorOverlay {
    pub at: Point,
    pub shape: CursorShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeViewEvent {
    /// Emitted by [`CodeView::apply_reload`] once the new text is shown.
    Reloaded { reload: Reload },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    Text,
    Minimap,
}

pub struct CodeView {
    path: Arc<Path>,
    /// The id of the file's language, for `"[lang]"` settings overrides.
    language_id: Option<Arc<str>>,
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
    /// A rectangular selection, one range per row; replaces `selection`
    /// while set.
    block_selection: Option<Vec<Range<Point>>>,
    /// Bumped by every selection change, whatever made it.
    selection_generation: u64,
    cursor_overlay: Option<CursorOverlay>,
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
    /// Rows painted across with `editor.highlighted_line.background`:
    /// half-open row ranges, such as the place a picker previews.
    line_highlights: Arc<[Range<u32>]>,
    /// A picker's preview: a click leaves the focus where it is, and the
    /// document's pending jumps go to other views.
    passive: bool,
    /// The find widget, kept while hidden so it remembers its query.
    find: Option<Entity<FindWidget>>,
    find_visible: bool,
    /// Whether the mouse is over the gutter: fold chevrons show only then
    /// (and always on folded rows).
    gutter_hovered: bool,
    /// The file's diagnostics, as last set by the language servers.
    diagnostics: Arc<[Diagnostic]>,
    /// Whether a language server feeds this view: the gutter gets its
    /// icon column from then on, even before anything is published.
    diagnostics_source: bool,
    /// `diagnostics` as painted, against the current snapshot.
    diagnostic_rows: DiagnosticRows,
    /// Bumped whenever `diagnostic_rows` is rebuilt.
    diagnostic_rows_generation: u64,
    /// The scrollbar marks last laid out, reused while nothing they
    /// depend on changed.
    scrollbar_marks: Option<element::ScrollbarMarks>,
    /// The LSP hover of this view.
    hover: HoverController,
    /// The "No definition found" message, until it times out or the
    /// cursor moves.
    goto_message: Option<GotoMessage>,
    /// The ⌘-hover definition link.
    link: link::Link,
    /// Hides the hover and the link when focus leaves the view; made on
    /// first render.
    _focus_out: Option<Subscription>,
}

impl CodeView {
    /// An empty view of `path`; [`CodeView::set_buffer`] gives it text.
    pub fn new(path: Arc<Path>, cx: &mut Context<Self>) -> Self {
        // A first guess; `Documents` sets the language it resolves.
        let language_id = cx
            .global::<AppState>()
            .languages
            .language_for_file(&path, "")
            .map(|lang| lang.id.clone());
        let (config, font_size, word_wrap) = Config::read(language_id.as_deref(), cx);
        let snapshot = Buffer::from_bytes(b"", false)
            .expect("empty text decodes")
            .snapshot();
        let hover = HoverController::new(path.clone(), config.hover, snapshot.clone());
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
            block_selection: None,
            selection_generation: 0,
            cursor_overlay: None,
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
            line_highlights: Arc::from([]),
            passive: false,
            find: None,
            find_visible: false,
            gutter_hovered: false,
            diagnostics: Arc::from([]),
            diagnostics_source: false,
            diagnostic_rows: DiagnosticRows::default(),
            diagnostic_rows_generation: 0,
            scrollbar_marks: None,
            hover,
            goto_message: None,
            link: link::Link::default(),
            _focus_out: None,
        }
    }

    /// A [`crate::TabBodyFactory`] that gives each file tab an empty code
    /// view (other tabs get [`crate::placeholder_body`]).
    pub fn tab_body(tab: &Tab, window: &mut Window, cx: &mut App) -> AnyView {
        match tab.path() {
            Some(path) => {
                let path = path.clone();
                cx.new(|cx| CodeView::new(path, cx)).into()
            }
            None => crate::placeholder_body(tab, window, cx),
        }
    }

    pub fn path(&self) -> &Arc<Path> {
        &self.path
    }

    pub fn snapshot(&self) -> &BufferSnapshot {
        &self.snapshot
    }

    /// The id of the file's language: `editorLangId` and the `"[id]"`
    /// settings override key.
    pub fn language_id(&self) -> Option<&str> {
        self.language_id.as_deref()
    }

    /// Sets the file's language id (as its document resolved it) and
    /// re-reads the settings that may be overridden per language.
    pub fn set_language_id(&mut self, id: Option<Arc<str>>, cx: &mut Context<Self>) {
        if self.language_id != id {
            self.language_id = id;
            self.reload_config(cx);
            cx.notify();
        }
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
        self.block_selection = None;
        self.selection_generation += 1;
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
        self.hover.hide(cx);
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
        // Like the selection: dropped if any of its rows changed.
        self.block_selection = self
            .block_selection
            .take()
            .filter(|block| {
                block
                    .iter()
                    .all(|r| r.end.row < changed.start || r.start.row >= changed.end)
            })
            .map(|block| {
                block
                    .into_iter()
                    .map(|r| adjust(r.start)..adjust(r.end))
                    .collect()
            });
        self.selection_generation += 1;
        let old = std::mem::replace(&mut self.snapshot, snapshot.clone());
        self.syntax = self.syntax.as_ref().and_then(|syntax| {
            catch_panic("edit syntax", || syntax.edited(&old, &snapshot, &reload))
                .ok()
                .map(Arc::new)
        });
        self.replace_snapshot(snapshot, cx);
        self.folds.remap(&reload, rows);
        self.scroll.top_row = self.folds.prev_visible(self.scroll.top_row);
        self.update_brackets();
        cx.emit(CodeViewEvent::Reloaded { reload });
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
        self.hover.set_snapshot(self.snapshot.clone(), cx);
        self.reset_link(cx);
        self.set_goto_message(None, cx);
        self.revision += 1;
        self.max_line_width = 0.;
        self.caches.clear();
        self.fold_ranges.clear();
        self._fold_task = (!self.large).then(|| self.compute_folds(cx));
        // Clamped to the new text until the servers publish again.
        self.diagnostic_rows = DiagnosticRows::build(&self.diagnostics, &self.snapshot);
        self.diagnostic_rows_generation += 1;
    }

    /// Re-reads the `editor.*` settings and applies what changed:
    /// font, line height, wrap, minimap, whitespace, tab size and the
    /// large-file limits. Font-size zoom and the wrap toggle survive
    /// unless their setting changed, and the top visible row stays. Called on every [`AppState`] change.
    pub fn reload_config(&mut self, cx: &mut Context<Self>) {
        let (config, font_size, word_wrap) = Config::read(self.language_id.as_deref(), cx);
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
            self.hover.set_config(self.config.hover);
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

    /// Shows `diagnostics` (the file's, from every server) and turns on
    /// the gutter's icon column: call it, even with none, once a language
    /// server is set up for the file.
    pub fn set_diagnostics(&mut self, diagnostics: Arc<[Diagnostic]>, cx: &mut Context<Self>) {
        self.diagnostic_rows = DiagnosticRows::build(&diagnostics, &self.snapshot);
        self.diagnostic_rows_generation += 1;
        self.hover.set_diagnostics(diagnostics.clone());
        self.diagnostics = diagnostics;
        self.diagnostics_source = true;
        cx.notify();
    }

    /// Drops the diagnostics and the gutter's icon column: no language
    /// server is set up for the file any more.
    pub(crate) fn clear_diagnostics(&mut self, cx: &mut Context<Self>) {
        if self.diagnostics_source || !self.diagnostics.is_empty() {
            self.diagnostics = Arc::from([]);
            self.hover.set_diagnostics(self.diagnostics.clone());
            self.diagnostics_source = false;
            self.diagnostic_rows = DiagnosticRows::default();
            self.diagnostic_rows_generation += 1;
            cx.notify();
        }
    }

    /// The "No … found" message shown at the cursor.
    pub fn goto_message(&self) -> Option<&str> {
        self.goto_message.as_ref().map(GotoMessage::text)
    }

    /// Shows `message` at its point, replacing any other; `None` hides it.
    pub(crate) fn set_goto_message(
        &mut self,
        message: Option<GotoMessage>,
        cx: &mut Context<Self>,
    ) {
        if self.goto_message.is_some() || message.is_some() {
            self.goto_message = message;
            cx.notify();
        }
    }

    /// Where the "No … found" message is anchored.
    pub(crate) fn goto_message_anchor(&self) -> Option<Point> {
        self.goto_message.as_ref().map(GotoMessage::point)
    }

    /// The LSP hover of this view.
    pub fn hover(&self) -> &HoverController {
        &self.hover
    }

    pub(crate) fn hover_mut(&mut self) -> &mut HoverController {
        &mut self.hover
    }

    /// Where hover and link answers come from and where their links go.
    pub(crate) fn set_hover_sources(
        &mut self,
        workbench: WeakEntity<Workbench>,
        servers: WeakEntity<LanguageServers>,
    ) {
        self.hover.set_sources(workbench.clone(), servers.clone());
        self.set_link_sources(workbench, servers);
    }

    /// The middle of the char at `p` in window coordinates, as last
    /// painted; `None` when it is not on screen.
    pub fn char_position(&self, p: Point) -> Option<gpui::Point<gpui::Pixels>> {
        self.frame.as_ref()?.char_position(p)
    }

    /// The diagnostics last set, sorted by range start.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The diagnostics as painted: marks, gutter icons, inline messages.
    pub fn diagnostic_rows(&self) -> &DiagnosticRows {
        &self.diagnostic_rows
    }

    /// The diagnostics of a view fed by a language server, for another
    /// view of the same file; `None` when no server feeds this one.
    pub(crate) fn diagnostics_source(&self) -> Option<Arc<[Diagnostic]>> {
        self.diagnostics_source.then(|| self.diagnostics.clone())
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

    /// Paints the rows of `rows` (half-open row ranges) across the text
    /// with `editor.highlighted_line.background`, under the selection and
    /// the text. New text keeps them.
    pub fn set_line_highlights(&mut self, rows: Vec<Range<u32>>, cx: &mut Context<Self>) {
        self.line_highlights = rows.into();
        cx.notify();
    }

    pub fn line_highlights(&self) -> &[Range<u32>] {
        &self.line_highlights
    }

    /// Whether `row` is in a range of [`CodeView::set_line_highlights`].
    fn is_line_highlighted(&self, row: u32) -> bool {
        self.line_highlights.iter().any(|r| r.contains(&row))
    }

    /// Whether this is a picker's preview, made by
    /// [`Documents::open_preview`].
    pub fn is_passive(&self) -> bool {
        self.passive
    }

    pub(crate) fn set_passive(&mut self, passive: bool) {
        self.passive = passive;
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

    /// Whether the mouse is over the gutter.
    pub fn gutter_hovered(&self) -> bool {
        self.gutter_hovered
    }

    pub(crate) fn set_gutter_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.gutter_hovered != hovered {
            self.gutter_hovered = hovered;
            cx.notify();
        }
    }

    /// Whether `row` shows a fold chevron: it must start a fold, and the
    /// mouse must be over the gutter unless the fold is closed.
    pub fn fold_chevron_visible(&self, row: u32) -> bool {
        let header = self
            .fold_ranges
            .binary_search_by_key(&row, |f| f.start)
            .is_ok();
        header && (self.gutter_hovered || self.folds.is_folded(row))
    }

    /// Folds or unfolds the fold starting at `header_row`.
    pub fn toggle_fold(&mut self, header_row: u32, cx: &mut Context<Self>) {
        self.folds.toggle(header_row, &self.fold_ranges);
        self.scroll.top_row = self.folds.prev_visible(self.scroll.top_row);
        cx.notify();
    }

    /// Toggles, closes or opens the innermost fold containing `row` (its
    /// header included); returns that fold's header row, or `None` when
    /// no fold contains `row`.
    pub fn fold_at(&mut self, row: u32, op: FoldOp, cx: &mut Context<Self>) -> Option<u32> {
        let header = self
            .fold_ranges
            .iter()
            .filter(|f| f.contains(&row))
            .min_by_key(|f| f.end - f.start)?
            .start;
        let act = match op {
            FoldOp::Toggle => true,
            FoldOp::Close => !self.folds.is_folded(header),
            FoldOp::Open => self.folds.is_folded(header),
        };
        if act {
            self.toggle_fold(header, cx);
        }
        Some(header)
    }

    /// Whether `row` is inside a closed fold (its header is not).
    pub fn is_row_hidden(&self, row: u32) -> bool {
        self.folds.is_hidden(row)
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

    /// The overlay scrollbar thumb as last painted, in window coordinates;
    /// `None` while the rows fit (or before the first paint).
    pub fn scrollbar_thumb(&self) -> Option<gpui::Bounds<gpui::Pixels>> {
        self.frame.as_ref().and_then(|f| f.thumb())
    }

    /// The gutter's width in pixels as last laid out; 0 before the first
    /// paint.
    pub fn gutter_width(&self) -> f32 {
        self.frame.as_ref().map_or(0., |f| f.gutter_width())
    }

    /// The buffer rows that got an inline diagnostic message in the last
    /// frame.
    pub fn painted_inline_rows(&self) -> Vec<u32> {
        self.frame
            .as_ref()
            .map(|f| f.inline_rows())
            .unwrap_or_default()
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

    /// The cursor: the painted cursor overlay (vim's) when there is one,
    /// else the moving end of the selection. The status bar, find and the
    /// bracket highlight follow it.
    pub fn cursor(&self) -> Point {
        self.cursor_overlay
            .map_or_else(|| self.selection_head(), |o| o.at)
    }

    /// The moving end of the selection, whatever the overlay shows.
    pub fn selection_head(&self) -> Point {
        self.selection
            .map_or(Point { row: 0, column: 0 }, |s| s.head)
    }

    /// Whether a rectangular selection is shown.
    pub fn has_block_selection(&self) -> bool {
        self.block_selection.is_some()
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

    /// Sets the selection, dropping any block selection.
    fn set_selection(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.set_goto_message(None, cx);
        self.selection = Some(selection);
        self.block_selection = None;
        self.selection_generation += 1;
        self.update_brackets();
        cx.notify();
    }

    /// Drops the selection and block selection; the cursor stays where it
    /// was.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selection = self.selection.map(|s| Selection::cursor(s.head));
        self.block_selection = None;
        self.selection_generation += 1;
        self.update_brackets();
        cx.notify();
    }

    /// Selects a rectangle given as one range per row, collapsing the
    /// normal selection to its head; `None` or no ranges clears it.
    pub fn set_block_selection(
        &mut self,
        ranges: Option<Vec<Range<Point>>>,
        cx: &mut Context<Self>,
    ) {
        self.selection = self.selection.map(|s| Selection::cursor(s.head));
        self.block_selection = ranges.filter(|r| !r.is_empty());
        self.selection_generation += 1;
        self.update_brackets();
        cx.notify();
    }

    /// Counts selection changes from any source (API, mouse, new text), so
    /// a caller can tell whether someone else changed the selection.
    pub fn selection_generation(&self) -> u64 {
        self.selection_generation
    }

    /// The cursor painted over the text, if any.
    pub fn cursor_overlay(&self) -> Option<CursorOverlay> {
        self.cursor_overlay
    }

    /// Paints `overlay` as the cursor; `None` paints no cursor.
    pub fn set_cursor_overlay(&mut self, overlay: Option<CursorOverlay>, cx: &mut Context<Self>) {
        if self.cursor_overlay != overlay {
            self.set_goto_message(None, cx);
            self.cursor_overlay = overlay;
            self.update_brackets();
            cx.notify();
        }
    }

    /// The selected text, with ends snapped to char boundaries; a block
    /// selection gives its rows joined with `\n`.
    pub fn copy_selection(&self) -> Option<String> {
        if let Some(block) = &self.block_selection {
            let rows: Vec<String> = block
                .iter()
                .map(|r| selection::text_for(&self.snapshot, r.clone()))
                .collect();
            return Some(rows.join("\n"));
        }
        let range = self.selection()?;
        Some(selection::text_for(&self.snapshot, range))
    }

    /// Finds the pair of the bracket next to the cursor; uses the syntax
    /// tree when there is one, so brackets in strings are skipped.
    fn update_brackets(&mut self) {
        let shown = self.selection.is_some() || self.cursor_overlay.is_some();
        self.brackets = shown.then(|| self.cursor()).and_then(|cursor| {
            let at = selection::boundary_offset(&self.snapshot, cursor);
            let partner = self.matching_bracket(at)?;
            Some([self.matching_bracket(partner)?, partner])
        });
    }

    /// The offset of the bracket paired with the one at (or just before)
    /// `offset`; uses the syntax tree when there is one, so brackets in
    /// strings are skipped.
    pub fn matching_bracket(&self, offset: usize) -> Option<usize> {
        // On the UI thread: a panic is logged and gives no pair.
        catch_panic("brackets", || match &self.syntax {
            Some(syntax) if syntax.tree().is_some() => {
                syntax.matching_bracket(&self.snapshot, offset)
            }
            _ => lucerna_syntax::matching_bracket(&self.snapshot, offset),
        })
        .ok()
        .flatten()
        .filter(|&at| at < self.snapshot.len_bytes())
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
        self.reveal_centered(p, cx);
    }

    /// Puts the cursor at `p` and scrolls its row to the middle of the
    /// viewport, opening any fold that hides it.
    pub fn reveal_centered(&mut self, p: Point, cx: &mut Context<Self>) {
        let p = self.place_cursor(p, cx);
        match self.visible_rows() {
            0 => self.center_after_layout(p.row),
            visible => self.center(p.row, visible),
        }
        cx.notify();
    }

    /// Puts the cursor at `p`, opening any fold that hides it, and
    /// scrolls only as far as needed to keep its row
    /// `editor.cursorSurroundingLines` rows inside the viewport. Before
    /// the first paint it centers the row, like [`CodeView::go_to`].
    pub fn set_cursor(&mut self, p: Point, cx: &mut Context<Self>) {
        let p = self.place_cursor(p, cx);
        match self.visible_rows() {
            0 => self.center_after_layout(p.row),
            visible => self.scroll_into_view(p.row, visible),
        }
        cx.notify();
    }

    /// Clamps `p` to the last row, unfolds it and collapses the selection
    /// there.
    fn place_cursor(&mut self, p: Point, cx: &mut Context<Self>) -> Point {
        let last = self.snapshot.line_count().saturating_sub(1);
        let p = Point {
            row: p.row.min(last),
            column: p.column,
        };
        while let Some(header) = self.folds.hiding_header(p.row) {
            self.folds.toggle(header, &self.fold_ranges);
        }
        self.set_selection(Selection::cursor(p), cx);
        p
    }

    fn center_after_layout(&mut self, row: u32) {
        self.scroll = Scroll {
            top_row: row,
            offset: 0.,
        };
        self.pending_center = Some(row);
    }

    /// Scrolls minimally so `row` sits between the top and bottom
    /// margins (`editor.cursorSurroundingLines`, at most half the view),
    /// never past the top that shows the last row at the bottom. Counts
    /// display lines, so wrapped rows and folds are taken into account.
    fn scroll_into_view(&mut self, row: u32, visible_rows: u32) {
        let margin = self
            .config
            .cursor_surrounding_lines
            .min(visible_rows.saturating_sub(1) / 2);
        let top = self.scroll.top_row;
        let highest = self.top_with_lines_above(row, margin);
        let new_top = if row < top || highest < top || (highest == top && self.scroll.offset > 0.) {
            highest
        } else {
            let lowest = self.top_fitting(row, visible_rows.saturating_sub(margin));
            let last = self
                .folds
                .prev_visible(self.snapshot.line_count().saturating_sub(1));
            let lowest = lowest.min(self.top_fitting(last, visible_rows));
            if lowest > top {
                lowest
            } else {
                return;
            }
        };
        self.scroll = Scroll {
            top_row: new_top,
            offset: 0.,
        };
        self.pending_center = None;
    }

    /// The highest visible row with at least `lines` display lines between
    /// it and `row` (stopping at row 0).
    fn top_with_lines_above(&mut self, row: u32, lines: u32) -> u32 {
        let mut top = self.folds.prev_visible(row);
        let mut used = 0;
        while used < lines && top > 0 {
            top = self.folds.prev_visible(top - 1);
            used += self.wrap_count(top);
        }
        top
    }

    /// The highest top row from which all of `row`'s display lines fit in
    /// the first `lines` lines of the view (`row` itself if it alone does
    /// not fit).
    fn top_fitting(&mut self, row: u32, lines: u32) -> u32 {
        let mut top = self.folds.prev_visible(row);
        let mut used = self.wrap_count(top);
        while top > 0 {
            let prev = self.folds.prev_visible(top - 1);
            let count = self.wrap_count(prev);
            if used + count > lines {
                break;
            }
            used += count;
            top = prev;
        }
        top
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
            if crate::lsp_log::copy_from_log(wb, cx) {
                return;
            }
            with_active_view(wb, cx, |v, cx| {
                if let Some(text) = v.copy_selection() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            })
        },
    );
}

impl EventEmitter<CodeViewEvent> for CodeView {}

impl Focusable for CodeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CodeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self._focus_out.is_none() {
            let focus = self.focus.clone();
            self._focus_out = Some(cx.on_focus_out(&focus, window, |v, _, _, cx| {
                v.hover.hide(cx);
                v.set_goto_message(None, cx);
                // Its release (⌘-Tab away) may never reach the view.
                v.link_modifiers_changed(false, cx);
            }));
        }
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
        // The find bar is a full-width row above the code and the minimap,
        // like Zed's buffer search bar: it pushes the text down.
        div()
            .id("code-view")
            .debug_selector(|| "code-view".into())
            .track_focus(&self.focus)
            // Any key that reaches the view hides the hover and the
            // "No … found" message.
            .on_key_down(cx.listener(|v, _, _, cx| {
                v.hover.hide(cx);
                v.set_goto_message(None, cx);
            }))
            .on_modifiers_changed(cx.listener(|v, ev: &gpui::ModifiersChangedEvent, _, cx| {
                let held = ev.modifiers.secondary(cx);
                v.link_modifiers_changed(held, cx);
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(color(ui.editor_background))
            .children(self.find_widget().cloned())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .debug_selector(|| "code-text-area".into())
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(CodeElement::new(view.clone())),
                    )
                    .when(show_minimap, |d| d.child(minimap::render(view))),
            )
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
