//! The bridge between the gpui-free vim state machine (`lucerna-vim`) and
//! the workbench: it routes keys to the [`Vim`] of the active
//! [`CodeView`], carries out the resulting [`Action`]s on the view and
//! keeps both in step when the view changes on its own (mouse, go to,
//! reload).

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, LazyLock};

use gpui::{
    App, AppContext as _, ClipboardItem, Context, Entity, EntityId, KeyDownEvent, Subscription,
    Task, WeakEntity, Window,
};
use lucerna_settings::{Context as WhenContext, WhenExpr, parse_when};
use lucerna_text::{BufferSnapshot, Point, Reload};
use lucerna_vim::{
    Action, FoldAction, Key, Mode, Outcome, SearchQuery, Settings, ViewInfo, Vim, VimGlobals,
    VimSelection, block_ranges, exclusive_range,
};

use crate::AppState;
use crate::code_view::{CodeView, CodeViewEvent, CursorOverlay, CursorShape, FoldOp};
use crate::documents::DocState;
use crate::editor_groups::EditorGroupsView;
use crate::find_widget::{
    self, BACKGROUND_SEARCH_BYTES, FindWidget, FindWidgetEvent, find_in_snapshot,
};
use crate::tasks::{catch_panic, guarded};

/// Where keys may reach vim: the editor has focus, and neither a text
/// input (find widget) nor Quick Open does.
static KEY_GATE: LazyLock<WhenExpr> = LazyLock::new(|| {
    parse_when("editorFocus && !textInputFocus && !inQuickOpen").expect("a valid when clause")
});

/// The vim state of one code view.
struct ViewState {
    vim: Vim,
    /// To clear vim's cursor and selection when vim is switched off.
    view: WeakEntity<CodeView>,
    /// The view's `selection_generation` after vim last wrote to it; any
    /// other value means someone else moved the selection.
    generation: u64,
    _subscriptions: [Subscription; 3],
}

/// The matches of one query in one revision of a view's text.
struct MatchCache {
    revision: u64,
    query: SearchQuery,
    matches: Arc<[Range<usize>]>,
}

/// A `/` or `?` whose find widget is open.
struct VimSearch {
    view: WeakEntity<CodeView>,
    find: WeakEntity<FindWidget>,
    backward: bool,
    /// Vim's cursor when the widget opened: the search starts there, and
    /// the cursor goes back there when the widget is dismissed.
    origin: Point,
    _sub: Subscription,
}

/// What to run again once a background search has found the matches vim
/// asked for.
enum Redo {
    Key(Key),
    Search {
        query: SearchQuery,
        backward: bool,
        origin: Point,
    },
}

/// How [`VimLayer::sync_view`] brings the cursor into view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reveal {
    /// Leave the scroll position alone.
    None,
    /// Scroll just enough (`editor.cursorSurroundingLines` included).
    Scroll,
    /// Centre the cursor when it is off-screen, else as `Scroll`.
    Jump,
}

pub struct VimLayer {
    enabled: bool,
    settings: Settings,
    globals: VimGlobals,
    views: HashMap<EntityId, ViewState>,
    match_cache: HashMap<EntityId, MatchCache>,
    /// The last reload whose marks were remapped, per file: every view of
    /// a file reports the same reload, but the marks move only once.
    last_remap: HashMap<Arc<Path>, (Reload, u32)>,
    /// Bumped by every key; a background search re-feeds its key only
    /// while no other key arrived.
    key_generation: u64,
    _retry: Option<Task<()>>,
    /// The open `/` or `?`, if any.
    search: Option<VimSearch>,
    /// Set by [`Action::OpenSearch`]: the widget needs the window, which
    /// [`VimLayer::handle_key`] has.
    open_search: Option<bool>,
    /// The editor groups whose code views vim follows, set by
    /// [`VimLayer::attach`].
    groups: Option<WeakEntity<EditorGroupsView>>,
    _new_views: Option<Subscription>,
    _settings: Subscription,
}

