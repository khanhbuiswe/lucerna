//! The read-only code view: a virtualized, syntax-highlighted view of a
//! [`BufferSnapshot`] with selection, word wrap, folding and a minimap.
//!
//! Only rows on screen (plus a small overscan) are read from the buffer
//! and shaped; shaped rows, highlights and wrap counts are cached per
//! snapshot revision. Large files (`lucerna.largeFile.*`) get no
//! highlighting, folding or minimap.

mod cache;
mod caret;
mod diagnostics;
mod element;
mod frame_layout;
mod input;
mod layout;
mod link;
mod minimap;
mod selection;
mod transact;

use std::cell::Cell;
use std::collections::VecDeque;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyView, App, Context, Entity, EntityInputHandler as _, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Task, WeakEntity,
    Window, div, prelude::*, px,
};
use lucerna_editor::indentation::Indentation;
use lucerna_editor::{EditKind, Goal, Motion, Selection, SelectionSet, autoclose, move_selections};
use lucerna_git::Hunk;
use lucerna_lsp::diagnostics::Diagnostic;
use lucerna_settings::SettingsView;
use lucerna_syntax::{SyntaxState, indent_folds};
use lucerna_text::{Bias, Buffer, BufferSnapshot, LoadError, Patch, Point, Reload};

pub use diagnostics::{DiagnosticRows, Mark, MarkStyle};
pub(crate) use element::hunk_bar_width;
pub use layout::{
    DisplayLine, FoldState, MAX_RENDERED_LINE_CHARS, display_lines, display_lines_with_tab_size,
};
pub(crate) use selection::word_at;
pub(crate) use transact::{ClipboardMetadata, clipboard_item};

use self::cache::Caches;
use self::caret::{Caret, CaretStyle};
use self::element::{CodeElement, Frame};
use self::frame_layout::FrameLayout;
use crate::documents::{DocState, Documents};
use crate::editor_groups::{EditorGroupsView, Tab};
use crate::find_widget::FindWidget;
use crate::goto::GotoMessage;
use crate::hover::{HoverConfig, HoverController};
use crate::language_servers::LanguageServers;
use crate::layout::color;
use crate::tasks::{catch_panic, guarded};
use crate::ui::{GRID, ScrollMetrics, ScrollbarSource, ScrollbarState, scrollbar};
use crate::{AppState, CommandRegistry, WindowSettings, Workbench};

/// The status-bar text shown while the active view is a large file.
pub const LARGE_FILE_STATUS: &str = "Large file: highlighting disabled";

/// `editor.wordSeparators` when the setting is missing.
const DEFAULT_WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

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
    /// `editor.tabSize`, or the detected indentation's.
    tab_size: u32,
    /// `editor.insertSpaces`, or the detected indentation's.
    insert_spaces: bool,
    max_bytes: u64,
    max_lines: u32,
    /// `editor.cursorSurroundingLines`: rows kept between the cursor and
    /// the viewport edges by [`CodeView::set_cursor`].
    cursor_surrounding_lines: u32,
    /// `lucerna.diagnostics.gutterIcons`.
    gutter_icons: bool,
    /// `lucerna.diagnostics.inline.enabled`.
    inline_diagnostics: bool,
    /// `scm.diffDecorations` is `all` or `gutter`: git change bars show.
    diff_gutter: bool,
    /// `scm.diffDecorationsGutterAction` is not `none`: a click on a bar
    /// emits [`CodeViewEvent::DiffHunkClicked`].
    diff_gutter_click: bool,
    hover: HoverConfig,
    /// `editor.cursorBlinking` is not `solid`.
    cursor_blinking: bool,
    /// `editor.cursorStyle`.
    cursor_style: CaretStyle,
    /// `editor.wordSeparators`.
    word_separators: String,
    /// `editor.scrollbar.scrollByPage`: a track click pages instead of
    /// jumping there.
    scroll_by_page: bool,
}

