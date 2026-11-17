//! The bridge between the gpui-free vim state machine (`lucerna-vim`) and
//! the workbench: it routes keys to the [`Vim`] of the active
//! [`CodeView`], carries out the resulting [`Action`]s on the view and
//! keeps both in step when the view changes on its own (mouse, go to,
//! reload).

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, LazyLock};

use gpui::{
    App, AppContext as _, Context, Entity, EntityId, KeyDownEvent, Subscription, Task, WeakEntity,
    Window,
};
use lucerna_editor::clipboard::ClipboardText;
use lucerna_editor::{Ctx, EditConfig};
use lucerna_settings::{Context as WhenContext, SettingsView, WhenExpr, parse_when};
use lucerna_syntax::Language;
use lucerna_text::{Bias, BufferSnapshot, Patch, Point, Reload};
use lucerna_vim::{
    Action, Clipboard, ClipboardMode, FoldAction, InsertAt, Key, Mode, Outcome, ReplayStep,
    SearchQuery, Settings, Typed, ViewInfo, Vim, VimGlobals, VimSelection, block_ranges,
    exclusive_range,
};

use crate::WindowSettings;
use crate::code_view::{
    ClipboardMetadata, CodeView, CodeViewEvent, CursorOverlay, CursorShape, FoldOp, clipboard_item,
};
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
    /// The view's `revision` (its text) vim's cursor and anchor are in: an
    /// edit reported for a later revision moves them.
    revision: u64,
    /// The view's `record_breaks` when last seen in Insert mode: a
    /// cursor move or an edit other than typing there breaks the typing
    /// record of `.`.
    moves: u64,
    _subscriptions: [Subscription; 3],
}

/// How [`VimLayer::run`] answers an [`Outcome::Retry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Retry {
    /// Search now for a small file, in the background for a large one.
    Async,
    /// Search now whatever the size: a replay runs its keys in order.
    Sync,
    /// The search already ran.
    No,
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
    /// Set while an [`Outcome::Replay`] runs: its typing is not recorded
    /// and its undo step stays open until it ends.
    replaying: bool,
    /// Set when a replay met a typing step outside Insert mode: the rest
    /// is dropped.
    replay_stopped: bool,
    /// Per file, the edits vim made whose marks it already moved, oldest
    /// first: their reports, still to come, must not move them again.
    own_edits: HashMap<Arc<Path>, VecDeque<Arc<Patch>>>,
    /// Bumped by every key; a background search re-feeds its key only
    /// while no other key arrived.
    key_generation: u64,
    _retry: Option<Task<()>>,
    /// The view whose key or search `_retry` will run again. Its vim was
    /// restored to the state before it, which may hold an operator (`d/`
    /// waiting for its search, the `d` of `dn`).
    retry_view: Option<EntityId>,
    /// The open `/` or `?`, if any.
    search: Option<VimSearch>,
    /// Set by [`Action::RunCommand`]: the workbench runs it once the key
    /// is consumed, since a command needs the workbench itself.
    run_command: Option<&'static str>,
    /// Set by [`Action::OpenSearch`]: the widget needs the window, which
    /// [`VimLayer::handle_key`] has.
    open_search: Option<bool>,
    /// The editor groups whose code views vim follows, set by
    /// [`VimLayer::attach`].
    groups: Option<WeakEntity<EditorGroupsView>>,
    _new_views: Option<Subscription>,
    /// The window's settings, which `vim.*` is read from.
    window_settings: Entity<WindowSettings>,
    _settings: Subscription,
}

