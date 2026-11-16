//! A list of files with code excerpts, as in Zed's multibuffer: a header
//! per file, then its excerpts as editor-like lines with a line-number
//! gutter, colored in the background. Files fold, the keyboard moves over
//! the lines, and a click or `Enter` asks the host to open a line
//! ([`ExcerptListEvent::Activate`]). The Search and References tabs show
//! their results in one; the host keeps its own toolbar and status.
//!
//! A diff view shows its hunks in one too: lines are context, added or
//! deleted ([`lucerna_project::LineKind`]), colored from the texts the
//! host gives ([`ExcerptList::set_text_source`]), with per-file counts,
//! tags and notes on the headers ([`ExcerptList::set_file_meta`]) and
//! change navigation ([`ExcerptList::step_change`]).

pub(crate) mod highlight;
pub(crate) mod model;
mod render;

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Hsla, KeyDownEvent, ListAlignment, ListOffset,
    ListState, Pixels, Subscription, Task, px,
};
use lucerna_project::{Excerpt, LineKind};

use crate::layout::color;
use crate::{AppState, WindowSettings};
use highlight::{FileHighlights, HighlightCache, LineSpans};
use model::{FileMeta, LineAnchor, LineRef, Results, Row};
use render::{editor_line_height, header_height, row_height};

/// The gutter always has room for this many digits.
pub const MIN_GUTTER_DIGITS: u32 = 4;

/// Pixels of rows rendered above and below the visible ones.
const LIST_OVERDRAW: f32 = 200.;

/// The texts a diff view's lines are colored from: current lines from
/// `current` (by line number; the file on disk when `None`), deleted lines
/// from `base` (by base line; plain when `None`). LF line endings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextSource {
    pub current: Option<Arc<str>>,
    pub base: Option<Arc<str>>,
}

pub enum ExcerptListEvent {
    /// Open line `r` (a click, `Enter`, or a header's "open file" button).
    /// The line is already selected.
    Activate(LineRef),
}

pub struct ExcerptList {
    /// File paths are relative to this; highlight jobs read the files here.
    root: PathBuf,
    /// Starts the debug selectors of the rows (`"search"` gives
    /// `search-file-0`, `search-line-…`), so each host's are distinct.
    id_prefix: &'static str,
    /// The window's settings: the rows use the editor's font and sizes.
    settings: Entity<WindowSettings>,
    _settings: Subscription,
    /// Focus of the list: the host tracks it on the list area and passes
    /// keys to [`ExcerptList::key_down`].
    focus: FocusHandle,
    results: Results,
    /// The keyboard / last clicked line, named by path and line number so
    /// streamed results inserting files before it do not move it.
    selected: Option<LineAnchor>,
    /// The largest line number shown since the list was last cleared:
    /// sizes the gutter. It never shrinks until [`clear`](Self::clear), so
    /// a re-search never shifts the code sideways.
    max_line_number: u32,
    /// The window's scale factor at the last frame: row heights are
    /// rounded to its device pixels.
    scale_factor: f32,
    /// The rows: of different heights (headers are two lines tall), only
    /// the visible ones rendered.
    list: ListState,
    /// The rows `list` was last told about, to splice only what changed.
    list_rows: Vec<Row>,
    /// Syntax colors per line, kept across changes (a line's colors are
    /// used only while its text is unchanged).
    highlight_cache: HighlightCache,
    /// Files queued or running; a file in here is never queued again.
    highlight_pending: HashSet<Arc<Path>>,
    highlight_queue: VecDeque<Arc<Path>>,
    highlight_tasks: HashMap<Arc<Path>, Task<()>>,
    /// Bumped by a theme change: results of an older epoch are dropped.
    highlight_epoch: u64,
    /// The `theme_generation` the highlights were computed for.
    highlight_theme: u64,
    peak_highlight_jobs: usize,
    highlight_jobs_started: usize,
    /// Frames rendered, each resolving the selection once.
    selection_resolves: usize,
    /// The texts of the files colored from the host's texts, not the
    /// disk, each with the generation it was set at.
    text_sources: HashMap<Arc<Path>, (TextSource, u64)>,
    /// The last generation given to a text source; files without one
    /// are generation 0.
    last_source_generation: u64,
}

impl EventEmitter<ExcerptListEvent> for ExcerptList {}