impl VimLayer {
    /// Reads the settings from [`AppState`] and follows their changes.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (enabled, settings) = read_settings(cx);
        VimLayer {
            enabled,
            settings,
            globals: VimGlobals::default(),
            views: HashMap::new(),
            match_cache: HashMap::new(),
            last_remap: HashMap::new(),
            key_generation: 0,
            _retry: None,
            search: None,
            open_search: None,
            groups: None,
            _new_views: None,
            _settings: cx.observe_global::<AppState>(Self::settings_changed),
        }
    }

    /// `vim.enabled`, or the `vim_mode` alias when that is unset.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The mode of the code view `view`: Normal until it got a key;
    /// `None` while vim is off.
    pub fn mode_for(&self, view: EntityId) -> Option<Mode> {
        if !self.enabled {
            return None;
        }
        Some(self.views.get(&view).map_or(Mode::Normal, |s| s.vim.mode()))
    }

    /// The mode indicator for the code view `view`, e.g. `-- NORMAL --`
    /// or, with keys pending, `-- NORMAL --  2f`; `None` while vim is off.
    pub fn indicator_text(&self, view: EntityId) -> Option<String> {
        let (label, pending) = self.indicator_parts(view)?;
        Some(if pending.is_empty() {
            label.to_string()
        } else {
            format!("{label}  {pending}")
        })
    }

    /// The mode label and the pending keys of the indicator.
    pub(crate) fn indicator_parts(&self, view: EntityId) -> Option<(&'static str, &str)> {
        let mode = self.mode_for(view)?;
        let pending = self.views.get(&view).map_or("", |s| s.vim.pending());
        Some((mode.label(), pending))
    }

    /// Feeds `ev` to the vim of `view`. Returns true when the key was
    /// consumed.
    pub(crate) fn handle_key(
        &mut self,
        ev: &KeyDownEvent,
        view: &Entity<CodeView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        let Some(key) = to_vim_key(ev) else {
            return false;
        };
        self.key_generation += 1;
        self._retry = None;
        self.open_search = None;
        let consumed = self.feed(key, view, true, cx);
        if let Some(backward) = self.open_search.take() {
            self.start_search(view, backward, window, cx);
        }
        consumed
    }

    /// `/` or `?`: opens the find widget of `view` for vim.
    fn start_search(
        &mut self,
        view: &Entity<CodeView>,
        backward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.end_search(cx);
        let Some(origin) = self.views.get(&view.entity_id()).map(|s| s.vim.cursor()) else {
            return;
        };
        let find = find_widget::open_for_vim(view, window, cx);
        let sub = cx.subscribe_in(&find, window, Self::search_event);
        self.search = Some(VimSearch {
            view: view.downgrade(),
            find: find.downgrade(),
            backward,
            origin,
            _sub: sub,
        });
    }

    /// Forgets the open search; its widget, if still shown, becomes a
    /// plain find widget.
    fn end_search(&mut self, cx: &mut App) {
        if let Some(find) = self.search.take().and_then(|s| s.find.upgrade()) {
            find.update(cx, |find, _| find.set_vim(false));
        }
    }

    /// Whether a `/` or `?` is open over `view`.
    fn searching(&self, view: EntityId) -> bool {
        self.search
            .as_ref()
            .is_some_and(|s| s.view.entity_id() == view)
    }

    /// Enter or Esc in the widget of the open search.
    fn search_event(
        &mut self,
        find: &Entity<FindWidget>,
        event: &FindWidgetEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search.as_ref().map(|s| s.find.entity_id()) != Some(find.entity_id()) {
            return;
        }
        let Some(search) = self.search.take() else {
            return;
        };
        let Some(view) = search.view.upgrade() else {
            return;
        };
        let id = view.entity_id();
        // The incremental search moved the view's selection; that was not
        // a change from outside.
        if let Some(state) = self.views.get_mut(&id) {
            state.generation = view.read(cx).selection_generation();
        }
        match event {
            FindWidgetEvent::Submitted => {
                let query = find.read(cx).query();
                // No `Dismissed` from this close.
                find.update(cx, |find, _| find.set_vim(false));
                find_widget::close(&view, window, cx);
                self.key_generation += 1;
                self._retry = None;
                let redo = Redo::Search {
                    query,
                    backward: search.backward,
                    origin: search.origin,
                };
                self.run(redo, &view, true, cx);
            }
            FindWidgetEvent::Dismissed => {
                if let Some(state) = self.views.get_mut(&id) {
                    state.vim.set_cursor(search.origin);
                    // The origin may be past the text after a reload.
                    state.vim.clamp(view.read(cx).snapshot());
                }
                self.sync_view(&view, Reveal::Scroll, cx);
                cx.notify();
            }
        }
    }

    /// Follows the code views of `groups`: each gets its vim state (and
    /// its block cursor) as soon as it is made, so marks follow reloads
    /// and the cursor shows before the first key.
    pub(crate) fn attach(&mut self, groups: &Entity<EditorGroupsView>, cx: &mut Context<Self>) {
        self.groups = Some(groups.downgrade());
        let this = cx.weak_entity();
        self._new_views = Some(cx.observe_new::<CodeView>(move |_, _, cx| {
            let (this, view) = (this.clone(), cx.entity());
            // The view is being made (inside the groups' render) and
            // cannot be read yet.
            cx.defer(move |cx| {
                this.update(cx, |this, cx| {
                    if this.enabled && this.is_ours(&view, cx) {
                        this.ensure_state(&view, cx);
                    }
                })
                .ok();
            });
        }));
        self.adopt_all(cx);
    }

    /// The code views of every drawn tab of the attached groups.
    fn group_views(&self, cx: &App) -> Vec<Entity<CodeView>> {
        let Some(groups) = self.groups.as_ref().and_then(WeakEntity::upgrade) else {
            return Vec::new();
        };
        let groups = groups.read(cx);
        groups
            .model()
            .groups()
            .iter()
            .flat_map(|g| g.tabs.iter())
            .filter_map(|tab| groups.body_of(tab.id))
            .filter_map(|body| body.downcast::<CodeView>().ok())
            .collect()
    }

    /// Whether `view` is a tab body of the attached groups (not, say, a
    /// view of another window).
    fn is_ours(&self, view: &Entity<CodeView>, cx: &App) -> bool {
        self.group_views(cx).iter().any(|v| v == view)
    }

    /// Gives every code view of the attached groups its state.
    fn adopt_all(&mut self, cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }
        for view in self.group_views(cx) {
            self.ensure_state(&view, cx);
        }
    }

    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let (enabled, settings) = read_settings(cx);
        self.settings = settings;
        if enabled == self.enabled {
            return;
        }
        self.enabled = enabled;
        if !enabled {
            self._retry = None;
            self.end_search(cx);
            self.match_cache.clear();
            self.last_remap.clear();
            // Marks are not kept up to date while off.
            self.globals = VimGlobals::default();
            for (_, state) in self.views.drain() {
                let Some(view) = state.view.upgrade() else {
                    continue;
                };
                view.update(cx, |v, cx| {
                    v.set_cursor_overlay(None, cx);
                    v.set_block_selection(None, cx);
                });
            }
        } else {
            // Every view starts in Normal mode with its block cursor.
            self.adopt_all(cx);
        }
        cx.notify();
    }

    /// Creates the state of `view` (when it is made, when vim is switched
    /// on, or at its first key) and draws its cursor.
    fn ensure_state(&mut self, view: &Entity<CodeView>, cx: &mut Context<Self>) {
        let id = view.entity_id();
        if self.views.contains_key(&id) {
            return;
        }
        let (cursor, generation) = {
            let v = view.read(cx);
            (v.cursor(), v.selection_generation())
        };
        let subscriptions = [
            cx.subscribe(view, Self::view_event),
            cx.observe(view, Self::view_changed),
            cx.observe_release(view, move |this, _, _| {
                this.views.remove(&id);
                this.match_cache.remove(&id);
            }),
        ];
        let mut vim = Vim::new(cursor);
        vim.clamp(view.read(cx).snapshot());
        let at = vim.cursor();
        if takes_keys(view.read(cx)) {
            view.update(cx, |v, cx| v.set_cursor_overlay(Some(block_cursor(at)), cx));
        }
        self.views.insert(
            id,
            ViewState {
                vim,
                view: view.downgrade(),
                generation,
                _subscriptions: subscriptions,
            },
        );
    }

    /// A reload: marks follow the moved lines, the cursor is kept on the
    /// text and the selection is drawn again.
    ///
    /// `CodeView::apply_reload` bumps the selection generation and emits
    /// `Reloaded` before it notifies, so this runs (and stores the new
    /// generation) before [`VimLayer::view_changed`] would take the bump
    /// for an outside change and drop the visual mode.
    fn view_event(
        &mut self,
        view: Entity<CodeView>,
        event: &CodeViewEvent,
        cx: &mut Context<Self>,
    ) {
        let CodeViewEvent::Reloaded { reload } = *event;
        let id = view.entity_id();
        self.match_cache.remove(&id);
        let v = view.read(cx);
        let snapshot = v.snapshot();
        let key = (reload, snapshot.line_count());
        if self.last_remap.get(v.path()) != Some(&key) {
            self.globals.remap_marks(v.path(), &reload, snapshot);
            self.last_remap.insert(v.path().clone(), key);
        }
        // The open `/` starts from (or goes back to) its origin on the
        // same line; the column is clamped where the origin is used.
        if let Some(search) = self.search.as_mut()
            && search.view.entity_id() == id
        {
            search.origin.row = reload.adjust_row(search.origin.row, snapshot.line_count());
        }
        let Some(state) = self.views.get_mut(&id) else {
            return;
        };
        if state.vim.mode() == Mode::Normal {
            // Like the view's own cursor, it stays on its line.
            let at = state.vim.cursor();
            state.vim.set_cursor(Point {
                row: reload.adjust_row(at.row, snapshot.line_count()),
                column: at.column,
            });
        }
        state.vim.clamp(snapshot);
        if state.vim.mode() == Mode::Normal {
            // A selection the view kept (its rows did not change) moved
            // the same way as the cursor and stays: it may be one made with
            // the mouse. A dropped one is replaced by vim's cursor, so the
            // view's cursor (status bar, find) matches the block cursor.
            let at = state.vim.cursor();
            let kept = v.selection().is_some();
            state.generation = view.update(cx, |v, cx| {
                if !kept {
                    v.select(at..at, cx);
                }
                v.set_cursor_overlay(Some(block_cursor(at)), cx);
                v.selection_generation()
            });
        } else {
            self.sync_view(&view, Reveal::None, cx);
        }
        cx.notify();
    }

    /// The selection was changed outside vim (a click, go to): back to
    /// Normal mode at the view's cursor.
    fn view_changed(&mut self, view: Entity<CodeView>, cx: &mut Context<Self>) {
        // The find widget of `/` moves the selection while the user types.
        if self.searching(view.entity_id()) {
            return;
        }
        let Some(state) = self.views.get_mut(&view.entity_id()) else {
            return;
        };
        let v = view.read(cx);
        let ready = takes_keys(v);
        if v.selection_generation() != state.generation {
            state.vim.reset(v.cursor());
            state.vim.clamp(v.snapshot());
            state.generation = v.selection_generation();
        } else if !ready || v.cursor_overlay().is_some() {
            return;
        }
        // Drawn once the document is loaded. The selection the user made
        // stays; only the cursor is drawn.
        if ready {
            let at = state.vim.cursor();
            view.update(cx, |v, cx| v.set_cursor_overlay(Some(block_cursor(at)), cx));
        }
        cx.notify();
    }

    /// Feeds `key`; `may_retry` allows one search for a [`Outcome::Retry`].
    fn feed(
        &mut self,
        key: Key,
        view: &Entity<CodeView>,
        may_retry: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        self.run(Redo::Key(key), view, may_retry, cx)
    }

    /// Feeds a key or runs a submitted search; `may_retry` allows one
    /// search for a [`Outcome::Retry`].
    fn run(
        &mut self,
        redo: Redo,
        view: &Entity<CodeView>,
        may_retry: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let id = view.entity_id();
        self.ensure_state(view, cx);
        if matches!(redo, Redo::Key(_)) && self.searching(id) {
            // A key reached the view while its `/` widget is open: the
            // user left the widget, so the search is over and the
            // selection the view has now is the user's.
            self.end_search(cx);
        }
        let outcome = {
            let v = view.read(cx);
            let Some(state) = self.views.get_mut(&id) else {
                return false;
            };
            if v.selection_generation() != state.generation {
                state.vim.reset(v.cursor());
                state.generation = v.selection_generation();
            }
            let info = Info {
                view: v,
                cache: self.match_cache.get(&id),
            };
            let (vim, globals, settings) = (&mut state.vim, &mut self.globals, &self.settings);
            match &redo {
                Redo::Key(key) => {
                    catch_panic("vim key", || vim.feed(*key, &info, globals, settings))
                }
                Redo::Search {
                    query,
                    backward,
                    origin,
                } => catch_panic("vim search", || {
                    vim.search_submitted(
                        query.clone(),
                        *backward,
                        *origin,
                        &info,
                        globals,
                        settings,
                    )
                }),
            }
        };
        let submitted = matches!(redo, Redo::Search { .. });
        match outcome {
            Err(_) => {
                if let Some(state) = self.views.get_mut(&id) {
                    let v = view.read(cx);
                    state.vim.reset(v.cursor());
                    state.vim.clamp(v.snapshot());
                }
                self.sync_view(view, Reveal::None, cx);
                cx.notify();
                true
            }
            Ok(Outcome::Unhandled) => {
                // A pending prefix may have been dropped.
                cx.notify();
                false
            }
            Ok(Outcome::Retry(query)) => {
                if may_retry {
                    self.search_then_run(redo, view, query, cx);
                } else {
                    tracing::warn!("vim: matches still missing after a search");
                }
                true
            }
            Ok(Outcome::Handled(actions)) => {
                let synced = self.apply(view, actions, cx);
                if submitted && !synced {
                    // No match: the view's selection is where the
                    // incremental search left it.
                    self.sync_view(view, Reveal::Scroll, cx);
                }
                true
            }
        }
    }

    /// Finds the matches of `query` vim asked for, then runs `redo` again:
    /// at once for a small file, after a background search for a large
    /// one (unless another key came first).
    fn search_then_run(
        &mut self,
        redo: Redo,
        view: &Entity<CodeView>,
        query: SearchQuery,
        cx: &mut Context<Self>,
    ) {
        let id = view.entity_id();
        let (snapshot, revision) = {
            let v = view.read(cx);
            (v.snapshot().clone(), v.revision())
        };
        if snapshot.len_bytes() <= BACKGROUND_SEARCH_BYTES {
            let matches = search(&snapshot, &query);
            self.match_cache.insert(
                id,
                MatchCache {
                    revision,
                    query,
                    matches,
                },
            );
            self.run(redo, view, false, cx);
            return;
        }
        let generation = self.key_generation;
        let pattern = query.clone();
        let result = cx.background_spawn(async move {
            guarded("vim search", || search(&snapshot, &pattern)).unwrap_or_else(|| Arc::from([]))
        });
        let view = view.downgrade();
        self._retry = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let matches = result.await;
            this.update(cx, |this, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                if this.key_generation != generation || !this.enabled {
                    return;
                }
                this.match_cache.insert(
                    id,
                    MatchCache {
                        revision,
                        query,
                        matches,
                    },
                );
                this.run(redo, &view, false, cx);
            })
            .ok();
        }));
    }

    /// Carries out vim's actions on `view`, then draws vim's cursor and
    /// selection. Returns false when only the cursor was drawn and the
    /// view's selection was left as it was.
    fn apply(
        &mut self,
        view: &Entity<CodeView>,
        actions: Vec<Action>,
        cx: &mut Context<Self>,
    ) -> bool {
        let id = view.entity_id();
        let mut reveal = None;
        let mut redraw = false;
        for action in actions {
            match action {
                Action::ScrollTopRow(row) => {
                    view.update(cx, |v, cx| v.set_scroll_top_row(row, cx));
                }
                Action::CursorMoved { jump } => {
                    let this = if jump { Reveal::Jump } else { Reveal::Scroll };
                    reveal = Some(reveal.map_or(this, |r: Reveal| if jump { this } else { r }));
                }
                Action::Yank(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                Action::Highlight(query) => {
                    if let Some(matches) = self.matches_for(view, &query, cx) {
                        view.update(cx, |v, cx| v.set_match_highlights(matches, None, cx));
                    }
                }
                Action::Fold { row, action } => {
                    let op = match action {
                        FoldAction::Toggle => FoldOp::Toggle,
                        FoldAction::Close => FoldOp::Close,
                        FoldAction::Open => FoldOp::Open,
                    };
                    let header = view.update(cx, |v, cx| v.fold_at(row, op, cx));
                    let v = view.read(cx);
                    if let (Some(header), Some(state)) = (header, self.views.get_mut(&id))
                        && v.is_row_hidden(state.vim.cursor().row)
                    {
                        state.vim.set_cursor(Point {
                            row: header,
                            column: 0,
                        });
                        state.vim.clamp(v.snapshot());
                    }
                    redraw = true;
                }
                Action::ModeChanged(_) => redraw = true,
                Action::OpenSearch { backward } => self.open_search = Some(backward),
            }
        }
        let synced = match (reveal, redraw) {
            (Some(reveal), _) => {
                self.sync_view(view, reveal, cx);
                true
            }
            (None, true) => {
                self.sync_view(view, Reveal::Scroll, cx);
                true
            }
            // A count or a prefix: the selection stays, the cursor is drawn.
            (None, false) => {
                if let Some(state) = self.views.get(&id) {
                    let at = state.vim.cursor();
                    view.update(cx, |v, cx| v.set_cursor_overlay(Some(block_cursor(at)), cx));
                }
                false
            }
        };
        cx.notify();
        synced
    }

    /// The matches of `query` in `view`: cached, or found now when the
    /// file is small enough.
    fn matches_for(
        &mut self,
        view: &Entity<CodeView>,
        query: &SearchQuery,
        cx: &App,
    ) -> Option<Arc<[Range<usize>]>> {
        let v = view.read(cx);
        let id = view.entity_id();
        if let Some(cache) = self.match_cache.get(&id)
            && cache.revision == v.revision()
            && cache.query == *query
        {
            return Some(cache.matches.clone());
        }
        if v.snapshot().len_bytes() > BACKGROUND_SEARCH_BYTES {
            return None;
        }
        let matches = search(v.snapshot(), query);
        self.match_cache.insert(
            id,
            MatchCache {
                revision: v.revision(),
                query: query.clone(),
                matches: matches.clone(),
            },
        );
        Some(matches)
    }

    /// Draws vim's cursor and selection in `view` and remembers the
    /// selection generation that results.
    fn sync_view(&mut self, view: &Entity<CodeView>, reveal: Reveal, cx: &mut Context<Self>) {
        let Some(state) = self.views.get_mut(&view.entity_id()) else {
            return;
        };
        let vim = &state.vim;
        let at = vim.cursor();
        state.generation = view.update(cx, |v, cx| {
            let selection = vim.selection(v.snapshot(), v.tab_size());
            // Every selection change drops a block selection, so the
            // cursor goes first.
            match reveal {
                Reveal::Jump if !row_on_screen(v, at.row) => v.reveal_centered(at, cx),
                Reveal::Jump | Reveal::Scroll => v.set_cursor(at, cx),
                Reveal::None => v.select(at..at, cx),
            }
            match selection {
                None => {}
                Some(sel @ VimSelection::Block { .. }) => {
                    let ranges = block_ranges(&sel, v.snapshot(), v.tab_size());
                    v.set_block_selection(Some(ranges), cx);
                }
                Some(sel) => {
                    if let Some(range) = exclusive_range(&sel, v.snapshot()) {
                        v.select(range, cx);
                    }
                }
            }
            v.set_cursor_overlay(Some(block_cursor(at)), cx);
            v.selection_generation()
        });
    }
}

