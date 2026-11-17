//! The multi-cursor commands of the code view (spec 4.3 §5.2): add or move
//! to the next find match (`⌘D`, `⌘K ⌘D`), select every match (`⇧⌘L`),
//! cursors above / below and at line ends, column selection and the two
//! Escape steps. The selections come from [`lucerna_editor::multicursor`];
//! this module keeps each command's session state, reveals, reports the
//! cursor limit and seeds the Find bar. The mouse (`⌥`-click, `⌥`-drag,
//! `⇧⌥`-drag) adds cursors through the same path.

use gpui::{App, AppContext as _, Context};
use lucerna_editor::multicursor::{self, ColumnMove, ColumnPos, MatchOptions, Step};
use lucerna_editor::{Goal, SelectionSet};

use super::frame_layout::FrameLayout;
use super::{CodeView, CodeViewEvent};
use crate::find_widget::BACKGROUND_SEARCH_BYTES;
use crate::status_message;
use crate::tasks::guarded;

/// `editor.multiCursorLimit` when the setting is missing.
const DEFAULT_CURSOR_LIMIT: usize = 10_000;

/// What scrolls into view after a multi-cursor command.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reveal {
    Nothing,
    Primary,
    Newest,
}

impl CodeView {
    /// `editor.multiCursorLimit` for this view's language, at least 1.
    pub fn multi_cursor_limit(&self, cx: &App) -> usize {
        self.settings
            .read(cx)
            .view(cx)
            .get::<usize>("editor.multiCursorLimit", self.language_id.as_deref())
            .unwrap_or(DEFAULT_CURSOR_LIMIT)
            .max(1)
    }

    /// `editor.multiCursorModifier` is `"ctrlCmd"`: ⌘ (Ctrl elsewhere)
    /// adds cursors with the mouse and ⌥ goes to definition. Otherwise
    /// (`"alt"`) the other way round.
    pub(super) fn add_cursor_with_secondary(&self, cx: &App) -> bool {
        self.settings
            .read(cx)
            .view(cx)
            .get::<String>("editor.multiCursorModifier", self.language_id.as_deref())
            .is_some_and(|m| m == "ctrlCmd")
    }

    /// The modifier of go to definition by mouse (⌘-hover underlines,
    /// ⌘-click jumps) is held: ⌥ when `editor.multiCursorModifier` is
    /// `"ctrlCmd"`, else the platform's secondary key.
    pub(crate) fn definition_modifier_held(&self, modifiers: &gpui::Modifiers, cx: &App) -> bool {
        if self.add_cursor_with_secondary(cx) {
            modifiers.alt
        } else {
            modifiers.secondary(cx)
        }
    }

    /// `⌥`-click: removes the selection touching `offset` (never the only
    /// one) or adds a cursor there. Returns the id of the cursor added.
    pub(super) fn mouse_toggle_cursor(
        &mut self,
        offset: usize,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let limit = self.multi_cursor_limit(cx);
        let before = self.selections.len();
        let set = multicursor::toggle_cursor_at(&self.selections, offset, limit);
        let limited = set.len() == before && set.len() >= limit;
        let added = (set.len() > before).then(|| set.primary().id);
        self.install(set, Reveal::Nothing, limited, cx);
        added
    }

    /// `⌥` double / triple-click: adds `anchor..head` as a new selection,
    /// in place of the cursor the first click of the series added. Returns
    /// its id, `None` at the cursor limit.
    pub(super) fn mouse_add_selection(
        &mut self,
        anchor: usize,
        head: usize,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let limit = self.multi_cursor_limit(cx);
        let primary = *self.selections.primary();
        let range = anchor.min(head)..=anchor.max(head);
        let base =
            if self.selections.len() > 1 && primary.is_empty() && range.contains(&primary.head) {
                multicursor::remove_ids(&self.selections, &[primary.id])
            } else {
                self.selections.clone()
            };
        let (set, id) = multicursor::add_selection(&base, anchor, head, limit);
        let added = id != multicursor::NO_SELECTION;
        self.install(set, Reveal::Nothing, !added, cx);
        added.then_some(id)
    }