impl ExcerptList {
    /// A list of the files under `root`; `id_prefix` starts its debug
    /// selectors.
    pub fn new(
        root: PathBuf,
        id_prefix: &'static str,
        settings: Entity<WindowSettings>,
        cx: &mut Context<Self>,
    ) -> Self {
        ExcerptList {
            root,
            id_prefix,
            _settings: cx.observe(&settings, |_, _, cx| cx.notify()),
            settings,
            focus: cx.focus_handle(),
            results: Results::default(),
            selected: None,
            max_line_number: 0,
            scale_factor: 1.,
            list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            list_rows: Vec::new(),
            highlight_cache: HighlightCache::default(),
            highlight_pending: HashSet::new(),
            highlight_queue: VecDeque::new(),
            highlight_tasks: HashMap::new(),
            highlight_epoch: 0,
            highlight_theme: cx.global::<AppState>().theme_generation,
            peak_highlight_jobs: 0,
            highlight_jobs_started: 0,
            selection_resolves: 0,
            text_sources: HashMap::new(),
            last_source_generation: 0,
        }
    }

    /// Replaces every file; the gutter is sized for the new ones. The
    /// text sources and meta of the paths still present are kept.
    pub fn set_files(&mut self, files: Vec<(Arc<Path>, Vec<Excerpt>)>, cx: &mut Context<Self>) {
        self.max_line_number = 0;
        let present: HashSet<&Arc<Path>> = files.iter().map(|(p, _)| p).collect();
        self.text_sources.retain(|p, _| present.contains(p));
        self.update_results(|r| r.replace_files(files), cx);
    }

    /// Colors the lines of file `path` from `source` instead of the disk.
    /// A changed source colors the file again, showing the old colors
    /// until then. Kept until the file leaves the list.
    pub fn set_text_source(&mut self, path: Arc<Path>, source: TextSource, cx: &mut Context<Self>) {
        if self
            .text_sources
            .get(&path)
            .is_some_and(|(s, _)| *s == source)
        {
            return;
        }
        self.last_source_generation += 1;
        self.text_sources
            .insert(path, (source, self.last_source_generation));
        cx.notify();
    }

    /// The generation of file `path`'s text source; 0 without one.
    fn source_generation(&self, path: &Path) -> u64 {
        self.text_sources.get(path).map_or(0, |(_, g)| *g)
    }

    /// Sets the diff meta of file `path`, shown on its header. Kept until
    /// the file leaves the list.
    pub fn set_file_meta(&mut self, path: Arc<Path>, meta: FileMeta, cx: &mut Context<Self>) {
        self.update_results(|r| r.set_meta(path, meta), cx);
    }

    /// Applies a streamed batch (see [`Results::update_files`]).
    pub fn add_files(&mut self, batch: Vec<(Arc<Path>, Vec<Excerpt>)>, cx: &mut Context<Self>) {
        self.update_results(|r| r.update_files(batch), cx);
    }

    /// Replaces the excerpts of file `path`, keeping its fold, or adds it.
    pub fn set_file(&mut self, path: Arc<Path>, excerpts: Vec<Excerpt>, cx: &mut Context<Self>) {
        self.update_results(|r| r.set_file(path, excerpts), cx);
    }

    /// Drops the files `paths`, with their text sources and meta.
    pub fn remove_files(&mut self, paths: &[Arc<Path>], cx: &mut Context<Self>) {
        self.update_results(|r| r.remove_files(paths), cx);
    }

    /// Folds or unfolds the file `path`.
    pub fn set_collapsed(&mut self, path: &Path, collapsed: bool, cx: &mut Context<Self>) {
        if let Some(file) = self.results.file_index(path) {
            self.update_results(|r| r.set_collapsed(file, collapsed), cx);
        }
    }

    /// Whether file `path` is colored from a text source.
    #[doc(hidden)]
    pub fn has_text_source(&self, path: &Path) -> bool {
        self.text_sources.contains_key(path)
    }

    /// Changes the results, then tells the list once: the gutter grows to
    /// the largest line number left, a selection whose line is gone is
    /// dropped, and only the changed rows are spliced. Files that left
    /// take their text sources and meta with them.
    pub fn update_results(&mut self, change: impl FnOnce(&mut Results), cx: &mut Context<Self>) {
        let listed: Vec<Arc<Path>> = if self.text_sources.is_empty() && !self.results.has_meta() {
            Vec::new()
        } else {
            self.results
                .files()
                .iter()
                .map(|f| f.path.clone())
                .collect()
        };
        change(&mut self.results);
        for path in listed {
            if self.results.file_index(&path).is_none() {
                self.text_sources.remove(&path);
                self.results.forget_meta(&path);
            }
        }
        self.max_line_number = self.max_line_number.max(self.results.max_line_number());
        if self
            .selected
            .as_ref()
            .is_some_and(|a| self.results.resolve(a).is_none())
        {
            self.selected = None;
        }
        self.sync_list(cx);
        cx.notify();
    }