/// Feeds `ev` to vim when vim is on, the editor has focus (not a text
/// input or Quick Open), no keymap chord is pending, the key has no cmd
/// and the active tab is a loaded code view. Returns true when vim
/// consumed the key.
pub(crate) fn dispatch(
    vim: &Entity<VimLayer>,
    groups: &Entity<EditorGroupsView>,
    ev: &KeyDownEvent,
    ctx: &WhenContext,
    chord_pending: bool,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    if chord_pending
        || ev.keystroke.modifiers.platform
        || !vim.read(cx).enabled()
        || !KEY_GATE.eval(ctx)
    {
        return false;
    }
    let Some(view) = groups
        .read(cx)
        .active_body()
        .and_then(|body| body.downcast::<CodeView>().ok())
    else {
        return false;
    };
    if !takes_keys(view.read(cx)) {
        return false;
    }
    vim.update(cx, |vim, cx| vim.handle_key(ev, &view, window, cx))
}

/// The key as vim sees it; `None` for keys vim never takes.
pub(crate) fn to_vim_key(ev: &KeyDownEvent) -> Option<Key> {
    let stroke = &ev.keystroke;
    let mods = stroke.modifiers;
    // cmd and alt combinations (alt+z, alt+arrows...) are the keymap's.
    if mods.platform || mods.alt || mods.function {
        return None;
    }
    let plain = !mods.control && !mods.shift;
    let named = match stroke.key.as_str() {
        "escape" => Some(Key::Escape),
        "enter" => Some(Key::Enter),
        "left" => Some(Key::Char('h')),
        "down" => Some(Key::Char('j')),
        "up" => Some(Key::Char('k')),
        "right" => Some(Key::Char('l')),
        _ => None,
    };
    if named.is_some() {
        return named.filter(|_| plain);
    }
    if mods.control {
        if mods.shift {
            return None;
        }
        let mut chars = stroke.key.chars();
        return match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_alphabetic() => Some(Key::Ctrl(c.to_ascii_lowercase())),
            _ => None,
        };
    }
    let mut chars = stroke.key_char.as_deref()?.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(Key::Char(c)),
        _ => None,
    }
}

