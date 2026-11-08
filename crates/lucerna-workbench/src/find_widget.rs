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
    App, AppContext, Context, Entity, Focusable, IntoElement, KeyDownEvent, ParentElement, Render,
    Styled, Task, WeakEntity, Window, div, prelude::*, px,
};
use lucerna_project::build_pattern;
use lucerna_text::BufferSnapshot;
use regex::{Regex, RegexBuilder};

use crate::code_view::CodeView;
use crate::layout::color;
use crate::quick_open::view::wrap_index;
use crate::text_input::TextInput;
use crate::ui::{
    CHEVRON_DOWN_ICON, CHEVRON_UP_ICON, CLOSE_ICON, GRID, icon_button, popover, toggle_button,
};
use crate::{AppState, CommandRegistry, Workbench};

/// Snapshots larger than this are searched off the main thread.
pub const BACKGROUND_SEARCH_BYTES: usize = 1024 * 1024;

const FIND_WIDGET_WIDTH: f32 = 420.;

/// The byte ranges of every non-empty match of `pattern` in `snapshot`, in
/// order. The regex is built like the project search's
/// ([`build_pattern`]); `^` and `$` match at line ends. An invalid regex
/// gives its error message; an empty pattern finds nothing.
pub fn find_in_snapshot(
    snapshot: &BufferSnapshot,
    pattern: &str,
    is_regex: bool,
    case_sensitive: bool,
    whole_word: bool,
) -> Result<Vec<Range<usize>>, String> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let regex = build_regex(pattern, is_regex, case_sensitive, whole_word)?;
    let text = snapshot.text_for_range(0..snapshot.len_bytes());
    Ok(regex
        .find_iter(&text)
        .filter(|m| !m.is_empty())
        .map(|m| m.range())
        .collect())
}

/// The regex for a find or search query, as both build it: the source from
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
}

pub struct FindWidget {
    view: WeakEntity<CodeView>,
    input: Entity<TextInput>,
    /// The input's text.
    query: String,
    case_sensitive: bool,
    whole_word: bool,
    is_regex: bool,
    matches: Arc<[Range<usize>]>,
    active: Option<usize>,
    error: Option<String>,
    /// The view revision the matches were computed for.
    revision: u64,
    /// Bumped by every search; a background result applies only while
    /// its generation is still the latest.
    generation: u64,
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
        FindWidget {
            view: view.downgrade(),
            input,
            query: String::new(),
            case_sensitive: false,
            whole_word: false,
            is_regex: false,
            matches: Arc::from([]),
            active: None,
            error: None,
            revision: view.read(cx).revision(),
            generation: 0,
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

    pub fn matches(&self) -> &[Range<usize>] {
        &self.matches
    }

    /// Index of the active match.
    pub fn active_index(&self) -> Option<usize> {
        self.active
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
        self._search = Task::ready(());
    }

    /// Searches the view's current snapshot.
    fn search(&mut self, pick: Pick, cx: &mut Context<Self>) {
        self.cancel();
        let Some(view) = self.view.upgrade() else {
            return;
        };
        let (snapshot, revision) = {
            let view = view.read(cx);
            (view.snapshot().clone(), view.revision())
        };
        self.revision = revision;
        let pattern = self.query.clone();
        let (regex, case, word) = (self.is_regex, self.case_sensitive, self.whole_word);
        if snapshot.len_bytes() <= BACKGROUND_SEARCH_BYTES {
            let result = find_in_snapshot(&snapshot, &pattern, regex, case, word);
            self.apply(result, pick, cx);
            return;
        }
        let generation = self.generation;
        let result =
            cx.background_spawn(
                async move { find_in_snapshot(&snapshot, &pattern, regex, case, word) },
            );
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
        let last = self.matches.len().checked_sub(1);
        self.active = match (pick, self.active, last) {
            (_, _, None) => None,
            (Pick::Keep, Some(i), Some(last)) => Some(i.min(last)),
            _ => self.nearest(cx).or(Some(0)),
        };
        let matches = self.matches.clone();
        let active = self.active;
        if let Some(view) = self.view.upgrade() {
            view.update(cx, |v, cx| v.set_match_highlights(matches, active, cx));
        }
        if pick == Pick::Nearest {
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

    fn key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if ev.keystroke.key != "enter" || m.platform || m.control || m.alt {
            return;
        }
        if m.shift {
            self.previous(cx);
        } else {
            self.next(cx);
        }
        cx.stop_propagation();
    }
}

impl Focusable for FindWidget {
    fn focus_handle(&self, cx: &App) -> gpui::FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for FindWidget {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let muted = color(ui.text_muted);
        let view = self.view.clone();
        let toggle =
            |id, label, on, f: fn(&mut Self, &mut Context<Self>), cx: &mut Context<Self>| {
                toggle_button(id, label, on, cx)
                    .on_click(cx.listener(move |find, _, _, cx| f(find, cx)))
            };
        popover(cx)
            .id("find-widget")
            .debug_selector(|| "find-widget".into())
            .w(px(FIND_WIDGET_WIDTH))
            .occlude()
            .on_key_down(cx.listener(Self::key_down))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(GRID))
                    .h(px(8. * GRID))
                    .pl(px(2. * GRID))
                    .pr(px(GRID))
                    .child(div().flex_1().min_w_0().child(self.input.clone()))
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
                    ))
                    .child(
                        div()
                            .flex_none()
                            .min_w(px(18. * GRID))
                            .px(px(GRID))
                            .text_size(px(12.))
                            .text_color(muted)
                            .child(self.counter()),
                    )
                    .child(
                        icon_button("find-previous", CHEVRON_UP_ICON, false, cx)
                            .on_click(cx.listener(|find, _, _, cx| find.previous(cx))),
                    )
                    .child(
                        icon_button("find-next", CHEVRON_DOWN_ICON, false, cx)
                            .on_click(cx.listener(|find, _, _, cx| find.next(cx))),
                    )
                    .child(icon_button("find-close", CLOSE_ICON, false, cx).on_click(
                        move |_, window, cx| {
                            if let Some(view) = view.upgrade() {
                                close(&view, window, cx);
                            }
                        },
                    )),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .debug_selector(|| "find-error".into())
                    .px(px(2. * GRID))
                    .pb(px(2. * GRID))
                    .text_size(px(12.))
                    .text_color(muted)
                    .child(error)
            }))
    }
}