    /// `⌥`-drag: the selection `id` now runs from `anchor` to `head`; the
    /// others stay. Nothing when it is gone.
    pub(super) fn mouse_update_selection(
        &mut self,
        id: u64,
        anchor: usize,
        head: usize,
        cx: &mut Context<Self>,
    ) {
        let mut sels = self.selections.all().to_vec();
        let Some(sel) = sels.iter_mut().find(|s| s.id == id) else {
            return;
        };
        if sel.anchor == anchor && sel.head == head {
            return;
        }
        sel.anchor = anchor;
        sel.head = head;
        sel.goal = Goal::None;
        let set = self.selections.rebuild(sels, id);
        self.install(set, Reveal::Nothing, false, cx);
    }

    /// Drops the state of every multi-cursor session.
    pub(super) fn clear_multi_cursor_state(&mut self) {
        self.match_session = None;
        self.cursor_layers.clear();
        self.column_select = None;
    }

    /// The Find bar's case and whole-word toggles (off without one), for
    /// matching text the user selected.
    fn find_options(&self, cx: &App) -> MatchOptions {
        self.find_widget_any().map_or(
            MatchOptions {
                case_sensitive: false,
                whole_word: false,
            },
            |find| find.read(cx).match_options(),
        )
    }

    /// Installs `set` from a multi-cursor command as a cursor move (so
    /// `cursorUndo` restores the old one), scrolls as `reveal` says,
    /// reports a cut at the cursor limit and emits
    /// [`CodeViewEvent::MultiCursor`]. Every session ends here; the
    /// command stores its own state again afterwards.
    fn install(
        &mut self,
        set: SelectionSet,
        reveal: Reveal,
        limited: bool,
        cx: &mut Context<Self>,
    ) {
        self.clear_multi_cursor_state();
        self.multi_cursor_running = true;
        self.set_selections(set, reveal == Reveal::Primary, cx);
        self.multi_cursor_running = false;
        if reveal == Reveal::Newest {
            let head = self.selections.newest().head;
            let row = self.snapshot.offset_to_point(head).row;
            self.reveal_row(row);
        }
        if limited {
            self.show_cursor_limit(cx);
        }
        cx.emit(CodeViewEvent::MultiCursor);
    }

    /// `The number of cursors has been limited to N.` in the window's
    /// status bar.
    fn show_cursor_limit(&self, cx: &mut Context<Self>) {
        let limit = self.multi_cursor_limit(cx);
        let window = self
            .documents
            .as_ref()
            .and_then(|d| d.upgrade())
            .and_then(|d| d.read(cx).window());
        status_message::show_or_log(
            window,
            format!("The number of cursors has been limited to {limit}."),
            cx,
        );
    }

    /// `editor.action.addSelectionToNextFindMatch` (`⌘D`), or with
    /// `backwards` `addSelectionToPreviousFindMatch`: adds the next match
    /// of the selected text, or selects the word under each cursor.
    pub fn add_next_match(&mut self, backwards: bool, cx: &mut Context<Self>) {
        let find = self.find_options(cx);
        let limit = self.multi_cursor_limit(cx);
        let session = self.match_session.clone();
        let step = self.contained(
            "add match",
            |v| {
                multicursor::add_match(
                    &v.snapshot,
                    &v.selections,
                    session.as_ref(),
                    find,
                    &v.config.word_separators,
                    backwards,
                    limit,
                )
            },
            cx,
        );
        if let Some(step) = step.flatten() {
            self.finish_match_step(step, cx);
        }
    }

