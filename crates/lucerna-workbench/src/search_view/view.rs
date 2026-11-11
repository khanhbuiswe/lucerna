//! The Search tab's body (`workbench.action.findInFiles`, `⇧⌘F`): a query
//! with case / word / regex toggles, optional include and exclude globs,
//! and the results as excerpts grouped by file as they stream in.
//!
//! Every change waits [`SEARCH_DEBOUNCE`] before searching and cancels the
//! running search through its `AtomicBool`. Each search gets a fresh
//! channel and a generation; events of a superseded search are dropped
//! (ruling R17).

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt;
use gpui::{
    App, ClickEvent, Context, Div, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    MouseButton, ParentElement, Render, SharedString, Styled, Subscription, Task, WeakEntity,
    Window, div, prelude::*, px, svg,
};
use lucerna_project::{SearchEvent, SearchQuery, search};
use lucerna_text::Point;

use super::model::{first_match, globs, summary};
use crate::code_view::CodeView;
use crate::excerpt_list::highlight::FileHighlights;
use crate::excerpt_list::model::{LineRef, Results, thousands};
use crate::excerpt_list::{ExcerptList, ExcerptListEvent};
use crate::find_widget::{build_regex, single_line_selection};
use crate::icons::{
    CASE_SENSITIVE, CHEVRONS_DOWN_UP, CHEVRONS_UP_DOWN, LIST_FILTER, REFRESH_CW, REGEX, WHOLE_WORD,
};
use crate::layout::color;
use crate::tasks::guarded;
use crate::text_input::TextInput;
use crate::ui::{
    ClickHandler, GRID, UI_FONT_SIZE, UI_FONT_SIZE_SMALL, bar_icon_button, bar_separator,
    input_box, match_nav, search_bar_row, search_query_box, search_query_column, search_toolbar,
    toggle_button,
};
use crate::workbench::enabled_globs;
use crate::workspace_state::SearchState;
use crate::{AppState, CommandRegistry, DocState, OpenMode, Workbench};

/// How long the inputs must stay unchanged before a search starts.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);

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
    /// The results: files with excerpts, the selection and their colors.
    list: Entity<ExcerptList>,
    /// Focus of the results list (`list`'s handle): arrows, Enter and Esc
    /// work here.
    list_focus: FocusHandle,
    query: Entity<TextInput>,
    include: Entity<TextInput>,
    exclude: Entity<TextInput>,
    case_sensitive: bool,
    whole_word: bool,
    is_regex: bool,
    /// The include / exclude row is shown.
    filters_shown: bool,
    /// A search for a non-empty query was started.
    searched: bool,
    /// `Some(truncated)` once the search finished.
    done: Option<bool>,
    error: Option<String>,
    /// `error` is the query's own fault (a bad pattern), not an I/O or
    /// root failure: only then does the query box turn red.
    pattern_error: bool,
    /// The query box shows the error color. Kept from the last finished
    /// search while the next one runs (Zed's `previous_completion`), so
    /// the border never flickers.
    invalid: bool,
    /// The query the shown results came from.
    last_query: Option<SearchQuery>,
    /// The inputs changed since the last search started (a debounced
    /// search is still waiting): `Enter` searches instead of stepping.
    stale: bool,
    /// Bumped by every change; events of older searches are dropped.
    generation: u64,
    cancel: Arc<AtomicBool>,
    _debounce: Task<()>,
    _search: Task<()>,
    /// Highlights a clicked match once its file has loaded.
    _highlight: Option<Subscription>,
    /// Opens the lines the list activates.
    _list_events: Subscription,
    /// `false` in single-file mode (R29): the folder is never walked.
    searchable: bool,
    /// Input changes still to come from [`SearchView::restore`] setting the
    /// inputs, which must not start a search.
    restore_changes: usize,
}

/// The most query characters the tab title shows.
const TITLE_MAX_CHARS: usize = 30;

