//! Find in file: [`find_in_snapshot`] and the [`FindWidget`] that floats
//! over the top right of a [`CodeView`] (`actions.find`, `⌘F`).
//!
//! Matches are recomputed whenever the query, a toggle or the view's
//! snapshot changes. Files over [`BACKGROUND_SEARCH_BYTES`] are searched
//! on the background executor; a generation counter drops results of
//! superseded searches.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, AppContext, ClickEvent, Context, Entity, EventEmitter, Focusable, IntoElement,
    KeyDownEvent, ParentElement, Render, Styled, Task, WeakEntity, Window, div, prelude::*, px,
};
use lucerna_editor::find::{FindQuery, find_all};
use lucerna_editor::{EditKind, EditPlan, SelectionSet, mk_patch};
use lucerna_project::build_pattern;
use lucerna_project::replace::{ReplacePattern, replace_ranges, replacement_at};
use lucerna_text::{BufferSnapshot, Point};
use regex::{Regex, RegexBuilder};

use crate::code_view::{CodeView, DEFAULT_WORD_SEPARATORS};
use crate::layout::color;
use crate::quick_open::picker::wrap_index;
use crate::tasks::guarded;
use crate::text_input::TextInput;
use crate::ui::{
    CLOSE_ICON, ClickHandler, GRID, REPLACE_ALL_ICON, REPLACE_ICON, REPLACE_NEXT_ICON,
    UI_FONT_SIZE_SMALL, bar_icon_button, bar_separator, input_box, match_nav, search_bar_row,
    search_query_box, search_query_column, search_toolbar, toggle_button, tooltip,
};
use crate::{AppState, CommandRegistry, Workbench};

/// Snapshots larger than this are searched off the main thread.
pub const BACKGROUND_SEARCH_BYTES: usize = 1024 * 1024;

/// The byte ranges of every non-empty match of `pattern` in `snapshot`, in
/// order, with the default word separators for `whole_word`. A thin wrapper
/// over [`lucerna_editor::find::find_all`] for vim's `/` and tests; the
/// Find bar passes the view's own separators.
pub fn find_in_snapshot(
    snapshot: &BufferSnapshot,
    pattern: &str,
    is_regex: bool,
    case_sensitive: bool,
    whole_word: bool,
) -> Result<Vec<Range<usize>>, String> {
    find_all(
        snapshot,
        &FindQuery {
            pattern,
            is_regex,
            case_sensitive,
            whole_word,
            word_separators: DEFAULT_WORD_SEPARATORS,
        },
    )
}

/// The regex for a project search query: the source from
/// [`build_pattern`], case folding unless `case_sensitive`, and `^`/`$`
/// at line ends.
pub(crate) fn build_regex(
    pattern: &str,
    is_regex: bool,
    case_sensitive: bool,
    whole_word: bool,
) -> Result<Regex, String> {
    RegexBuilder::new(&build_pattern(pattern, is_regex, whole_word))
        .case_insensitive(!case_sensitive)
        .multi_line(true)
        .build()
        .map_err(|err| err.to_string())
}

/// Which match becomes active after a search.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    /// The first match at or after the selection start, revealed: the
    /// query or a toggle changed.
    Nearest,
    /// The same index, clamped, without moving the view: the file reloaded.
    Keep,
    /// The nearest match, without moving the view or the selection: `⌘D`
    /// put the word it selected into the box.
    Seed,
}

/// What a find widget opened for vim's `/` or `?` reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindWidgetEvent {
    /// Enter: vim jumps to the match.
    Submitted,
    /// Closed without Enter: vim's cursor goes back.
    Dismissed,
}

