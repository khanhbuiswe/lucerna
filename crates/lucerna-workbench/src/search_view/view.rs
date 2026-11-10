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
    AnyElement, App, Context, Div, Entity, FocusHandle, Focusable, HighlightStyle, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Pixels, Render, SharedString, Stateful, Styled,
    StyledText, Subscription, Task, UniformListScrollHandle, WeakEntity, Window, div, font,
    prelude::*, px, svg, uniform_list,
};
use lucerna_project::{SearchEvent, SearchQuery, search};
use lucerna_text::Point;

use super::model::{LineRef, Results, Row, first_match, globs, summary};
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
    /// `false` in single-file mode (R29): the folder is never walked.
    searchable: bool,
}

/// How result rows are drawn, read from the `editor.*` settings each frame.
#[derive(Clone)]
struct RowStyle {
    font_family: SharedString,
    font_size: Pixels,
    line_height: Pixels,
    gutter: Pixels,
}

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
            searchable,
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

    /// The files with results, sorted by path, and the rows shown.
    pub fn results(&self) -> &Results {
        &self.results
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
        self.results.set_all_collapsed(!all);
        cx.notify();
    }

    /// Focuses the query input, first replacing its text with `text`.
    pub fn focus_query(
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

    /// Adds a batch of events of search `generation`; stale ones are
    /// dropped. Runs once per batch, never per frame: each file added
    /// rebuilds the rows.
    fn apply_events(&mut self, generation: u64, events: Vec<SearchEvent>, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
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
                    self.results.add_file(path, excerpts);
                }
                SearchEvent::Done { truncated, .. } => self.done = Some(truncated),
                SearchEvent::Error(err) => self.error = Some(err),
            }
        }
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

    fn query_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if ev.keystroke.key == "enter" && !(m.platform || m.control || m.alt || m.shift) {
            self.run_search(cx);
            cx.stop_propagation();
        }
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
        }
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        style: &RowStyle,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
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
        row(ix, format!("search-file-{index}"), style, cx)
            .bg(color(ui.element_background))
            .px(px(2. * GRID))
            .on_click(cx.listener(move |view, _, _, cx| view.toggle_collapsed(index, cx)))
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
            .child(div().flex_none().text_color(color(ui.text)).child(name))
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
        let ui = &cx.global::<AppState>().theme.ui;
        let Some(line) = self.results.line(r) else {
            return div().into_any_element();
        };
        let text = &line.line_text;
        let mark = HighlightStyle {
            background_color: Some(color(ui.search_match_background).into()),
            ..Default::default()
        };
        // An empty range (a match past the 1,000-char cut) draws nothing.
        let highlights: Vec<(Range<usize>, HighlightStyle)> = line
            .ranges
            .iter()
            .map(|r| r.start..r.end.min(text.len()))
            .filter(|r| {
                r.start < r.end && text.is_char_boundary(r.start) && text.is_char_boundary(r.end)
            })
            .map(|r| (r, mark))
            .collect();
        let styled = StyledText::new(text.clone()).with_highlights(highlights);
        let selector = format!("search-line-{}-{}-{}", r.file, r.excerpt, r.line);
        row(ix, selector, style, cx)
            .on_click(cx.listener(move |view, _, window, cx| view.open_line(r, window, cx)))
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
                    cx.listener(|view, _, _, cx| {
                        view.filters_shown = !view.filters_shown;
                        cx.notify();
                    }),
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
                .child(input_box(&self.include, cx).flex_1().min_w_0())
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
                div().flex_1().min_h_0().child(
                    uniform_list(
                        "search-results",
                        self.results.rows().len(),
                        cx.processor(move |view, range, _, cx| view.render_rows(range, &style, cx)),
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

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.findInFiles",
        "Search: Find in Files",
        |wb, _, window, cx| find_in_files(wb, window, cx),
    );
    commands.register(
        "workbench.view.search",
        "View: Show Search",
        |wb, _, window, cx| find_in_files(wb, window, cx),
    );
}