impl SearchView {
    pub fn new(
        root: PathBuf,
        workbench: WeakEntity<Workbench>,
        searchable: bool,
        window: &mut Window,
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
        let list = {
            let root = root.clone();
            cx.new(|cx| ExcerptList::new(root, cx))
        };
        let list_focus = list.read(cx).focus_handle().clone();
        let list_events =
            cx.subscribe_in(&list, window, |view, _, event, window, cx| match event {
                ExcerptListEvent::Activate(r) => view.open_line(*r, window, cx),
            });
        SearchView {
            workbench,
            root,
            focus: cx.focus_handle(),
            list,
            list_focus,
            query: input("Search", cx),
            include: input("e.g. src/**, *.rs", cx),
            exclude: input("e.g. target, *.min.js", cx),
            case_sensitive: false,
            whole_word: false,
            is_regex: false,
            filters_shown: false,
            searched: false,
            done: None,
            error: None,
            pattern_error: false,
            invalid: false,
            last_query: None,
            stale: false,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            _debounce: Task::ready(()),
            _search: Task::ready(()),
            _highlight: None,
            _list_events: list_events,
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
    pub fn selected(&self, cx: &App) -> Option<LineRef> {
        self.list.read(cx).selected()
    }

    /// The files with results, sorted by path, and the rows shown.
    pub fn results<'a>(&self, cx: &'a App) -> &'a Results {
        self.list.read(cx).results()
    }

    /// The cached syntax colors of the result file `path` (relative to the
    /// root), by line number, once computed.
    pub fn highlights(&self, path: &Path, cx: &App) -> Option<FileHighlights> {
        self.list.read(cx).highlights(path)
    }

    /// Highlight jobs running now; never above
    /// [`MAX_HIGHLIGHT_JOBS`](crate::excerpt_list::highlight::MAX_HIGHLIGHT_JOBS).
    pub fn highlight_jobs_in_flight(&self, cx: &App) -> usize {
        self.list.read(cx).highlight_jobs_in_flight()
    }

    /// The most highlight jobs that ran at once since this view was made.
    pub fn peak_highlight_jobs(&self, cx: &App) -> usize {
        self.list.read(cx).peak_highlight_jobs()
    }

    /// The digits the line-number gutter is sized for: at least
    /// [`MIN_GUTTER_DIGITS`](crate::excerpt_list::MIN_GUTTER_DIGITS),
    /// growing with the largest line number shown.
    pub fn gutter_digits(&self, cx: &App) -> u32 {
        self.list.read(cx).gutter_digits()
    }

    /// Highlight jobs started since this view was made.
    pub fn highlight_jobs_started(&self, cx: &App) -> usize {
        self.list.read(cx).highlight_jobs_started()
    }

    /// The regex error of the last search, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// `"N results in M files"` (see [`summary`]); `None` before any search
    /// and while an error shows. In single-file mode it is the
    /// "unavailable" message instead.
    pub fn summary(&self, cx: &App) -> Option<String> {
        if !self.searchable {
            return Some(UNAVAILABLE.into());
        }
        if self.error.is_some() {
            return None;
        }
        let results = self.results(cx);
        summary(
            results.match_count(),
            results.files().len(),
            self.searched,
            self.done,
        )
    }

    /// Folds or unfolds the excerpts of file `file`.
    pub fn toggle_collapsed(&mut self, file: usize, cx: &mut Context<Self>) {
        self.list
            .update(cx, |list, cx| list.toggle_collapsed(file, cx));
    }

    /// Folds every file, or unfolds them all when all are folded.
    fn toggle_all_collapsed(&mut self, cx: &mut Context<Self>) {
        let all = self.results(cx).files().iter().all(|f| f.collapsed);
        self.set_all_collapsed(!all, cx);
    }

    /// `search.action.collapseSearchResults` and the button's two states.
    fn set_all_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        self.list
            .update(cx, |list, cx| list.set_all_collapsed(collapsed, cx));
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

