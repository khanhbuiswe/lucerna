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
    AnyElement, App, Context, Div, Entity, FocusHandle, Focusable, FontStyle, FontWeight,
    HighlightStyle, IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Render,
    ScrollStrategy, SharedString, Stateful, Styled, StyledText, Subscription, Task,
    UniformListScrollHandle, WeakEntity, Window, div, font, prelude::*, px, svg, uniform_list,
};
use lucerna_project::{SearchEvent, SearchQuery, search};
use lucerna_text::Point;

use super::highlight::{FileHighlights, LineSpans, highlight_file};
use super::model::{LineAnchor, LineRef, Results, Row, expand_tabs, first_match, globs, summary};
use crate::code_view::CodeView;
use crate::find_widget::single_line_selection;
use crate::icons::{CHEVRONS_DOWN_UP, LIST_FILTER, REFRESH_CW};
use crate::layout::color;
use crate::tasks::guarded;
use crate::text_input::TextInput;
use crate::ui::{
    CHEVRON_DOWN_ICON, CHEVRON_RIGHT_ICON, FILE_ICON_SIZE, GRID, TAB_HEIGHT, UI_FONT_SIZE_SMALL,
    file_icon_sized, icon_button, toggle_button,
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
    /// The query the shown results came from.
    last_query: Option<SearchQuery>,
    /// Bumped by every change; events of older searches are dropped.
    generation: u64,
    cancel: Arc<AtomicBool>,
    _debounce: Task<()>,
    _search: Task<()>,
    /// Highlights a clicked match once its file has loaded.
    _highlight: Option<Subscription>,
    scroll: UniformListScrollHandle,
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
            last_query: None,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            _debounce: Task::ready(()),
            _search: Task::ready(()),
            _highlight: None,
            scroll: UniformListScrollHandle::new(),
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
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        cx.notify();
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
        let Some(r) = self.results.next_match(self.selected(), forward) else {
            return;
        };
        self.results.expand(r.file);
        self.select(r, cx);
        self.open_line(r, window, cx);
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
        self.selected = None;
        self.reset_highlights();
        self.max_line_number = 0;
        self.done = None;
        self.error = None;
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
                SearchEvent::Error(err) => self.error = Some(err),
            }
        }
        self.results.add_files(batch);
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
    fn row_style(&self, window: &Window, cx: &App) -> RowStyle {
        let settings = &cx.global::<AppState>().settings;
        let font_family: SharedString = settings
            .get::<String>("editor.fontFamily", None)
            .unwrap_or_else(|| crate::fonts::EDITOR_FONT_FAMILY.into())
            .into();
        let size = settings
            .get::<f32>("editor.fontSize", None)
            .unwrap_or(14.)
            .clamp(6., 40.);
        let lh = settings
            .get::<f32>("editor.lineHeight", None)
            .unwrap_or(1.6);
        let line_height = if lh < 8. { lh * size } else { lh }.max(1.);
        let font_size = px(size);
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
            line_height: px(line_height),
            gutter: char_width * digits as f32 + px(4. * GRID),
            tab_size: settings
                .get::<u32>("editor.tabSize", None)
                .unwrap_or(4)
                .max(1),
        }
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        style: &RowStyle,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut visible: Vec<usize> = Vec::new();
        for row in self.results.rows().get(range.clone()).unwrap_or_default() {
            let file = match row {
                Row::Header(file) | Row::Separator { file } => *file,
                Row::Line(r) => r.file,
            };
            if visible.last() != Some(&file) {
                visible.push(file);
            }
        }
        self.request_highlights(visible, cx);
        range
            .filter_map(|ix| {
                let row = *self.results.rows().get(ix)?;
                Some(match row {
                    Row::Header(file) => self.render_header(ix, file, style, cx),
                    Row::Line(r) => self.render_line(ix, r, style, cx),
                    Row::Separator { file } => {
                        // The excerpt it comes before.
                        let excerpt = match self.results.rows().get(ix + 1) {
                            Some(Row::Line(r)) => r.excerpt,
                            _ => 0,
                        };
                        self.render_separator(ix, file, excerpt, style, cx)
                    }
                })
            })
            .collect()
    }

    fn render_header(
        &self,
        ix: usize,
        index: usize,
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
        row(ix, format!("search-file-{index}"), style, cx)
            .bg(color(ui.element_background))
            .px(px(2. * GRID))
            .on_click(cx.listener(move |view, _, _, cx| view.toggle_path(&path, cx)))
            .child(
                svg()
                    .data(chevron)
                    .flex_none()
                    .size(px(12.))
                    .text_color(color(ui.icon_muted)),
            )
            .child(file_icon_sized(
                &icon,
                px(FILE_ICON_SIZE),
                color(ui.icon_muted).into(),
            ))
            .child(
                div()
                    .min_w_0()
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
                    .child(file.match_count().to_string()),
            )
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
        let el = row(ix, selector, style, cx);
        let el = if selected {
            el.bg(color(ui.element_selected))
        } else {
            el
        };
        el.on_click(cx.listener(move |view, _, window, cx| {
            if let Some(anchor) = &anchor {
                view.open_anchor(anchor, window, cx);
            }
        }))
        .child(gutter(style, color(ui.text_muted)).child(line.line_number.to_string()))
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

    fn render_separator(
        &self,
        ix: usize,
        file: usize,
        excerpt: usize,
        style: &RowStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = color(cx.global::<AppState>().theme.ui.text_muted);
        let selector = format!("search-separator-{file}-{excerpt}");
        div()
            .id(("search-row", ix))
            .debug_selector(move || selector)
            .flex()
            .items_center()
            .h(style.line_height)
            .w_full()
            .child(gutter(style, muted).child("┈"))
            .into_any_element()
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

/// A clickable results row, one editor line tall, with hover; add
/// `on_click` and the content.
fn row(ix: usize, selector: String, style: &RowStyle, cx: &App) -> Stateful<Div> {
    let hover = color(cx.global::<AppState>().theme.ui.ghost_element_hover);
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
        .hover(move |s| s.bg(hover))
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

/// A bordered box holding `input`; add any controls after it.
fn input_box(input: &Entity<TextInput>, cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .flex()
        .items_center()
        .gap(px(GRID / 2.))
        .h(px(7. * GRID))
        .pl(px(2. * GRID))
        .pr(px(GRID))
        .rounded(px(GRID))
        .border_1()
        .border_color(color(ui.border_variant))
        .bg(color(ui.editor_background))
        .child(div().flex_1().min_w_0().child(input.clone()))
}

/// An [`icon_button`] that leaves focus where it was.
fn bar_button(id: &'static str, icon: &'static [u8], active: bool, cx: &App) -> Stateful<Div> {
    icon_button(id, icon, active, cx)
        .debug_selector(move || id.into())
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
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
        let query = input_box(&self.query, cx)
            .flex_1()
            .min_w_0()
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
        let bar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(GRID))
            .h(px(TAB_HEIGHT))
            .px(px(2. * GRID))
            .child(query)
            .child(
                div()
                    .flex_none()
                    .w(px(1.))
                    .h(px(5. * GRID))
                    .bg(color(ui.border_variant)),
            )
            .child(
                bar_button("search-filters", LIST_FILTER, self.filters_shown, cx).on_click(
                    cx.listener(|view, _, window, cx| view.toggle_query_details(window, cx)),
                ),
            )
            .child(
                bar_button("search-refresh", REFRESH_CW, false, cx)
                    .on_click(cx.listener(|view, _, _, cx| view.run_search(cx))),
            )
            .child(
                bar_button("search-collapse-all", CHEVRONS_DOWN_UP, false, cx)
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_all_collapsed(cx))),
            );
        let filters = self.filters_shown.then(|| {
            div()
                .debug_selector(|| "search-filter-row".into())
                .flex()
                .flex_none()
                .gap(px(GRID))
                .px(px(2. * GRID))
                .pb(px(GRID))
                .child(
                    input_box(&self.include, cx)
                        .debug_selector(|| "search-include".into())
                        .flex_1()
                        .min_w_0(),
                )
                .child(input_box(&self.exclude, cx).flex_1().min_w_0())
        });
        let status = match (&self.error, self.summary()) {
            (Some(error), _) => Some(("search-error", error.clone())),
            (None, Some(summary)) => Some(("search-summary", summary)),
            (None, None) => None,
        };
        let status = status.map(|(selector, text)| {
            div()
                .debug_selector(move || selector.into())
                .flex_none()
                .px(px(3. * GRID))
                .pb(px(GRID))
                .text_size(px(UI_FONT_SIZE_SMALL))
                .text_color(muted)
                .child(text)
        });
        let style = self.row_style(window, cx);
        div()
            .id("search-view")
            .debug_selector(|| "search-view".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(color(ui.editor_background))
            .child(bar)
            .children(filters)
            .children(status)
            .child(
                div()
                    .id("search-list")
                    .track_focus(&self.list_focus)
                    .on_key_down(cx.listener(Self::list_key_down))
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list(
                            "search-results",
                            self.results.rows().len(),
                            cx.processor(move |view, range, _, cx| {
                                view.render_rows(range, &style, cx)
                            }),
                        )
                        .size_full()
                        .track_scroll(&self.scroll),
                    ),
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