pub struct FindWidget {
    view: WeakEntity<CodeView>,
    input: Entity<TextInput>,
    /// The input's text.
    query: String,
    /// The Replace row's input. Its text stays while the bar is hidden.
    replace_input: Entity<TextInput>,
    /// The Replace row is shown (and the view can be edited).
    replace_visible: bool,
    /// Replacements take the case shape of what they replace.
    preserve_case: bool,
    case_sensitive: bool,
    whole_word: bool,
    is_regex: bool,
    matches: Arc<[Range<usize>]>,
    active: Option<usize>,
    error: Option<String>,
    /// A background search is running: `matches` and `error` are of the
    /// previous query, so the box shows no error color yet.
    pending: bool,
    /// The view revision the matches were computed for.
    revision: u64,
    /// Bumped by every search; a background result applies only while
    /// its generation is still the latest.
    generation: u64,
    /// Opened for vim's `/` or `?`: Enter emits
    /// [`FindWidgetEvent::Submitted`] instead of stepping.
    vim: bool,
    /// Set by a replace: the next search makes the first match at or
    /// after this offset active (wrapping to the first) and reveals it.
    pending_pick_from: Option<(u64, usize)>,
    _search: Task<()>,
    _view_changed: gpui::Subscription,
}

impl FindWidget {
    pub(crate) fn new(view: &Entity<CodeView>, cx: &mut Context<Self>) -> Self {
        let this = cx.weak_entity();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder("Find");
            // Runs inside the input's update: the text comes as an argument.
            input.on_change(move |text, _, cx| {
                this.update(cx, |find, cx| {
                    find.query = text.to_owned();
                    find.search(Pick::Nearest, cx);
                })
                .ok();
            });
            input
        });
        let replace_input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder("Replace");
            input
        });
        FindWidget {
            view: view.downgrade(),
            input,
            replace_input,
            replace_visible: false,
            preserve_case: false,
            query: String::new(),
            case_sensitive: false,
            whole_word: false,
            is_regex: false,
            matches: Arc::from([]),
            active: None,
            error: None,
            pending: false,
            revision: view.read(cx).revision(),
            generation: 0,
            vim: false,
            pending_pick_from: None,
            _search: Task::ready(()),
            _view_changed: cx.observe(view, |find, view, cx| {
                let view = view.read(cx);
                if view.find_widget().is_some() && view.revision() != find.revision {
                    find.search(Pick::Keep, cx);
                }
            }),
        }
    }

    pub fn input(&self) -> &Entity<TextInput> {
        &self.input
    }

    pub fn replace_input(&self) -> &Entity<TextInput> {
        &self.replace_input
    }

    /// The Replace row is shown.
    pub fn replace_visible(&self) -> bool {
        self.replace_visible
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

    pub fn preserve_case(&self) -> bool {
        self.preserve_case
    }

    /// Replacing is possible: the view is alive and editable, and the bar
    /// is not vim's `/`.
    fn can_replace(&self, cx: &App) -> bool {
        !self.vim
            && self
                .view
                .upgrade()
                .is_some_and(|view| !view.read(cx).is_readonly())
    }

    /// Shows or hides the Replace row, then focuses the input that applies.
    pub fn toggle_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_visible = !self.replace_visible && self.can_replace(cx);
        let focus = if self.replace_visible {
            self.replace_input.focus_handle(cx)
        } else {
            self.input.focus_handle(cx)
        };
        window.focus(&focus, cx);
        cx.notify();
    }

    pub fn toggle_preserve_case(&mut self, cx: &mut Context<Self>) {
        self.preserve_case = !self.preserve_case;
        cx.notify();
    }

    /// Whether the Replace input has focus.
    pub(crate) fn replace_input_focused(&self, window: &Window, cx: &App) -> bool {
        self.replace_visible
            && self.can_replace(cx)
            && self.replace_input.focus_handle(cx).is_focused(window)
    }

    /// The view, when a replace may run: the Replace row is shown, the
    /// view takes edits, and the matches are of its current text.
    fn replace_target(&self, cx: &App) -> Option<Entity<CodeView>> {
        if !self.replace_visible || self.pending || !self.can_replace(cx) {
            return None;
        }
        let view = self.view.upgrade()?;
        (view.read(cx).revision() == self.revision).then_some(view)
    }

    /// The query's regex and the Replace input's pattern. The regex is the
    /// one the bar searched with: whole word is not in it but in the bar's
    /// matches (`editor.wordSeparators`), so replaces act on those ranges.
    fn replace_parts(&self, cx: &App) -> Option<(Regex, ReplacePattern)> {
        let regex =
            lucerna_editor::find::build_regex(&self.query, self.is_regex, self.case_sensitive)
                .ok()?;
        let replace = self.replace_input.read(cx).text();
        Some((regex, ReplacePattern::parse(replace, self.is_regex)))
    }

    /// Replaces the active match and moves to the next one (VS Code's
    /// `editor.action.replaceOne`). When the selection is not the active
    /// match, only selects the match nearest the cursor.
    pub fn replace_one(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.replace_target(cx) else {
            return;
        };
        let Some(range) = self.active.map(|i| self.matches[i].clone()) else {
            return self.select_nearest(cx);
        };
        let selected = {
            let view = view.read(cx);
            let snapshot = view.snapshot();
            view.selection()
                .map(|r| snapshot.point_to_offset(r.start)..snapshot.point_to_offset(r.end))
        };
        if selected != Some(range.clone()) {
            return self.select_nearest(cx);
        }
        let Some((regex, pattern)) = self.replace_parts(cx) else {
            return;
        };
        // Only the lines of the match: `^`, `$` and `\b` see the same
        // neighbours there as in the whole text.
        let snapshot = view.read(cx).snapshot().clone();
        let first = snapshot.offset_to_point(range.start).row;
        let last = snapshot.offset_to_point(range.end).row;
        let lo = snapshot.point_to_offset(Point {
            row: first,
            column: 0,
        });
        let hi = if last + 1 < snapshot.line_count() {
            snapshot.point_to_offset(Point {
                row: last + 1,
                column: 0,
            })
        } else {
            snapshot.len_bytes()
        };
        let text = snapshot.text_for_range(lo..hi);
        let local = range.start - lo..range.end - lo;
        let Some(replacement) = replacement_at(&text, &regex, local, &pattern, self.preserve_case)
        else {
            return;
        };
        let end = range.start + replacement.len();
        let mut plan = EditPlan::new(
            vec![(range, Arc::from(replacement))],
            SelectionSet::cursor(end),
            EditKind::Other,
        );
        plan.stop_before = true;
        plan.stop_after = true;
        // Before the edit: the search its revision change starts picks the
        // match after the replacement.
        if view.update(cx, |v, cx| v.transact(plan, cx)) {
            // The offset belongs to the search of this revision only; the
            // view's change notification reaches us after this returns.
            self.pending_pick_from = Some((view.read(cx).revision(), end));
        }
    }

    /// Makes the match nearest the selection active and selects it.
    fn select_nearest(&mut self, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        self.active = self.nearest(cx).or(Some(0));
        self.reveal(cx);
        cx.notify();
    }

    /// Replaces every match as one undo step (VS Code's
    /// `editor.action.replaceAll`). Files over [`BACKGROUND_SEARCH_BYTES`]
    /// compute the edits on the background executor; they apply only if
    /// the text did not change meanwhile.
    pub fn replace_all(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.replace_target(cx) else {
            return;
        };
        if self.matches.is_empty() {
            return;
        }
        let Some((regex, pattern)) = self.replace_parts(cx) else {
            return;
        };
        let preserve = self.preserve_case;
        let matches = self.matches.clone();
        let snapshot = view.read(cx).snapshot().clone();
        let small = snapshot.len_bytes() <= BACKGROUND_SEARCH_BYTES;
        let edits = move || {
            let text = snapshot.text_for_range(0..snapshot.len_bytes());
            replace_ranges(&text, &regex, &matches, &pattern, preserve)
        };
        if small {
            apply_replacements(&view, edits(), cx);
            return;
        }
        // Shares the search's task and generation: a new search drops it.
        self.cancel();
        let (generation, revision) = (self.generation, self.revision);
        self.pending = true;
        let edits =
            cx.background_spawn(async move { guarded("replace all", edits).unwrap_or_default() });
        self._search = cx.spawn(async move |this, cx| {
            let edits = edits.await;
            this.update(cx, |find, cx| {
                if find.generation != generation {
                    return;
                }
                find.pending = false;
                if find.replace_visible
                    && let Some(view) = find.view.upgrade()
                    && view.read(cx).revision() == revision
                {
                    apply_replacements(&view, edits, cx);
                }
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    pub fn matches(&self) -> &[Range<usize>] {
        &self.matches
    }

    /// Index of the active match.
    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    /// The query and its toggles, for vim.
    pub(crate) fn query(&self) -> lucerna_vim::SearchQuery {
        lucerna_vim::SearchQuery {
            pattern: self.query.clone(),
            is_regex: self.is_regex,
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
        }
    }

    /// The case-sensitive and whole-word toggles, which `⌘D` and `⇧⌘L`
    /// use for text the user selected.
    pub(crate) fn match_options(&self) -> lucerna_editor::multicursor::MatchOptions {
        lucerna_editor::multicursor::MatchOptions {
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
        }
    }

    /// Replaces the query with `text` (the word `⌘D` selected) and
    /// searches again, like typing it, but leaves the selections alone.
    pub(crate) fn seed_query(&mut self, text: &str, cx: &mut Context<Self>) {
        // With the regex toggle on the word is searched literally.
        let text = if self.is_regex {
            regex::escape(text)
        } else {
            text.to_owned()
        };
        self.query = text.clone();
        self.input
            .update(cx, |input, cx| input.set_text_quietly(&text, cx));
        self.search(Pick::Seed, cx);
    }

    /// Ends vim's use of the widget; Enter steps again.
    pub(crate) fn set_vim(&mut self, vim: bool) {
        self.vim = vim;
    }

    /// The regex error of the current query, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// `"N of M"` (1-based), `"No results"`, or empty with no query.
    pub fn counter(&self) -> String {
        if self.query.is_empty() {
            return String::new();
        }
        match (self.matches.len(), self.active) {
            (0, _) => "No results".into(),
            (total, Some(i)) => format!("{} of {total}", i + 1),
            (total, None) => format!("? of {total}"),
        }
    }

    /// The bar's match counter, as in Zed: `"3/19"` (1-based), or `"0/0"`
    /// with no query, no match or no active match.
    pub fn match_label(&self) -> String {
        match (self.active, self.matches.len()) {
            (Some(i), total) if total > 0 && !self.query.is_empty() => {
                format!("{}/{total}", i + 1)
            }
            _ => "0/0".into(),
        }
    }

    /// The query box shows the error color: a regex error, or a non-empty
    /// query without matches, once its search finished.
    pub fn is_invalid(&self) -> bool {
        !self.pending
            && (self.error.is_some() || (!self.query.is_empty() && self.matches.is_empty()))
    }

    pub fn toggle_case_sensitive(&mut self, cx: &mut Context<Self>) {
        self.case_sensitive = !self.case_sensitive;
        self.search(Pick::Nearest, cx);
    }

    pub fn toggle_whole_word(&mut self, cx: &mut Context<Self>) {
        self.whole_word = !self.whole_word;
        self.search(Pick::Nearest, cx);
    }

    pub fn toggle_regex(&mut self, cx: &mut Context<Self>) {
        self.is_regex = !self.is_regex;
        self.search(Pick::Nearest, cx);
    }

    /// Selects the next match, wrapping to the first (`Enter`).
    pub fn next(&mut self, cx: &mut Context<Self>) {
        self.step(1, cx);
    }

    /// Selects the previous match, wrapping to the last (`Shift+Enter`).
    pub fn previous(&mut self, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        self.active = Some(match self.active {
            Some(i) => wrap_index(i, self.matches.len(), delta),
            None => self.nearest(cx).unwrap_or(0),
        });
        self.reveal(cx);
        cx.notify();
    }

    /// Drops the result of any search in flight.
    fn cancel(&mut self) {
        self.generation += 1;
        self.pending = false;
        self._search = Task::ready(());
    }

    /// Searches the view's current snapshot.
    fn search(&mut self, pick: Pick, cx: &mut Context<Self>) {
        self.cancel();
        if pick == Pick::Nearest {
            // A replace's offset belongs to the search its edit started;
            // a search the user starts afterwards must not consume it.
            self.pending_pick_from = None;
        }
        let Some(view) = self.view.upgrade() else {
            return;
        };
        let (snapshot, revision, separators) = {
            let view = view.read(cx);
            (
                view.snapshot().clone(),
                view.revision(),
                view.edit_config(cx).word_separators,
            )
        };
        self.revision = revision;
        let pattern = self.query.clone();
        let (regex, case, word) = (self.is_regex, self.case_sensitive, self.whole_word);
        let search = move |snapshot: &BufferSnapshot| {
            find_all(
                snapshot,
                &FindQuery {
                    pattern: &pattern,
                    is_regex: regex,
                    case_sensitive: case,
                    whole_word: word,
                    word_separators: &separators,
                },
            )
        };
        if snapshot.len_bytes() <= BACKGROUND_SEARCH_BYTES {
            let result = search(&snapshot);
            self.apply(result, pick, cx);
            return;
        }
        let generation = self.generation;
        self.pending = true;
        let result = cx.background_spawn(async move {
            guarded("find", || search(&snapshot)).unwrap_or_else(|| Err("internal error".into()))
        });
        self._search = cx.spawn(async move |this, cx| {
            let result = result.await;
            this.update(cx, |find, cx| {
                if find.generation == generation {
                    find.apply(result, pick, cx);
                }
            })
            .ok();
        });
    }

    fn apply(
        &mut self,
        result: Result<Vec<Range<usize>>, String>,
        pick: Pick,
        cx: &mut Context<Self>,
    ) {
        let (matches, error) = match result {
            Ok(matches) => (matches, None),
            Err(err) => (Vec::new(), Some(err)),
        };
        self.matches = matches.into();
        self.error = error;
        self.pending = false;
        let last = self.matches.len().checked_sub(1);
        // A user edit before this search's revision makes the offset stale.
        let pick_from = self
            .pending_pick_from
            .take()
            .filter(|(revision, _)| *revision == self.revision)
            .map(|(_, offset)| offset);
        self.active = match (pick, self.active, last) {
            (_, _, None) => None,
            _ if let Some(offset) = pick_from => {
                let i = self.matches.partition_point(|r| r.start < offset);
                Some(if i < self.matches.len() { i } else { 0 })
            }
            (Pick::Keep, Some(i), Some(last)) => Some(i.min(last)),
            _ => self.nearest(cx).or(Some(0)),
        };
        let matches = self.matches.clone();
        let active = self.active;
        // A hidden bar (an option toggled by key) shows no highlights and
        // does not move the selection.
        let visible = self
            .view
            .upgrade()
            .is_some_and(|view| view.read(cx).find_widget().is_some());
        if visible && let Some(view) = self.view.upgrade() {
            view.update(cx, |v, cx| v.set_match_highlights(matches, active, cx));
        }
        if visible && (pick == Pick::Nearest || pick_from.is_some()) {
            self.reveal(cx);
        }
        cx.notify();
    }

    /// The first match starting at or after the selection start (or the
    /// cursor), if any.
    fn nearest(&self, cx: &App) -> Option<usize> {
        let view = self.view.upgrade()?;
        let view = view.read(cx);
        let from = view.selection().map_or(view.cursor(), |r| r.start);
        let offset = view.snapshot().point_to_offset(from);
        let i = self.matches.partition_point(|r| r.start < offset);
        (i < self.matches.len()).then_some(i)
    }

    /// Scrolls the active match into the middle and selects it.
    fn reveal(&self, cx: &mut Context<Self>) {
        let (Some(view), Some(i)) = (self.view.upgrade(), self.active) else {
            return;
        };
        let range = self.matches[i].clone();
        view.update(cx, |v, cx| {
            let start = v.snapshot().offset_to_point(range.start);
            let end = v.snapshot().offset_to_point(range.end);
            v.go_to(start, cx);
            v.select(start..end, cx);
            v.set_match_highlights(self.matches.clone(), Some(i), cx);
        });
    }

    fn key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        if ev.keystroke.key == "tab" && self.replace_visible && self.can_replace(cx) {
            // Tab and Shift+Tab move between the two inputs.
            let (from, to) = if m.shift {
                (&self.replace_input, &self.input)
            } else {
                (&self.input, &self.replace_input)
            };
            if from.focus_handle(cx).is_focused(window) {
                let to = to.focus_handle(cx);
                window.focus(&to, cx);
                cx.stop_propagation();
            }
            return;
        }
        // Enter in the Replace input is the keymap's (`replaceInputFocussed`).
        if ev.keystroke.key != "enter" || !self.input.focus_handle(cx).is_focused(window) {
            return;
        }
        if self.vim {
            cx.emit(FindWidgetEvent::Submitted);
        } else if m.shift {
            self.previous(cx);
        } else {
            self.next(cx);
        }
        cx.stop_propagation();
    }
}

impl EventEmitter<FindWidgetEvent> for FindWidget {}

impl Focusable for FindWidget {
    fn focus_handle(&self, cx: &App) -> gpui::FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for FindWidget {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_replace = self.can_replace(cx);
        let ui = cx.global::<AppState>().theme.ui.clone();
        let view = self.view.clone();
        let toggle =
            |id, label, on, f: fn(&mut Self, &mut Context<Self>), cx: &mut Context<Self>| {
                toggle_button(id, label, on, cx)
                    .on_click(cx.listener(move |find, _, _, cx| f(find, cx)))
            };
        let invalid = self.is_invalid();
        let field = search_query_box(&self.input, invalid, cx)
            .debug_selector(move || {
                if invalid {
                    "find-query-invalid".into()
                } else {
                    "find-query".into()
                }
            })
            .child(toggle(
                "find-toggle-case",
                "Aa",
                self.case_sensitive,
                Self::toggle_case_sensitive,
                cx,
            ))
            .child(toggle(
                "find-toggle-word",
                "ab",
                self.whole_word,
                Self::toggle_whole_word,
                cx,
            ))
            .child(toggle(
                "find-toggle-regex",
                ".*",
                self.is_regex,
                Self::toggle_regex,
                cx,
            ));
        let this = cx.weak_entity();
        let step = move |forward: bool| -> ClickHandler {
            let this = this.clone();
            Box::new(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                this.update(cx, |find, cx| {
                    if forward {
                        find.next(cx)
                    } else {
                        find.previous(cx)
                    }
                })
                .ok();
            })
        };
        let nav = match_nav(
            ("find-previous", step(false)),
            ("find-next", step(true)),
            self.match_label(),
            cx,
        );
        let close_button =
            bar_icon_button("find-close", CLOSE_ICON, false, cx).on_click(move |_, window, cx| {
                if let Some(view) = view.upgrade() {
                    close(&view, window, cx);
                }
            });
        let toggle_replace = bar_icon_button(
            "find-toggle-replace",
            REPLACE_ICON,
            self.replace_visible,
            cx,
        )
        .tooltip(tooltip("Toggle Replace"))
        .on_click(cx.listener(|find, _, window, cx| find.toggle_replace(window, cx)));
        let replace_row = (self.replace_visible && can_replace).then(|| {
            // Replacing needs a match to act on.
            let ready = !self.matches.is_empty() && self.error.is_none();
            let action = |id, icon, tip: &'static str, f: fn(&mut Self, &mut Context<Self>)| {
                let button = bar_icon_button(id, icon, false, cx).tooltip(tooltip(tip));
                if ready {
                    button.on_click(cx.listener(move |find, _, _, cx| f(find, cx)))
                } else {
                    // The occluding cover keeps the button from seeing the
                    // mouse: no hover, no click.
                    button
                        .relative()
                        .opacity(0.4)
                        .cursor_default()
                        .child(div().absolute().inset_0().occlude())
                }
            };
            let field = search_query_column(input_box(&self.replace_input, cx))
                .debug_selector(|| "find-replace-input".into())
                .child(
                    toggle_button("find-toggle-preserve-case", "AB", self.preserve_case, cx)
                        .tooltip(tooltip("Preserve Case"))
                        .on_click(cx.listener(|find, _, _, cx| find.toggle_preserve_case(cx))),
                );
            search_bar_row()
                .debug_selector(|| "find-replace-row".into())
                .child(field)
                .child(bar_separator(cx))
                .child(action(
                    "find-replace-next",
                    REPLACE_NEXT_ICON,
                    "Replace Next Match",
                    Self::replace_one,
                ))
                .child(action(
                    "find-replace-all",
                    REPLACE_ALL_ICON,
                    "Replace All Matches",
                    Self::replace_all,
                ))
        });
        search_toolbar(cx)
            .id("find-widget")
            .debug_selector(|| "find-widget".into())
            .occlude()
            .on_key_down(cx.listener(Self::key_down))
            .child(
                search_bar_row()
                    .child(field)
                    .children(can_replace.then_some(toggle_replace))
                    .child(bar_separator(cx))
                    .child(nav)
                    .child(div().flex_1())
                    .child(bar_separator(cx))
                    .child(close_button),
            )
            .children(replace_row)
            .children(self.error.clone().map(|error| {
                div()
                    .debug_selector(|| "find-error".into())
                    .px(px(2. * GRID))
                    .pb(px(GRID))
                    .text_size(px(UI_FONT_SIZE_SMALL))
                    .text_color(color(ui.error))
                    .child(error)
            }))
    }
}

/// Applies `edits` (sorted, in `view`'s current offsets) as one undo step;
/// the selections move through them.
fn apply_replacements(view: &Entity<CodeView>, edits: Vec<(Range<usize>, String)>, cx: &mut App) {
    if edits.is_empty() {
        return;
    }
    let edits: Vec<(Range<usize>, Arc<str>)> = edits
        .into_iter()
        .map(|(range, text)| (range, Arc::from(text)))
        .collect();
    view.update(cx, |v, cx| {
        let mut selections = v.selections().clone();
        selections.remap(&mk_patch(&edits));
        let mut plan = EditPlan::new(edits, selections, EditKind::Other);
        plan.stop_before = true;
        plan.stop_after = true;
        v.transact(plan, cx);
    });
}

/// The selected text when the selection is on one line and not empty.
pub(crate) fn single_line_selection(view: &CodeView) -> Option<String> {
    let range = view.selection()?;
    if range.start.row != range.end.row {
        return None;
    }
    view.primary_text()
}

/// Shows the find widget over `view`, pre-filled with a one-line
/// selection. `replace` (`⌥⌘F`) also shows the Replace row and focuses its
/// input when the Find input already had focus or was just filled in;
/// plain `⌘F` keeps the row as it is, hidden on a hidden bar.
fn open(view: &Entity<CodeView>, replace: bool, window: &mut Window, cx: &mut App) {
    let prefill = single_line_selection(view.read(cx));
    let was_hidden = view.read(cx).find_widget().is_none();
    let find = show(view, cx);
    let find_had_focus = find.read(cx).input.focus_handle(cx).is_focused(window);
    let filled = prefill.is_some();
    match prefill {
        Some(text) => find.update(cx, |find, cx| {
            find.input
                .update(cx, |input, cx| input.set_text(&text, window, cx))
        }),
        None => find.update(cx, |find, cx| find.search(Pick::Nearest, cx)),
    }
    let can_replace = find.read(cx).can_replace(cx);
    find.update(cx, |find, cx| {
        if replace {
            find.replace_visible = can_replace;
        } else if was_hidden {
            find.replace_visible = false;
        }
        cx.notify();
    });
    let focus = if replace && can_replace && (find_had_focus || filled) {
        find.read(cx).replace_input.focus_handle(cx)
    } else {
        find.focus_handle(cx)
    };
    window.focus(&focus, cx);
}

/// Shows the find widget over `view`, made on first use. A widget still
/// open for vim is dismissed first.
fn show(view: &Entity<CodeView>, cx: &mut App) -> Entity<FindWidget> {
    let find = match view.read(cx).find_widget_any().cloned() {
        Some(find) => find,
        None => cx.new(|cx| FindWidget::new(view, cx)),
    };
    find.update(cx, end_vim);
    view.update(cx, |v, cx| v.show_find_widget(find.clone(), cx));
    find
}

/// Shows the find widget over `view` for vim's `/` or `?`: empty, with
/// Enter reported as [`FindWidgetEvent::Submitted`].
pub(crate) fn open_for_vim(
    view: &Entity<CodeView>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<FindWidget> {
    let find = show(view, cx);
    find.update(cx, |find, cx| {
        find.vim = true;
        find.replace_visible = false;
        find.input
            .update(cx, |input, cx| input.set_text("", window, cx));
    });
    let focus = find.focus_handle(cx);
    window.focus(&focus, cx);
    find
}

/// A widget open for vim reports [`FindWidgetEvent::Dismissed`] and
/// becomes a plain one.
fn end_vim(find: &mut FindWidget, cx: &mut Context<FindWidget>) {
    if std::mem::take(&mut find.vim) {
        cx.emit(FindWidgetEvent::Dismissed);
    }
}

/// Hides the find widget over `view`, clears its highlights and gives the
/// view focus.
pub(crate) fn close(view: &Entity<CodeView>, window: &mut Window, cx: &mut App) {
    if let Some(find) = view.read(cx).find_widget_any().cloned() {
        find.update(cx, |find, cx| {
            end_vim(find, cx);
            find.cancel();
        });
    }
    view.update(cx, |v, cx| v.hide_find_widget(cx));
    let focus = view.focus_handle(cx);
    window.focus(&focus, cx);
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register("actions.find", "Find", |wb, _, window, cx| {
        if let Some(view) = wb.active_code_view(cx) {
            open(&view, false, window, cx);
        }
    });
    commands.register(
        "editor.action.startFindReplaceAction",
        "Replace",
        |wb, _, window, cx| {
            if let Some(view) = wb.active_code_view(cx) {
                open(&view, true, window, cx);
            }
        },
    );
    commands.register("editor.action.replaceOne", "Replace One", |wb, _, _, cx| {
        if let Some(find) = wb.visible_find_widget(cx) {
            find.update(cx, FindWidget::replace_one);
        }
    });
    commands.register("editor.action.replaceAll", "Replace All", |wb, _, _, cx| {
        if let Some(find) = wb.visible_find_widget(cx) {
            find.update(cx, FindWidget::replace_all);
        }
    });
    commands.register(
        "toggleFindCaseSensitive",
        "Toggle Case Sensitive",
        |wb, _, _, cx| {
            if let Some(find) = wb.toggle_target(cx) {
                find.update(cx, FindWidget::toggle_case_sensitive);
            }
        },
    );
    commands.register(
        "toggleFindWholeWord",
        "Toggle Whole Word",
        |wb, _, _, cx| {
            if let Some(find) = wb.toggle_target(cx) {
                find.update(cx, FindWidget::toggle_whole_word);
            }
        },
    );
    commands.register(
        "toggleFindRegex",
        "Toggle Use Regular Expression",
        |wb, _, _, cx| {
            if let Some(find) = wb.toggle_target(cx) {
                find.update(cx, FindWidget::toggle_regex);
            }
        },
    );
    commands.register(
        "togglePreserveCase",
        "Toggle Preserve Case",
        |wb, _, _, cx| {
            if let Some(find) = wb.toggle_target(cx) {
                find.update(cx, FindWidget::toggle_preserve_case);
            }
        },
    );
    commands.register("closeFindWidget", "Close Find", |wb, _, window, cx| {
        // Only while focus is in the view or its widget: Esc elsewhere (the
        // Search panel) must not close it or take focus.
        if let Some(view) = wb.active_code_view(cx)
            && view.focus_handle(cx).contains_focused(window, cx)
        {
            close(&view, window, cx);
        }
    });
}

impl Workbench {
    /// The active code view's find widget for a toggle key: the shown one,
    /// or a hidden one made on first use, so the option is already set the
    /// next time the bar opens.
    fn toggle_target(&self, cx: &mut App) -> Option<Entity<FindWidget>> {
        let view = self.active_code_view(cx)?;
        if let Some(find) = view.read(cx).find_widget_any().cloned() {
            return Some(find);
        }
        let find = cx.new(|cx| FindWidget::new(&view, cx));
        view.update(cx, |v, _| v.set_find_widget_hidden(find.clone()));
        Some(find)
    }

    /// The active code view's find widget, while it is shown.
    fn visible_find_widget(&self, cx: &App) -> Option<Entity<FindWidget>> {
        self.active_code_view(cx)
            .and_then(|view| view.read(cx).find_widget().cloned())
    }

    /// Whether the active code view shows its find widget with focus in
    /// the Replace input.
    pub(crate) fn replace_input_focused(&self, window: &Window, cx: &App) -> bool {
        self.active_code_view(cx)
            .and_then(|view| view.read(cx).find_widget().cloned())
            .is_some_and(|find| find.read(cx).replace_input_focused(window, cx))
    }

    /// Whether the active code view shows its find widget.
    pub(crate) fn find_widget_visible(&self, cx: &App) -> bool {
        self.active_code_view(cx)
            .is_some_and(|view| view.read(cx).find_widget().is_some())
    }
}