    /// The search status shown by the bar's counter, as in Zed:
    /// `"Searching…"` while a search runs (the rows may still hold the
    /// previous search's files), then `"3/19"` for the selected match line
    /// among all match lines, `"19 results"` with no match selected, `"0/0"`
    /// without matches. A search cut short says
    /// `"20,000+"` (the cap is [`lucerna_project::MAX_RESULTS`]).
    pub fn match_label(&self, cx: &App) -> String {
        if self.searched && self.done.is_none() && self.error.is_none() {
            return "Searching…".into();
        }
        let list = self.list.read(cx);
        let results = list.results();
        let total = results.match_count();
        if total == 0 {
            return "0/0".into();
        }
        let plus = if self.done == Some(true) { "+" } else { "" };
        match list.selected().and_then(|r| results.match_index(r)) {
            Some(i) => format!("{}/{}{plus}", thousands(i + 1), thousands(total)),
            None if total == 1 => "1 result".into(),
            None => format!("{}{plus} results", thousands(total)),
        }
    }

    /// The query box shows the error color: a bad pattern, or a finished
    /// search without matches. Other errors (I/O, the root) leave it alone.
    /// While a search runs this is still the last finished search's state.
    pub fn is_invalid(&self) -> bool {
        self.invalid
    }

    /// [`is_invalid`](Self::is_invalid) from the current state; taken when
    /// a search finishes or reports an error.
    fn compute_invalid(&self, cx: &App) -> bool {
        (self.error.is_some() && self.pattern_error)
            || (self.error.is_none()
                && self.searched
                && self.done.is_some()
                && self.results(cx).match_count() == 0)
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
        self.list
            .update(cx, |list, cx| list.step_match(forward, cx))
    }