    /// `editor.action.moveSelectionToNextFindMatch` (`⌘K ⌘D`), or with
    /// `backwards` `moveSelectionToPreviousFindMatch`: the newest
    /// selection moves to the next match.
    pub fn move_next_match(&mut self, backwards: bool, cx: &mut Context<Self>) {
        let find = self.find_options(cx);
        let session = self.match_session.clone();
        let step = self.contained(
            "move match",
            |v| {
                multicursor::move_match(
                    &v.snapshot,
                    &v.selections,
                    session.as_ref(),
                    find,
                    &v.config.word_separators,
                    backwards,
                )
            },
            cx,
        );
        if let Some(step) = step.flatten() {
            self.finish_match_step(step, cx);
        }
    }

    /// Installs a `⌘D` / `⌘K ⌘D` step, keeps its session and puts a word
    /// it selected into the open Find bar.
    fn finish_match_step(&mut self, step: Step, cx: &mut Context<Self>) {
        self.install(step.set, Reveal::Newest, step.limited, cx);
        self.match_session = step.session;
        if let (Some(word), Some(find)) = (step.seeded_word, self.find_widget().cloned()) {
            // The widget reads this view while it searches: after this
            // update.
            cx.defer(move |cx| find.update(cx, |find, cx| find.seed_query(&word, cx)));
        }
    }

    /// `editor.action.selectHighlights` (`⇧⌘L`) and `changeAll` (`⌘F2`):
    /// selects every match of the selected text or the word under the
    /// cursor. A file over [`BACKGROUND_SEARCH_BYTES`] is searched in the
    /// background; the result is dropped if the text or the selections
    /// change meanwhile. Does not scroll.
    pub fn select_all_matches(&mut self, cx: &mut Context<Self>) {
        let find = self.find_options(cx);
        let limit = self.multi_cursor_limit(cx);
        let session = self.match_session.clone();
        let separators = self.config.word_separators.clone();
        if self.snapshot.len_bytes() <= BACKGROUND_SEARCH_BYTES {
            let step = self.contained(
                "select all matches",
                |v| {
                    multicursor::select_all_matches(
                        &v.snapshot,
                        &v.selections,
                        session.as_ref(),
                        find,
                        &separators,
                        limit,
                    )
                },
                cx,
            );
            if let Some(step) = step.flatten() {
                self.finish_select_all(step, cx);
            }
            return;
        }
        let snapshot = self.snapshot.clone();
        let set = self.selections.clone();
        let (revision, generation) = (self.revision, self.selection_generation);
        let search = cx.background_spawn(async move {
            guarded("select all matches", || {
                multicursor::select_all_matches(
                    &snapshot,
                    &set,
                    session.as_ref(),
                    find,
                    &separators,
                    limit,
                )
            })
            .flatten()
        });
        self._select_all_task = Some(cx.spawn(async move |view, cx| {
            let step = search.await;
            view.update(cx, |v, cx| {
                if v.revision == revision
                    && v.selection_generation == generation
                    && let Some(step) = step
                {
                    v.finish_select_all(step, cx);
                }
            })
            .ok();
        }));
    }

    fn finish_select_all(&mut self, step: Step, cx: &mut Context<Self>) {
        self.install(step.set, Reveal::Nothing, step.limited, cx);
        self.match_session = step.session;
    }