impl Config {
    /// The settings for a view of `language` (a language id such as
    /// `"rust"`), so `"[rust]": {...}` overrides apply.
    fn read(
        settings: SettingsView<'_>,
        language: Option<&str>,
        detected: Option<Indentation>,
    ) -> (Config, f32, bool) {
        let config = Config {
            font_family: crate::fonts::editor_font_family(settings, language),
            line_height: settings.get("editor.lineHeight", language).unwrap_or(1.6),
            minimap: settings
                .get("editor.minimap.enabled", language)
                .unwrap_or(false),
            render_whitespace: settings
                .get::<String>("editor.renderWhitespace", language)
                .is_some_and(|w| w == "all"),
            tab_size: detected.map_or_else(
                || {
                    settings
                        .get::<u32>("editor.tabSize", language)
                        .unwrap_or(4)
                        .max(1)
                },
                |i| i.tab_size.max(1),
            ),
            insert_spaces: detected.map_or_else(
                || {
                    settings
                        .get("editor.insertSpaces", language)
                        .unwrap_or(true)
                },
                |i| i.insert_spaces,
            ),
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
            diff_gutter: !matches!(
                settings
                    .get::<String>("scm.diffDecorations", language)
                    .as_deref(),
                Some("none" | "overview" | "minimap")
            ),
            diff_gutter_click: settings
                .get::<String>("scm.diffDecorationsGutterAction", language)
                .is_none_or(|a| a != "none"),
            hover: HoverConfig::read(settings, language),
            cursor_blinking: settings
                .get::<String>("editor.cursorBlinking", language)
                .is_none_or(|b| b != "solid"),
            cursor_style: CaretStyle::from_setting(
                &settings
                    .get::<String>("editor.cursorStyle", language)
                    .unwrap_or_default(),
            ),
            word_separators: settings
                .get("editor.wordSeparators", language)
                .unwrap_or_else(|| DEFAULT_WORD_SEPARATORS.to_string()),
            scroll_by_page: settings
                .get("editor.scrollbar.scrollByPage", language)
                .unwrap_or(false),
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
    /// Fills the cell of the char under the cursor at `at` (vim's Normal
    /// mode). The view paints no caret and takes no typing.
    Block,
    /// vim's Insert mode: the view's own carets, each painted as a bar
    /// whatever `editor.cursorStyle` says; the view types as usual and
    /// `at` is not used.
    Bar,
}

/// A cursor shape set from outside the view (vim's); the view paints its
/// own carets by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorOverlay {
    pub at: Point,
    pub shape: CursorShape,
}

#[derive(Debug, Clone)]
pub enum CodeViewEvent {
    /// Emitted by [`CodeView::apply_edit`] once the new text is shown,
    /// with the rows the edit changed, the text before and after it, the
    /// edit itself and the view's [`CodeView::revision`] after it.
    Reloaded {
        reload: Reload,
        old: BufferSnapshot,
        snapshot: BufferSnapshot,
        patch: Arc<Patch>,
        revision: u64,
    },
    /// One typing operation (not one buffer change) during vim's Insert
    /// session, for `.`: typed chars or an IME commit's final text, Enter,
    /// Backspace or Tab. Only while [`CodeView::set_typed_events`] is on.
    Typed(lucerna_vim::Typed),
    /// The topmost visible buffer row (folds skipped) changed: the user
    /// scrolled, the view followed the cursor or an edit, or
    /// [`CodeView::set_scroll_top_row`] was called.
    ScrolledTo(u32),
    /// A click on a git change bar or deletion marker in the gutter, with
    /// the first row of its hunk (`scm.diffDecorationsGutterAction`).
    DiffHunkClicked { row: u32 },
}

/// What changed the selections, which decides the undo stop and the
/// `cursorUndo` history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionChange {
    /// A cursor move (keys, mouse, API): an undo stop before the next
    /// edit, and the old selections go on the `cursorUndo` stack.
    Move,
    /// An edit, an undo or text following an edit: neither.
    Edit,
    /// `cursorUndo` / `cursorRedo` themselves: an undo stop only.
    History,
}

/// How many selection states `cursorUndo` keeps, as VS Code.
const CURSOR_HISTORY: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    Text,
    Minimap,
}

pub struct CodeView {
    path: Arc<Path>,
    /// The id of the file's language, for `"[lang]"` settings overrides.
    language_id: Option<Arc<str>>,
    /// The document's detected indentation, over the settings.
    indentation: Option<Indentation>,
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
    /// The window's settings, which the config is read from.
    settings: Entity<WindowSettings>,
    /// Re-reads the config whenever the window's settings change.
    _settings: Subscription,
    scroll: Scroll,
    scroll_x: f32,
    /// Widest line laid out since the text, wrap or font last changed.
    max_line_width: f32,
    /// The cursors and selections; never empty.
    selections: SelectionSet,
    /// A rectangular selection, one range per row (vim's visual block);
    /// shown instead of `selections` while set.
    block_selection: Option<Vec<Range<Point>>>,
    /// Bumped by every selection change, whatever made it.
    selection_generation: u64,
    cursor_overlay: Option<CursorOverlay>,
    /// The caret's blink.
    caret: Caret,
    /// Shared with the document's other views: set by a selection change
    /// so the next edit starts a new undo step (see
    /// [`Documents::apply_edits`]).
    stop_requested: Option<Rc<Cell<bool>>>,
    /// The kind of this view's last edit, for the undo stops of the next.
    last_kind: EditKind,
    /// vim's Insert session: no automatic undo stops (edit kinds, cursor
    /// moves), so the whole session is one step.
    suppress_stops: bool,
    /// vim's Insert session wants [`CodeViewEvent::Typed`] for each typing
    /// operation.
    typed_events: bool,
    /// Bumped by every cursor move (keys, mouse, API) and, while
    /// `typed_events` is on, by every edit that is not typing: what the
    /// Insert session did can no longer be replayed as typing.
    record_breaks: u64,
    /// Set while a typing path (`type_text`, `delete_left`, `tab`, the
    /// input handler) applies its edit.
    typing: bool,
    /// The composition in progress replaced text away from the caret: its
    /// commit is not recorded as typing (the record broke).
    composition_unrecorded: bool,
    /// The last edit shown: the text before it and the patch.
    last_edit: Option<(BufferSnapshot, Arc<Patch>)>,
    /// The IME composition in progress: one byte range per selection, in
    /// selection order. The composed text is in the buffer, underlined.
    marked: Option<Vec<Range<usize>>>,
    /// `cursorUndo`: the selections before each cursor move, newest last;
    /// at most [`CURSOR_HISTORY`]. Any edit clears it and `cursor_redo`.
    cursor_undo: VecDeque<SelectionSet>,
    /// The selections before each smart select expansion, to shrink back
    /// to; any other selection change or edit clears it.
    smart_select_stack: Vec<SelectionSet>,
    /// Set while expand / shrink apply their selections.
    smart_selecting: bool,
    cursor_redo: Vec<SelectionSet>,
    /// The pairs typing auto-closed (open start .. close end): typing a
    /// closer moves over theirs and Backspace deletes an empty one. Each
    /// follows the edits and goes when no cursor is inside it.
    autoclose_regions: Vec<Range<usize>>,
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
    /// The top row last sent as [`CodeViewEvent::ScrolledTo`].
    reported_top_row: Option<u32>,
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
    /// The file's changes against the git base, as gutter bars. Kept
    /// through edits until the next result, so the bars do not flicker.
    diff_hunks: Arc<[Hunk]>,
    /// The scrollbar marks last laid out, reused while nothing they
    /// depend on changed.
    scrollbar_marks: Option<element::ScrollbarMarks>,
    /// The overlay scrollbar's hover and drag.
    scrollbar: ScrollbarState,
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
    pub fn new(path: Arc<Path>, settings: Entity<WindowSettings>, cx: &mut Context<Self>) -> Self {
        // A first guess; `Documents` sets the language it resolves.
        let language_id = settings
            .read(cx)
            .languages(cx)
            .language_for_file(&path, "")
            .map(|lang| lang.id.clone());
        let (config, font_size, word_wrap) =
            Config::read(settings.read(cx).view(cx), language_id.as_deref(), None);
        let snapshot = Buffer::from_bytes(b"", false)
            .expect("empty text decodes")
            .snapshot();
        let hover = HoverController::new(path.clone(), config.hover, snapshot.clone());
        CodeView {
            path,
            language_id,
            indentation: None,
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
            _settings: cx.observe(&settings, |view, _, cx| view.reload_config(cx)),
            settings,
            scroll: Scroll::default(),
            scroll_x: 0.,
            max_line_width: 0.,
            selections: SelectionSet::cursor(0),
            block_selection: None,
            selection_generation: 0,
            cursor_overlay: None,
            caret: Caret::default(),
            stop_requested: None,
            last_kind: EditKind::Other,
            suppress_stops: false,
            typed_events: false,
            record_breaks: 0,
            typing: false,
            composition_unrecorded: false,
            last_edit: None,
            marked: None,
            cursor_undo: VecDeque::new(),
            smart_select_stack: Vec::new(),
            smart_selecting: false,
            cursor_redo: Vec::new(),
            autoclose_regions: Vec::new(),
            brackets: None,
            drag: None,
            folds: FoldState::default(),
            fold_ranges: Vec::new(),
            _fold_task: None,
            caches: Caches::default(),
            frame: None,
            pending_center: None,
            reported_top_row: None,
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
            diff_hunks: Arc::from([]),
            scrollbar_marks: None,
            scrollbar: ScrollbarState::default(),
            hover,
            goto_message: None,
            link: link::Link::default(),
            _focus_out: None,
        }
    }