    /// Drops every file (with its text source and meta) and the
    /// selection; the gutter shrinks back.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.results.clear();
        self.text_sources.clear();
        self.sync_list(cx);
        self.selected = None;
        self.max_line_number = 0;
        cx.notify();
    }

    /// The files, sorted by path, and the rows shown.
    pub fn results(&self) -> &Results {
        &self.results
    }

    /// The selected line, at its current position.
    pub fn selected(&self) -> Option<LineRef> {
        self.results.resolve(self.selected.as_ref()?)
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// The cached syntax colors of file `path` (relative to the root), by
    /// line number, once computed.
    pub fn highlights(&self, path: &Path) -> Option<FileHighlights> {
        self.highlight_cache.file(path)
    }

    /// The cached syntax colors of line `r`, once computed.
    #[doc(hidden)]
    pub fn line_spans(&self, r: LineRef) -> Option<LineSpans> {
        let path = &self.results.files().get(r.file)?.path;
        self.highlight_cache
            .get(path, self.results.line(r)?)
            .cloned()
    }

    /// The background line `r` is drawn on: the diff color of an added or
    /// deleted line (the selection's active-line color is laid over it),
    /// else the active-line color when selected, else none.
    #[doc(hidden)]
    pub fn row_background(&self, r: LineRef, cx: &App) -> Option<Hsla> {
        let kind = self.results.line(r)?.kind;
        self.background_of(kind, self.selected() == Some(r), cx)
    }

    /// [`row_background`](Self::row_background) of a line of `kind`,
    /// `selected` or not.
    fn background_of(&self, kind: LineKind, selected: bool, cx: &App) -> Option<Hsla> {
        let ui = &cx.global::<AppState>().theme.ui;
        let c = match kind {
            LineKind::Added => ui.diff_added_background,
            LineKind::Deleted { .. } => ui.diff_deleted_background,
            LineKind::Normal if selected => ui.editor_active_line_background,
            LineKind::Normal => return None,
        };
        Some(color(c).into())
    }

    /// How often a frame resolved the selection: once per frame.
    #[doc(hidden)]
    pub fn selection_resolves(&self) -> usize {
        self.selection_resolves
    }

    /// How often the highlight queue scanned a file's lines for missing
    /// colors: not again while the file and its colors are unchanged.
    #[doc(hidden)]
    pub fn highlight_scans(&self) -> usize {
        self.highlight_cache.scans()
    }

    /// Highlight jobs running now; never above
    /// [`MAX_HIGHLIGHT_JOBS`](highlight::MAX_HIGHLIGHT_JOBS).
    pub fn highlight_jobs_in_flight(&self) -> usize {
        self.highlight_tasks.len()
    }

    /// The most highlight jobs that ran at once since this list was made.
    pub fn peak_highlight_jobs(&self) -> usize {
        self.peak_highlight_jobs
    }

    /// Highlight jobs started since this list was made.
    pub fn highlight_jobs_started(&self) -> usize {
        self.highlight_jobs_started
    }

    /// The digits the line-number gutter is sized for: at least
    /// [`MIN_GUTTER_DIGITS`], growing with the largest line number shown.
    pub fn gutter_digits(&self) -> u32 {
        (self.max_line_number.max(1).ilog10() + 1).max(MIN_GUTTER_DIGITS)
    }

    /// Folds or unfolds the excerpts of file `file`.
    pub fn toggle_collapsed(&mut self, file: usize, cx: &mut Context<Self>) {
        self.results.toggle_collapsed(file);
        self.sync_list(cx);
        cx.notify();
    }

    /// Folds or unfolds every file.
    pub fn set_all_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        self.results.set_all_collapsed(collapsed);
        self.sync_list(cx);
        cx.notify();
    }

    /// Folds or unfolds the file `path`; the click handlers name files by
    /// path because indices shift while results stream in.
    fn toggle_path(&mut self, path: &Path, cx: &mut Context<Self>) {
        if let Some(file) = self.results.file_index(path) {
            self.toggle_collapsed(file, cx);
        }
    }

    /// Selects the next or previous match line (from the selection,
    /// wrapping), unfolds its file and scrolls it into view.
    pub fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) -> Option<LineRef> {
        let r = self.results.next_match(self.selected(), forward)?;
        self.results.expand(r.file);
        self.sync_list(cx);
        self.select(r, cx);
        Some(r)
    }

    /// Selects the first line of the next or previous run of added or
    /// deleted lines (from the selection, wrapping, across files), unfolds
    /// its file and scrolls it into view.
    pub fn step_change(&mut self, forward: bool, cx: &mut Context<Self>) -> Option<LineRef> {
        let r = self.results.next_change(self.selected(), forward)?;
        self.results.expand(r.file);
        self.sync_list(cx);
        self.select(r, cx);
        Some(r)
    }

    /// Selects line `r`, unfolding its file and scrolling it into view.
    pub fn select_line(&mut self, r: LineRef, cx: &mut Context<Self>) {
        if self.results.line(r).is_none() {
            return;
        }
        self.results.expand(r.file);
        self.sync_list(cx);
        self.select(r, cx);
    }

    /// Selects line `r`, scrolling it into view.
    fn select(&mut self, r: LineRef, cx: &mut Context<Self>) {
        self.selected = self.results.anchor_of(r);
        if let Some(ix) = self.results.row_of(r) {
            self.reveal_row(ix, cx);
        }
        cx.notify();
    }

    /// Selects line `r` and asks the host to open it.
    fn activate(&mut self, r: LineRef, cx: &mut Context<Self>) {
        self.select(r, cx);
        cx.emit(ExcerptListEvent::Activate(r));
    }

    /// Selects the first match of the file `path` (in a diff, its first
    /// change) and opens it (the header's "open file" button).
    fn open_file_at_first_match(&mut self, path: &Path, cx: &mut Context<Self>) {
        if let Some(r) = self
            .results
            .file_index(path)
            .and_then(|file| self.results.first_line_to_open(file))
        {
            self.activate(r, cx);
        }
    }

    /// Selects the line `anchor` names and opens it (a click).
    fn open_anchor(&mut self, anchor: &LineAnchor, cx: &mut Context<Self>) {
        if let Some(r) = self.results.resolve(anchor) {
            self.activate(r, cx);
        }
    }

    /// Folds or unfolds the file of the sticky header, then shows its
    /// header at the top.
    fn toggle_sticky(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.toggle_path(path, cx);
        if let Some(row) = self
            .results
            .file_index(path)
            .and_then(|file| self.results.header_row(file))
        {
            self.list.scroll_to(ListOffset {
                item_ix: row,
                offset_in_item: px(0.),
            });
        }
    }

    /// The list's keys, for the host's key handler on the list area:
    /// `↑` / `↓` move over the lines, `Enter` opens the selected one.
    /// Returns whether the key was handled.
    pub fn key_down(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let m = &ev.keystroke.modifiers;
        if m.platform || m.control || m.alt || m.shift {
            return false;
        }
        match ev.keystroke.key.as_str() {
            "down" => self.move_selection(true, cx),
            "up" => self.move_selection(false, cx),
            "enter" => {
                if let Some(r) = self.selected() {
                    cx.emit(ExcerptListEvent::Activate(r));
                }
            }
            _ => return false,
        }
        true
    }

    /// `↑` / `↓`: moves over the lines, skipping headers and separators
    /// and stopping at the ends.
    fn move_selection(&mut self, forward: bool, cx: &mut Context<Self>) {
        let lines: Vec<(usize, LineRef)> = self
            .results
            .rows()
            .iter()
            .enumerate()
            .filter_map(|(ix, row)| match row {
                Row::Line(r) => Some((ix, *r)),
                _ => None,
            })
            .collect();
        let current = self
            .selected()
            .and_then(|r| lines.iter().position(|(_, l)| *l == r));
        let next = match (current, forward) {
            (None, true) => 0,
            (None, false) => lines.len().saturating_sub(1),
            (Some(i), true) => (i + 1).min(lines.len().saturating_sub(1)),
            (Some(i), false) => i.saturating_sub(1),
        };
        if let Some((_, r)) = lines.get(next) {
            self.select(*r, cx);
        }
    }

    /// Tells the list which rows changed since it was last told. Row heights
    /// depend only on the row's kind, so the longest common prefix and suffix
    /// of kinds keep their measured heights; only the middle is spliced.
    /// The scroll position stays at the same pixel offset, as a uniform list
    /// keeps it (a splice at the scroll top would push it down). Runs once
    /// per change (an event batch, a fold), never per frame.
    fn sync_list(&mut self, cx: &App) {
        let new = self.results.rows();
        let old = &self.list_rows;
        let same = |(a, b): &(&Row, &Row)| a.same_kind(b);
        let prefix = old.iter().zip(new).take_while(same).count();
        if prefix == old.len() && prefix == new.len() {
            self.list_rows = new.to_vec();
            return;
        }
        let room = old.len().min(new.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(room)
            .take_while(same)
            .count();
        let lh = editor_line_height(self.scale_factor, self.settings.read(cx).view(cx));
        let top = self.list.logical_scroll_top();
        let scrolled: Pixels = old[..top.item_ix.min(old.len())]
            .iter()
            .map(|row| row_height(row, lh))
            .fold(top.offset_in_item, |a, b| a + b);
        self.list
            .splice(prefix..old.len() - suffix, new.len() - prefix - suffix);
        self.list_rows = new.to_vec();
        self.list.scroll_to(self.offset_at(scrolled, lh));
    }

    /// The scroll top `y` pixels below the top of the rows.
    fn offset_at(&self, y: Pixels, lh: Pixels) -> ListOffset {
        let mut top = px(0.);
        for (item_ix, row) in self.results.rows().iter().enumerate() {
            let height = row_height(row, lh);
            if top + height > y {
                return ListOffset {
                    item_ix,
                    offset_in_item: y - top,
                };
            }
            top += height;
        }
        ListOffset {
            item_ix: self.results.rows().len(),
            offset_in_item: px(0.),
        }
    }

    /// The scroll top that puts row `ix`'s top `distance` below the top of
    /// the list (or the list's start, when there is not enough above).
    fn scroll_top_above(&self, ix: usize, distance: Pixels, lh: Pixels) -> ListOffset {
        let rows = self.results.rows();
        let (mut remaining, mut i) = (distance, ix);
        while remaining > px(0.) && i > 0 {
            i -= 1;
            remaining -= row_height(&rows[i], lh);
        }
        if remaining > px(0.) {
            ListOffset::default()
        } else {
            ListOffset {
                item_ix: i,
                offset_in_item: -remaining,
            }
        }
    }

    /// Scrolls the least that shows row `ix` whole, below the sticky header
    /// for a line. Row heights are known exactly, so this holds for rows
    /// the list has not measured yet.
    fn reveal_row(&mut self, ix: usize, cx: &App) {
        let lh = editor_line_height(self.scale_factor, self.settings.read(cx).view(cx));
        let rows = self.results.rows();
        let Some(row) = rows.get(ix) else {
            return;
        };
        let height = row_height(row, lh);
        let margin = match row {
            Row::Header(_) => px(0.),
            _ => header_height(lh),
        };
        let viewport = self.list.viewport_bounds().size.height;
        let top = self.list.logical_scroll_top();
        let target = if ix < top.item_ix {
            Some(margin)
        } else {
            let mut y = -top.offset_in_item;
            for row in &rows[top.item_ix.min(ix)..ix] {
                y += row_height(row, lh);
                if y > viewport {
                    break;
                }
            }
            if y < margin {
                Some(margin)
            } else if y + height > viewport {
                Some((viewport - height).max(margin))
            } else {
                None
            }
        };
        if let Some(distance) = target {
            self.list.scroll_to(self.scroll_top_above(ix, distance, lh));
        }
    }

    /// The rows inside a `viewport` tall list at its scroll position.
    fn visible_rows(&self, viewport: Pixels, lh: Pixels) -> Range<usize> {
        let rows = self.results.rows();
        let top = self.list.logical_scroll_top();
        let start = top.item_ix.min(rows.len());
        let mut y = -top.offset_in_item;
        let mut end = start;
        while end < rows.len() && y < viewport {
            y += row_height(&rows[end], lh);
            end += 1;
        }
        start..end
    }

    /// The file whose header sticks to the top of the list, and how far up
    /// the next file's header pushes it: `None` while a header is at the
    /// very top.
    fn sticky_header(&self, lh: Pixels) -> Option<(usize, Pixels)> {
        let rows = self.results.rows();
        let top = self.list.logical_scroll_top();
        let first = rows.get(top.item_ix)?;
        if matches!(first, Row::Header(_)) && top.offset_in_item <= px(0.) {
            return None;
        }
        let file = first.file();
        let header = header_height(lh);
        let mut y = -top.offset_in_item;
        for row in &rows[top.item_ix..] {
            if y >= header {
                break;
            }
            if matches!(row, Row::Header(f) if *f != file) {
                return Some((file, (y - header).min(px(0.))));
            }
            y += row_height(row, lh);
        }
        Some((file, px(0.)))
    }
}