/// The selected text when the selection is on one line and not empty.
pub(crate) fn single_line_selection(view: &CodeView) -> Option<String> {
    let range = view.selection()?;
    if range.start.row != range.end.row {
        return None;
    }
    view.copy_selection().filter(|text| !text.is_empty())
}

/// Shows the find widget over `view`, pre-filled with a one-line
/// selection, and focuses its input.
fn open(view: &Entity<CodeView>, window: &mut Window, cx: &mut App) {
    let prefill = single_line_selection(view.read(cx));
    let find = match view.read(cx).find_widget_any().cloned() {
        Some(find) => find,
        None => cx.new(|cx| FindWidget::new(view, cx)),
    };
    view.update(cx, |v, cx| v.show_find_widget(find.clone(), cx));
    match prefill {
        Some(text) => find.update(cx, |find, cx| {
            find.input
                .update(cx, |input, cx| input.set_text(&text, window, cx))
        }),
        None => find.update(cx, |find, cx| find.search(Pick::Nearest, cx)),
    }
    let focus = find.focus_handle(cx);
    window.focus(&focus, cx);
}

/// Hides the find widget over `view`, clears its highlights and gives the
/// view focus.
fn close(view: &Entity<CodeView>, window: &mut Window, cx: &mut App) {
    if let Some(find) = view.read(cx).find_widget_any().cloned() {
        find.update(cx, |find, _| find.cancel());
    }
    view.update(cx, |v, cx| v.hide_find_widget(cx));
    let focus = view.focus_handle(cx);
    window.focus(&focus, cx);
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register("actions.find", "Find", |wb, _, window, cx| {
        if let Some(view) = wb.active_code_view(cx) {
            open(&view, window, cx);
        }
    });
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
    /// Whether the active code view shows its find widget.
    pub(crate) fn find_widget_visible(&self, cx: &App) -> bool {
        self.active_code_view(cx)
            .is_some_and(|view| view.read(cx).find_widget().is_some())
    }
}
