//! The Search panel in the sidebar (`workbench.action.findInFiles`,
//! `⇧⌘F`): a query with case / word / regex toggles, include and exclude
//! globs, and the results grouped by file as they stream in.
//!
//! Every change waits [`SEARCH_DEBOUNCE`] before searching and cancels
//! the running search through its `AtomicBool`. Each search gets a fresh
//! channel and a generation; events of a superseded search are dropped
//! (ruling R17).

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt;
use gpui::{
    AnyElement, App, Context, Div, Entity, Focusable, HighlightStyle, IntoElement, KeyDownEvent,
    ParentElement, Render, Styled, StyledText, Subscription, Task, UniformListScrollHandle,
    WeakEntity, Window, div, prelude::*, px, svg, uniform_list,
};
use lucerna_project::{LineMatch, MAX_RESULTS, SearchEvent, SearchQuery, search};
use lucerna_text::Point;

use crate::code_view::CodeView;
use crate::find_widget::{build_regex, single_line_selection};
use crate::layout::color;
use crate::tasks::guarded;
use crate::text_input::TextInput;
use crate::ui::{CHEVRON_DOWN_ICON, CHEVRON_RIGHT_ICON, GRID, TREE_ROW_HEIGHT, toggle_button};
use crate::workbench::enabled_globs;
use crate::{AppState, CommandRegistry, DocState, OpenMode, SidebarView, Workbench};

/// How long the inputs must stay unchanged before a search starts.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);

/// The matching lines of one file.
struct FileResult {
    /// Relative to the project root.
    path: Arc<Path>,
    matches: Vec<LineMatch>,
    collapsed: bool,
}

/// What a list row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    File(usize),
    /// File index, match index.
    Match(usize, usize),
}

pub struct SearchPanel {
    workbench: WeakEntity<Workbench>,
    root: PathBuf,
    query: Entity<TextInput>,
    include: Entity<TextInput>,
    exclude: Entity<TextInput>,
    case_sensitive: bool,
    whole_word: bool,
    is_regex: bool,
    /// Sorted by path.
    files: Vec<FileResult>,
    /// The list row of each file's header.
    file_rows: Vec<usize>,
    row_count: usize,
    /// Matching lines received.
    results: usize,
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
}

impl SearchPanel {
    pub(crate) fn new(
        root: PathBuf,
        workbench: WeakEntity<Workbench>,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = |placeholder: &'static str, cx: &mut Context<Self>| {
            let this = cx.weak_entity();
            cx.new(|cx| {
                let mut input = TextInput::new(cx);
                input.set_placeholder(placeholder);
                input.on_change(move |_, _, cx| {
                    this.update(cx, |panel, cx| panel.inputs_changed(cx)).ok();
                });
                input
            })
        };
        SearchPanel {
            workbench,
            root,
            query: input("Search", cx),
            include: input("e.g. src/**, *.rs", cx),
            exclude: input("e.g. target, *.min.js", cx),
            case_sensitive: false,
            whole_word: false,
            is_regex: false,
            files: Vec::new(),
            file_rows: Vec::new(),
            row_count: 0,
            results: 0,
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
        }
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

    /// Each file with results, root-relative, and its number of matching
    /// lines, sorted by path.
    pub fn files(&self) -> impl Iterator<Item = (&Path, usize)> {
        self.files.iter().map(|f| (&*f.path, f.matches.len()))
    }

    /// Rows in the results list: one per file plus one per matching line
    /// of each expanded file.
    pub fn row_count(&self) -> usize {
        self.row_count
    }

    /// The regex error of the last search, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// `"N results in M files"`, with `" (truncated at 20,000)"` when the
    /// search hit [`MAX_RESULTS`]; `None` before any search.
    pub fn summary(&self) -> Option<String> {
        if !self.searched || self.error.is_some() {
            return None;
        }
        if self.results == 0 {
            return Some(match self.done {
                Some(_) => "No results found".into(),
                None => "Searching…".into(),
            });
        }
        let plural = |n: usize, word: &str| match n {
            1 => format!("1 {word}"),
            n => format!("{n} {word}s"),
        };
        let mut summary = format!(
            "{} in {}",
            plural(self.results, "result"),
            plural(self.files.len(), "file")
        );
        if self.done == Some(true) {
            summary.push_str(&format!(" (truncated at {})", thousands(MAX_RESULTS)));
        }
        Some(summary)
    }

    /// Folds or unfolds the matches of file `index`.
    pub fn toggle_collapsed(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(file) = self.files.get_mut(index) {
            file.collapsed = !file.collapsed;
            self.rebuild_rows();
            cx.notify();
        }
    }