/// Whether vim takes keys in `view`: its document is loaded (or gone from
/// disk with its text still shown).
pub(crate) fn takes_keys(view: &CodeView) -> bool {
    matches!(view.doc_state(), DocState::Ready | DocState::Deleted)
}

fn read_settings(cx: &App) -> (bool, Settings) {
    let store = &cx.global::<AppState>().settings;
    let enabled = store
        .get::<bool>("vim.enabled", None)
        .or_else(|| store.get::<bool>("vim_mode", None))
        .unwrap_or(false);
    let defaults = Settings::default();
    let settings = Settings {
        ignorecase: store
            .get("vim.ignorecase", None)
            .unwrap_or(defaults.ignorecase),
        wrapscan: store.get("vim.wrapscan", None).unwrap_or(defaults.wrapscan),
        startofline: store
            .get("vim.startofline", None)
            .unwrap_or(defaults.startofline),
        scroll: store.get("vim.scroll", None).unwrap_or(defaults.scroll),
        scrolloff: store
            .get("editor.cursorSurroundingLines", None)
            .unwrap_or(defaults.scrolloff),
    };
    (enabled, settings)
}

/// Every match of `query`; an invalid regex finds nothing.
fn search(snapshot: &BufferSnapshot, query: &SearchQuery) -> Arc<[Range<usize>]> {
    find_in_snapshot(
        snapshot,
        &query.pattern,
        query.is_regex,
        query.case_sensitive,
        query.whole_word,
    )
    .unwrap_or_default()
    .into()
}