    /// `editor.action.insertCursorAbove` / `insertCursorBelow` (`⌥⌘↑` /
    /// `⌥⌘↓`): a cursor one display line above (`up`) or below each
    /// selection. Right after the opposite key it removes the cursors
    /// that key added instead.
    pub fn insert_cursor_vertical(&mut self, up: bool, cx: &mut Context<Self>) {
        let mut layers = std::mem::take(&mut self.cursor_layers);
        if let Some((direction, ids)) = layers.last()
            && *direction != up
        {
            let set = multicursor::remove_ids(&self.selections, ids);
            layers.pop();
            self.install(set, Reveal::Newest, false, cx);
            self.cursor_layers = layers;
            return;
        }
        let limit = self.multi_cursor_limit(cx);
        let result = self.contained(
            "insert cursors",
            |v| {
                let layout = FrameLayout {
                    folds: &v.folds,
                    wrap_chars: v.wrap_chars(),
                    tab_size: v.config.tab_size,
                    page_rows: v.visible_rows().saturating_sub(1).max(1),
                };
                multicursor::insert_cursors_vertical(&v.snapshot, &v.selections, up, &layout, limit)
            },
            cx,
        );
        let Some((set, added, limited)) = result else {
            self.cursor_layers = layers;
            return;
        };
        if added.is_empty() && !limited {
            self.cursor_layers = layers;
            return;
        }
        if !added.is_empty() {
            layers.push((up, added));
        }
        self.install(set, Reveal::Newest, limited, cx);
        self.cursor_layers = layers;
    }

    /// `editor.action.insertCursorAtEndOfEachLineSelected` (`⇧⌥I`): a
    /// cursor at the end of every selected line.
    pub fn cursors_at_line_ends(&mut self, cx: &mut Context<Self>) {
        let limit = self.multi_cursor_limit(cx);
        let result = self.contained(
            "cursors at line ends",
            |v| multicursor::cursors_at_line_ends(&v.snapshot, &v.selections, limit),
            cx,
        );
        if let Some((set, limited)) = result.flatten() {
            self.install(set, Reveal::Nothing, limited, cx);
        }
    }

    /// `cursorColumnSelectLeft` / `Right` / `Up` / `Down` / `PageUp` /
    /// `PageDown` (`⇧⌥⌘` + key): grows a column selection from the primary
    /// selection, or the one these keys made last.
    pub fn column_select_move(&mut self, mv: ColumnMove, cx: &mut Context<Self>) {
        let (from, to) = self.column_select.unwrap_or_else(|| {
            let primary = self.selections.primary();
            (
                self.column_pos(primary.anchor),
                self.column_pos(primary.head),
            )
        });
        let page_rows = self.visible_rows().saturating_sub(1).max(1);
        let to = multicursor::column_move(&self.snapshot, from, to, mv, page_rows);
        self.column_selection(from, to, Reveal::Primary, cx);
    }

    /// Selects the column (box) from `from` to `to`: one selection per row,
    /// by visual column (tabs at full width). For `⇧⌥`-drag.
    pub fn set_column_selection(&mut self, from: ColumnPos, to: ColumnPos, cx: &mut Context<Self>) {
        self.column_selection(from, to, Reveal::Nothing, cx);
    }

    fn column_selection(
        &mut self,
        from: ColumnPos,
        to: ColumnPos,
        reveal: Reveal,
        cx: &mut Context<Self>,
    ) {
        let limit = self.multi_cursor_limit(cx);
        let result = self.contained(
            "column select",
            |v| multicursor::column_select(&v.snapshot, from, to, v.config.tab_size, limit),
            cx,
        );
        if let Some((set, limited)) = result {
            self.install(set, reveal, limited, cx);
            self.column_select = Some((from, to));
        }
    }

    /// The row and visual column of `offset`.
    pub(super) fn column_pos(&self, offset: usize) -> ColumnPos {
        ColumnPos {
            row: self.snapshot.offset_to_point(offset).row,
            col: multicursor::visual_column(&self.snapshot, offset, self.config.tab_size),
        }
    }

    /// `removeSecondaryCursors` (first Escape): only the primary selection
    /// stays.
    pub fn remove_secondary_cursors(&mut self, cx: &mut Context<Self>) {
        if let Some(set) = multicursor::remove_secondary(&self.selections) {
            self.set_selections(set, true, cx);
        }
    }

    /// `cancelSelection` (second Escape): the lone selection collapses to
    /// its cursor.
    pub fn cancel_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(set) = multicursor::cancel_selection(&self.selections) {
            self.set_selections(set, true, cx);
        }
    }
}