    /// A [`crate::TabBodyFactory`] that gives each file tab an empty code
    /// view reading `settings` (other tabs get [`crate::placeholder_body`]).
    pub fn tab_body(settings: Entity<WindowSettings>) -> crate::TabBodyFactory {
        Rc::new(
            move |tab: &Tab, window: &mut Window, cx: &mut App| -> AnyView {
                match tab.path() {
                    Some(path) => {
                        let (path, settings) = (path.clone(), settings.clone());
                        cx.new(|cx| CodeView::new(path, settings, cx)).into()
                    }
                    None => crate::placeholder_body(tab, window, cx),
                }
            },
        )
    }

    /// The window's settings this view reads.
    pub fn settings(&self) -> &Entity<WindowSettings> {
        &self.settings
    }

    pub fn path(&self) -> &Arc<Path> {
        &self.path
    }

    /// The file was renamed to `path` (see `Documents::rename`): the view
    /// keeps its text, cursor and scroll. The old file's diagnostics no
    /// longer apply; the language servers send the new path's.
    pub(crate) fn set_path(&mut self, path: Arc<Path>, cx: &mut Context<Self>) {
        if self.path == path {
            return;
        }
        self.hover.set_path(path.clone(), cx);
        self.path = path;
        self.clear_diagnostics(cx);
        cx.notify();
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

    /// Ties the view to its document: `stop_requested` is the document's
    /// flag for "a selection changed since the last edit".
    pub(crate) fn set_documents(
        &mut self,
        documents: WeakEntity<Documents>,
        stop_requested: Rc<Cell<bool>>,
    ) {
        self.documents = Some(documents);
        self.stop_requested = Some(stop_requested);
    }

    /// Shows `snapshot` from the top, dropping the selection, folds and
    /// syntax. Use [`CodeView::apply_edit`] for a new version of the
    /// same file.
    pub fn set_buffer(&mut self, snapshot: BufferSnapshot, cx: &mut Context<Self>) {
        self.replace_snapshot(snapshot, cx);
        self.syntax = None;
        self.scroll = Scroll::default();
        self.scroll_x = 0.;
        self.selections = SelectionSet::cursor(0);
        self.block_selection = None;
        self.selection_generation += 1;
        self.brackets = None;
        self.marked = None;
        self.last_kind = EditKind::Other;
        self.cursor_undo.clear();
        self.cursor_redo.clear();
        self.smart_select_stack.clear();
        self.autoclose_regions.clear();
        self.folds.clear();
        self.pending_center = None;
        cx.notify();
    }

    /// Shows the text after an edit, an undo or a reload: `patch` turned
    /// `old` (the text shown) into `snapshot`. The selection, search
    /// matches and diagnostics follow the text around them (diagnostics
    /// until the servers publish again); the scroll position and closed
    /// folds stay on the same lines, except a fold whose header changed.
    /// The syntax tree is edited to the new text and stays until the next
    /// [`CodeView::set_syntax`], so colours do not flicker off.
    pub fn apply_edit(
        &mut self,
        old: &BufferSnapshot,
        snapshot: BufferSnapshot,
        patch: &Patch,
        cx: &mut Context<Self>,
    ) {
        let rows = snapshot.line_count();
        let reload = patch.reload(old, &snapshot);
        let map = |p: Point| map_point(old, &snapshot, patch, p);
        self.scroll = Scroll {
            top_row: reload.adjust_row(self.scroll.top_row, rows),
            offset: 0.,
        };
        self.hover.hide(cx);
        let mut selections = self.selections.clone();
        selections.remap(patch);
        self.autoclose_regions =
            autoclose::remap_regions(&self.autoclose_regions, old, &snapshot, patch);
        // A rectangle stays one only while no edit reaches into it.
        self.block_selection = self
            .block_selection
            .take()
            .filter(|block| {
                !block.iter().any(|r| {
                    touches(
                        patch,
                        old.point_to_offset(r.start)..old.point_to_offset(r.end),
                    )
                })
            })
            .map(|block| {
                block
                    .into_iter()
                    .map(|r| map(r.start)..map(r.end))
                    .collect()
            });
        let (matches, active) = remap_ranges(&self.match_highlights, self.active_match, patch);
        let diagnostics: Arc<[Diagnostic]> = self
            .diagnostics
            .iter()
            .map(|d| Diagnostic {
                range: map(d.range.start)..map(d.range.end),
                ..d.clone()
            })
            .collect();
        self.hover.set_diagnostics(diagnostics.clone());
        self.diagnostics = diagnostics;
        self.syntax = self.syntax.as_ref().and_then(|syntax| {
            catch_panic("edit syntax", || syntax.edited(old, &snapshot, patch))
                .ok()
                .map(Arc::new)
        });
        self.replace_snapshot(snapshot, cx);
        self.match_highlights = matches;
        self.active_match = active;
        self.folds.remap(&reload, rows);
        self.scroll.top_row = self.folds.prev_visible(self.scroll.top_row);
        // Any change to the text ends a composition (the edit that makes
        // one marks it again) and the typing run, and clears the cursor
        // history, as in VS Code.
        self.marked = None;
        self.last_kind = EditKind::Other;
        self.cursor_undo.clear();
        self.cursor_redo.clear();
        // Emitted before the selection change notifies, so listeners (vim)
        // see the event before they see the bumped selection generation.
        let patch = Arc::new(patch.clone());
        self.last_edit = Some((old.clone(), patch.clone()));
        cx.emit(CodeViewEvent::Reloaded {
            reload,
            old: old.clone(),
            snapshot: self.snapshot.clone(),
            patch,
            revision: self.revision,
        });
        // Following the text is no cursor move: no undo stop.
        self.set_selections_inner(selections, false, SelectionChange::Edit, cx);
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
    /// unless their setting changed, and the top visible row stays. Called
    /// on every change to the window's settings.
    pub fn reload_config(&mut self, cx: &mut Context<Self>) {
        let (config, font_size, word_wrap) = Config::read(
            self.settings.read(cx).view(cx),
            self.language_id.as_deref(),
            self.indentation,
        );
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

    /// Shows `hunks`, the file's changes against the git base, as gutter
    /// bars.
    pub fn set_diff_hunks(&mut self, hunks: Arc<[Hunk]>, cx: &mut Context<Self>) {
        if self.diff_hunks != hunks {
            self.diff_hunks = hunks;
            cx.notify();
        }
    }

    /// Drops the gutter bars.
    pub fn clear_diff_hunks(&mut self, cx: &mut Context<Self>) {
        self.set_diff_hunks(Arc::from([]), cx);
    }

    /// The file's changes against the git base, as last set: sorted by
    /// row.
    pub fn diff_hunks(&self) -> &[Hunk] {
        &self.diff_hunks
    }

    /// The hunks, for another view of the same file.
    pub(crate) fn shared_diff_hunks(&self) -> Arc<[Hunk]> {
        self.diff_hunks.clone()
    }

    /// Moves the cursor to column 0 of the next change after the cursor
    /// row (`forward`) or the previous one before it, wrapping around the
    /// file, and scrolls it into view. `false` when there are no changes.
    pub fn next_change(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let last = self.snapshot.line_count().saturating_sub(1);
        let Some(row) = next_change_row(&self.diff_hunks, self.cursor().row, last, forward) else {
            return false;
        };
        self.set_cursor(Point { row, column: 0 }, cx);
        true
    }

    /// The gutter bars, deletion markers and fold markers laid out in the
    /// last frame.
    #[doc(hidden)]
    pub fn painted_hunk_bars(&self) -> usize {
        self.frame.as_ref().map_or(0, |f| f.hunk_bar_count())
    }

    /// The visible bounds of gutter bar `i` in the last frame (change bars
    /// first, then deletion markers), in window coordinates.
    #[doc(hidden)]
    pub fn hunk_bar_bounds(&self, i: usize) -> Option<gpui::Bounds<gpui::Pixels>> {
        self.frame.as_ref()?.hunk_bar_bounds(i)
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

    #[doc(hidden)]
    pub fn set_passive(&mut self, passive: bool) {
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
        self.report_scroll(cx);
        cx.notify();
    }

    /// Emits [`CodeViewEvent::ScrolledTo`] when the top row is not the one
    /// last reported. Called where the top row settles: an explicit
    /// scroll, the wheel, and each layout (which applies the scrolls that
    /// follow the cursor or an edit).
    pub(crate) fn report_scroll(&mut self, cx: &mut Context<Self>) {
        let top = self.scroll.top_row;
        if self.reported_top_row != Some(top) {
            self.reported_top_row = Some(top);
            cx.emit(CodeViewEvent::ScrolledTo(top));
        }
    }

    /// The overlay scrollbar thumb as last painted, in window coordinates;
    /// `None` while the rows fit (or before the first paint).
    pub fn scrollbar_thumb(&self) -> Option<gpui::Bounds<gpui::Pixels>> {
        self.scrollbar.thumb()
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

    /// The primary selection's range, start before end; `None` when it
    /// is empty.
    pub fn selection(&self) -> Option<Range<Point>> {
        let primary = self.selections.primary();
        (!primary.is_empty()).then(|| {
            let range = primary.range();
            self.snapshot.offset_to_point(range.start)..self.snapshot.offset_to_point(range.end)
        })
    }

    /// The cursor: vim's block cursor when one is painted,
    /// else the moving end of the primary selection. The status bar, find
    /// and the bracket highlight follow it.
    pub fn cursor(&self) -> Point {
        self.block_cursor().unwrap_or_else(|| self.selection_head())
    }

    /// Where the block cursor (vim's Normal mode) is painted, if one is.
    pub(crate) fn block_cursor(&self) -> Option<Point> {
        self.cursor_overlay
            .filter(|o| o.shape == CursorShape::Block)
            .map(|o| o.at)
    }

    /// The moving end of the primary selection, whatever the overlay
    /// shows.
    pub fn selection_head(&self) -> Point {
        self.snapshot
            .offset_to_point(self.selections.primary().head)
    }

    /// Every cursor and selection.
    pub fn selections(&self) -> &SelectionSet {
        &self.selections
    }

    /// Replaces the selections, dropping any block selection; with
    /// `autoscroll`, scrolls the primary cursor into view. Every call is
    /// a cursor move, so the next edit starts a new undo step.
    pub fn set_selections(&mut self, set: SelectionSet, autoscroll: bool, cx: &mut Context<Self>) {
        self.set_goto_message(None, cx);
        self.block_selection = None;
        self.set_selections_inner(set, autoscroll, SelectionChange::Move, cx);
    }

    /// Replaces the selections like [`CodeView::set_selections`], but is
    /// no cursor move: no undo stop. For edits and undo / redo restoring
    /// their selections.
    pub(crate) fn set_selections_without_stop(
        &mut self,
        set: SelectionSet,
        autoscroll: bool,
        cx: &mut Context<Self>,
    ) {
        self.set_goto_message(None, cx);
        self.block_selection = None;
        self.set_selections_inner(set, autoscroll, SelectionChange::Edit, cx);
    }

    /// Replaces the selections (clamped to the text) and restarts the
    /// caret blink. A cursor move asks the document for an undo stop
    /// before the next edit and is remembered for `cursorUndo`; following
    /// an edit or restoring an undo step is neither.
    fn set_selections_inner(
        &mut self,
        mut set: SelectionSet,
        autoscroll: bool,
        change: SelectionChange,
        cx: &mut Context<Self>,
    ) {
        set.clamp(&self.snapshot);
        let old = std::mem::replace(&mut self.selections, set);
        if !self.smart_selecting {
            self.smart_select_stack.clear();
        }
        if change == SelectionChange::Move && old != self.selections {
            if self.cursor_undo.len() == CURSOR_HISTORY {
                self.cursor_undo.pop_front();
            }
            self.cursor_undo.push_back(old);
            self.cursor_redo.clear();
        }
        self.selection_generation += 1;
        self.caret.reset(cx);
        if change != SelectionChange::Edit {
            self.record_breaks += 1;
            // A cursor move ends the typing run and any composition.
            self.last_kind = EditKind::Other;
            self.marked = None;
            if let Some(flag) = &self.stop_requested
                && !self.suppress_stops
            {
                flag.set(true);
            }
        }
        autoclose::retain_regions(&mut self.autoclose_regions, &self.selections);
        self.update_brackets();
        if autoscroll {
            self.autoscroll();
        }
        cx.notify();
    }

    /// Opens any fold hiding the primary cursor and scrolls it into view.
    fn autoscroll(&mut self) {
        let row = self.selection_head().row;
        while let Some(header) = self.folds.hiding_header(row) {
            self.folds.toggle(header, &self.fold_ranges);
        }
        match self.visible_rows() {
            0 => self.center_after_layout(row),
            visible => self.scroll_into_view(row, visible),
        }
    }

    /// Moves every cursor by `motion`; with `extend`, their anchors stay
    /// (shift).
    pub fn move_cursor(&mut self, motion: Motion, extend: bool, cx: &mut Context<Self>) {
        self.move_with(
            |v| {
                let layout = FrameLayout {
                    folds: &v.folds,
                    wrap_chars: v.wrap_chars(),
                    tab_size: v.config.tab_size,
                    page_rows: v.visible_rows().saturating_sub(1).max(1),
                };
                move_selections(
                    &v.snapshot,
                    &v.selections,
                    motion,
                    extend,
                    &layout,
                    &v.config.word_separators,
                )
            },
            cx,
        );
    }

    /// Moves the cursors to the selections `f` computes. A panic in `f`
    /// leaves them where they are (spec §16).
    #[doc(hidden)]
    pub fn move_with(&mut self, f: impl FnOnce(&Self) -> SelectionSet, cx: &mut Context<Self>) {
        if let Some(set) = self.contained("cursor motion", f, cx) {
            self.set_selections(set, true, cx);
        }
    }

    /// `editor.action.smartSelect.expand`: grows every selection to the
    /// next word, syntax node, line or the whole text.
    pub fn smart_select_expand(&mut self, cx: &mut Context<Self>) {
        let syntax = self.syntax.clone();
        let next = lucerna_editor::smart_select::expand(
            &self.snapshot,
            syntax.as_deref(),
            &self.selections,
            &self.config.word_separators,
        );
        if next == self.selections {
            return;
        }
        let mut stack = std::mem::take(&mut self.smart_select_stack);
        stack.push(self.selections.clone());
        self.smart_selecting = true;
        self.set_selections(next, true, cx);
        self.smart_selecting = false;
        self.smart_select_stack = stack;
    }

    /// `editor.action.smartSelect.shrink`: back to the selections before
    /// the last expansion; nothing when there is none.
    pub fn smart_select_shrink(&mut self, cx: &mut Context<Self>) {
        let Some(previous) = self.smart_select_stack.pop() else {
            return;
        };
        let stack = std::mem::take(&mut self.smart_select_stack);
        self.smart_selecting = true;
        self.set_selections(previous, true, cx);
        self.smart_selecting = false;
        self.smart_select_stack = stack;
    }

    /// How many expansions `shrink` can undo, for tests.
    #[doc(hidden)]
    pub fn smart_select_depth(&self) -> usize {
        self.smart_select_stack.len()
    }

    /// Selects the whole text.
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let set = self.single(0, self.snapshot.len_bytes());
        self.set_selections(set, false, cx);
    }

    /// One selection from `anchor` to `head` (byte offsets), keeping the
    /// primary's id.
    fn single(&self, anchor: usize, head: usize) -> SelectionSet {
        let id = self.selections.primary().id;
        SelectionSet::from_selections(
            vec![Selection {
                id,
                anchor,
                head,
                goal: Goal::None,
            }],
            id,
            false,
        )
    }

    /// The byte offset of `p`, moved back to a char boundary within its
    /// line.
    fn offset_of(&self, p: Point) -> usize {
        selection::boundary_offset(&self.snapshot, p)
    }

    /// Selects from `anchor` to `head` as the only selection.
    fn set_selection_points(&mut self, anchor: Point, head: Point, cx: &mut Context<Self>) {
        let set = self.single(self.offset_of(anchor), self.offset_of(head));
        self.set_selections(set, false, cx);
    }

    /// The caret is in the on half of its blink (always, with
    /// `editor.cursorBlinking: "solid"`). Painted only while the view has
    /// focus and no cursor overlay is set.
    pub fn caret_visible(&self) -> bool {
        self.caret.visible()
    }

    /// Whether a rectangular selection is shown.
    pub fn has_block_selection(&self) -> bool {
        self.block_selection.is_some()
    }

    /// Selects `range` as the only selection; its end becomes the cursor.
    pub fn select(&mut self, range: Range<Point>, cx: &mut Context<Self>) {
        self.set_selection_points(range.start, range.end, cx);
    }

    /// Collapses every selection to its cursor and drops the block
    /// selection.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        let collapsed = self
            .selections
            .all()
            .iter()
            .map(|s| Selection {
                anchor: s.head,
                ..*s
            })
            .collect();
        let set = SelectionSet::from_selections(collapsed, self.selections.primary().id, true);
        self.set_selections(set, false, cx);
    }

    /// Selects a rectangle given as one range per row, collapsing the
    /// selections to the primary cursor; `None` or no ranges clears it.
    pub fn set_block_selection(
        &mut self,
        ranges: Option<Vec<Range<Point>>>,
        cx: &mut Context<Self>,
    ) {
        let head = self.selections.primary().head;
        let set = self.single(head, head);
        self.set_selections_inner(set, false, SelectionChange::Move, cx);
        self.block_selection = ranges.filter(|r| !r.is_empty());
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

    /// Starts (`true`) or ends vim's Insert session: while it lasts, edits
    /// and cursor moves push no undo stops, so the session is one step.
    /// Either way the current step ends here.
    pub fn set_suppress_stops(&mut self, suppress: bool, cx: &mut Context<Self>) {
        if self.suppress_stops == suppress {
            return;
        }
        self.suppress_stops = suppress;
        if let Some(flag) = &self.stop_requested {
            flag.set(false);
        }
        if let Some(documents) = self.documents.as_ref().and_then(WeakEntity::upgrade) {
            let path = self.path.clone();
            documents.update(cx, |docs, _| docs.push_stop(&path));
        }
    }

    /// Whether vim's Insert session (or a vim command) holds the undo
    /// stops: see [`CodeView::set_suppress_stops`].
    pub fn suppresses_stops(&self) -> bool {
        self.suppress_stops
    }

    /// Runs `f` as one undo step: a stop before, no stops while it runs, a
    /// stop after unless `keep_open` (a command that ends in Insert mode:
    /// the Insert session's Escape closes the step). Inside a step that is
    /// already open (an Insert session, a `.` replay) `f` just joins it.
    pub fn transact_group<R>(
        &mut self,
        keep_open: bool,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Self, &mut Context<Self>) -> R,
    ) -> R {
        let opened = self.begin_group(cx);
        let result = f(self, cx);
        self.end_group(opened, keep_open, cx);
        result
    }

    /// [`CodeView::transact_group`] in two halves, for a caller that cannot
    /// run its work inside a closure: returns whether this call opened the
    /// step (false inside one already open).
    pub(crate) fn begin_group(&mut self, cx: &mut Context<Self>) -> bool {
        if self.suppress_stops {
            return false;
        }
        self.set_suppress_stops(true, cx);
        true
    }

    /// Closes the step [`CodeView::begin_group`] opened, unless
    /// `keep_open`.
    pub(crate) fn end_group(&mut self, opened: bool, keep_open: bool, cx: &mut Context<Self>) {
        if opened && !keep_open {
            self.set_suppress_stops(false, cx);
        }
    }

    /// Runs a typing path's edit: no record break for it.
    pub(crate) fn typing<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let outer = std::mem::replace(&mut self.typing, true);
        let result = f(self);
        self.typing = outer;
        result
    }

    /// An edit other than typing during vim's Insert session.
    pub(crate) fn break_record(&mut self) {
        if self.typed_events && !self.typing {
            self.record_breaks += 1;
        }
    }

    /// The text before the last edit shown and its patch.
    pub(crate) fn last_edit(&self) -> Option<&(BufferSnapshot, Arc<Patch>)> {
        self.last_edit.as_ref()
    }

    /// Turns [`CodeViewEvent::Typed`] on (vim's Insert session) or off.
    pub(crate) fn set_typed_events(&mut self, on: bool) {
        self.typed_events = on;
    }

    /// Emits [`CodeViewEvent::Typed`] when vim's Insert session wants it.
    pub(crate) fn emit_typed(&mut self, typed: lucerna_vim::Typed, cx: &mut Context<Self>) {
        if self.typed_events {
            cx.emit(CodeViewEvent::Typed(typed));
        }
    }

    /// Bumped by every cursor move (keys, mouse, API) and by every edit
    /// that is not typing during vim's Insert session.
    pub(crate) fn record_breaks(&self) -> u64 {
        self.record_breaks
    }

    /// Paints `overlay` as the cursor instead of the caret; `None` paints
    /// the caret again.
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
        let range = self.selections.primary().range();
        (!range.is_empty()).then(|| self.snapshot.text_for_range(range))
    }

    /// Finds the pair of the bracket next to the cursor; uses the syntax
    /// tree when there is one, so brackets in strings are skipped.
    fn update_brackets(&mut self) {
        let at = selection::boundary_offset(&self.snapshot, self.cursor());
        self.brackets = self
            .matching_bracket(at)
            .and_then(|partner| Some([self.matching_bracket(partner)?, partner]));
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

    /// The tab size in effect: the document's detected indentation, else
    /// `editor.tabSize` for this view's language.
    pub fn tab_size(&self) -> u32 {
        self.config.tab_size
    }

    /// Whether indenting inserts spaces: detected, else `editor.insertSpaces`.
    pub fn insert_spaces(&self) -> bool {
        self.config.insert_spaces
    }

    /// Sets the document's detected indentation (`None`: follow the
    /// settings); it drives rendering and editing like the settings do.
    pub(crate) fn set_indentation(
        &mut self,
        indentation: Option<Indentation>,
        cx: &mut Context<Self>,
    ) {
        if self.indentation != indentation {
            self.indentation = indentation;
            self.reload_config(cx);
        }
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
        self.set_selection_points(p, p, cx);
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

    /// Scrolls to `scroll_top` px on the scrollbar's scale, where each
    /// buffer row not hidden by a fold is one line tall; a wrapped row
    /// spreads its share over its display lines.
    fn set_scrollbar_top(&mut self, scroll_top: f32, cx: &mut Context<Self>) {
        let lh = self.line_height();
        let rows = scroll_top.max(0.) / lh;
        // `499 * lh / lh` can come back a hair under 499.
        let n = (rows + 1e-3).floor();
        let last = self
            .folds
            .prev_visible(self.snapshot.line_count().saturating_sub(1));
        let row = self.folds.nth_visible(n as u32).min(last);
        let offset = (rows - n).max(0.) * self.wrap_count(row) as f32 * lh;
        self.scroll = Scroll {
            top_row: row,
            offset,
        };
        // Clamps the offset at the last row.
        self.scroll_by(0.);
        self.hover.hide(cx);
        self.report_scroll(cx);
        cx.notify();
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

/// The editor as the scrollbar's [`ScrollbarSource`].
struct CodeViewScroll(WeakEntity<CodeView>);

impl ScrollbarSource for CodeViewScroll {
    fn metrics(&self, cx: &mut App) -> Option<ScrollMetrics> {
        let view = self.0.upgrade()?;
        let view = view.read(cx);
        view.frame.as_ref().map(|f| f.scroll_metrics())
    }

    fn set_scroll_top(&self, scroll_top: f32, cx: &mut App) {
        if let Some(view) = self.0.upgrade() {
            view.update(cx, |v, cx| v.set_scrollbar_top(scroll_top, cx));
        }
    }

    fn track_click_pages(&self, cx: &App) -> bool {
        self.0
            .upgrade()
            .is_some_and(|v| v.read(cx).config.scroll_by_page)
    }

    /// A screenful of display lines, which with word wrap is fewer rows
    /// than the scale's viewport.
    fn scroll_page(&self, down: bool, metrics: ScrollMetrics, cx: &mut App) {
        if let Some(view) = self.0.upgrade() {
            view.update(cx, |v, cx| {
                v.scroll_by(if down {
                    metrics.viewport
                } else {
                    -metrics.viewport
                });
                v.hover.hide(cx);
                v.report_scroll(cx);
                cx.notify();
            });
        }
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
            with_active_view(wb, cx, CodeView::copy)
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
            self._focus_out = Some(cx.on_focus_out(&focus, window, |v, _, window, cx| {
                // Focus left the view for something outside it (the view's
                // own find widget is inside), for `files.autoSave`.
                if !v.passive
                    && !v.focus.contains_focused(window, cx)
                    && let Some(docs) = v.documents.as_ref().and_then(WeakEntity::upgrade)
                {
                    let path = v.path.clone();
                    docs.update(cx, |d, cx| d.view_focus_lost(&path, cx));
                }
                v.caret.stop();
                v.hover.hide(cx);
                v.set_goto_message(None, cx);
                // Its release (⌘-Tab away) may never reach the view.
                v.link_modifiers_changed(false, cx);
            }));
        }
        let focused = self.focus.is_focused(window) && self.block_cursor().is_none();
        self.caret.ensure(focused, self.config.cursor_blinking, cx);
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
            // "No … found" message; Escape ends an IME composition.
            .on_key_down(cx.listener(|v, ev: &gpui::KeyDownEvent, window, cx| {
                v.hover.hide(cx);
                v.set_goto_message(None, cx);
                if ev.keystroke.key == "escape" && v.marked.is_some() {
                    v.unmark_text(window, cx);
                }
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
                            .child(CodeElement::new(view.clone()))
                            .child(scrollbar(
                                self.scrollbar.clone(),
                                CodeViewScroll(view.downgrade()),
                                view.entity_id(),
                            )),
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

/// The start row of the first hunk after `row` (`forward`) or the last one
/// before it, wrapping around the file; `None` without hunks. Starts are
/// clamped to `last_row`: a deletion at the end of the file starts one past
/// it, where the cursor cannot go.
fn next_change_row(hunks: &[Hunk], row: u32, last_row: u32, forward: bool) -> Option<u32> {
    let mut starts = hunks.iter().map(|h| h.rows.start.min(last_row));
    if forward {
        starts.clone().find(|&s| s > row).or_else(|| starts.next())
    } else {
        starts
            .clone()
            .rfind(|&s| s < row)
            .or_else(|| starts.next_back())
    }
}

/// Converts a theme color to a GPUI fill color.
fn hsla(c: lucerna_theme::Rgba) -> gpui::Hsla {
    color(c).into()
}

/// Where `p` of `old` lands in `new` after `patch`. A point where text was
/// inserted moves past it, like a cursor typing; one at the start of or
/// inside replaced text goes to the start of the replacement.
fn map_point(old: &BufferSnapshot, new: &BufferSnapshot, patch: &Patch, p: Point) -> Point {
    patch.map_point(old, new, p, Bias::Right)
}

/// Byte ranges (sorted, non-overlapping) shifted by `patch`; a range an
/// edit touches is dropped. `active` is an index into `ranges`; it follows
/// its range, or is `None` once that is dropped.
fn remap_ranges(
    ranges: &[Range<usize>],
    active: Option<usize>,
    patch: &Patch,
) -> (Arc<[Range<usize>]>, Option<usize>) {
    let mut out = Vec::with_capacity(ranges.len());
    let mut new_active = None;
    for (i, r) in ranges.iter().enumerate() {
        if touches(patch, r.clone()) {
            continue;
        }
        if active == Some(i) {
            new_active = Some(out.len());
        }
        out.push(patch.map_offset(r.start, Bias::Left)..patch.map_offset(r.end, Bias::Left));
    }
    (out.into(), new_active)
}

/// Whether an edit of `patch` overlaps or touches the old byte `range`.
fn touches(patch: &Patch, range: Range<usize>) -> bool {
    let edits = patch.edits();
    let first = edits.partition_point(|e| e.old.end < range.start);
    edits.get(first).is_some_and(|e| e.old.start <= range.end)
}

#[cfg(test)]
mod tests {
    use lucerna_git::HunkKind;

    use super::*;

    fn hunk(kind: HunkKind, rows: Range<u32>) -> Hunk {
        Hunk {
            kind,
            rows,
            base_rows: 0..0,
        }
    }

    #[test]
    fn next_change_row_wraps_both_ways() {
        let hunks = [
            hunk(HunkKind::Added, 2..4),
            hunk(HunkKind::Deleted, 6..6),
            hunk(HunkKind::Modified, 10..11),
        ];
        let next = |row, forward| next_change_row(&hunks, row, 20, forward);
        assert_eq!(next(0, true), Some(2));
        assert_eq!(next(2, true), Some(6));
        assert_eq!(next(3, true), Some(6));
        assert_eq!(next(11, true), Some(2));
        assert_eq!(next(11, false), Some(10));
        assert_eq!(next(10, false), Some(6));
        assert_eq!(next(2, false), Some(10));
        assert_eq!(next_change_row(&[], 2, 20, false), None);
        assert_eq!(next_change_row(&[], 2, 20, true), None);
    }

    /// A deletion at the end of a 5-row file starts at row 5, past the
    /// last row: it counts as row 4, so forward navigation from there
    /// wraps instead of landing on it again.
    #[test]
    fn next_change_row_clamps_a_deletion_at_eof() {
        let hunks = [hunk(HunkKind::Added, 1..2), hunk(HunkKind::Deleted, 5..5)];
        assert_eq!(next_change_row(&hunks, 1, 4, true), Some(4));
        assert_eq!(next_change_row(&hunks, 4, 4, true), Some(1));
        assert_eq!(next_change_row(&hunks, 4, 4, false), Some(1));
        assert_eq!(next_change_row(&hunks, 1, 4, false), Some(4));
    }
}