impl VimLayer {
    /// Reads the window's settings and follows their changes.
    pub fn new(window_settings: Entity<WindowSettings>, cx: &mut Context<Self>) -> Self {
        let (enabled, settings) = read_settings(window_settings.read(cx).view(cx));
        VimLayer {
            enabled,
            settings,
            globals: VimGlobals::default(),
            views: HashMap::new(),
            match_cache: HashMap::new(),
            replaying: false,
            replay_stopped: false,
            own_edits: HashMap::new(),
            key_generation: 0,
            _retry: None,
            retry_view: None,
            search: None,
            run_command: None,
            open_search: None,
            groups: None,
            _new_views: None,
            _settings: cx.observe(&window_settings, |vim, _, cx| vim.settings_changed(cx)),
            window_settings,
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
        // Insert mode: every key but Escape types, through the keymap and
        // the view's input handler (IME included).
        if key != Key::Escape && self.mode_for(view.entity_id()) == Some(Mode::Insert) {
            return false;
        }
        self.key_generation += 1;
        self.abandon_retry();
        self.open_search = None;
        self.run_command = None;
        let consumed = self.feed(key, view, Retry::Async, cx);
        if let Some(backward) = self.open_search.take() {
            self.start_search(view, backward, window, cx);
        }
        consumed
    }

    /// Feeds `keys` to the vim of `view` as [`VimLayer::handle_key`]
    /// would, for tests of views that never take the focus (a preview).
    #[doc(hidden)]
    pub fn feed_keys(&mut self, view: &Entity<CodeView>, keys: &[Key], cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }
        for &key in keys {
            self.key_generation += 1;
            self.abandon_retry();
            self.feed(key, view, Retry::Async, cx);
        }
    }

    /// Drops the background search a key or a submitted search waits for,
    /// since another key or search came first. What that key started is
    /// dropped too: the operator restored with the state before it would
    /// otherwise take the new key as its motion (`d/foo<CR>w` as `dw`).
    fn abandon_retry(&mut self) {
        self._retry = None;
        if let Some(id) = self.retry_view.take()
            && let Some(state) = self.views.get_mut(&id)
        {
            state.vim.search_dismissed();
        }
    }

