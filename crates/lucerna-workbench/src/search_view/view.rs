//! The Search tab's body (`workbench.action.findInFiles`, `⇧⌘F`): a query
//! with case / word / regex toggles, optional include and exclude globs,
//! and the results as excerpts grouped by file as they stream in.
//!
//! Every change waits [`SEARCH_DEBOUNCE`] before searching and cancels the
//! running search through its `AtomicBool`. Each search gets a fresh
//! channel and a generation; events of a superseded search are dropped
//! (ruling R17).

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt;
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, Entity, FocusHandle, Focusable,
    FontStyle, FontWeight, HighlightStyle, IntoElement, KeyDownEvent, ListAlignment, ListOffset,
    ListState, MouseButton, ParentElement, Pixels, Render, SharedString, Stateful, Styled,
    StyledText, Subscription, Task, WeakEntity, Window, div, font, list, prelude::*, px, relative,
    svg,
};
use lucerna_project::{SearchEvent, SearchQuery, search};
use lucerna_text::Point;

use super::highlight::{FileHighlights, LineSpans, highlight_file};
use super::model::{
    LineAnchor, LineRef, Results, Row, expand_tabs, first_match, globs, summary, thousands,
};
use crate::code_view::CodeView;
use crate::find_widget::{build_regex, single_line_selection};
use crate::icons::{
    ARROW_UP_RIGHT, CASE_SENSITIVE, CHEVRONS_DOWN_UP, CHEVRONS_UP_DOWN, LIST_FILTER, REFRESH_CW,
    REGEX, WHOLE_WORD,
};
use crate::layout::color;
use crate::tasks::guarded;
use crate::text_input::TextInput;
use crate::ui::{
    CHEVRON_DOWN_ICON, CHEVRON_RIGHT_ICON, ClickHandler, FILE_ICON_SIZE, GRID, UI_FONT_SIZE,
    UI_FONT_SIZE_SMALL, bar_icon_button, bar_separator, file_icon_sized, input_box, match_nav,
    search_bar_row, search_query_box, search_query_column, search_toolbar, toggle_button,
};
use crate::workbench::enabled_globs;
use crate::workspace_state::SearchState;
use crate::{AppState, CommandRegistry, DocState, OpenMode, Workbench};

/// How long the inputs must stay unchanged before a search starts.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);

/// The most files highlighted at once, so a long list never starves the
/// background executor.
pub const MAX_HIGHLIGHT_JOBS: usize = 3;

/// Shown instead of results when the view is not [`searchable`](SearchView).
const UNAVAILABLE: &str = "Search is unavailable when a single file is opened";

/// The setting with the context lines kept around each matching line.
const CONTEXT_LINES_SETTING: &str = "search.searchEditor.defaultNumberOfContextLines";

/// The most context lines a search keeps, whatever the setting says.
const MAX_CONTEXT_LINES: u32 = 5;

/// Pixels of rows rendered above and below the visible ones.
const LIST_OVERDRAW: f32 = 200.;

/// A file header is this many editor lines tall, as in Zed.
const FILE_HEADER_LINES: f32 = 2.;

/// Padding around a file header's card (Zed's `BUFFER_HEADER_PADDING`).
const FILE_HEADER_PADDING: f32 = GRID;

pub struct SearchView {
    workbench: WeakEntity<Workbench>,
    root: PathBuf,
    focus: FocusHandle,
    /// Focus of the results list: arrows, Enter and Esc work here.
    list_focus: FocusHandle,
    /// The keyboard / last clicked line, named by path and line number so
    /// streamed results inserting files before it do not move it.
    selected: Option<LineAnchor>,
    query: Entity<TextInput>,
    include: Entity<TextInput>,
    exclude: Entity<TextInput>,
    case_sensitive: bool,
    whole_word: bool,
    is_regex: bool,
    /// The include / exclude row is shown.
    filters_shown: bool,
    results: Results,
    /// The largest line number among the results: sizes the gutter.
    max_line_number: u32,
    /// A search for a non-empty query was started.
    searched: bool,
    /// `Some(truncated)` once the search finished.
    done: Option<bool>,
    error: Option<String>,
    /// `error` is the query's own fault (a bad pattern), not an I/O or
    /// root failure: only then does the query box turn red.
    pattern_error: bool,
    /// The window's scale factor at the last frame: row heights are
    /// rounded to its device pixels.
    scale_factor: f32,
    /// The query the shown results came from.
    last_query: Option<SearchQuery>,
    /// Bumped by every change; events of older searches are dropped.
    generation: u64,
    cancel: Arc<AtomicBool>,
    _debounce: Task<()>,
    _search: Task<()>,
    /// Highlights a clicked match once its file has loaded.
    _highlight: Option<Subscription>,
    /// The results list: rows of different heights (headers are two lines
    /// tall), only the visible ones rendered.
    list: ListState,
    /// The rows `list` was last told about, to splice only what changed.
    list_rows: Vec<Row>,
    /// Syntax colors per result file (path relative to the root).
    highlights: HashMap<Arc<Path>, FileHighlights>,
    /// Files queued or running; a file in here or in `highlights` is never
    /// queued again.
    highlight_pending: HashSet<Arc<Path>>,
    highlight_queue: VecDeque<Arc<Path>>,
    highlight_tasks: HashMap<Arc<Path>, Task<()>>,
    /// Bumped by a new search and by a theme change: results of an older
    /// epoch are dropped.
    highlight_epoch: u64,
    /// The `theme_generation` the highlights were computed for.
    highlight_theme: u64,
    peak_highlight_jobs: usize,
    /// `false` in single-file mode (R29): the folder is never walked.
    searchable: bool,
    /// Input changes still to come from [`SearchView::restore`] setting the
    /// inputs, which must not start a search.
    restore_changes: usize,
}

/// How result rows are drawn, read from the `editor.*` settings each frame.
#[derive(Clone)]
struct RowStyle {
    font_family: SharedString,
    font_size: Pixels,
    line_height: Pixels,
    gutter: Pixels,
    tab_size: u32,
}

/// The most query characters the tab title shows.
const TITLE_MAX_CHARS: usize = 30;