    /// Keys in the results list: `Esc` goes back to the query, the list
    /// handles the rest (`↑` / `↓`, `Enter`).
    fn list_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.platform || m.control || m.alt || m.shift {
            return;
        }
        if ev.keystroke.key == "escape" {
            self.focus_query(None, window, cx);
        } else if !self.list.update(cx, |list, cx| list.key_down(ev, cx)) {
            return;
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
        self.stale = true;
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

    /// Starts searching for the current inputs now, cancelling any search
    /// still running (`Enter` in the query input). As in Zed, the shown
    /// files stay: each file the new search reports replaces its old
    /// excerpts in place, and the files it does not report go when it
    /// finishes, so rows never blank or jump between searches. An empty
    /// query clears the results at once.
    pub fn run_search(&mut self, cx: &mut Context<Self>) {
        if !self.searchable {
            return;
        }
        self._debounce = Task::ready(());
        self.stale = false;
        self.cancel_search();
        self.done = None;
        self.error = None;
        self.pattern_error = false;
        cx.notify();
        let pattern = self.query.read(cx).text().to_owned();
        self.searched = !pattern.is_empty();
        if pattern.is_empty() {
            self.list.update(cx, |list, cx| list.clear(cx));
            self.invalid = false;
            return;
        }
        self.list.update(cx, |list, cx| {
            list.update_results(Results::begin_update, cx)
        });
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

    /// Applies a batch of events of search `generation`; stale ones are
    /// dropped. Runs once per batch, never per frame: the batch's files
    /// replace or join the shown ones, `Done` drops the files the search
    /// did not report, and the list is told once. The selection stays
    /// while its line is still there.
    fn apply_events(&mut self, generation: u64, events: Vec<SearchEvent>, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        let mut batch = Vec::new();
        let mut finished = None;
        let mut failed = false;
        for event in events {
            match event {
                SearchEvent::File { path, excerpts } => {
                    let path: Arc<Path> = match path.strip_prefix(&self.root) {
                        Ok(relative) => Arc::from(relative),
                        Err(_) => path,
                    };
                    batch.push((path, excerpts));
                }
                SearchEvent::Done { truncated, .. } => finished = Some(truncated),
                SearchEvent::Error(err) => {
                    // `search` reports an error when the matcher cannot be
                    // built; tell that apart from any other failure by
                    // building the pattern here too.
                    self.pattern_error = self.last_query.as_ref().is_some_and(|q| {
                        build_regex(&q.pattern, q.is_regex, q.case_sensitive, q.whole_word).is_err()
                    });
                    self.error = Some(err);
                    failed = true;
                }
            }
        }
        // `Done` prunes in the same change, so the list is told once and
        // the gutter is sized after the prune.
        self.list.update(cx, |list, cx| match finished {
            Some(_) => list.update_results(
                |results| {
                    results.update_files(batch);
                    results.prune();
                },
                cx,
            ),
            None => list.add_files(batch, cx),
        });
        if let Some(truncated) = finished {
            self.done = Some(truncated);
        }
        if finished.is_some() || failed {
            self.invalid = self.compute_invalid(cx);
        }
        cx.notify();
    }

    /// Opens the file of line `r` as a preview with the cursor on it. A
    /// matching line puts the cursor on its first match and highlights it;
    /// a context line opens at its start with nothing highlighted.
    pub fn open_line(&mut self, r: LineRef, window: &mut Window, cx: &mut Context<Self>) {
        let results = self.results(cx);
        let (Some(file), Some(line), Some(workbench)) = (
            results.files().get(r.file),
            results.line(r),
            self.workbench.upgrade(),
        ) else {
            return;
        };
        let row = line.line_number.saturating_sub(1);
        let (path, is_context) = (file.path.clone(), line.ranges.is_empty());
        // A context line has no ranges; a match cut off at 1,000 chars has
        // only an empty one, and is matched again like any other.
        let query = if is_context {
            None
        } else {
            match self.last_query.clone() {
                Some(query) => Some(query),
                None => return,
            }
        };
        // The reported line may be cut at 1,000 chars and was decoded
        // lossily: the column comes from matching again on the buffer's
        // own decoded line once it is loaded (ruling R24).
        let point = Point { row, column: 0 };
        let view = workbench.update(cx, |wb, cx| {
            wb.jump_to(path, point, OpenMode::Preview, cx);
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
        if m.platform || m.control || m.alt {
            return;
        }
        match (ev.keystroke.key.as_str(), m.shift) {
            // As in Zed: `Enter` / `Shift+Enter` step to the next or
            // previous match inside the tab once the results are current;
            // with a change still waiting, `Enter` searches now.
            ("enter", shift) => {
                if self.stale || !self.searched {
                    self.run_search(cx);
                } else {
                    self.step_match(!shift, cx);
                }
            }
            ("escape", false) => {
                window.focus(&self.list_focus.clone(), cx);
            }
            _ => return,
        }
        cx.stop_propagation();
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

impl Drop for SearchView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// A message filling the list area, centered like the empty state: the
/// search error, "Searching…" or "No results found".
fn centered_message(selector: &'static str, text: String, text_color: gpui::Rgba) -> Div {
    div()
        .debug_selector(move || selector.into())
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .px(px(3. * GRID))
        .text_size(px(UI_FONT_SIZE))
        .text_color(text_color)
        .child(text)
}

impl Focusable for SearchView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SearchView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let files = self.results(cx).files();
        let all_collapsed = !files.is_empty() && files.iter().all(|f| f.collapsed);
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
                self.match_label(cx),
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
        // The status lives in the counter, and a message without results
        // sits in the list area: nothing above the list ever comes or
        // goes while searching, so the rows never jump (as in Zed).
        let message = match (&self.error, self.summary(cx)) {
            (Some(error), _) => Some(("search-error", error.clone(), color(ui.error))),
            (None, Some(summary)) if self.results(cx).rows().is_empty() => {
                Some(("search-summary", summary, muted))
            }
            _ => None,
        };
        let empty = self.results(cx).rows().is_empty() && self.query.read(cx).text().is_empty();
        let body = if let Some((selector, text, text_color)) = message {
            centered_message(selector, text, text_color).into_any_element()
        } else if empty {
            self.render_empty_state(cx).into_any_element()
        } else {
            self.list.clone().into_any_element()
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