fn block_cursor(at: Point) -> CursorOverlay {
    CursorOverlay {
        at,
        shape: CursorShape::Block,
    }
}

/// Whether `row` is among the rows on screen; false before the first
/// paint.
fn row_on_screen(v: &CodeView, row: u32) -> bool {
    let (top, rows) = (v.scroll_top_row(), v.visible_rows());
    rows > 0 && row >= top && row < top.saturating_add(rows)
}

/// What vim may read from a code view, with the match cache.
struct Info<'a> {
    view: &'a CodeView,
    cache: Option<&'a MatchCache>,
}

impl ViewInfo for Info<'_> {
    fn path(&self) -> &Arc<Path> {
        self.view.path()
    }

    fn snapshot(&self) -> &BufferSnapshot {
        self.view.snapshot()
    }

    fn top_row(&self) -> u32 {
        self.view.scroll_top_row()
    }

    fn visible_rows(&self) -> u32 {
        self.view.visible_rows()
    }

    fn is_row_hidden(&self, row: u32) -> bool {
        self.view.is_row_hidden(row)
    }

    fn tab_size(&self) -> u32 {
        self.view.tab_size()
    }

    fn matching_bracket(&self, offset: usize) -> Option<usize> {
        self.view.matching_bracket(offset)
    }

    fn matches(&self, query: &SearchQuery) -> Option<Arc<[Range<usize>]>> {
        self.cache
            .filter(|c| c.revision == self.view.revision() && c.query == *query)
            .map(|c| c.matches.clone())
    }
}