impl SearchView {
    pub fn new(
        root: PathBuf,
        workbench: WeakEntity<Workbench>,
        searchable: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = |placeholder: &'static str, cx: &mut Context<Self>| {
            let this = cx.weak_entity();
            cx.new(|cx| {
                let mut input = TextInput::new(cx);
                input.set_placeholder(placeholder);
                input.on_change(move |_, _, cx| {
                    this.update(cx, |view, cx| view.inputs_changed(cx)).ok();
                });
                input
            })
        };
        SearchView {
            workbench,
            root,
            focus: cx.focus_handle(),
            list_focus: cx.focus_handle(),
            selected: None,
            query: input("Search", cx),
            include: input("e.g. src/**, *.rs", cx),
            exclude: input("e.g. target, *.min.js", cx),
            case_sensitive: false,
            whole_word: false,
            is_regex: false,
            filters_shown: false,
            results: Results::default(),
            max_line_number: 0,
            searched: false,
            done: None,
            error: None,
            pattern_error: false,
            scale_factor: 1.,
            last_query: None,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            _debounce: Task::ready(()),
            _search: Task::ready(()),
            _highlight: None,
            list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            list_rows: Vec::new(),
            highlights: HashMap::new(),
            highlight_pending: HashSet::new(),
            highlight_queue: VecDeque::new(),
            highlight_tasks: HashMap::new(),
            highlight_epoch: 0,
            highlight_theme: cx.global::<AppState>().theme_generation,
            peak_highlight_jobs: 0,
            searchable,
            restore_changes: 0,
        }
    }

    /// The Search tab's title: the query; `None` (the tab says "Search")
    /// while it is empty. Long queries are cut to [`TITLE_MAX_CHARS`]
    /// characters plus "…" so one cannot widen the tab; the saved query
    /// is untouched.
    pub fn title(&self, cx: &App) -> Option<SharedString> {
        let query = self.query.read(cx).text();
        if query.trim().is_empty() {
            return None;
        }
        let mut chars = query.chars();
        let mut title: String = chars.by_ref().take(TITLE_MAX_CHARS).collect();
        if chars.next().is_some() {
            title.push('…');
        }
        Some(title.into())
    }

    /// The inputs and toggles, as saved with the workspace.
    pub fn state(&self, cx: &App) -> SearchState {
        SearchState {
            query: self.query.read(cx).text().to_owned(),
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            is_regex: self.is_regex,
            include: self.include.read(cx).text().to_owned(),
            exclude: self.exclude.read(cx).text().to_owned(),
            filters_visible: self.filters_shown,
        }
    }

    /// Puts saved inputs back. Searches nothing: the caller runs
    /// [`SearchView::run_search`] when the tab is open.
    pub fn restore(&mut self, state: SearchState, window: &mut Window, cx: &mut Context<Self>) {
        self.case_sensitive = state.case_sensitive;
        self.whole_word = state.whole_word;
        self.is_regex = state.is_regex;
        self.filters_shown = state.filters_visible;
        for (input, text) in [
            (&self.query, &state.query),
            (&self.include, &state.include),
            (&self.exclude, &state.exclude),
        ] {
            if input.read(cx).text() != text.as_str() {
                self.restore_changes += 1;
                input.update(cx, |input, cx| input.set_text(text, window, cx));
            }
        }
        cx.notify();
    }

    pub fn query_input(&self) -> &Entity<TextInput> {
        &self.query
    }

    /// "files to include": comma-separated globs.
    pub fn include_input(&self) -> &Entity<TextInput> {
        &self.include
    }

    /// "files to exclude": comma-separated globs.
    pub fn exclude_input(&self) -> &Entity<TextInput> {
        &self.exclude
    }

    pub fn case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    pub fn whole_word(&self) -> bool {
        self.whole_word
    }

    pub fn is_regex(&self) -> bool {
        self.is_regex
    }

    /// The focus handle of the results list.
    pub fn list_focus_handle(&self) -> &FocusHandle {
        &self.list_focus
    }

    /// The selected result line, at its current position.
    pub fn selected(&self) -> Option<LineRef> {
        self.results.resolve(self.selected.as_ref()?)
    }

    /// The files with results, sorted by path, and the rows shown.
    pub fn results(&self) -> &Results {
        &self.results
    }

    /// The syntax colors of the result file `path` (relative to the root),
    /// once computed.
    pub fn highlights(&self, path: &Path) -> Option<&FileHighlights> {
        self.highlights.get(path)
    }

    /// Highlight jobs running now; never above [`MAX_HIGHLIGHT_JOBS`].
    pub fn highlight_jobs_in_flight(&self) -> usize {
        self.highlight_tasks.len()
    }

    /// The most highlight jobs that ran at once since this view was made.
    pub fn peak_highlight_jobs(&self) -> usize {
        self.peak_highlight_jobs
    }

    /// The regex error of the last search, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// `"N results in M files"` (see [`summary`]); `None` before any search
    /// and while an error shows. In single-file mode it is the
    /// "unavailable" message instead.
    pub fn summary(&self) -> Option<String> {
        if !self.searchable {
            return Some(UNAVAILABLE.into());
        }
        if self.error.is_some() {
            return None;
        }
        summary(
            self.results.match_count(),
            self.results.files().len(),
            self.searched,
            self.done,
        )
    }

    /// Folds or unfolds the excerpts of file `file`.
    pub fn toggle_collapsed(&mut self, file: usize, cx: &mut Context<Self>) {
        self.results.toggle_collapsed(file);
        self.sync_list(cx);
        cx.notify();
    }

    /// Folds every file, or unfolds them all when all are folded.
    fn toggle_all_collapsed(&mut self, cx: &mut Context<Self>) {
        let all = self.results.files().iter().all(|f| f.collapsed);
        self.set_all_collapsed(!all, cx);
    }

    /// `search.action.collapseSearchResults` and the button's two states.
    fn set_all_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
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

    pub fn toggle_case_sensitive(&mut self, cx: &mut Context<Self>) {
        self.toggle(|v| &mut v.case_sensitive, cx);
    }

    pub fn toggle_whole_word(&mut self, cx: &mut Context<Self>) {
        self.toggle(|v| &mut v.whole_word, cx);
    }

    pub fn toggle_regex(&mut self, cx: &mut Context<Self>) {
        self.toggle(|v| &mut v.is_regex, cx);
    }

    /// Shows or hides the include / exclude row; showing it focuses
    /// "files to include" (as VS Code does).
    pub fn toggle_query_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filters_shown = !self.filters_shown;
        if self.filters_shown {
            let focus = self.include.focus_handle(cx);
            window.focus(&focus, cx);
        } else if self.include.focus_handle(cx).contains_focused(window, cx)
            || self.exclude.focus_handle(cx).contains_focused(window, cx)
        {
            self.focus_query(None, window, cx);
        }
        cx.notify();
    }

    /// Selects line `r`, scrolling it into view.
    fn select(&mut self, r: LineRef, cx: &mut Context<Self>) {
        self.selected = self.results.anchor_of(r);
        if let Some(ix) = self.results.row_of(r) {
            self.reveal_row(ix, cx);
        }
        cx.notify();
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
        let lh = editor_line_height(self.scale_factor, cx);
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
        let lh = editor_line_height(self.scale_factor, cx);
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

    /// The bar's match counter, as in Zed: `"3/19"` for the selected match
    /// line among all match lines, `"19 results"` with no match selected,
    /// `"0/0"` without matches.
    pub fn match_label(&self) -> String {
        let total = self.results.match_count();
        if total == 0 {
            return "0/0".into();
        }
        match self.selected().and_then(|r| self.results.match_index(r)) {
            Some(i) => format!("{}/{}", thousands(i + 1), thousands(total)),
            None if total == 1 => "1 result".into(),
            None => format!("{} results", thousands(total)),
        }
    }

    /// The query box shows the error color: a bad pattern, or a finished
    /// search without matches. Other errors (I/O, the root) leave it alone.
    pub fn is_invalid(&self) -> bool {
        (self.error.is_some() && self.pattern_error)
            || (self.error.is_none()
                && self.searched
                && self.done.is_some()
                && self.results.match_count() == 0)
    }

    /// Selects the first match of the file `path` and opens it (the
    /// header's "open file" button).
    fn open_file_at_first_match(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(r) = self
            .results
            .file_index(path)
            .and_then(|file| self.results.first_match_of(file))
        else {
            return;
        };
        self.select(r, cx);
        self.open_line(r, window, cx);
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

    /// Selects the line `anchor` names and opens it (a click).
    fn open_anchor(&mut self, anchor: &LineAnchor, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.results.resolve(anchor) else {
            return;
        };
        self.select(r, cx);
        self.open_line(r, window, cx);
    }

    /// `F4` / `Shift+F4`: selects the next / previous match line (from the
    /// selection, wrapping), unfolds its file and opens it as a preview.
    pub fn focus_match(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = self.step_match(forward, cx) {
            self.open_line(r, window, cx);
        }
    }

    /// The `‹` / `›` bar buttons: selects the next or previous match line
    /// and scrolls it into view inside the tab, wrapping, without opening
    /// it, so the Search tab stays in front (as in Zed). `Enter` or a
    /// click opens the selected line.
    pub fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) -> Option<LineRef> {
        let r = self.results.next_match(self.selected(), forward)?;
        self.results.expand(r.file);
        self.sync_list(cx);
        self.select(r, cx);
        Some(r)
    }

    /// `↑` / `↓` in the list: moves over the lines, skipping headers and
    /// separators and stopping at the ends.
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

    fn list_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.platform || m.control || m.alt || m.shift {
            return;
        }
        match ev.keystroke.key.as_str() {
            "down" => self.move_selection(true, cx),
            "up" => self.move_selection(false, cx),
            "enter" => {
                if let Some(r) = self.selected() {
                    self.open_line(r, window, cx);
                }
            }
            "escape" => self.focus_query(None, window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Applies one event of the current search. For tests, which stand in
    /// for the stream.
    #[doc(hidden)]
    pub fn apply_event(&mut self, event: SearchEvent, cx: &mut Context<Self>) {
        self.apply_events(self.generation, vec![event], cx);
    }

    /// Focuses the query input, first replacing its text with `text`.
    pub fn focus_query(
        &mut self,
        text: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match text {
            Some(text) => self
                .query
                .update(cx, |input, cx| input.set_text(&text, window, cx)),
            // Typing replaces the old query, as in VS Code.
            None => self
                .query
                .update(cx, |input, cx| input.select_all(window, cx)),
        }
        let focus = self.query.focus_handle(cx);
        window.focus(&focus, cx);
    }

    fn toggle(&mut self, flag: fn(&mut Self) -> &mut bool, cx: &mut Context<Self>) {
        let flag = flag(self);
        *flag = !*flag;
        self.inputs_changed(cx);
        cx.notify();
    }

    /// Stops the running search and searches again once the inputs have
    /// been still for [`SEARCH_DEBOUNCE`].
    fn inputs_changed(&mut self, cx: &mut Context<Self>) {
        // The workbench observes this view to follow the query in the tab title.
        cx.notify();
        if self.restore_changes > 0 {
            self.restore_changes -= 1;
            return;
        }
        self.cancel_search();
        self._debounce = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            this.update(cx, |view, cx| view.run_search(cx)).ok();
        });
    }

    fn cancel_search(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.generation += 1;
        self._search = Task::ready(());
    }

    /// Clears the results and starts searching for the current inputs now,
    /// cancelling any search still running (`Enter` in the query input).
    pub fn run_search(&mut self, cx: &mut Context<Self>) {
        if !self.searchable {
            return;
        }
        self._debounce = Task::ready(());
        self.cancel_search();
        self.results.clear();
        self.sync_list(cx);
        self.selected = None;
        self.reset_highlights();
        self.max_line_number = 0;
        self.done = None;
        self.error = None;
        self.pattern_error = false;
        cx.notify();
        let pattern = self.query.read(cx).text().to_owned();
        self.searched = !pattern.is_empty();
        if pattern.is_empty() {
            return;
        }
        let context_lines = cx
            .global::<AppState>()
            .settings
            .get::<u32>(CONTEXT_LINES_SETTING, None)
            .unwrap_or(1)
            .min(MAX_CONTEXT_LINES);
        let query = SearchQuery {
            pattern,
            is_regex: self.is_regex,
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            include: globs(self.include.read(cx).text()),
            exclude: globs(self.exclude.read(cx).text()),
            context_lines,
        };
        self.last_query = Some(query.clone());
        // VS Code: `search.exclude` inherits `files.exclude`.
        let mut settings_exclude = enabled_globs(cx, "search.exclude");
        settings_exclude.extend(enabled_globs(cx, "files.exclude"));
        let (root, cancel, generation) = (self.root.clone(), self.cancel.clone(), self.generation);
        let (sink, events) = crossbeam_channel::unbounded();
        let (forward, mut received) = futures::channel::mpsc::unbounded();
        // `search` blocks: it runs on its own thread while this background
        // task forwards its events to the foreground as they come.
        let job = cx.background_spawn(async move {
            guarded("search", || {
                std::thread::scope(|scope| {
                    let (root, query, exclude) = (&root, &query, &settings_exclude);
                    scope.spawn(move || search(root, query, exclude, sink, cancel));
                    for event in events {
                        if forward.unbounded_send(event).is_err() {
                            break;
                        }
                    }
                });
            });
        });
        self._search = cx.spawn(async move |this, cx| {
            let _job = job;
            while let Some(first) = received.next().await {
                let mut batch = vec![first];
                while let Ok(event) = received.try_recv() {
                    batch.push(event);
                }
                let applied = this.update(cx, |view, cx| view.apply_events(generation, batch, cx));
                if applied.is_err() {
                    break;
                }
            }
        });
    }

    /// Forgets every color and drops the queued and running jobs.
    fn reset_highlights(&mut self) {
        self.highlight_epoch += 1;
        self.highlights.clear();
        self.highlight_pending.clear();
        self.highlight_queue.clear();
        self.highlight_tasks.clear();
    }

    /// Queues the files of the visible rows that have no colors yet and
    /// starts jobs up to [`MAX_HIGHLIGHT_JOBS`].
    fn request_highlights(&mut self, files: Vec<usize>, cx: &mut Context<Self>) {
        let theme = cx.global::<AppState>().theme_generation;
        if theme != self.highlight_theme {
            self.highlight_theme = theme;
            self.reset_highlights();
        }
        let visible: HashSet<Arc<Path>> = files
            .iter()
            .filter_map(|&f| self.results.files().get(f))
            .map(|f| f.path.clone())
            .collect();
        // Files scrolled past must not hold up the ones on screen.
        let pending = &mut self.highlight_pending;
        self.highlight_queue.retain(|path| {
            let keep = visible.contains(path);
            if !keep {
                pending.remove(path);
            }
            keep
        });
        for file in files {
            let Some(result) = self.results.files().get(file) else {
                continue;
            };
            let path = result.path.clone();
            if self.highlights.contains_key(&path) || !self.highlight_pending.insert(path.clone()) {
                continue;
            }
            self.highlight_queue.push_back(path);
        }
        self.start_highlight_jobs(cx);
    }

    fn start_highlight_jobs(&mut self, cx: &mut Context<Self>) {
        while self.highlight_tasks.len() < MAX_HIGHLIGHT_JOBS {
            let Some(path) = self.highlight_queue.pop_front() else {
                break;
            };
            let Some(excerpts) = self
                .results
                .files()
                .iter()
                .find(|f| f.path == path)
                .map(|f| f.excerpts.clone())
            else {
                self.highlight_pending.remove(&path);
                continue;
            };
            let state = cx.global::<AppState>();
            let (languages, theme) = (state.languages.clone(), state.theme.clone());
            let (abs, rel, epoch) = (self.root.join(&path), path.clone(), self.highlight_epoch);
            let job = cx.background_spawn(async move {
                guarded("search highlight", || {
                    highlight_file(&abs, &rel, &excerpts, &languages, &|c| {
                        theme.style_for(c).is_some()
                    })
                })
            });
            let key = path.clone();
            let task = cx.spawn(async move |this, cx| {
                let highlights = job.await.unwrap_or_default();
                this.update(cx, |view, cx| {
                    view.highlights_done(epoch, key, highlights, cx)
                })
                .ok();
            });
            self.highlight_tasks.insert(path, task);
            self.peak_highlight_jobs = self.peak_highlight_jobs.max(self.highlight_tasks.len());
        }
    }

    fn highlights_done(
        &mut self,
        epoch: u64,
        path: Arc<Path>,
        highlights: FileHighlights,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.highlight_epoch {
            return;
        }
        self.highlight_tasks.remove(&path);
        self.highlights.insert(path, highlights);
        self.start_highlight_jobs(cx);
        cx.notify();
    }

    /// Adds a batch of events of search `generation`; stale ones are
    /// dropped. Runs once per batch, never per frame: the batch's
    /// files are added and the rows rebuilt once.
    fn apply_events(&mut self, generation: u64, events: Vec<SearchEvent>, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        let mut batch = Vec::new();
        for event in events {
            match event {
                SearchEvent::File { path, excerpts } => {
                    let path: Arc<Path> = match path.strip_prefix(&self.root) {
                        Ok(relative) => Arc::from(relative),
                        Err(_) => path,
                    };
                    let last = excerpts
                        .iter()
                        .flat_map(|e| e.lines.last())
                        .map(|l| l.line_number)
                        .max()
                        .unwrap_or(0);
                    self.max_line_number = self.max_line_number.max(last);
                    batch.push((path, excerpts));
                }
                SearchEvent::Done { truncated, .. } => self.done = Some(truncated),
                SearchEvent::Error(err) => {
                    // `search` reports an error when the matcher cannot be
                    // built; tell that apart from any other failure by
                    // building the pattern here too.
                    self.pattern_error = self.last_query.as_ref().is_some_and(|q| {
                        build_regex(&q.pattern, q.is_regex, q.case_sensitive, q.whole_word).is_err()
                    });
                    self.error = Some(err);
                }
            }
        }
        self.results.add_files(batch);
        self.sync_list(cx);
        cx.notify();
    }

    /// Opens the file of line `r` as a preview with the cursor on it. A
    /// matching line puts the cursor on its first match and highlights it;
    /// a context line opens at its start with nothing highlighted.
    pub fn open_line(&mut self, r: LineRef, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(file), Some(line), Some(workbench)) = (
            self.results.files().get(r.file),
            self.results.line(r),
            self.workbench.upgrade(),
        ) else {
            return;
        };
        let row = line.line_number.saturating_sub(1);
        // A context line has no ranges; a match cut off at 1,000 chars has
        // only an empty one, and is matched again like any other.
        let query = if line.ranges.is_empty() {
            None
        } else {
            match self.last_query.clone() {
                Some(query) => Some(query),
                None => return,
            }
        };
        let path = file.path.clone();
        // The reported line may be cut at 1,000 chars and was decoded
        // lossily: the column comes from matching again on the buffer's
        // own decoded line once it is loaded (ruling R24).
        let point = Point { row, column: 0 };
        let view = workbench.update(cx, |wb, cx| {
            wb.open_path_at(path, OpenMode::Preview, point, cx);
            wb.editor_groups()
                .update(cx, |groups, cx| groups.ensure_active_body(window, cx))
        });
        let Some(view) = view.and_then(|v| v.downcast::<CodeView>().ok()) else {
            return;
        };
        let reveal = move |view: &mut CodeView, cx: &mut Context<CodeView>| {
            if view.doc_state() != &DocState::Ready || row >= view.snapshot().line_count() {
                return false;
            }
            let range = match &query {
                Some(query) => {
                    let line = view.snapshot().line(row);
                    first_match(query, &line).unwrap_or_else(|| {
                        tracing::debug!(row, "search result no longer matches its line");
                        0..0
                    })
                }
                None => 0..0,
            };
            let at = |column: usize| Point {
                row,
                column: column as u32,
            };
            view.go_to(at(range.start), cx);
            let highlights: Vec<Range<usize>> = if range.is_empty() {
                Vec::new()
            } else {
                let snapshot = view.snapshot();
                let start = snapshot.point_to_offset(at(range.start));
                std::iter::once(start..start + range.len()).collect()
            };
            let active = (!highlights.is_empty()).then_some(0);
            view.set_match_highlights(highlights, active, cx);
            true
        };
        self._highlight = None;
        if !view.update(cx, reveal.clone()) {
            self._highlight = Some(cx.observe(&view, move |this, view, cx| {
                if view.update(cx, reveal.clone()) {
                    this._highlight = None;
                }
            }));
        }
    }

    fn query_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.platform || m.control || m.alt || m.shift {
            return;
        }
        match ev.keystroke.key.as_str() {
            "enter" => self.run_search(cx),
            "escape" => {
                window.focus(&self.list_focus.clone(), cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Fonts and sizes of the result rows: the editor's font, every row one
    /// editor line tall (as `CodeView` computes it), and a gutter as wide as
    /// the largest line number.
    fn row_style(&mut self, window: &Window, cx: &App) -> RowStyle {
        self.scale_factor = window.scale_factor();
        let settings = &cx.global::<AppState>().settings;
        let font_family: SharedString = settings
            .get::<String>("editor.fontFamily", None)
            .unwrap_or_else(|| crate::fonts::EDITOR_FONT_FAMILY.into())
            .into();
        let font_size = px(editor_font_size(cx));
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font(font_family.clone()));
        let char_width = text_system
            .em_advance(font_id, font_size)
            .ok()
            .filter(|w| *w > px(0.))
            .unwrap_or(font_size * 0.6);
        let digits = self.max_line_number.max(1).ilog10() + 1;
        RowStyle {
            font_family,
            font_size,
            line_height: editor_line_height(self.scale_factor, cx),
            gutter: char_width * digits as f32 + px(4. * GRID),
            tab_size: settings
                .get::<u32>("editor.tabSize", None)
                .unwrap_or(4)
                .max(1),
        }
    }

    /// Row `ix` of the results list.
    fn render_row(&mut self, ix: usize, style: &RowStyle, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.results.rows().get(ix).copied() else {
            return div().into_any_element();
        };
        match row {
            Row::Header(file) => self.render_header(file, false, style, cx),
            Row::Line(r) => self.render_line(ix, r, style, cx),
            Row::Separator { file } => {
                // The excerpt it comes before.
                let excerpt = match self.results.rows().get(ix + 1) {
                    Some(Row::Line(r)) => r.excerpt,
                    _ => 0,
                };
                self.render_separator(ix, file, excerpt, style, cx)
            }
        }
    }

    /// A file header, two editor lines tall as in Zed: a card with a 1px
    /// border on `editor.subheader.background` holding the fold chevron,
    /// the file icon, the name, the folder (muted, cut with "…"), the
    /// match count and an "open file" button. Clicking the card folds or
    /// unfolds the file. `sticky` is the copy pinned over the list.
    fn render_header(
        &self,
        index: usize,
        sticky: bool,
        style: &RowStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = cx.global::<AppState>();
        let ui = &state.theme.ui;
        let file = &self.results.files()[index];
        let name = file
            .path
            .file_name()
            .unwrap_or(file.path.as_os_str())
            .to_string_lossy()
            .into_owned();
        let dir = file
            .path
            .parent()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let chevron = if file.collapsed {
            CHEVRON_RIGHT_ICON
        } else {
            CHEVRON_DOWN_ICON
        };
        let icon = state.icons.file(&file.path);
        let path = file.path.clone();
        let hover = color(ui.ghost_element_hover);
        let (card_id, selector, open_id, open_selector): (ElementId, String, ElementId, String) =
            if sticky {
                (
                    "search-sticky-header".into(),
                    "search-sticky-card".into(),
                    "search-sticky-open-file".into(),
                    "search-sticky-open-file".into(),
                )
            } else {
                (
                    ("search-file", index).into(),
                    format!("search-file-{index}"),
                    ("search-open-file", index).into(),
                    format!("search-open-file-{index}"),
                )
            };
        let group: SharedString = format!("{selector}-group").into();
        let selected_here = self.selected().is_some_and(|r| r.file == index);
        let open_path = path.clone();
        let open = div()
            .id(open_id)
            .debug_selector(move || open_selector)
            .flex()
            .flex_none()
            .justify_center()
            .items_center()
            .w(px(6. * GRID))
            .h(px(5. * GRID))
            .rounded(px(GRID))
            .cursor_pointer()
            .when(!selected_here, |d| {
                d.opacity(0.).group_hover(group.clone(), |s| s.opacity(1.))
            })
            .hover(move |s| s.bg(hover))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_click(cx.listener(move |view, _, window, cx| {
                cx.stop_propagation();
                view.open_file_at_first_match(&open_path, window, cx);
            }))
            .child(
                svg()
                    .data(ARROW_UP_RIGHT)
                    .size(px(14.))
                    .text_color(color(ui.icon_muted)),
            );
        let card = div()
            .id(card_id)
            .debug_selector(move || selector)
            .group(group)
            .flex()
            .items_center()
            .gap(px(1.5 * GRID))
            .size_full()
            .pl(px(GRID))
            .pr(px(GRID))
            .rounded_sm()
            .border_1()
            .border_color(color(ui.border))
            .bg(color(ui.editor_subheader_background))
            .hover(move |s| s.bg(hover))
            .when(sticky, |d| d.shadow_md())
            .cursor_pointer()
            .text_size(px(UI_FONT_SIZE_SMALL))
            .on_click(cx.listener(move |view, _, _, cx| {
                if sticky {
                    view.toggle_sticky(&path, cx);
                } else {
                    view.toggle_path(&path, cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_center()
                    .items_center()
                    .w(px(5. * GRID))
                    .child(
                        svg()
                            .data(chevron)
                            .size(px(12.))
                            .text_color(color(ui.icon_muted)),
                    ),
            )
            .child(file_icon_sized(
                &icon,
                px(FILE_ICON_SIZE),
                color(ui.icon_muted).into(),
            ))
            .child(
                div()
                    .flex_none()
                    .max_w(relative(0.6))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(color(ui.text))
                    .child(name),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(color(ui.text_muted))
                    .child(dir),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(color(ui.text_muted))
                    .child(thousands(file.match_count())),
            )
            .child(open);
        div()
            .w_full()
            .h(header_height(style.line_height))
            .p(px(FILE_HEADER_PADDING))
            .child(card)
            .into_any_element()
    }

    fn render_line(
        &self,
        ix: usize,
        r: LineRef,
        style: &RowStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = cx.global::<AppState>();
        let (ui, theme) = (&state.theme.ui, &state.theme);
        let Some(line) = self.results.line(r) else {
            return div().into_any_element();
        };
        let text = &line.line_text;
        let mark = HighlightStyle {
            background_color: Some(color(ui.search_match_background).into()),
            ..Default::default()
        };
        let spans = self
            .results
            .files()
            .get(r.file)
            .and_then(|f| self.highlights.get(&f.path))
            .and_then(|h| h.get(&line.line_number));
        // `CodeView` expands tabs itself; gpui would draw them raw.
        let expanded = expand_tabs(text, style.tab_size);
        let ranges: Vec<Range<usize>> = line
            .ranges
            .iter()
            .map(|r| expanded.map(r.start)..expanded.map(r.end))
            .collect();
        let spans: Option<LineSpans> = spans.map(|s| {
            s.iter()
                .map(|(r, c)| (expanded.map(r.start)..expanded.map(r.end), c.clone()))
                .collect()
        });
        let highlights = line_highlights(&expanded.text, &ranges, spans.as_ref(), theme, mark);
        let styled = StyledText::new(expanded.text.clone()).with_highlights(highlights);
        let selector = format!("search-line-{}-{}-{}", r.file, r.excerpt, r.line);
        let anchor = self.results.anchor_of(r);
        let selected = self.selected() == Some(r);
        // Code lines read like editor lines (as in Zed's multibuffer): no
        // hover; the selected line gets the editor's active-line colors.
        let el = row(ix, selector, style);
        let (el, number) = if selected {
            (
                el.bg(color(ui.editor_active_line_background)),
                color(ui.editor_active_line_number),
            )
        } else {
            (el, color(ui.text_muted))
        };
        el.on_click(cx.listener(move |view, _, window, cx| {
            if let Some(anchor) = &anchor {
                view.open_anchor(anchor, window, cx);
            }
        }))
        .child(gutter(style, number).child(line.line_number.to_string()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .font_family(style.font_family.clone())
                .text_size(style.font_size)
                .text_color(color(ui.editor_foreground))
                .child(styled),
        )
        .into_any_element()
    }

    /// The gap between two excerpts of a file: one editor line with a thin
    /// `border.variant` rule across the code (Zed draws excerpt
    /// boundaries as a line, not a glyph in the gutter).
    fn render_separator(
        &self,
        ix: usize,
        file: usize,
        excerpt: usize,
        style: &RowStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rule = color(cx.global::<AppState>().theme.ui.border_variant);
        let selector = format!("search-separator-{file}-{excerpt}");
        div()
            .id(("search-row", ix))
            .debug_selector(move || selector)
            .flex()
            .items_center()
            .h(style.line_height)
            .w_full()
            .pr(px(2. * GRID))
            .child(div().flex_none().w(style.gutter))
            .child(div().flex_1().h(px(1.)).bg(rule))
            .into_any_element()
    }

    /// The landing block of an empty Search tab, after Zed's "Search All
    /// Files": a title, a hint, and one row per option with its key.
    fn render_empty_state(&self, cx: &mut Context<Self>) -> Div {
        let state = cx.global::<AppState>();
        let ui = state.theme.ui.clone();
        let key = |command: &str, fallback: &str| {
            state
                .keymap
                .label_for(command)
                .unwrap_or_else(|| fallback.into())
        };
        let options: [(
            &'static str,
            &'static [u8],
            &'static str,
            String,
            EmptyAction,
        ); 4] = [
            (
                "search-empty-filters",
                LIST_FILTER,
                "Include/exclude paths",
                key("workbench.action.search.toggleQueryDetails", "⇧⌘J"),
                |v, window, cx| v.toggle_query_details(window, cx),
            ),
            (
                "search-empty-case",
                CASE_SENSITIVE,
                "Match case",
                key("toggleSearchCaseSensitive", "⌥⌘C"),
                |v, _, cx| v.toggle_case_sensitive(cx),
            ),
            (
                "search-empty-word",
                WHOLE_WORD,
                "Match whole words",
                key("toggleSearchWholeWord", "⌥⌘W"),
                |v, _, cx| v.toggle_whole_word(cx),
            ),
            (
                "search-empty-regex",
                REGEX,
                "Match with regex",
                key("toggleSearchRegex", "⌥⌘R"),
                |v, _, cx| v.toggle_regex(cx),
            ),
        ];
        let hover = color(ui.ghost_element_hover);
        let rows = options.map(|(id, icon, label, keys, action)| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex()
                .items_center()
                .gap(px(2. * GRID))
                .h(px(7. * GRID))
                .px(px(2. * GRID))
                .rounded(px(GRID))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(cx.listener(move |view, _, window, cx| action(view, window, cx)))
                .child(
                    svg()
                        .data(icon)
                        .flex_none()
                        .size(px(14.))
                        .text_color(color(ui.icon_muted)),
                )
                .child(div().flex_1().text_color(color(ui.text)).child(label))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(UI_FONT_SIZE_SMALL))
                        .text_color(color(ui.text_muted))
                        .child(keys),
                )
        });
        div()
            .debug_selector(|| "search-empty-state".into())
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(GRID))
                    .w(px(80. * GRID))
                    .max_w_full()
                    .text_size(px(UI_FONT_SIZE))
                    .child(
                        div()
                            .px(px(2. * GRID))
                            .text_size(px(UI_FONT_SIZE + 2.))
                            .text_color(color(ui.text))
                            .child("Search all files"),
                    )
                    .child(
                        div()
                            .px(px(2. * GRID))
                            .pb(px(2. * GRID))
                            .text_color(color(ui.text_muted))
                            .child("Start typing to search. For more options:"),
                    )
                    .children(rows),
            )
    }
}

/// What a row of the empty state does.
type EmptyAction = fn(&mut SearchView, &mut Window, &mut Context<SearchView>);

/// The editor font size in pixels, from `editor.fontSize`.
fn editor_font_size(cx: &App) -> f32 {
    cx.global::<AppState>()
        .settings
        .get::<f32>("editor.fontSize", None)
        .unwrap_or(14.)
        .clamp(6., 40.)
}

/// One editor line, as `CodeView` computes it from `editor.lineHeight`
/// (a factor of the font size below 8, else pixels), rounded to the
/// device pixels of `scale_factor`: layout snaps row heights to that grid,
/// and scrolling to a row sums these heights, so they must match what the
/// list measures.
fn editor_line_height(scale_factor: f32, cx: &App) -> Pixels {
    let size = editor_font_size(cx);
    let lh = cx
        .global::<AppState>()
        .settings
        .get::<f32>("editor.lineHeight", None)
        .unwrap_or(1.6);
    let lh = if lh < 8. { lh * size } else { lh };
    let scale = if scale_factor > 0. { scale_factor } else { 1. };
    px((lh.max(1.) * scale).round().max(1.) / scale)
}

/// A file header's height: [`FILE_HEADER_LINES`] editor lines.
fn header_height(lh: Pixels) -> Pixels {
    lh * FILE_HEADER_LINES
}

/// The drawn height of `row`: every row but a header is one line.
fn row_height(row: &Row, lh: Pixels) -> Pixels {
    match row {
        Row::Header(_) => header_height(lh),
        Row::Line(_) | Row::Separator { .. } => lh,
    }
}

/// The runs of one result line: syntax colors from `spans` with `mark`
/// laid over the match `ranges`. Runs never overlap; where a match and a
/// syntax span meet, the run takes the syntax style plus the match
/// background. Ranges are clamped and anything off a char boundary or
/// empty (a match past the 1,000-char cut) is dropped.
fn line_highlights(
    text: &str,
    ranges: &[Range<usize>],
    spans: Option<&LineSpans>,
    theme: &lucerna_theme::Theme,
    mark: HighlightStyle,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let valid = |r: &Range<usize>| {
        r.start < r.end && text.is_char_boundary(r.start) && text.is_char_boundary(r.end)
    };
    let matches: Vec<Range<usize>> = ranges
        .iter()
        .map(|r| r.start..r.end.min(text.len()))
        .filter(valid)
        .collect();
    let syntax: Vec<(Range<usize>, &lucerna_theme::HighlightStyle)> = spans
        .into_iter()
        .flatten()
        .filter(|(r, _)| valid(r) && r.end <= text.len())
        .filter_map(|(r, capture)| Some((r.clone(), theme.style_for(capture)?)))
        .collect();
    let mut edges: Vec<usize> = matches
        .iter()
        .map(|r| r.start)
        .chain(matches.iter().map(|r| r.end))
        .chain(syntax.iter().flat_map(|(r, _)| [r.start, r.end]))
        .collect();
    edges.sort_unstable();
    edges.dedup();
    edges
        .windows(2)
        .filter_map(|w| {
            let seg = w[0]..w[1];
            // The narrowest span covering the segment wins; the first on a tie.
            let style = syntax
                .iter()
                .filter(|(r, _)| r.start <= seg.start && seg.end <= r.end)
                .min_by_key(|(r, _)| r.len())
                .map(|(_, s)| *s);
            let marked = matches
                .iter()
                .any(|r| r.start <= seg.start && seg.end <= r.end);
            if style.is_none() && !marked {
                return None;
            }
            let mut run = if marked {
                mark
            } else {
                HighlightStyle::default()
            };
            if let Some(s) = style {
                run.color = s.color.map(|c| color(c).into());
                if s.bold {
                    run.font_weight = Some(FontWeight::BOLD);
                }
                if s.italic {
                    run.font_style = Some(FontStyle::Italic);
                }
            }
            Some((seg, run))
        })
        .collect()
}

impl Drop for SearchView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// A clickable results row, one editor line tall; add `on_click` and the
/// content.
fn row(ix: usize, selector: String, style: &RowStyle) -> Stateful<Div> {
    div()
        .id(("search-row", ix))
        .debug_selector(move || selector)
        .flex()
        .items_center()
        .gap(px(1.5 * GRID))
        .h(style.line_height)
        .w_full()
        .pr(px(2. * GRID))
        .cursor_pointer()
}

/// The line-number column: right-aligned, in the editor font.
fn gutter(style: &RowStyle, muted: gpui::Rgba) -> Div {
    div()
        .flex()
        .flex_none()
        .justify_end()
        .w(style.gutter)
        .pr(px(2. * GRID))
        .font_family(style.font_family.clone())
        .text_size(style.font_size)
        .text_color(muted)
}

impl Focusable for SearchView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SearchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<AppState>().theme.ui.clone();
        let muted = color(ui.text_muted);
        if !self.searchable {
            return div()
                .id("search-view")
                .debug_selector(|| "search-view".into())
                .track_focus(&self.focus)
                .size_full()
                .bg(color(ui.editor_background))
                .child(
                    div()
                        .debug_selector(|| "search-summary".into())
                        .px(px(3. * GRID))
                        .py(px(2. * GRID))
                        .text_size(px(UI_FONT_SIZE_SMALL))
                        .text_color(muted)
                        .child(UNAVAILABLE),
                );
        }
        let toggle = |id, text, on, flag: fn(&mut Self) -> &mut bool, cx: &mut Context<Self>| {
            toggle_button(id, text, on, cx)
                .on_click(cx.listener(move |view, _, _, cx| view.toggle(flag, cx)))
        };
        let invalid = self.is_invalid();
        let query = search_query_box(&self.query, invalid, cx)
            .debug_selector(move || {
                if invalid {
                    "search-query-invalid".into()
                } else {
                    "search-query".into()
                }
            })
            .on_key_down(cx.listener(Self::query_key_down))
            .child(toggle(
                "search-toggle-case",
                "Aa",
                self.case_sensitive,
                |v| &mut v.case_sensitive,
                cx,
            ))
            .child(toggle(
                "search-toggle-word",
                "ab",
                self.whole_word,
                |v| &mut v.whole_word,
                cx,
            ))
            .child(toggle(
                "search-toggle-regex",
                ".*",
                self.is_regex,
                |v| &mut v.is_regex,
                cx,
            ));
        let all_collapsed =
            !self.results.files().is_empty() && self.results.files().iter().all(|f| f.collapsed);
        let collapse_icon = if all_collapsed {
            CHEVRONS_UP_DOWN
        } else {
            CHEVRONS_DOWN_UP
        };
        let this = cx.weak_entity();
        let step = move |forward: bool| -> ClickHandler {
            let this = this.clone();
            Box::new(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                this.update(cx, |view, cx| view.step_match(forward, cx))
                    .ok();
            })
        };
        let bar = search_bar_row()
            .child(
                bar_icon_button("search-collapse-all", collapse_icon, false, cx)
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_all_collapsed(cx))),
            )
            .child(query)
            .child(
                bar_icon_button("search-filters", LIST_FILTER, self.filters_shown, cx).on_click(
                    cx.listener(|view, _, window, cx| view.toggle_query_details(window, cx)),
                ),
            )
            .child(
                bar_icon_button("search-refresh", REFRESH_CW, false, cx)
                    .on_click(cx.listener(|view, _, _, cx| view.run_search(cx))),
            )
            .child(bar_separator(cx))
            .child(match_nav(
                ("search-previous", step(false)),
                ("search-next", step(true)),
                self.match_label(),
                cx,
            ));
        // The filter inputs line up under the query box, as in Zed.
        let filters = self.filters_shown.then(|| {
            search_bar_row()
                .debug_selector(|| "search-filter-row".into())
                .child(div().flex_none().w(px(6. * GRID)))
                .child(
                    search_query_column(div().flex().gap(px(GRID)))
                        .child(
                            input_box(&self.include, cx)
                                .debug_selector(|| "search-include".into())
                                .flex_1()
                                .min_w_0(),
                        )
                        .child(input_box(&self.exclude, cx).flex_1().min_w_0()),
                )
        });
        let toolbar = search_toolbar(cx)
            .debug_selector(|| "search-toolbar".into())
            .child(bar)
            .children(filters);
        // Done with results: the counter says it all. Searching, nothing
        // found, cut short or failed: say so under the bar.
        let finished_with_results = self.done == Some(false) && self.results.match_count() > 0;
        let status = match (&self.error, self.summary()) {
            (Some(error), _) => Some(("search-error", error.clone(), color(ui.error))),
            (None, Some(summary)) if !finished_with_results => {
                Some(("search-summary", summary, muted))
            }
            _ => None,
        };
        let status = status.map(|(selector, text, text_color)| {
            div()
                .debug_selector(move || selector.into())
                .flex_none()
                .px(px(3. * GRID))
                .py(px(GRID))
                .text_size(px(UI_FONT_SIZE_SMALL))
                .text_color(text_color)
                .child(text)
        });
        let style = self.row_style(window, cx);
        let lh = style.line_height;
        // The rows on screen get their syntax colors first.
        let viewport = match self.list.viewport_bounds().size.height {
            h if h > px(0.) => h,
            _ => window.viewport_size().height,
        };
        let mut visible: Vec<usize> = Vec::new();
        for ix in self.visible_rows(viewport, lh) {
            let file = self.results.rows()[ix].file();
            if visible.last() != Some(&file) {
                visible.push(file);
            }
        }
        self.request_highlights(visible, cx);
        let empty = self.results.rows().is_empty() && self.query.read(cx).text().is_empty();
        let body = if empty {
            self.render_empty_state(cx).into_any_element()
        } else {
            // The pinned header: a clip at the list's top, as tall as the
            // part still shown while the next header pushes it up.
            let sticky = self.sticky_header(lh).map(|(file, push)| {
                div()
                    .debug_selector(|| "search-sticky-header".into())
                    .absolute()
                    .left_0()
                    .right_0()
                    .top_0()
                    .h(header_height(lh) + push)
                    .overflow_hidden()
                    // Clicks on the pinned header never reach the rows under it.
                    .occlude()
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(push)
                            .bg(color(ui.editor_background))
                            .child(self.render_header(file, true, &style, cx)),
                    )
            });
            let row_style = style.clone();
            div()
                .relative()
                .size_full()
                // Nothing in here (the pushed-up sticky header) draws over
                // the toolbar.
                .overflow_hidden()
                .child(
                    list(
                        self.list.clone(),
                        cx.processor(move |view, ix: usize, _, cx| {
                            view.render_row(ix, &row_style, cx)
                        }),
                    )
                    .size_full(),
                )
                .children(sticky)
                .into_any_element()
        };
        div()
            .id("search-view")
            .debug_selector(|| "search-view".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(color(ui.editor_background))
            .child(toolbar)
            .children(status)
            .child(
                div()
                    .id("search-list")
                    .debug_selector(|| "search-list".into())
                    .track_focus(&self.list_focus)
                    .on_key_down(cx.listener(Self::list_key_down))
                    .flex_1()
                    .min_h_0()
                    .child(body),
            )
    }
}