    /// Focuses the query input, first replacing its text with `text`.
    pub(crate) fn focus_query(
        &mut self,
        text: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(text) = text {
            self.query
                .update(cx, |input, cx| input.set_text(&text, window, cx));
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
        self.cancel_search();
        self._debounce = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            this.update(cx, |panel, cx| panel.run_search(cx)).ok();
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
        self._debounce = Task::ready(());
        self.cancel_search();
        self.files.clear();
        self.results = 0;
        self.done = None;
        self.error = None;
        self.rebuild_rows();
        cx.notify();
        let pattern = self.query.read(cx).text().to_owned();
        self.searched = !pattern.is_empty();
        if pattern.is_empty() {
            return;
        }
        let query = SearchQuery {
            pattern,
            is_regex: self.is_regex,
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            include: globs(self.include.read(cx).text()),
            exclude: globs(self.exclude.read(cx).text()),
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
                let applied =
                    this.update(cx, |panel, cx| panel.apply_events(generation, batch, cx));
                if applied.is_err() {
                    break;
                }
            }
        });
    }

    /// Adds a batch of events of search `generation`; stale ones are dropped.
    fn apply_events(&mut self, generation: u64, events: Vec<SearchEvent>, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        for event in events {
            match event {
                SearchEvent::File { path, matches } => {
                    let path: Arc<Path> = match path.strip_prefix(&self.root) {
                        Ok(relative) => Arc::from(relative),
                        Err(_) => path,
                    };
                    self.results += matches.len();
                    match self.files.binary_search_by(|f| f.path.cmp(&path)) {
                        Ok(i) => self.files[i].matches.extend(matches),
                        Err(i) => self.files.insert(
                            i,
                            FileResult {
                                path,
                                matches,
                                collapsed: false,
                            },
                        ),
                    }
                }
                SearchEvent::Done { truncated, .. } => self.done = Some(truncated),
                SearchEvent::Error(err) => self.error = Some(err),
            }
        }
        self.rebuild_rows();
        cx.notify();
    }

    fn rebuild_rows(&mut self) {
        self.file_rows.clear();
        let mut row = 0;
        for file in &self.files {
            self.file_rows.push(row);
            row += 1 + if file.collapsed {
                0
            } else {
                file.matches.len()
            };
        }
        self.row_count = row;
    }

    fn row(&self, index: usize) -> Option<Row> {
        let file = self
            .file_rows
            .partition_point(|&start| start <= index)
            .checked_sub(1)?;
        match index - self.file_rows[file] {
            0 => Some(Row::File(file)),
            n => Some(Row::Match(file, n - 1)),
        }
    }

    /// Opens the file of a match as a preview, puts the cursor on the
    /// match and highlights it.
    fn open_match(
        &mut self,
        file: usize,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(file), Some(workbench)) = (self.files.get(file), self.workbench.upgrade()) else {
            return;
        };
        let Some(m) = file.matches.get(index) else {
            return;
        };
        let row = m.line_number.saturating_sub(1);
        let Some(query) = self.last_query.clone() else {
            return;
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
            let line = view.snapshot().line(row);
            let range = first_match(&query, &line).unwrap_or_else(|| {
                tracing::debug!(row, "search result no longer matches its line");
                0..0
            });
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
            self._highlight = Some(cx.observe(&view, move |panel, view, cx| {
                if view.update(cx, reveal.clone()) {
                    panel._highlight = None;
                }
            }));
        }
    }

    fn query_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if ev.keystroke.key == "enter" && !(m.platform || m.control || m.alt || m.shift) {
            self.run_search(cx);
            cx.stop_propagation();
        }
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        range
            .filter_map(|index| match self.row(index)? {
                Row::File(file) => Some(self.render_file_row(file, cx)),
                Row::Match(file, m) => Some(self.render_match_row(file, m, cx)),
            })
            .collect()
    }

    fn render_file_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let file = &self.files[index];
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
        let icon = if file.collapsed {
            CHEVRON_RIGHT_ICON
        } else {
            CHEVRON_DOWN_ICON
        };
        row(("search-file", index), format!("search-file-{index}"), cx)
            .pl(px(2. * GRID))
            .on_click(cx.listener(move |panel, _, _, cx| panel.toggle_collapsed(index, cx)))
            .child(
                svg()
                    .data(icon)
                    .flex_none()
                    .size(px(12.))
                    .text_color(color(ui.icon_muted)),
            )
            .child(div().flex_none().child(name))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(11.))
                    .text_color(color(ui.text_muted))
                    .child(dir),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(1.5 * GRID))
                    .rounded_full()
                    .bg(color(ui.element_selected))
                    .text_size(px(11.))
                    .child(file.matches.len().to_string()),
            )
            .into_any_element()
    }

    fn render_match_row(&self, file: usize, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let m = &self.files[file].matches[index];
        let text = m.line_text.trim_start();
        let lead = m.line_text.len() - text.len();
        let style = HighlightStyle {
            background_color: Some(color(ui.search_match_background).into()),
            ..Default::default()
        };
        let highlights: Vec<(Range<usize>, HighlightStyle)> = m
            .ranges
            .iter()
            .map(|r| r.start.saturating_sub(lead)..r.end.saturating_sub(lead).min(text.len()))
            .filter(|r| {
                r.start < r.end && text.is_char_boundary(r.start) && text.is_char_boundary(r.end)
            })
            .map(|r| (r, style))
            .collect();
        let line = StyledText::new(text.to_owned()).with_highlights(highlights);
        row(
            ("search-match", file * MAX_RESULTS + index),
            format!("search-match-{file}-{index}"),
            cx,
        )
        .pl(px(8. * GRID))
        .on_click(
            cx.listener(move |panel, _, window, cx| panel.open_match(file, index, window, cx)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(line),
        )
        .into_any_element()
    }
}