    /// The command the last key asked for; the caller runs it.
    pub(crate) fn take_command(&mut self) -> Option<&'static str> {
        self.run_command.take()
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
                self.abandon_retry();
                let redo = Redo::Search {
                    query,
                    backward: search.backward,
                    origin: search.origin,
                };
                self.run(redo, &view, Retry::Async, cx);
            }
            FindWidgetEvent::Dismissed => {
                if let Some(state) = self.views.get_mut(&id) {
                    // An operator waiting for the search (`d/`) is dropped.
                    state.vim.search_dismissed();
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
        let (enabled, settings) = read_settings(self.window_settings.read(cx).view(cx));
        self.settings = settings;
        if enabled == self.enabled {
            return;
        }
        self.enabled = enabled;
        if !enabled {
            self._retry = None;
            self.retry_view = None;
            self.end_search(cx);
            self.match_cache.clear();
            self.own_edits.clear();
            // Marks are not kept up to date while off.
            self.globals = VimGlobals::default();
            for (_, state) in self.views.drain() {
                let Some(view) = state.view.upgrade() else {
                    continue;
                };
                // The cursor stays where vim had it (or, in Insert mode,
                // where the caret is); a visual selection goes, one made in
                // Normal mode (with the mouse) stays.
                let at = state.vim.cursor();
                let visual = !matches!(state.vim.mode(), Mode::Normal | Mode::Insert);
                view.update(cx, |v, cx| {
                    v.set_cursor_overlay(None, cx);
                    v.set_typed_events(false);
                    v.set_suppress_stops(false, cx);
                    if visual || v.has_block_selection() {
                        v.select(at..at, cx);
                    }
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
        let (cursor, generation, revision, moves) = {
            let v = view.read(cx);
            (
                v.selection_head(),
                v.selection_generation(),
                v.revision(),
                v.record_breaks(),
            )
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
                revision,
                moves,
                _subscriptions: subscriptions,
            },
        );
    }

    /// A typing operation of the Insert session goes to the record of
    /// `.`; an edit moves the marks and the cursors.
    fn view_event(
        &mut self,
        view: Entity<CodeView>,
        event: &CodeViewEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            CodeViewEvent::Typed(typed) => {
                if let Some(state) = self.views.get_mut(&view.entity_id()) {
                    state.vim.record_typed(typed.clone());
                }
            }
            CodeViewEvent::Reloaded {
                reload,
                old,
                snapshot,
                patch,
                revision,
            } => {
                let edit = Edit {
                    reload: *reload,
                    old,
                    snapshot,
                    patch,
                    revision: *revision,
                };
                self.view_edited(&view, edit, cx);
            }
            CodeViewEvent::ScrolledTo(_) | CodeViewEvent::DiffHunkClicked { .. } => {}
        }
    }

    /// The text of `view` changed (an edit from any view of the file, an
    /// undo, a reload): the file's marks move once per edit, and vim's
    /// cursor follows the text unless vim already placed it in the new
    /// text (the edit was vim's own).
    ///
    /// `CodeView::apply_edit` bumps the selection generation and emits
    /// `Reloaded` before it notifies, so this runs (and stores the new
    /// generation) before [`VimLayer::view_changed`] would take the bump
    /// for an outside change and drop the visual mode.
    fn view_edited(&mut self, view: &Entity<CodeView>, edit: Edit, cx: &mut Context<Self>) {
        let id = view.entity_id();
        self.match_cache.remove(&id);
        let path = view.read(cx).path().clone();
        if self.lead_view(&path, cx) == Some(id) {
            let own = self.own_edits.get_mut(&path);
            let at = own
                .as_ref()
                .and_then(|own| own.iter().position(|p| **p == *edit.patch));
            match (own, at) {
                // Vim moved the marks when it made this edit; any queued
                // before it went astray.
                (Some(own), Some(at)) => {
                    own.drain(..=at);
                }
                _ => self
                    .globals
                    .remap_marks_patch(&path, edit.old, edit.patch, edit.snapshot),
            }
        }
        // The open `/` starts from (or goes back to) its origin on the
        // same line; the column is clamped where the origin is used.
        if let Some(search) = self.search.as_mut()
            && search.view.entity_id() == id
        {
            let lines = edit.snapshot.line_count();
            search.origin.row = edit.reload.adjust_row(search.origin.row, lines);
        }
        let Some(state) = self.views.get_mut(&id) else {
            return;
        };
        if state.revision >= edit.revision {
            return;
        }
        state.revision = edit.revision;
        let (old, patch, new) = (edit.old, edit.patch, edit.snapshot);
        // Like the view's own cursor: after text inserted right at it.
        state
            .vim
            .remap_points(|p| patch.map_point(old, new, p, Bias::Right));
        state.vim.clamp(new);
        let v = view.read(cx);
        if state.vim.mode() == Mode::Insert {
            // The edit is (most likely) the user typing: the caret is the
            // view's, and `view_changed` follows it.
            cx.notify();
            return;
        }
        if edit.revision != v.revision() {
            // A later edit is still to be reported, and draws then.
            state.generation = v.selection_generation();
            return;
        }
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
            self.sync_view(view, Reveal::None, cx);
        }
        cx.notify();
    }

    /// The view whose edit reports move the marks of `path`: every view of
    /// the file reports each edit, and one of them must move them once.
    /// The tab of the groups with the lowest id (not a closed view still
    /// alive), else, for a file in no tab (a preview), the tracked view
    /// with the lowest id.
    fn lead_view(&self, path: &Path, cx: &App) -> Option<EntityId> {
        let of_path = |v: &Entity<CodeView>| &**v.read(cx).path() == path;
        let tab = self
            .group_views(cx)
            .iter()
            .filter(|v| self.views.contains_key(&v.entity_id()) && of_path(v))
            .map(|v| v.entity_id())
            .min();
        tab.or_else(|| {
            self.views
                .iter()
                .filter(|(_, s)| s.view.upgrade().is_some_and(|v| of_path(&v)))
                .map(|(id, _)| *id)
                .min()
        })
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
        if state.vim.mode() == Mode::Insert {
            // Typing, arrow keys and clicks move the caret; the mode stays.
            // A move other than by typing starts a new record for `.`.
            if v.record_breaks() != state.moves {
                state.moves = v.record_breaks();
                state.vim.break_insert_record();
            }
            follow_selection(&mut state.vim, v);
            state.generation = v.selection_generation();
            state.revision = v.revision();
            cx.notify();
            return;
        }
        if v.selection_generation() != state.generation {
            state.vim.reset(v.selection_head());
            state.vim.clamp(v.snapshot());
            state.generation = v.selection_generation();
            state.revision = v.revision();
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

    /// Feeds `key`; `retry` says how to answer an [`Outcome::Retry`].
    fn feed(
        &mut self,
        key: Key,
        view: &Entity<CodeView>,
        retry: Retry,
        cx: &mut Context<Self>,
    ) -> bool {
        self.run(Redo::Key(key), view, retry, cx)
    }

    /// Feeds a key or runs a submitted search; `retry` says how to answer
    /// an [`Outcome::Retry`].
    fn run(
        &mut self,
        redo: Redo,
        view: &Entity<CodeView>,
        retry: Retry,
        cx: &mut Context<Self>,
    ) -> bool {
        let id = view.entity_id();
        self.ensure_state(view, cx);
        if matches!(redo, Redo::Key(_)) && self.searching(id) {
            // A key reached the view while its `/` widget is open: the
            // user left the widget, so the search is over and the
            // selection the view has now is the user's, and an operator
            // waiting for the search (`d/`) is dropped.
            self.end_search(cx);
            if let Some(state) = self.views.get_mut(&id) {
                state.vim.search_dismissed();
            }
        }
        let outcome = {
            let v = view.read(cx);
            let edit = (!v.is_readonly()).then(|| (v.edit_config(cx), v.edit_language(cx)));
            let Some(state) = self.views.get_mut(&id) else {
                return false;
            };
            if v.selection_generation() != state.generation {
                follow_selection(&mut state.vim, v);
                state.generation = v.selection_generation();
                state.revision = v.revision();
            }
            // Registers compare the clipboard with what they last wrote;
            // it is read only for the keys that may write or read `"`.
            let reads = match &redo {
                Redo::Key(key) => may_use_register(&state.vim, *key),
                Redo::Search { .. } => !state.vim.pending().is_empty(),
            };
            self.globals
                .set_clipboard(if reads { read_clipboard(cx) } else { None });
            let info = Info {
                view: v,
                cache: self.match_cache.get(&id),
                edit,
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
                    state.vim.reset(v.selection_head());
                    state.vim.clamp(v.snapshot());
                    state.revision = v.revision();
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
            Ok(Outcome::Replay(steps)) => {
                self.replay(view, steps, cx);
                true
            }
            Ok(Outcome::Retry(query)) => {
                match retry {
                    Retry::Async => self.search_then_run(redo, view, query, cx),
                    Retry::Sync => {
                        self.cache_matches(view, query, cx);
                        self.run(redo, view, Retry::No, cx);
                    }
                    Retry::No => {
                        // Vim restored the state before the key, a
                        // pending operator included: drop it, so the next
                        // key can never become its motion.
                        tracing::warn!("vim: matches still missing after a search");
                        if let Some(state) = self.views.get_mut(&id) {
                            state.vim.search_dismissed();
                        }
                        cx.notify();
                    }
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
            self.cache_matches(view, query, cx);
            self.run(redo, view, Retry::No, cx);
            return;
        }
        let generation = self.key_generation;
        let pattern = query.clone();
        let result = cx.background_spawn(async move {
            guarded("vim search", || search(&snapshot, &pattern)).unwrap_or_else(|| Arc::from([]))
        });
        let view = view.downgrade();
        self.retry_view = Some(id);
        self._retry = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let matches = result.await;
            this.update(cx, |this, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                if this.key_generation != generation || !this.enabled {
                    return;
                }
                this.retry_view = None;
                if view.read(cx).revision() != revision {
                    // The text changed under the search (an agent's
                    // reload, another view's edit): the matches are
                    // stale. What waited for them is dropped, as when
                    // another key comes first.
                    if let Some(state) = this.views.get_mut(&id) {
                        state.vim.search_dismissed();
                    }
                    cx.notify();
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
                this.run(redo, &view, Retry::No, cx);
            })
            .ok();
        }));
    }

    /// Finds the matches of `query` in `view` now, whatever the file's
    /// size, and caches them.
    fn cache_matches(&mut self, view: &Entity<CodeView>, query: SearchQuery, cx: &App) {
        let v = view.read(cx);
        let matches = search(v.snapshot(), &query);
        self.match_cache.insert(
            view.entity_id(),
            MatchCache {
                revision: v.revision(),
                query,
                matches,
            },
        );
    }

    /// Runs the steps of an [`Outcome::Replay`] in order, as one undo
    /// step: keys go through [`VimLayer::feed`] (searches run at once),
    /// an operator's search step through [`Vim::search_submitted`] from
    /// vim's cursor (at once, no widget; with no operator pending it drops
    /// the rest), and typing goes through the
    /// view's typing path, unrecorded. A typing
    /// step outside Insert mode (the command that should have entered it
    /// failed) drops the rest. A key may return a replay of its own (a
    /// counted insert ends): it runs here, in place.
    ///
    /// The user's Escape that ends a counted insert replays too; the
    /// Insert session's step is then still open and the replay joins it.
    fn replay(&mut self, view: &Entity<CodeView>, steps: Vec<ReplayStep>, cx: &mut Context<Self>) {
        let id = view.entity_id();
        let outermost = !self.replaying;
        if outermost {
            self.replaying = true;
            self.replay_stopped = false;
            view.update(cx, |v, cx| {
                v.set_typed_events(false);
                v.set_suppress_stops(true, cx);
            });
        }
        for step in steps {
            if self.replay_stopped {
                break;
            }
            match step {
                ReplayStep::Key(key) => {
                    self.feed(key, view, Retry::Sync, cx);
                }
                ReplayStep::Search {
                    pattern,
                    backward,
                    is_regex,
                    case_sensitive,
                    whole_word,
                } => {
                    // An operator's `/` or `?`: searched at once, from vim's
                    // cursor, with no find widget. Without the operator
                    // (its keys failed) the replay stops rather than run a
                    // plain search.
                    let origin = self
                        .views
                        .get(&id)
                        .filter(|s| s.vim.operator_pending())
                        .map(|s| s.vim.cursor());
                    let Some(origin) = origin else {
                        self.replay_stopped = true;
                        break;
                    };
                    let query = SearchQuery {
                        pattern,
                        is_regex,
                        case_sensitive,
                        whole_word,
                    };
                    let redo = Redo::Search {
                        query,
                        backward,
                        origin,
                    };
                    self.run(redo, view, Retry::Sync, cx);
                }
                ReplayStep::Type(typed) => {
                    let insert = self
                        .views
                        .get(&id)
                        .is_some_and(|s| s.vim.mode() == Mode::Insert);
                    if !insert {
                        self.replay_stopped = true;
                        break;
                    }
                    view.update(cx, |v, cx| match typed {
                        Typed::Text(text) => v.type_text(&text, cx),
                        Typed::Newline => v.type_text("\n", cx),
                        Typed::DeleteLeft => v.delete_left(cx),
                        Typed::Tab => v.tab(cx),
                    });
                }
            }
        }
        if !outermost {
            return;
        }
        self.replaying = false;
        self.replay_stopped = false;
        let insert = self
            .views
            .get(&id)
            .is_some_and(|s| s.vim.mode() == Mode::Insert);
        if insert {
            // The replay left an Insert session open: the user types on in
            // it, recorded, until Escape closes the step.
            view.update(cx, |v, _| v.set_typed_events(true));
            if let Some(state) = self.views.get_mut(&id) {
                let v = view.read(cx);
                follow_selection(&mut state.vim, v);
                state.generation = v.selection_generation();
                state.revision = v.revision();
            }
        } else {
            self.sync_view(view, Reveal::Scroll, cx);
        }
        cx.notify();
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
        let mut insert = None;
        // A command that ends in Insert mode keeps its undo step open until
        // the Insert session's Escape.
        let keep_open = self
            .views
            .get(&id)
            .is_some_and(|s| s.vim.mode() == Mode::Insert);
        // Every edit of one feed is one undo step.
        let group = actions
            .iter()
            .any(|a| matches!(a, Action::Edit(_)))
            .then(|| view.update(cx, |v, cx| v.begin_group(cx)));
        for action in actions {
            match action {
                Action::ScrollTopRow(row) => {
                    view.update(cx, |v, cx| v.set_scroll_top_row(row, cx));
                }
                Action::CursorMoved { jump } => {
                    let this = if jump { Reveal::Jump } else { Reveal::Scroll };
                    reveal = Some(reveal.map_or(this, |r: Reveal| if jump { this } else { r }));
                }
                Action::WriteClipboard { text, linewise } => {
                    cx.write_to_clipboard(clipboard_item(ClipboardText {
                        text,
                        pieces: None,
                        whole_line: linewise,
                    }));
                }
                Action::Edit(plan) => {
                    let before = view.read(cx).revision();
                    view.update(cx, |v, cx| v.transact(plan, cx));
                    if view.read(cx).revision() != before {
                        self.marks_follow_own_edit(view, cx);
                    }
                    // Vim placed its cursor in the new text.
                    if let Some(state) = self.views.get_mut(&id) {
                        let v = view.read(cx);
                        state.vim.clamp(v.snapshot());
                        state.revision = v.revision();
                    }
                    redraw = true;
                }
                Action::Undo(count) | Action::Redo(count) => {
                    let undo = matches!(action, Action::Undo(_));
                    // The step's selections come back: vim's cursor before
                    // the command, the start of a visual selection.
                    let at = view.update(cx, |v, cx| {
                        for _ in 0..count {
                            if !v.undo(undo, cx) {
                                break;
                            }
                        }
                        v.selection()
                            .map_or_else(|| v.selection_head(), |r| r.start)
                    });
                    if let Some(state) = self.views.get_mut(&id) {
                        let v = view.read(cx);
                        state.vim.reset(at);
                        state.vim.clamp(v.snapshot());
                        state.revision = v.revision();
                    }
                    redraw = true;
                }
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
                Action::RunCommand(id) => self.run_command = Some(id),
                Action::EnterInsert(at) => insert = Some(at),
                // The command line arrives with the workbench UI.
                Action::Failed
                | Action::RunEx(_)
                | Action::OpenCommandLine { .. }
                | Action::Message(_) => {}
            }
        }
        if let Some(opened) = group {
            view.update(cx, |v, cx| v.end_group(opened, keep_open, cx));
        }
        if let Some(at) = insert {
            self.start_insert(view, at, cx);
            cx.notify();
            return true;
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

    /// Moves the marks of `view`'s file through the edit vim just applied
    /// to it, at once: a later key of the same replay may set a mark in the
    /// new text before the edit's report arrives, and that report then
    /// leaves the marks alone.
    fn marks_follow_own_edit(&mut self, view: &Entity<CodeView>, cx: &App) {
        let v = view.read(cx);
        let Some((old, patch)) = v.last_edit() else {
            return;
        };
        let path = v.path().clone();
        self.globals
            .remap_marks_patch(&path, old, patch, v.snapshot());
        let own = self.own_edits.entry(path).or_default();
        // Reports always come; this only bounds a queue that went astray.
        if own.len() >= 256 {
            own.clear();
        }
        own.push_back(patch.clone());
    }

    /// Starts an Insert session in `view`: one undo step from here to
    /// Escape, the bar cursor, and the caret where `at` says: vim's cursor,
    /// or a line `o` / `O` opens with `commands::newline` (keeping the
    /// indent).
    fn start_insert(&mut self, view: &Entity<CodeView>, at: InsertAt, cx: &mut Context<Self>) {
        let replaying = self.replaying;
        let Some(state) = self.views.get_mut(&view.entity_id()) else {
            return;
        };
        let p = state.vim.cursor();
        let generation = view.update(cx, |v, cx| {
            v.set_suppress_stops(true, cx);
            // Before the edit: a block cursor refuses it.
            v.set_cursor_overlay(Some(bar_cursor(p)), cx);
            let line_len = |v: &CodeView, row: u32| v.snapshot().line(row).len() as u32;
            match at {
                InsertAt::LineBelow => {
                    let column = line_len(v, p.row);
                    v.set_cursor(Point { row: p.row, column }, cx);
                    v.edit(cx, lucerna_editor::commands::newline);
                }
                InsertAt::LineAbove => {
                    // A break after the indent leaves the indent alone on
                    // the line above (or, on a blank line, an empty line).
                    let line = v.snapshot().line(p.row);
                    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
                    v.set_cursor(
                        Point {
                            row: p.row,
                            column: indent as u32,
                        },
                        cx,
                    );
                    if v.edit(cx, lucerna_editor::commands::newline) {
                        let column = line_len(v, p.row);
                        v.set_cursor(Point { row: p.row, column }, cx);
                    }
                }
                InsertAt::Before | InsertAt::After | InsertAt::LineStart | InsertAt::LineEnd => {
                    v.set_cursor(p, cx)
                }
            }
            // The user's typing is recorded for `.`; a replay's is not.
            v.set_typed_events(!replaying);
            v.selection_generation()
        });
        state.generation = generation;
        let v = view.read(cx);
        state.revision = v.revision();
        state.moves = v.record_breaks();
        follow_selection(&mut state.vim, v);
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
        let replaying = self.replaying;
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
            // Leaving Insert mode ends its undo step, unless a replay
            // (one step as a whole) is running.
            if !replaying {
                v.set_typed_events(false);
                v.set_suppress_stops(false, cx);
            }
            v.selection_generation()
        });
        state.revision = view.read(cx).revision();
    }
}

/// Moves vim's cursor to the view's after the view moved it: in Insert
/// mode the mode stays, otherwise vim goes back to Normal mode there.
fn follow_selection(vim: &mut Vim, v: &CodeView) {
    if vim.mode() == Mode::Insert {
        vim.set_cursor(v.selection_head());
        vim.clamp(v.snapshot());
    } else {
        vim.reset(v.selection_head());
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

fn read_settings(store: SettingsView<'_>) -> (bool, Settings) {
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
        use_system_clipboard: clipboard_mode(
            store.get::<serde_json::Value>("vim.useSystemClipboard", None),
        ),
        change_word_includes_whitespace: store
            .get("vim.changeWordIncludesWhitespace", None)
            .unwrap_or(defaults.change_word_includes_whitespace),
        joinspaces: store
            .get("vim.joinspaces", None)
            .unwrap_or(defaults.joinspaces),
        tildeop: store.get("vim.tildeop", None).unwrap_or(defaults.tildeop),
    };
    (enabled, settings)
}

/// `vim.useSystemClipboard`: `true`, `false` or `"onYank"`; anything else
/// is `"onYank"`.
fn clipboard_mode(value: Option<serde_json::Value>) -> ClipboardMode {
    match value {
        Some(serde_json::Value::Bool(true)) => ClipboardMode::Always,
        Some(serde_json::Value::Bool(false)) => ClipboardMode::Never,
        _ => ClipboardMode::OnYank,
    }
}

/// Whether `key` may write or read the unnamed register: an operator,
/// `x`, a paste, a register name (Visual `R` too), or any key while one
/// is pending.
fn may_use_register(vim: &Vim, key: Key) -> bool {
    !vim.pending().is_empty()
        || matches!(
            key,
            Key::Char(
                'p' | 'P' | 'x' | 'X' | 'd' | 'c' | 's' | 'S' | 'C' | 'D' | 'y' | 'Y' | 'R' | '"'
            )
        )
}

/// The system clipboard as registers see it: its text, with `\r\n` and a
/// lone `\r` as `\n` (as the editor's paste), and whether the editor
/// copied it as a whole line.
fn read_clipboard(cx: &App) -> Option<Clipboard> {
    let item = cx.read_from_clipboard()?;
    let text = item.text()?.replace("\r\n", "\n").replace('\r', "\n");
    let whole_line = item
        .metadata()
        .and_then(|m| serde_json::from_str::<ClipboardMetadata>(m).ok())
        .is_some_and(|m| m.whole_line);
    Some(Clipboard { text, whole_line })
}

/// One edit as a code view reported it.
struct Edit<'a> {
    reload: Reload,
    old: &'a BufferSnapshot,
    snapshot: &'a BufferSnapshot,
    patch: &'a Patch,
    revision: u64,
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

fn bar_cursor(at: Point) -> CursorOverlay {
    CursorOverlay {
        at,
        shape: CursorShape::Bar,
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
    /// The view's editing config and language; `None` when read-only.
    edit: Option<(EditConfig, Option<Arc<Language>>)>,
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

    fn edit_ctx(&self) -> Option<Ctx<'_>> {
        let (config, language) = self.edit.as_ref()?;
        Some(Ctx {
            snapshot: self.view.snapshot(),
            config,
            language: language.as_deref().map(Language::editing),
            syntax: self.view.syntax().map(|s| &**s),
        })
    }

    fn enclosing_nodes(&self, range: Range<usize>) -> Vec<Range<usize>> {
        self.view
            .syntax()
            .map_or_else(Vec::new, |s| s.enclosing_nodes(range))
    }
}