/// Opens (or focuses) the Search tab and puts the cursor in its query,
/// filled with the editor's selection when it is on one line.
fn find_in_files(wb: &mut Workbench, window: &mut Window, cx: &mut Context<Workbench>) {
    let selection = wb
        .active_code_view(cx)
        .and_then(|view| single_line_selection(view.read(cx)));
    wb.open_search(window, cx);
    let view = wb.search_view().clone();
    view.update(cx, |view, cx| view.focus_query(selection, window, cx));
}

/// Opens the next / previous match. Deferred: opening a file updates the
/// workbench, which a command handler already holds.
fn focus_match(
    wb: &mut Workbench,
    forward: bool,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let view = wb.search_view().clone();
    window.defer(cx, move |window, cx| {
        view.update(cx, |view, cx| view.focus_match(forward, window, cx))
    });
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.findInFiles",
        "Search: Find in Files",
        |wb, _, window, cx| find_in_files(wb, window, cx),
    );
    // Command handlers are plain `fn`s: a macro stamps one per command.
    macro_rules! on_view {
        ($id:expr, $title:expr, |$view:ident, $window:ident, $cx:ident| $body:expr) => {
            commands.register($id, $title, |wb, _, $window, cx| {
                wb.search_view().clone().update(cx, |$view, $cx| $body)
            });
        };
    }
    on_view!(
        "toggleSearchCaseSensitive",
        "Search: Toggle Match Case",
        |v, _w, cx| v.toggle_case_sensitive(cx)
    );
    on_view!(
        "toggleSearchWholeWord",
        "Search: Toggle Match Whole Word",
        |v, _w, cx| v.toggle_whole_word(cx)
    );
    on_view!(
        "toggleSearchRegex",
        "Search: Toggle Use Regular Expression",
        |v, _w, cx| v.toggle_regex(cx)
    );
    on_view!(
        "workbench.action.search.toggleQueryDetails",
        "Search: Toggle Search Details",
        |v, window, cx| v.toggle_query_details(window, cx)
    );
    commands.register(
        "search.action.focusNextSearchResult",
        "Search: Focus Next Search Result",
        |wb, _, window, cx| focus_match(wb, true, window, cx),
    );
    commands.register(
        "search.action.focusPreviousSearchResult",
        "Search: Focus Previous Search Result",
        |wb, _, window, cx| focus_match(wb, false, window, cx),
    );
    on_view!(
        "search.action.refreshSearchResults",
        "Search: Refresh",
        |v, _w, cx| v.run_search(cx)
    );
    on_view!(
        "search.action.collapseSearchResults",
        "Search: Collapse All",
        |v, _w, cx| v.set_all_collapsed(true, cx)
    );
    commands.register(
        "workbench.view.search",
        "View: Show Search",
        |wb, _, window, cx| find_in_files(wb, window, cx),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_theme::ThemeRegistry;

    #[test]
    fn match_background_wins_over_syntax_and_runs_do_not_overlap() {
        let theme = ThemeRegistry::builtin().get_or_default("One Dark");
        let mark = HighlightStyle {
            background_color: Some(gpui::red()),
            ..Default::default()
        };
        // "fn main": keyword over 0..2, function over 3..7, match on 1..5.
        let spans: LineSpans = vec![(0..2, "keyword".into()), (3..7, "function".into())];
        let ranges = [1..3, 3..5];
        let runs = line_highlights("fn main", &ranges, Some(&spans), &theme, mark);
        assert!(runs.windows(2).all(|w| w[0].0.end <= w[1].0.start));
        let covered: Vec<_> = runs
            .iter()
            .filter(|(r, _)| r.start >= 1 && r.end <= 5)
            .collect();
        assert!(!covered.is_empty());
        assert!(covered.iter().all(|(_, s)| s.background_color.is_some()));
        assert!(
            runs.iter()
                .filter(|(r, _)| r.end <= 1 || r.start >= 5)
                .all(|(_, s)| s.background_color.is_none())
        );
        assert!(
            runs.iter().any(|(_, s)| s.color.is_some()),
            "syntax color kept"
        );
    }

    #[test]
    fn bad_ranges_are_dropped() {
        let theme = ThemeRegistry::builtin().get_or_default("One Dark");
        let spans: LineSpans = vec![(0..1, "keyword".into()), (0..99, "keyword".into())];
        let runs = line_highlights(
            "é",
            &[1..2, 5..9, 0..0],
            Some(&spans),
            &theme,
            Default::default(),
        );
        assert!(runs.is_empty(), "{runs:?}");
    }
}