impl Drop for SearchPanel {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// A 24 px results row with hover; add `on_click` and the content.
fn row(id: impl Into<gpui::ElementId>, selector: String, cx: &App) -> gpui::Stateful<Div> {
    let hover = color(cx.global::<AppState>().theme.ui.ghost_element_hover);
    div()
        .id(id)
        .debug_selector(move || selector)
        .flex()
        .items_center()
        .gap(px(1.5 * GRID))
        .h(px(TREE_ROW_HEIGHT))
        .w_full()
        .pr(px(2. * GRID))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
}

/// The byte range of the first non-empty match of `query` in `line`.
fn first_match(query: &SearchQuery, line: &str) -> Option<Range<usize>> {
    let regex = build_regex(
        &query.pattern,
        query.is_regex,
        query.case_sensitive,
        query.whole_word,
    )
    .ok()?;
    regex
        .find_iter(line)
        .find(|m| !m.is_empty())
        .map(|m| m.range())
}

/// Comma-separated globs, trimmed, empty ones dropped.
fn globs(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `20000` as `"20,000"`.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A bordered box holding `input` and any `extra` controls after it.
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

fn label(text: &'static str, cx: &App) -> Div {
    div()
        .text_size(px(11.))
        .text_color(color(cx.global::<AppState>().theme.ui.text_muted))
        .child(text)
}

impl Render for SearchPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = color(cx.global::<AppState>().theme.ui.text_muted);
        let toggle = |id, text, on, flag: fn(&mut Self) -> &mut bool, cx: &mut Context<Self>| {
            toggle_button(id, text, on, cx)
                .on_click(cx.listener(move |panel, _, _, cx| panel.toggle(flag, cx)))
        };
        let query = input_box(&self.query, cx)
            .on_key_down(cx.listener(Self::query_key_down))
            .child(toggle(
                "search-toggle-case",
                "Aa",
                self.case_sensitive,
                |p| &mut p.case_sensitive,
                cx,
            ))
            .child(toggle(
                "search-toggle-word",
                "ab",
                self.whole_word,
                |p| &mut p.whole_word,
                cx,
            ))
            .child(toggle(
                "search-toggle-regex",
                ".*",
                self.is_regex,
                |p| &mut p.is_regex,
                cx,
            ));
        div()
            .debug_selector(|| "search-panel".into())
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(px(GRID))
                    .px(px(2. * GRID))
                    .pb(px(2. * GRID))
                    .child(query)
                    .children(self.error.clone().map(|error| {
                        div()
                            .debug_selector(|| "search-error".into())
                            .text_size(px(12.))
                            .text_color(muted)
                            .child(error)
                    }))
                    .child(label("files to include", cx))
                    .child(input_box(&self.include, cx))
                    .child(label("files to exclude", cx))
                    .child(input_box(&self.exclude, cx))
                    .children(self.summary().map(|summary| {
                        div()
                            .debug_selector(|| "search-summary".into())
                            .pt(px(GRID))
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(summary)
                    })),
            )
            .child(
                div().flex_1().min_h_0().child(
                    uniform_list(
                        "search-results",
                        self.row_count,
                        cx.processor(|panel, range, _, cx| panel.render_rows(range, cx)),
                    )
                    .size_full()
                    .track_scroll(&self.scroll),
                ),
            )
    }
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.findInFiles",
        "Search: Find in Files",
        |wb, _, window, cx| {
            let selection = wb
                .active_code_view(cx)
                .and_then(|view| single_line_selection(view.read(cx)));
            wb.show_sidebar_view(SidebarView::Search, window, cx);
            let panel = wb.search_panel().clone();
            panel.update(cx, |panel, cx| panel.focus_query(selection, window, cx));
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_split_on_commas() {
        assert_eq!(globs(" src/**, *.rs ,,"), vec!["src/**", "*.rs"]);
        assert!(globs("").is_empty());
    }

    #[test]
    fn thousands_separators() {
        assert_eq!(thousands(20_000), "20,000");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
