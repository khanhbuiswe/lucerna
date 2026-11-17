//! The per-view state machine: mode, cursor, goal column, pending keys.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use lucerna_text::{BufferSnapshot, Point};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::classify::{self, display_col, last_char_col, line};
use crate::marks::FileMarks;
use crate::motion::{self, FindChar, Goal, WordMotion};
use crate::object::{self, Object};
use crate::operator::{self, MotionKind, OpRange, Operator, PendingOp};
use crate::register::{Registers, WriteKind};
use crate::repeat::{LastChange, ReplayStep, Shape, Typed};
use crate::scroll::{self, Screen, ScrollKind};
use crate::search;
use crate::visual::{self, VimSelection};
use crate::{Action, FoldAction, InsertAt, Key, Mode, Outcome, SearchQuery, Settings, ViewInfo};
use crate::{Clipboard, ClipboardMode, RegContent, RegKind};

/// The largest count; more digits are ignored.
const MAX_COUNT: u32 = 99_999;

/// State shared by every view of one workbench.
#[derive(Debug, Default)]
pub struct VimGlobals {
    /// The last `f`/`F`/`t`/`T`, for `;` and `,`.
    pub(crate) last_find: Option<FindChar>,
    /// Rows for `Ctrl+d`/`Ctrl+u` set by a count; 0 means use
    /// `Settings::scroll`, else half the visible rows.
    pub(crate) scroll: u32,
    /// Marks per file path.
    pub(crate) marks: HashMap<Arc<Path>, FileMarks>,
    /// The last search and whether it went backward, for `n` and `N`.
    pub(crate) last_search: Option<(SearchQuery, bool)>,
    pub(crate) registers: Registers,
    /// The last change, for `.`.
    pub(crate) last_change: Option<LastChange>,
    /// The system clipboard as the workbench last read it.
    clipboard: Option<Clipboard>,
}

impl VimGlobals {
    /// The workbench calls this before every feed with the current
    /// clipboard (`None` when it cannot be read).
    pub fn set_clipboard(&mut self, clip: Option<Clipboard>) {
        self.clipboard = clip;
    }

    /// Reads register `name` (`None` is `"`). `%` is the file at `path`
    /// and `/` the last search pattern; the rest go to [`Registers`].
    pub(crate) fn read_register(
        &self,
        name: Option<char>,
        mode: ClipboardMode,
        path: &Path,
    ) -> Option<RegContent> {
        let charwise = |text: String| RegContent {
            text,
            kind: RegKind::Charwise,
        };
        match name {
            Some('%') => Some(charwise(path.to_string_lossy().into_owned())),
            Some('/') => self
                .last_search
                .as_ref()
                .map(|(q, _)| charwise(q.pattern.clone())),
            _ => self.registers.read(name, mode, self.clipboard.as_ref()),
        }
    }

    /// Writes `content` to register `name`; the clipboard write that is
    /// due is returned.
    pub(crate) fn write_register(
        &mut self,
        name: Option<char>,
        content: RegContent,
        kind: WriteKind,
        mode: ClipboardMode,
    ) -> Option<Action> {
        self.registers
            .write(name, content, kind, mode, self.clipboard.as_ref())
    }
}

/// The vim state of one view.
#[derive(Debug, Clone)]
pub struct Vim {
    pub(crate) mode: Mode,
    pub(crate) cursor: Point,
    /// `None`: derive it from the cursor on the next vertical motion.
    pub(crate) goal: Option<Goal>,
    /// The other end of a visual selection.
    pub(crate) anchor: Point,
    /// The pending keys, for the indicator.
    pending: String,
    count: Option<u32>,
    /// The first key of a two-key command, e.g. `g`.
    prefix: Option<char>,
    /// The count typed before `/` or `?`, used when the search is submitted.
    search_count: Option<u32>,
    /// The register chosen with `"x` for the next command.
    register: Option<char>,
    /// An operator waiting for its motion; `count` then holds the count
    /// typed after it.
    op: Option<PendingOp>,
    /// The count of the `i a I A o O` that began the Insert session:
    /// `3ifoo<Esc>` types the text twice more at Escape, which `take()`s
    /// it so a later Insert session does not inherit it.
    insert_count: Option<u32>,
    /// The Insert session began with `o` or `O`: each repeat of a counted
    /// insert starts on a new line.
    insert_lines: bool,
    /// The keys of the command being typed, without the register and the
    /// counts, for `.`.
    pub(crate) cmd_keys: Vec<Key>,
    /// The change that ended in this Insert session, recording its typing
    /// until Escape.
    recording: Option<LastChange>,
    /// The command of `recording` changed the text itself (`c`, `o`), so
    /// it is a change even when nothing is typed.
    recorded_edit: bool,
    /// A change [`Vim::break_insert_record`] finished, saved as the last
    /// change on the next feed.
    unsaved_change: Option<LastChange>,
    /// The typing a `.` replay will do in the Insert session it opens:
    /// the replay does not report it, so it is recorded from here.
    replay_typed: Option<Vec<Typed>>,
    /// The text the last `:` put in the command line (`'<,'>` from Visual,
    /// `.,.+2` after a count), for macros.
    pub(crate) last_prefill: Option<String>,
}

impl Vim {
    pub fn new(cursor: Point) -> Vim {
        Vim {
            mode: Mode::Normal,
            cursor,
            goal: None,
            anchor: cursor,
            pending: String::new(),
            count: None,
            prefix: None,
            search_count: None,
            register: None,
            op: None,
            insert_count: None,
            insert_lines: false,
            cmd_keys: Vec::new(),
            recording: None,
            recorded_edit: false,
            unsaved_change: None,
            replay_typed: None,
            last_prefill: None,
        }
    }

    /// Handles one key. On [`Outcome::Retry`] the state is as it was before.
    pub fn feed(
        &mut self,
        key: Key,
        view: &dyn ViewInfo,
        globals: &mut VimGlobals,
        settings: &Settings,
    ) -> Outcome {
        self.clamp(view.snapshot());
        if let Some(change) = self.unsaved_change.take() {
            globals.last_change = Some(change);
        }
        let saved = self.clone();
        let outcome = self.step(key, view, globals, settings);
        self.finish(&saved, saved.cursor, &outcome, view, globals);
        if !matches!(outcome, Outcome::Retry(_)) {
            self.record(key, &saved, &outcome, view, globals);
        }
        outcome
    }

    /// The workbench calls this for each typing event of an Insert session
    /// vim opened (not for a replay's typing), for `.`.
    pub fn record_typed(&mut self, t: Typed) {
        if self.mode == Mode::Insert
            && let Some(change) = &mut self.recording
        {
            change.typed.push(t);
        }
    }

    /// The caret moved in Insert mode other than by typing (an arrow key,
    /// a click): the change so far is finished, and the typing from here
    /// on is a new one, repeated by `.` as an `i`. A counted insert no
    /// longer repeats.
    pub fn break_insert_record(&mut self) {
        if self.mode != Mode::Insert {
            return;
        }
        self.insert_count = None;
        if let Some(change) = self.recording.take()
            && (self.recorded_edit || !change.typed.is_empty())
        {
            self.unsaved_change = Some(change);
        }
        self.recording = Some(LastChange {
            register: None,
            count: None,
            keys: vec![Key::Char('i')],
            search: None,
            shape: None,
            insert: true,
            typed: Vec::new(),
        });
        self.recorded_edit = false;
    }

    /// After `key` (fed with the state `saved` before it): collects the
    /// keys of the command being typed and, once it is done, records it as
    /// the last change if it changed the text. A command that enters
    /// Insert is recorded at Escape, with the typing.
    ///
    /// Every fed key goes through here, a replay's too: a replay is
    /// recorded like typed keys, which is how `N.` and `"1p.` update the
    /// record. V2b macros (`@a`) must not overwrite the last change this
    /// way and need a flag that turns this off while a macro runs.
    fn record(
        &mut self,
        key: Key,
        saved: &Vim,
        outcome: &Outcome,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
    ) {
        if saved.mode == Mode::Insert {
            if self.mode != Mode::Insert
                && let Some(change) = self.recording.take()
                && (self.recorded_edit || !change.typed.is_empty())
            {
                g.registers.set_dot(change.inserted_text());
                g.last_change = Some(change);
            }
            return;
        }
        let count_key = saved.prefix.is_none()
            && matches!(key, Key::Char(c) if c.to_digit(10)
                .is_some_and(|d| d != 0 || saved.count.is_some()));
        let register_key = saved.prefix == Some('"')
            || (saved.prefix.is_none() && saved.op.is_none() && key == Key::Char('"'));
        if !count_key && !register_key {
            self.cmd_keys.push(key);
        }
        if !self.pending.is_empty() || self.op.is_some() || self.prefix.is_some() {
            return;
        }
        let keys = std::mem::take(&mut self.cmd_keys);
        // `.` itself returns a replay and is not recorded.
        let Outcome::Handled(actions) = outcome else {
            return;
        };
        // A command finished back in Normal mode without entering Insert:
        // a `.` replay that would have typed failed (`ci(` with no pair),
        // so its typing must not reach a later Insert session. The keys
        // that rebuild a Visual shape leave vim in Visual and keep it, and
        // a count or `"x` key (which leaves them pending anyway) is not a
        // finished command.
        if self.mode == Mode::Normal && !count_key && !register_key {
            self.replay_typed = None;
        }
        self.save_change(keys, None, saved, actions, view, g);
    }

    /// Records the command whose `keys` (and the `search` of a `/` or `?`
    /// motion) just ran from the state `saved`, returning `actions`, as the
    /// last change when it changed the text. One that entered Insert is
    /// recorded at Escape, with the typing.
    fn save_change(
        &mut self,
        keys: Vec<Key>,
        search: Option<(SearchQuery, bool)>,
        saved: &Vim,
        actions: &[Action],
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
    ) {
        let edited = actions.iter().any(|a| matches!(a, Action::Edit(_)));
        let entered = self.mode == Mode::Insert;
        if !edited && !entered {
            return;
        }
        let (count, register) = match &saved.op {
            Some(p) => (
                match (p.count, saved.count) {
                    (None, None) => None,
                    (a, b) => Some(
                        (u64::from(a.unwrap_or(1)) * u64::from(b.unwrap_or(1)))
                            .min(u64::from(MAX_COUNT)) as u32,
                    ),
                },
                p.register,
            ),
            None => (saved.count, saved.register),
        };
        let shape = if matches!(saved.mode, Mode::Visual | Mode::VisualLine) {
            saved
                .selection(view.snapshot(), view.tab_size())
                .and_then(|sel| Shape::of(&sel, view.snapshot()))
        } else {
            None
        };
        let mut change = LastChange {
            register,
            count,
            keys,
            search,
            shape,
            insert: entered,
            typed: Vec::new(),
        };
        if entered {
            change.typed = self.replay_typed.take().unwrap_or_default();
            self.recording = Some(change);
            self.recorded_edit = edited
                || actions.iter().any(|a| {
                    matches!(
                        a,
                        Action::EnterInsert(InsertAt::LineBelow | InsertAt::LineAbove)
                    )
                });
        } else {
            g.last_change = Some(change);
        }
    }

    /// `.`: the steps that repeat the last change, `count` replacing its
    /// count.
    fn dot(&mut self, count: Option<u32>, view: &dyn ViewInfo, g: &VimGlobals) -> Outcome {
        let Some(change) = &g.last_change else {
            return Outcome::Handled(vec![]);
        };
        if view.edit_ctx().is_none() {
            return Outcome::Handled(vec![]);
        }
        self.replay_typed = change.insert.then(|| change.typed.clone());
        Outcome::Replay(change.replay(count))
    }

    /// Escape in Insert mode. After a counted insert, first the steps that
    /// type the text `count - 1` more times (each on a new line after `o`
    /// or `O`), then Escape again.
    fn escape_insert(&mut self, view: &dyn ViewInfo) -> Outcome {
        let typed = self.recording.as_ref().map_or(&[][..], |r| &r.typed[..]);
        let steps = match self.insert_count.take() {
            Some(n) if self.insert_lines || !typed.is_empty() => {
                insert_copies(typed, self.insert_lines, n as usize - 1)
            }
            _ => None,
        };
        match steps {
            Some(mut steps) => {
                steps.push(ReplayStep::Key(Key::Escape));
                Outcome::Replay(steps)
            }
            None => self.exit_insert(view),
        }
    }

    /// After a command: syncs the anchor, restores `saved` on a retry, and
    /// records `from` as the `''` mark when the cursor jumped.
    fn finish(
        &mut self,
        saved: &Vim,
        from: Point,
        outcome: &Outcome,
        view: &dyn ViewInfo,
        globals: &mut VimGlobals,
    ) {
        if self.mode == Mode::Normal {
            self.anchor = self.cursor;
        }
        match outcome {
            Outcome::Retry(_) => *self = saved.clone(),
            Outcome::Handled(actions)
                if self.cursor != from && actions.contains(&Action::CursorMoved { jump: true }) =>
            {
                globals
                    .marks
                    .entry(view.path().clone())
                    .or_default()
                    .last_jump = Some(from);
            }
            _ => {}
        }
    }

    /// Runs a search the user typed after `/` or `?`: jumps to the first
    /// match after (before, when `backward`) `origin`, the count typed before
    /// `/` picking a later one. The match is highlighted and remembered for
    /// `n` and `N`. On [`Outcome::Retry`] the state is as it was before.
    pub fn search_submitted(
        &mut self,
        query: SearchQuery,
        backward: bool,
        origin: Point,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Outcome {
        self.clamp(view.snapshot());
        let saved = self.clone();
        let origin = self.clamp_in_mode(view.snapshot(), origin);
        self.cursor = origin;
        if let Some(p) = self.op.take() {
            let outcome = self.search_op(p, query.clone(), backward, view, g, s);
            self.finish(&saved, origin, &outcome, view, g);
            if let Outcome::Handled(actions) = &outcome {
                // The keys of a typed `d/` end with the `/`; a replay feeds
                // only `d` and then searches.
                let mut keys = std::mem::take(&mut self.cmd_keys);
                if matches!(keys.last(), Some(Key::Char('/' | '?'))) {
                    keys.pop();
                }
                if self.mode == Mode::Normal {
                    self.replay_typed = None;
                }
                self.save_change(keys, Some((query, backward)), &saved, actions, view, g);
            }
            return outcome;
        }
        let n = self.search_count.take().unwrap_or(1);
        let outcome = if query.pattern.is_empty() {
            Outcome::Handled(vec![])
        } else {
            match self.jump_to_match(view, query.clone(), origin, backward, n, s) {
                Ok(out) => {
                    g.last_search = Some((query, backward));
                    out
                }
                Err(retry) => retry,
            }
        };
        self.finish(&saved, origin, &outcome, view, g);
        outcome
    }

    /// The find widget of a `/` or `?` closed without a search (Esc, or
    /// the user left it): the operator waiting for it (`d/`) and the count
    /// typed before it are dropped.
    pub fn search_dismissed(&mut self) {
        self.clear_pending();
        self.search_count = None;
        self.cmd_keys.clear();
    }

    /// Runs the operator `p` waiting for a `/` or `?` on the text from the
    /// cursor to the nth match of `query` (an exclusive motion), `n` the
    /// counts before and after the operator. The search is remembered for
    /// `n` and highlighted, as a plain `/`. No match cancels the operator.
    ///
    /// Two deviations from Vim, consistent with V1 and the other operator
    /// motions: no `''` mark is set (Vim sets it for `/` even under an
    /// operator), and an empty pattern (`d/<CR>`) cancels instead of
    /// reusing the last one.
    fn search_op(
        &mut self,
        p: PendingOp,
        query: SearchQuery,
        backward: bool,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Outcome {
        let n = match (p.count, self.count) {
            (None, None) => 1,
            (a, b) => {
                (u64::from(a.unwrap_or(1)) * u64::from(b.unwrap_or(1))).min(MAX_COUNT as u64) as u32
            }
        };
        self.clear_pending();
        self.search_count = None;
        if query.pattern.is_empty() {
            return Outcome::Handled(vec![]);
        }
        let Some(matches) = view.matches(&query) else {
            return Outcome::Retry(query);
        };
        g.last_search = Some((query.clone(), backward));
        let snapshot = view.snapshot();
        let from = self.cursor;
        let at = snapshot.point_to_offset(from);
        let Some(start) = search::nth_match(&matches, at, backward, n, s.wrapscan) else {
            return Outcome::Handled(vec![Action::Highlight(query)]);
        };
        let to = snapshot.offset_to_point(start);
        let change = p.op == Operator::Change;
        let mut outcome =
            match operator::motion_range(snapshot, from, to, MotionKind::Exclusive, p.force) {
                Some(r) => {
                    // A yank keeps the cursor when the match is ahead.
                    let forward = (to.row, to.column) >= (from.row, from.column);
                    let yank_cursor = if forward { from } else { to };
                    self.finish_op(p, r, Some(yank_cursor), view, g, s)
                }
                // The match is under the cursor: `c` still inserts.
                None if change => self.change_nothing(view),
                None => Outcome::Handled(vec![]),
            };
        if let Outcome::Handled(actions) = &mut outcome {
            actions.push(Action::Highlight(query));
        }
        outcome
    }

    /// An operator waits for its motion (`d`, or `d/` waiting for the
    /// search). A replay's [`ReplayStep::Search`] needs one.
    pub fn operator_pending(&self) -> bool {
        self.op.is_some()
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn cursor(&self) -> Point {
        self.cursor
    }

    /// The visual selection between the anchor and the cursor (the head);
    /// `None` in Normal mode. `tab` is the view's tab size, for the display
    /// columns of a block.
    pub fn selection(&self, s: &BufferSnapshot, tab: u32) -> Option<VimSelection> {
        let (a, h) = (self.anchor, self.cursor);
        let (start, end) = if (a.row, a.column) <= (h.row, h.column) {
            (a, h)
        } else {
            (h, a)
        };
        match self.mode {
            Mode::Normal | Mode::Insert => None,
            Mode::Visual => Some(VimSelection::Char {
                start,
                end_inclusive: end,
            }),
            Mode::VisualLine => Some(VimSelection::Line {
                start_row: start.row,
                end_row: end.row,
            }),
            Mode::VisualBlock => {
                let span = |p: Point| {
                    let text = line(s, p.row);
                    let left = display_col(&text, p.column, tab);
                    let right = text[(p.column as usize).min(text.len())..]
                        .chars()
                        .next()
                        .map_or(left + 1, |c| classify::advance(left, c, tab.max(1)));
                    (left, right)
                };
                let (a, h) = (span(a), span(h));
                Some(VimSelection::Block {
                    rows: start.row..end.row + 1,
                    display_cols: a.0.min(h.0)..a.1.max(h.1),
                    to_eol: self.goal == Some(Goal::Eol),
                })
            }
        }
    }

    /// Moves the cursor from outside vim and forgets the goal column. A
    /// visual mode ends without a [`Action::ModeChanged`] (the caller made
    /// the change) and without setting the `<` and `>` marks; Insert mode
    /// stays (the view's caret moved while typing). The anchor moves with
    /// the cursor. The point is clamped to the buffer on the next
    /// [`Vim::feed`] or [`Vim::clamp`].
    pub fn set_cursor(&mut self, p: Point) {
        if self.mode != Mode::Insert {
            self.mode = Mode::Normal;
        }
        self.cursor = p;
        self.anchor = p;
        self.goal = None;
    }

    /// Moves the cursor and the anchor to the rows `f` gives (a reload
    /// moved the lines), keeping the mode, the columns and the goal. The
    /// points are clamped on the next [`Vim::feed`] or [`Vim::clamp`].
    pub fn remap_rows(&mut self, f: impl Fn(u32) -> u32) {
        self.cursor.row = f(self.cursor.row);
        self.anchor.row = f(self.anchor.row);
    }

    /// Moves the cursor and the anchor to the points `f` gives (an edit
    /// moved the text), keeping the mode and the goal. The points are
    /// clamped on the next [`Vim::feed`] or [`Vim::clamp`].
    pub fn remap_points(&mut self, f: impl Fn(Point) -> Point) {
        self.cursor = f(self.cursor);
        self.anchor = f(self.anchor);
    }

    /// The pending keys for the indicator, e.g. "5", "g", "2f".
    pub fn pending(&self) -> &str {
        &self.pending
    }

    /// Back to Normal mode with nothing pending, the cursor at `cursor`
    /// (clamped on the next [`Vim::feed`] or [`Vim::clamp`]).
    pub fn reset(&mut self, cursor: Point) {
        *self = Vim::new(cursor);
    }

    /// Keeps the cursor and anchor on a char of `snapshot`, e.g. after a
    /// reload.
    pub fn clamp(&mut self, snapshot: &BufferSnapshot) {
        self.cursor = self.clamp_in_mode(snapshot, self.cursor);
        self.anchor = self.clamp_in_mode(snapshot, self.anchor);
    }

    /// `p` clamped for the current mode: onto a char, except that Visual
    /// (char) mode also allows the line break, where `$` puts the head, and
    /// Insert mode the line end, where `A` types.
    fn clamp_in_mode(&self, snapshot: &BufferSnapshot, p: Point) -> Point {
        if visual::phantom_row(snapshot) == Some(p.row) {
            return Point {
                row: p.row,
                column: 0,
            };
        }
        if matches!(self.mode, Mode::Visual | Mode::Insert) {
            let row = p.row.min(snapshot.line_count().saturating_sub(1));
            let len = visual::line_len(snapshot, row);
            if p.column >= len {
                return Point { row, column: len };
            }
        }
        clamp_point(snapshot, p)
    }

    /// The column `$` and the end-of-line goal reach on `row`: the last
    /// char, or the line break in Visual (char) mode. The latter is the
    /// real line length and deliberately exceeds the `MAX_LINE_CHARS` cap.
    fn eol_col(&self, snapshot: &BufferSnapshot, row: u32) -> u32 {
        if self.mode == Mode::Visual {
            visual::line_len(snapshot, row)
        } else {
            last_char_col(&line(snapshot, row))
        }
    }

    fn step(
        &mut self,
        key: Key,
        view: &dyn ViewInfo,
        globals: &mut VimGlobals,
        settings: &Settings,
    ) -> Outcome {
        if self.mode == Mode::Insert {
            // Every other key types.
            return match key {
                Key::Escape => self.escape_insert(view),
                _ => Outcome::Unhandled,
            };
        }
        match key {
            Key::Escape => self.escape(view, globals),
            Key::Ctrl(c) => {
                let cancelled = self.prefix.is_some() || self.op.is_some();
                let explicit = self.count;
                let n = self.take_count();
                if cancelled {
                    // Keys the workbench binds (ctrl+g, ctrl+tab...) still
                    // reach it; only vim's own scroll keys are swallowed.
                    return if matches!(c, 'd' | 'u' | 'f' | 'b' | 'v') {
                        Outcome::Handled(vec![])
                    } else {
                        Outcome::Unhandled
                    };
                }
                match c {
                    'd' | 'u' => {
                        if let Some(count) = explicit {
                            globals.scroll = count;
                        }
                        let rows = match (globals.scroll, settings.scroll) {
                            (0, 0) => (scroll::capacity(view) / 2).max(1),
                            (0, s) => s,
                            (g, _) => g,
                        };
                        self.scroll(view, ScrollKind::Rows(rows), c == 'd', settings)
                    }
                    'f' | 'b' => {
                        let page = scroll::capacity(view).saturating_sub(2).max(1);
                        let rows = page.saturating_mul(n);
                        self.scroll(view, ScrollKind::Page(rows), c == 'f', settings)
                    }
                    'v' => self.switch_mode(Mode::VisualBlock, view, globals),
                    'r' if self.mode == Mode::Normal => self.undo_redo(Action::Redo(n), view),
                    'o' | 'i' if self.mode == Mode::Normal => {
                        Outcome::Handled(vec![Action::RunCommand(if c == 'o' {
                            "workbench.action.navigateBack"
                        } else {
                            "workbench.action.navigateForward"
                        })])
                    }
                    _ => Outcome::Unhandled,
                }
            }
            Key::Enter => {
                if self.prefix == Some('r') && self.mode != Mode::Normal {
                    // Visual `r<CR>` is not supported: it cancels.
                    self.clear_pending();
                    return Outcome::Handled(vec![]);
                }
                if self.prefix == Some('r') {
                    let n = self.take_count();
                    return self.replace('\n', n, view);
                }
                if self.prefix.is_some() {
                    self.clear_pending();
                    return Outcome::Handled(vec![]);
                }
                if let Some(p) = self.op.take() {
                    let post = self.count;
                    self.clear_pending();
                    return self.op_key(p, None, post, '\n', view, globals, settings);
                }
                let n = self.take_count();
                self.next_line_start(view, n)
            }
            Key::Char(c) => self.char(c, view, globals, settings),
            Key::Backspace => Outcome::Unhandled,
        }
    }

    fn escape(&mut self, view: &dyn ViewInfo, globals: &mut VimGlobals) -> Outcome {
        if !self.pending.is_empty() {
            self.clear_pending();
            Outcome::Handled(vec![])
        } else if self.mode != Mode::Normal {
            Outcome::Handled(self.exit_visual(view, globals))
        } else {
            Outcome::Unhandled
        }
    }

    /// `i a I A o O`: enters Insert mode. The cursor goes where the text
    /// will be typed, except for `o` and `O`, whose new line the view opens.
    fn enter_insert(&mut self, view: &dyn ViewInfo, at: InsertAt, count: Option<u32>) -> Outcome {
        let snapshot = view.snapshot();
        let row = self.cursor.row;
        let text = line(snapshot, row);
        let col = self.cursor.column;
        let column = match at {
            InsertAt::Before | InsertAt::LineBelow | InsertAt::LineAbove => col,
            // After the whole grapheme cluster (a base char with its
            // combining marks), never inside it.
            InsertAt::After => text
                .get(col as usize..)
                .and_then(|rest| rest.graphemes(true).next())
                .map_or(col, |g| col + g.len() as u32),
            // On a blank line, after its whitespace.
            InsertAt::LineStart => text
                .char_indices()
                .find(|(_, c)| !c.is_whitespace())
                .map_or(visual::line_len(snapshot, row), |(i, _)| i as u32),
            InsertAt::LineEnd => visual::line_len(snapshot, row),
        };
        self.mode = Mode::Insert;
        self.insert_count = count.filter(|&n| n > 1);
        self.insert_lines = matches!(at, InsertAt::LineBelow | InsertAt::LineAbove);
        self.cursor = Point { row, column };
        self.anchor = self.cursor;
        self.goal = None;
        Outcome::Handled(vec![
            Action::EnterInsert(at),
            Action::ModeChanged(Mode::Insert),
        ])
    }

    /// Escape in Insert mode: back to Normal, the cursor one char left
    /// (never past column 0) and onto a char.
    fn exit_insert(&mut self, view: &dyn ViewInfo) -> Outcome {
        let snapshot = view.snapshot();
        let text = line(snapshot, self.cursor.row);
        let column = motion::left(&text, self.cursor.column, 1);
        self.mode = Mode::Normal;
        self.cursor = clamp_point(
            snapshot,
            Point {
                row: self.cursor.row,
                column,
            },
        );
        self.anchor = self.cursor;
        self.goal = None;
        Outcome::Handled(vec![
            Action::ModeChanged(Mode::Normal),
            Action::CursorMoved { jump: false },
        ])
    }

    /// `v`, `V`, `Ctrl+v`: enters `mode`, leaves it when already in it, or
    /// switches to it from another visual mode.
    fn switch_mode(
        &mut self,
        mode: Mode,
        view: &dyn ViewInfo,
        globals: &mut VimGlobals,
    ) -> Outcome {
        if self.mode == mode {
            return Outcome::Handled(self.exit_visual(view, globals));
        }
        if self.mode == Mode::Normal {
            self.anchor = self.cursor;
        }
        self.mode = mode;
        self.clamp(view.snapshot());
        Outcome::Handled(vec![Action::ModeChanged(mode)])
    }

    /// Back to Normal with the cursor on the head, keeping the goal.
    pub(crate) fn exit_visual(
        &mut self,
        view: &dyn ViewInfo,
        globals: &mut VimGlobals,
    ) -> Vec<Action> {
        let head = self.cursor;
        self.end_visual(view, globals);
        let mut actions = vec![Action::ModeChanged(Mode::Normal)];
        if self.cursor != head {
            actions.push(Action::CursorMoved { jump: false });
        }
        actions
    }

    /// Visual `y` (`Y`: `linewise`, whole rows): copies the selection and
    /// returns to Normal at its start.
    fn yank(
        &mut self,
        view: &dyn ViewInfo,
        globals: &mut VimGlobals,
        settings: &Settings,
        register: Option<char>,
        linewise: bool,
    ) -> Outcome {
        let snapshot = view.snapshot();
        let Some(mut sel) = self.end_visual(view, globals) else {
            return Outcome::Handled(vec![]);
        };
        if let (
            true,
            VimSelection::Char {
                start,
                end_inclusive,
            },
        ) = (linewise, &sel)
        {
            sel = VimSelection::Line {
                start_row: start.row,
                end_row: end_inclusive.row,
            };
        }
        let text = visual::yank_text(&sel, snapshot, view.tab_size());
        let kind = if matches!(sel, crate::VimSelection::Line { .. }) {
            RegKind::Linewise
        } else {
            RegKind::Charwise
        };
        self.cursor = clamp_point(snapshot, visual::bounds(&sel, snapshot, view.tab_size()).0);
        self.anchor = self.cursor;
        self.goal = None;
        let mut actions = Vec::new();
        actions.extend(globals.write_register(
            register,
            RegContent { text, kind },
            WriteKind::Yank,
            settings.use_system_clipboard,
        ));
        actions.push(Action::ModeChanged(Mode::Normal));
        actions.push(Action::CursorMoved { jump: false });
        Outcome::Handled(actions)
    }

    /// Leaves the visual mode: writes the `<` and `>` marks, puts the
    /// cursor back on a char and returns the selection there was.
    fn end_visual(
        &mut self,
        view: &dyn ViewInfo,
        globals: &mut VimGlobals,
    ) -> Option<VimSelection> {
        let snapshot = view.snapshot();
        let sel = self.selection(snapshot, view.tab_size())?;
        globals.marks.entry(view.path().clone()).or_default().visual =
            Some(visual::bounds(&sel, snapshot, view.tab_size()));
        self.mode = Mode::Normal;
        self.clamp(snapshot);
        self.anchor = self.cursor;
        Some(sel)
    }

    fn char(
        &mut self,
        c: char,
        view: &dyn ViewInfo,
        globals: &mut VimGlobals,
        settings: &Settings,
    ) -> Outcome {
        if self.prefix.is_none()
            && let Some(d) = c.to_digit(10)
            && (d != 0 || self.count.is_some())
        {
            let count = self.count.unwrap_or(0).saturating_mul(10).saturating_add(d);
            self.count = Some(count.min(MAX_COUNT));
            self.set_pending("");
            return Outcome::Handled(vec![]);
        }
        if self.prefix.is_none() && c == '"' {
            if self.op.is_some() {
                self.clear_pending();
                return Outcome::Handled(vec![]);
            }
            self.prefix = Some('"');
            self.set_pending("\"");
            return Outcome::Handled(vec![]);
        }
        if self.prefix == Some('"') {
            // A valid name is kept for the next command; anything else
            // cancels.
            if Registers::is_valid_name(c) {
                self.prefix = None;
                self.register = Some(c);
                self.set_pending("");
            } else {
                self.clear_pending();
            }
            return Outcome::Handled(vec![]);
        }
        // `i` or `a` after an operator or in Visual mode starts a text
        // object.
        let object_prefix = matches!(c, 'i' | 'a')
            && (self.op.is_some() || !matches!(self.mode, Mode::Normal | Mode::Insert));
        // `r` waits for the char to put; under an operator it cancels.
        let replace_prefix = c == 'r' && self.op.is_none();
        if self.prefix.is_none()
            && (object_prefix
                || replace_prefix
                || matches!(
                    c,
                    'g' | 'z' | 'f' | 'F' | 't' | 'T' | 'm' | '\'' | '`' | ']' | '['
                ))
        {
            self.prefix = Some(c);
            self.set_pending(&c.to_string());
            return Outcome::Handled(vec![]);
        }
        let prefix = self.prefix;
        let explicit = self.count;
        let register = self.register;
        if let Some(p) = self.op.take() {
            self.clear_pending();
            return self.op_key(p, prefix, explicit, c, view, globals, settings);
        }
        let n = self.take_count();
        if let Some(p @ ('i' | 'a')) = prefix {
            return self.visual_object(c, p == 'a', n, view);
        }
        if prefix == Some('r') && self.mode != Mode::Normal {
            return self.visual_op('r', Some(c), explicit, register, view, globals, settings);
        }
        if prefix == Some('r') {
            return self.replace(c, n, view);
        }
        if let Some(p @ ('f' | 'F' | 't' | 'T')) = prefix {
            let find = FindChar {
                ch: c,
                forward: matches!(p, 'f' | 't'),
                till: matches!(p, 't' | 'T'),
            };
            globals.last_find = Some(find);
            return self.find(view, find, n, false);
        }
        if prefix == Some('m') {
            if c.is_ascii_lowercase() {
                globals
                    .marks
                    .entry(view.path().clone())
                    .or_default()
                    .letters[c as usize - 'a' as usize] = Some(self.cursor);
            }
            return Outcome::Handled(vec![]);
        }
        if let Some(p @ ('\'' | '`')) = prefix {
            let to = globals
                .marks
                .get(view.path())
                .and_then(|m| m.get(c))
                .map(|to| clamp_point(view.snapshot(), to));
            return match to {
                Some(to) if p == '`' => self.move_to_jump(to, None, true),
                Some(to) => self.go_to_row(view, to.row, true, true),
                None => Outcome::Handled(vec![]),
            };
        }
        if prefix == Some('g') {
            return match c {
                'g' => {
                    let row = explicit.map_or(0, |n| n - 1);
                    self.go_to_row(view, row, settings.startofline, true)
                }
                'e' => self.word(view, WordMotion::PrevEnd, false, n),
                'E' => self.word(view, WordMotion::PrevEnd, true, n),
                'd' => self.run_command("editor.action.revealDefinition"),
                'D' => self.run_command("editor.action.revealDeclaration"),
                'h' => self.run_command("editor.action.showHover"),
                'J' if self.mode != Mode::Normal => {
                    self.visual_op('J', Some('g'), explicit, register, view, globals, settings)
                }
                'J' => {
                    let row = self.cursor.row;
                    self.edit_with(view, |ctx| {
                        crate::edit::join_lines(ctx, row, n, false, false)
                    })
                }
                'p' | 'P' if self.mode == Mode::Normal => {
                    let how = crate::edit::Put {
                        before: c == 'P',
                        after: true,
                    };
                    self.put(how, register, n, view, globals, settings)
                }
                'u' | 'U' | '~' if self.mode == Mode::Normal => {
                    let op = match c {
                        'u' => Operator::Lower,
                        'U' => Operator::Upper,
                        _ => Operator::ToggleCase,
                    };
                    self.start_op(op, explicit, register, &format!("g{c}"))
                }
                _ => Outcome::Handled(vec![]),
            };
        }
        if let Some(p @ (']' | '[')) = prefix {
            return match c {
                'd' if p == ']' => self.run_command("editor.action.marker.next"),
                'd' => self.run_command("editor.action.marker.prev"),
                'c' if p == ']' => self.run_command("workbench.action.editor.nextChange"),
                'c' => self.run_command("workbench.action.editor.previousChange"),
                _ => Outcome::Handled(vec![]),
            };
        }
        if prefix == Some('z') {
            let row = self.cursor.row;
            let fold = |action| Outcome::Handled(vec![Action::Fold { row, action }]);
            let top = |at| {
                Outcome::Handled(vec![Action::ScrollTopRow(scroll::top_for_cursor(
                    view, row, at,
                ))])
            };
            return match c {
                't' => top(Screen::Top),
                'z' => top(Screen::Middle),
                'b' => top(Screen::Bottom),
                'a' => fold(FoldAction::Toggle),
                'c' => fold(FoldAction::Close),
                'o' => fold(FoldAction::Open),
                _ => Outcome::Handled(vec![]),
            };
        }
        let snapshot = view.snapshot();
        let row = self.cursor.row;
        let text = line(snapshot, row);
        let col = self.cursor.column;
        let visual = self.mode != Mode::Normal;
        match c {
            'v' => self.switch_mode(Mode::Visual, view, globals),
            'V' => self.switch_mode(Mode::VisualLine, view, globals),
            'o' if visual => {
                let to = self.anchor;
                self.anchor = self.cursor;
                self.move_to(to, None)
            }
            'y' if visual => self.yank(view, globals, settings, register, false),
            'd' | 'x' | 'D' | 'X' | 'c' | 's' | 'C' | 'S' | 'R' | 'Y' | '>' | '<' | '~' | 'u'
            | 'U' | 'J' | 'p' | 'P'
                if visual =>
            {
                self.visual_op(c, None, explicit, register, view, globals, settings)
            }
            'd' | 'c' | 'y' | '>' | '<' if self.mode == Mode::Normal => {
                let op = match c {
                    'd' => Operator::Delete,
                    'c' => Operator::Change,
                    'y' => Operator::Yank,
                    '>' => Operator::Indent,
                    _ => Operator::Outdent,
                };
                self.start_op(op, explicit, register, &c.to_string())
            }
            'i' | 'a' | 'I' | 'A' | 'o' | 'O' if !visual => {
                let at = match c {
                    'i' => InsertAt::Before,
                    'a' => InsertAt::After,
                    'I' => InsertAt::LineStart,
                    'A' => InsertAt::LineEnd,
                    'o' => InsertAt::LineBelow,
                    _ => InsertAt::LineAbove,
                };
                self.enter_insert(view, at, explicit)
            }
            'x' | 'X' | 's' | 'S' | 'C' | 'D' | 'Y' if !visual => {
                self.shortcut_op(c, explicit, register, view, globals, settings)
            }
            '~' if !visual && settings.tildeop => {
                self.start_op(Operator::ToggleCase, explicit, register, "~")
            }
            '~' if !visual => {
                let at = self.cursor;
                self.edit_with(view, |ctx| crate::edit::toggle_chars(ctx, at, n))
            }
            'J' if !visual => {
                let row = self.cursor.row;
                let js = settings.joinspaces;
                self.edit_with(view, |ctx| crate::edit::join_lines(ctx, row, n, true, js))
            }
            'p' | 'P' if !visual => {
                let how = crate::edit::Put {
                    before: c == 'P',
                    after: false,
                };
                self.put(how, register, n, view, globals, settings)
            }
            'u' if !visual => self.undo_redo(Action::Undo(n), view),
            '.' if !visual => self.dot(explicit, view, globals),
            'h' => self.move_horizontal(Point {
                row,
                column: motion::left(&text, col, n),
            }),
            'l' => self.move_horizontal(Point {
                row,
                column: motion::right(&text, col, n),
            }),
            'j' => self.move_vertical(view, n, true),
            'k' => self.move_vertical(view, n, false),
            '0' => self.move_horizontal(Point { row, column: 0 }),
            '^' => self.move_horizontal(Point {
                row,
                column: motion::first_non_blank(&text),
            }),
            '$' => {
                let row = motion::step_rows(view, row, n - 1, true);
                let column = self.eol_col(snapshot, row);
                self.move_to(Point { row, column }, Some(Goal::Eol))
            }
            'w' => self.word(view, WordMotion::NextStart, false, n),
            'W' => self.word(view, WordMotion::NextStart, true, n),
            'b' => self.word(view, WordMotion::PrevStart, false, n),
            'B' => self.word(view, WordMotion::PrevStart, true, n),
            'e' => self.word(view, WordMotion::NextEnd, false, n),
            'E' => self.word(view, WordMotion::NextEnd, true, n),
            'G' => {
                let row = explicit.map_or(u32::MAX, |n| n - 1);
                self.go_to_row(view, row, settings.startofline, true)
            }
            '%' => match explicit {
                Some(n) if n <= 100 => {
                    let lines = u64::from(snapshot.line_count());
                    let row = (u64::from(n) * lines).div_ceil(100).saturating_sub(1) as u32;
                    self.go_to_row(view, row, true, true)
                }
                Some(_) => Outcome::Handled(vec![]),
                None => self.matching_bracket(view),
            },
            '}' | '{' => {
                let down = c == '}';
                let (row, boundary) = motion::paragraph(snapshot, row, n, down);
                let to = if boundary {
                    Point { row, column: 0 }
                } else if down {
                    Point {
                        row,
                        column: last_char_col(&line(snapshot, row)),
                    }
                } else {
                    Point { row: 0, column: 0 }
                };
                self.move_to_jump(to, None, true)
            }
            'H' | 'M' | 'L' => {
                let which = match c {
                    'H' => Screen::Top,
                    'M' => Screen::Middle,
                    _ => Screen::Bottom,
                };
                let row = scroll::screen_row(view, which, n, settings.scrolloff);
                self.go_to_row(view, row, settings.startofline, true)
            }
            ':' => self.open_command_line(explicit, view, globals),
            '/' | '?' => {
                self.search_count = explicit;
                Outcome::Handled(vec![Action::OpenSearch { backward: c == '?' }])
            }
            '*' | '#' => self.search_word(view, globals, c == '#', n, settings),
            'n' | 'N' => self.repeat_search(view, globals, c == 'N', n, settings),
            ';' | ',' => {
                let Some(mut find) = globals.last_find else {
                    return Outcome::Handled(vec![]);
                };
                if c == ',' {
                    find.forward = !find.forward;
                }
                self.find(view, find, n, true)
            }
            _ => Outcome::Handled(vec![]),
        }
    }

    /// `d c y > < gu gU g~`: waits for a motion, keeping the count and the
    /// register typed before.
    fn start_op(
        &mut self,
        op: Operator,
        count: Option<u32>,
        register: Option<char>,
        keys: &str,
    ) -> Outcome {
        let text = format!(
            "{}{}{keys}",
            register.map_or(String::new(), |r| format!("\"{r}")),
            count.map_or(String::new(), |n| n.to_string()),
        );
        self.op = Some(PendingOp {
            op,
            count,
            register,
            force: None,
            text,
        });
        self.set_pending("");
        Outcome::Handled(vec![])
    }

    /// A key after an operator (`c`, with `prefix` before it and `post`
    /// the count typed after the operator; `\n` is Enter): `v`/`V`, the
    /// doubled operator, or a motion. Anything else cancels.
    #[allow(clippy::too_many_arguments)]
    fn op_key(
        &mut self,
        mut p: PendingOp,
        prefix: Option<char>,
        post: Option<u32>,
        c: char,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Outcome {
        if prefix.is_none() && matches!(c, 'v' | 'V') {
            p.force = Some(if c == 'V' {
                MotionKind::Linewise
            } else {
                MotionKind::Exclusive
            });
            p.text.push(c);
            self.op = Some(p);
            self.count = post;
            self.set_pending("");
            return Outcome::Handled(vec![]);
        }
        if prefix.is_none() && matches!(c, '/' | '?') {
            // The operator waits for the search: `search_submitted` runs
            // it, `search_dismissed` drops it.
            self.op = Some(p);
            self.count = post;
            self.set_pending(&c.to_string());
            return Outcome::Handled(vec![Action::OpenSearch { backward: c == '?' }]);
        }
        let count = match (p.count, post) {
            (None, None) => None,
            (a, b) => Some(
                (u64::from(a.unwrap_or(1)) * u64::from(b.unwrap_or(1))).min(MAX_COUNT as u64)
                    as u32,
            ),
        };
        let n = count.unwrap_or(1);
        let snapshot = view.snapshot();
        let from = self.cursor;
        if let Some(pre @ ('i' | 'a')) = prefix {
            // A text object; `v`/`V` before it are ignored.
            let r = Object::from_key(c)
                .and_then(|obj| object::object_range(snapshot, from, obj, pre == 'a', n, view));
            return match r {
                None => Outcome::Handled(vec![]),
                Some(r) if r.range.is_empty() && r.kind != MotionKind::Linewise => {
                    // `ci(` on `()`: Insert between the brackets.
                    if p.op == Operator::Change && view.edit_ctx().is_some() {
                        self.cursor = snapshot.offset_to_point(r.range.start);
                        return self.change_nothing(view);
                    }
                    Outcome::Handled(vec![])
                }
                Some(r) => self.finish_op(p, r, None, view, g, s),
            };
        }
        let doubled = matches!(
            (p.op, prefix, c),
            (Operator::Delete, None, 'd')
                | (Operator::Change, None, 'c')
                | (Operator::Yank, None, 'y')
                | (Operator::Indent, None, '>')
                | (Operator::Outdent, None, '<')
                | (Operator::Lower, None | Some('g'), 'u')
                | (Operator::Upper, None | Some('g'), 'U')
                | (Operator::ToggleCase, None | Some('g'), '~')
        );
        if doubled {
            let last = snapshot.line_count().saturating_sub(1);
            let to = Point {
                row: from.row.saturating_add(n - 1).min(last),
                column: 0,
            };
            // `v`/`V` are ignored here: a doubled operator always takes
            // whole lines, so a forced kind has no motion end to act on.
            let Some(r) = operator::motion_range(snapshot, from, to, MotionKind::Linewise, None)
            else {
                return Outcome::Handled(vec![]);
            };
            return self.finish_op(p, r, Some(from), view, g, s);
        }
        let big = c == 'W';
        // `cw` on a non-blank is `ce`; on an empty line it changes nothing.
        if p.op == Operator::Change
            && prefix.is_none()
            && matches!(c, 'w' | 'W')
            && !s.change_word_includes_whitespace
        {
            let text = line(snapshot, from.row);
            match text
                .get(from.column as usize..)
                .and_then(|t| t.chars().next())
            {
                None => {
                    return self.change_nothing(view);
                }
                Some(ch) if !ch.is_whitespace() => {
                    let to = operator::change_word_end(snapshot, from, big, n);
                    let r =
                        operator::motion_range(snapshot, from, to, MotionKind::Inclusive, p.force);
                    return match r {
                        Some(r) => self.finish_op(p, r, None, view, g, s),
                        None => self.change_nothing(view),
                    };
                }
                Some(_) => {}
            }
        }
        // `dw` on an empty line deletes it.
        if p.op == Operator::Delete
            && prefix.is_none()
            && matches!(c, 'w' | 'W')
            && n == 1
            && p.force.is_none()
            && visual::line_len(snapshot, from.row) == 0
        {
            let r = operator::motion_range(snapshot, from, from, MotionKind::Linewise, None);
            return match r {
                Some(r) => self.finish_op(p, r, None, view, g, s),
                None => Outcome::Handled(vec![]),
            };
        }
        let (to, kind) = match self.op_target(prefix, c, count, n, view, g, s) {
            Ok(Some(target)) => target,
            Ok(None) => return Outcome::Handled(vec![]),
            Err(retry) => return retry,
        };
        let Some(r) = operator::motion_range(snapshot, from, to, kind, p.force) else {
            // The motion worked but covers nothing: `c` still inserts.
            if p.op == Operator::Change {
                return self.change_nothing(view);
            }
            return Outcome::Handled(vec![]);
        };
        // A yank keeps the cursor when the motion went forward, else it goes
        // to the motion's end.
        let forward = (to.row, to.column) >= (from.row, from.column);
        let yank_cursor = if forward { from } else { to };
        self.finish_op(p, r, Some(yank_cursor), view, g, s)
    }

    /// Where the motion `c` (after `prefix`) goes from the cursor, without
    /// moving it, and its kind. `Ok(None)`: not a motion, or it failed.
    #[allow(clippy::too_many_arguments)]
    fn op_target(
        &self,
        prefix: Option<char>,
        c: char,
        count: Option<u32>,
        n: u32,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Result<Option<(Point, MotionKind)>, Outcome> {
        use MotionKind::{Exclusive, Inclusive, Linewise};
        let snapshot = view.snapshot();
        let from = self.cursor;
        let text = line(snapshot, from.row);
        let at = |column| {
            Some(Point {
                row: from.row,
                column,
            })
        };
        let find = |find: FindChar, repeat: bool| {
            let column = motion::find_char(&text, from.column, find, n, repeat)?;
            let kind = if find.forward { Inclusive } else { Exclusive };
            Some((at(column)?, kind))
        };
        // Runs the Normal-mode motion on a copy of the state.
        let probe = |prefix: Option<char>, count: Option<u32>, key: Key, g: &mut VimGlobals| {
            let mut probe = self.clone();
            probe.op = None;
            probe.pending.clear();
            probe.register = None;
            probe.count = count;
            probe.prefix = prefix;
            match probe.step(key, view, g, s) {
                Outcome::Retry(q) => Err(Outcome::Retry(q)),
                _ => Ok(probe.cursor),
            }
        };
        // A motion that cannot move fails.
        let moved = |to: Point, kind| (to != from).then_some((to, kind));
        Ok(match prefix {
            Some(p @ ('f' | 'F' | 't' | 'T')) => {
                let f = FindChar {
                    ch: c,
                    forward: matches!(p, 'f' | 't'),
                    till: matches!(p, 't' | 'T'),
                };
                g.last_find = Some(f);
                find(f, false)
            }
            Some('g') => match c {
                'g' => Some((probe(Some('g'), count, Key::Char(c), g)?, Linewise)),
                'e' | 'E' => Some((probe(Some('g'), count, Key::Char(c), g)?, Inclusive)),
                _ => None,
            },
            Some(p @ ('\'' | '`')) => g.marks.get(view.path()).and_then(|m| m.get(c)).map(|to| {
                let to = clamp_point(snapshot, to);
                (to, if p == '`' { Exclusive } else { Linewise })
            }),
            Some(_) => None,
            None => match c {
                'h' | 'b' | 'B' | '^' => Some((probe(None, count, Key::Char(c), g)?, Exclusive)),
                '0' => Some((probe(None, None, Key::Char(c), g)?, Exclusive)),
                'l' => {
                    // Unlike the motion, `l` here may reach the line end.
                    let mut column = from.column as usize;
                    for ch in text.get(column..).unwrap_or("").chars().take(n as usize) {
                        column += ch.len_utf8();
                    }
                    at(column as u32).map(|to| (to, Exclusive))
                }
                'w' | 'W' => Some((
                    operator::word_end_for_operator(snapshot, from, c == 'W', n),
                    Exclusive,
                )),
                '$' | 'e' | 'E' => Some((probe(None, count, Key::Char(c), g)?, Inclusive)),
                'G' | 'H' | 'M' | 'L' => Some((probe(None, count, Key::Char(c), g)?, Linewise)),
                'j' | 'k' => moved(probe(None, count, Key::Char(c), g)?, Linewise),
                '\n' => moved(probe(None, count, Key::Enter, g)?, Linewise),
                '%' => match count {
                    Some(n) if n > 100 => None,
                    Some(_) => Some((probe(None, count, Key::Char(c), g)?, Linewise)),
                    None => moved(probe(None, None, Key::Char(c), g)?, Inclusive),
                },
                '{' | '}' => {
                    let down = c == '}';
                    let (row, boundary) = motion::paragraph(snapshot, from.row, n, down);
                    Some(if boundary {
                        (Point { row, column: 0 }, Exclusive)
                    } else if down {
                        // The last paragraph: up to the end of the buffer.
                        let column = last_char_col(&line(snapshot, row));
                        (Point { row, column }, Inclusive)
                    } else {
                        (Point { row: 0, column: 0 }, Exclusive)
                    })
                }
                'n' | 'N' | '*' | '#' => moved(probe(None, count, Key::Char(c), g)?, Exclusive),
                ';' | ',' => {
                    let Some(mut f) = g.last_find else {
                        return Ok(None);
                    };
                    if c == ',' {
                        f.forward = !f.forward;
                    }
                    find(f, true)
                }
                _ => None,
            },
        })
    }

    /// `i…`/`a…` in a visual mode: selects the object at the head, or
    /// grows a selection of more than one char to take it in. A selection
    /// that already holds the object grows to the next one: the next outer
    /// pair for brackets and tags, the object after the selection for the
    /// rest. Paragraphs switch to VisualLine and the rest to Visual.
    fn visual_object(&mut self, c: char, around: bool, n: u32, view: &dyn ViewInfo) -> Outcome {
        let Some(obj) = Object::from_key(c) else {
            return Outcome::Handled(vec![]);
        };
        let s = view.snapshot();
        let tab = view.tab_size();
        // The selection's first and last positions, when it spans more
        // than one char.
        let sel = (self.anchor != self.cursor)
            .then(|| self.selection(s, tab))
            .flatten()
            .map(|sel| visual::bounds(&sel, s, tab));
        let at = |p: Point, count: u32| {
            let mut r = object::object_range(s, p, obj, around, count, view)?;
            if r.kind == MotionKind::Linewise && obj != Object::Paragraph {
                // `vi{` over rows of their own selects them charwise, up
                // to and including the last one's line break.
                let last = s.offset_to_point(r.range.end).row;
                r = OpRange {
                    range: r.range.start..visual::next_row_start(s, last),
                    kind: MotionKind::Exclusive,
                };
            }
            Some(r)
        };
        let Some(mut r) = at(self.cursor, n) else {
            return Outcome::Handled(vec![]);
        };
        let key = |p: Point| (p.row, p.column);
        if let Some((first, last)) = sel {
            let (a, b) = object_bounds(s, &r);
            if key(first) <= key(a) && key(b) <= key(last) {
                let next = if obj.nests() {
                    at(self.cursor, n.saturating_add(1))
                } else {
                    // The position after the selection's last char.
                    let one = VimSelection::Char {
                        start: last,
                        end_inclusive: last,
                    };
                    visual::exclusive_range(&one, s)
                        .map(|r| r.end)
                        .filter(|&p| s.point_to_offset(p) < s.len_bytes())
                        .and_then(|p| at(p, n))
                };
                match next {
                    Some(next) => r = next,
                    None => return Outcome::Handled(vec![]),
                }
            }
        }
        let linewise = r.kind == MotionKind::Linewise;
        if r.range.is_empty() && !linewise {
            return Outcome::Handled(vec![]);
        }
        let (mut start, mut end) = object_bounds(s, &r);
        if let Some((first, last)) = sel {
            start = std::cmp::min_by_key(start, first, |p| key(*p));
            end = std::cmp::max_by_key(end, last, |p| key(*p));
        }
        let mode = if linewise {
            Mode::VisualLine
        } else {
            Mode::Visual
        };
        let mut actions = Vec::new();
        if mode != self.mode {
            actions.push(Action::ModeChanged(mode));
        }
        self.mode = mode;
        self.anchor = start;
        self.cursor = end;
        self.goal = None;
        actions.push(Action::CursorMoved { jump: false });
        Outcome::Handled(actions)
    }

    /// `x X s S C D Y`: the operator they stand for (`dl dh cl cc c$ d$
    /// yy`), so the register rules stay in one place.
    fn shortcut_op(
        &mut self,
        c: char,
        count: Option<u32>,
        register: Option<char>,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Outcome {
        let (op, motion) = match c {
            'x' => (Operator::Delete, 'l'),
            'X' => (Operator::Delete, 'h'),
            's' => (Operator::Change, 'l'),
            'S' => (Operator::Change, 'c'),
            'C' => (Operator::Change, '$'),
            'D' => (Operator::Delete, '$'),
            _ => (Operator::Yank, 'y'),
        };
        let p = PendingOp {
            op,
            count,
            register,
            force: None,
            text: c.to_string(),
        };
        self.op_key(p, None, None, motion, view, g, s)
    }

    /// An operator key in Visual or Visual Line mode (spec §7): `d x D X
    /// c s C S R Y > < ~ u U J p P`, `r` with `arg` the char to put, and
    /// `J` with `arg` `g` for `gJ`. It acts on the selection, writes the
    /// `<` and `>` marks and goes back to Normal (Insert for a change) with
    /// the cursor at the selection start. Visual Block keeps its selection:
    /// only `y` works there. A read-only view only leaves the visual mode.
    #[allow(clippy::too_many_arguments)]
    fn visual_op(
        &mut self,
        c: char,
        arg: Option<char>,
        count: Option<u32>,
        register: Option<char>,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Outcome {
        if self.mode == Mode::VisualBlock {
            return Outcome::Handled(vec![]);
        }
        if c == 'Y' {
            return self.yank(view, g, s, register, true);
        }
        let snapshot = view.snapshot();
        let tab = view.tab_size();
        let Some(sel) = self.end_visual(view, g) else {
            return Outcome::Handled(vec![]);
        };
        let (start, end) = visual::bounds(&sel, snapshot, tab);
        self.cursor = clamp_point(snapshot, start);
        self.anchor = self.cursor;
        self.goal = None;
        let linewise =
            matches!(sel, VimSelection::Line { .. }) || matches!(c, 'D' | 'X' | 'C' | 'S' | 'R');
        let r = if linewise {
            OpRange {
                range: operator::row_start(snapshot, start.row)
                    ..visual::line_end(snapshot, end.row),
                kind: MotionKind::Linewise,
            }
        } else {
            let range = visual::exclusive_range(&sel, snapshot).unwrap_or(start..start);
            OpRange {
                range: snapshot.point_to_offset(range.start)..snapshot.point_to_offset(range.end),
                kind: MotionKind::Inclusive,
            }
        };
        if view.edit_ctx().is_none() {
            return self.left_visual(Outcome::Handled(vec![]));
        }
        let n = count.unwrap_or(1);
        let (first, last) = (start.row, end.row);
        let out = match c {
            '>' | '<' => self.edit_with(view, |ctx| {
                crate::edit::shift_rows(ctx, first, last, c == '<', n)
            }),
            'r' => {
                let Some(ch) = arg else {
                    return self.left_visual(Outcome::Handled(vec![]));
                };
                let range = r.range.clone();
                self.edit_with(view, |ctx| crate::edit::replace_selection(ctx, range, ch))
            }
            'J' => {
                let spaces = arg.is_none();
                let js = s.joinspaces;
                let rows = last - first + 1;
                self.edit_with(view, |ctx| {
                    crate::edit::join_lines(ctx, first, rows, spaces, js)
                })
            }
            'p' | 'P' => {
                let Some(content) = g.read_register(register, s.use_system_clipboard, view.path())
                else {
                    return self.left_visual(Outcome::Handled(vec![]));
                };
                let built = view
                    .edit_ctx()
                    .and_then(|ctx| crate::edit::put_over(ctx, &r, &content, n));
                let Some((plan, cursor)) = built else {
                    return self.left_visual(Outcome::Handled(vec![]));
                };
                let mut actions = Vec::new();
                // `p` puts the old selection in `"`; `P` leaves it alone.
                if c == 'p' && (linewise || !r.range.is_empty()) {
                    actions.extend(crate::edit::write(
                        &r,
                        snapshot,
                        g,
                        s,
                        None,
                        WriteKind::Delete,
                    ));
                }
                actions.push(Action::Edit(plan));
                self.cursor = cursor;
                self.anchor = cursor;
                Outcome::Handled(actions)
            }
            _ => {
                let op = match c {
                    'd' | 'x' | 'D' | 'X' => Operator::Delete,
                    'c' | 's' | 'C' | 'S' | 'R' => Operator::Change,
                    '~' => Operator::ToggleCase,
                    'u' => Operator::Lower,
                    _ => Operator::Upper,
                };
                if r.range.is_empty() && !linewise {
                    // The empty row after a final line break.
                    return if op == Operator::Change {
                        self.change_nothing(view)
                    } else {
                        self.left_visual(Outcome::Handled(vec![]))
                    };
                }
                let p = PendingOp {
                    op,
                    count: None,
                    register,
                    force: None,
                    text: c.to_string(),
                };
                self.finish_op(p, r, None, view, g, s)
            }
        };
        self.left_visual(out)
    }

    /// The actions of a visual operator that ended in Normal mode: the
    /// mode change, then one cursor move.
    fn left_visual(&self, out: Outcome) -> Outcome {
        let Outcome::Handled(mut actions) = out else {
            return out;
        };
        if self.mode == Mode::Normal {
            actions.retain(|a| !matches!(a, Action::CursorMoved { .. }));
            actions.push(Action::ModeChanged(Mode::Normal));
            actions.push(Action::CursorMoved { jump: false });
        }
        Outcome::Handled(actions)
    }

    /// `u` and `Ctrl+r`; nothing in a read-only view.
    fn undo_redo(&self, action: Action, view: &dyn ViewInfo) -> Outcome {
        if view.edit_ctx().is_none() {
            return Outcome::Handled(vec![]);
        }
        Outcome::Handled(vec![action])
    }

    /// `r{c}` (`\n` for Enter) on `n` chars.
    fn replace(&mut self, c: char, n: u32, view: &dyn ViewInfo) -> Outcome {
        let at = self.cursor;
        self.edit_with(view, |ctx| crate::edit::replace_chars(ctx, at, c, n))
    }

    /// `p P gp gP` from `register`; nothing when it is empty.
    fn put(
        &mut self,
        how: crate::edit::Put,
        register: Option<char>,
        n: u32,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Outcome {
        let Some(content) = g.read_register(register, s.use_system_clipboard, view.path()) else {
            return Outcome::Handled(vec![]);
        };
        let at = self.cursor;
        self.edit_with(view, |ctx| crate::edit::put(ctx, at, &content, n, how))
    }

    /// Applies the plan `build` makes (with the cursor after it) and stays
    /// in Normal mode; nothing when the view is read-only or `build` finds
    /// nothing to do.
    fn edit_with(
        &mut self,
        view: &dyn ViewInfo,
        build: impl FnOnce(lucerna_editor::Ctx) -> Option<(lucerna_editor::EditPlan, Point)>,
    ) -> Outcome {
        let Some((plan, cursor)) = view.edit_ctx().and_then(build) else {
            return Outcome::Handled(vec![]);
        };
        let mut actions = Vec::new();
        if !plan.edits.is_empty() {
            actions.push(Action::Edit(plan));
        }
        if actions.is_empty() && cursor == self.cursor {
            return Outcome::Handled(actions);
        }
        actions.push(Action::CursorMoved { jump: false });
        self.cursor = cursor;
        self.anchor = cursor;
        self.goal = None;
        Outcome::Handled(actions)
    }

    /// `c` over an empty range: Insert at the cursor with no edit; nothing
    /// in a read-only view.
    fn change_nothing(&mut self, view: &dyn ViewInfo) -> Outcome {
        if view.edit_ctx().is_none() {
            return Outcome::Handled(vec![]);
        }
        self.enter_insert(view, InsertAt::Before, None)
    }

    /// Runs the operator of `p` on `r`. `yank_cursor` is where a yank
    /// leaves the cursor, when not at the range start.
    fn finish_op(
        &mut self,
        p: PendingOp,
        r: OpRange,
        yank_cursor: Option<Point>,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Outcome {
        let linewise = r.kind == MotionKind::Linewise;
        let Some((mut actions, cursor, mode)) =
            crate::edit::run_operator(p.op, r, view, g, s, p.register)
        else {
            return Outcome::Handled(vec![]);
        };
        let cursor = match (p.op, yank_cursor) {
            (Operator::Yank, Some(at)) => at,
            // Without `startofline`, linewise `d` keeps the column; vim's
            // cursor is clamped to the new text once the edit is applied.
            (Operator::Delete, _) if linewise && !s.startofline => Point {
                row: cursor.row,
                column: self.cursor.column,
            },
            _ => cursor,
        };
        let edited = actions.iter().any(|a| matches!(a, Action::Edit(_)));
        if mode == Mode::Normal && (edited || cursor != self.cursor) {
            actions.push(Action::CursorMoved { jump: false });
        }
        self.mode = mode;
        self.insert_count = None;
        self.cursor = cursor;
        self.anchor = cursor;
        self.goal = None;
        Outcome::Handled(actions)
    }

    /// A command key of normal mode (`gd`, `]d`...); not bound in Visual.
    fn run_command(&self, id: &'static str) -> Outcome {
        if self.mode == Mode::Normal {
            Outcome::Handled(vec![Action::RunCommand(id)])
        } else {
            Outcome::Handled(vec![])
        }
    }

    /// `*` and `#`: searches for the word under the cursor, or the next one
    /// on the line.
    fn search_word(
        &mut self,
        view: &dyn ViewInfo,
        globals: &mut VimGlobals,
        backward: bool,
        n: u32,
        settings: &Settings,
    ) -> Outcome {
        let snapshot = view.snapshot();
        let text = line(snapshot, self.cursor.row);
        let Some(word) = search::word_at(&text, self.cursor.column) else {
            return Outcome::Handled(vec![]);
        };
        let query = SearchQuery {
            pattern: search::escape_regex(&text[word.clone()]),
            is_regex: true,
            case_sensitive: !settings.ignorecase,
            whole_word: true,
        };
        let from = Point {
            row: self.cursor.row,
            column: word.start as u32,
        };
        match self.jump_to_match(view, query.clone(), from, backward, n, settings) {
            Ok(out) => {
                globals.last_search = Some((query, backward));
                out
            }
            Err(retry) => retry,
        }
    }

    /// `n` and `N`: the next match of the last search, in its direction or
    /// the opposite one.
    fn repeat_search(
        &mut self,
        view: &dyn ViewInfo,
        globals: &mut VimGlobals,
        reverse: bool,
        n: u32,
        settings: &Settings,
    ) -> Outcome {
        let Some((query, backward)) = globals.last_search.clone() else {
            return Outcome::Handled(vec![]);
        };
        let from = self.cursor;
        self.jump_to_match(view, query, from, backward != reverse, n, settings)
            .unwrap_or_else(|retry| retry)
    }

    /// Moves to the `n`th match of `query` from `from`, highlighting it.
    /// `Err` is the retry outcome while the view is still searching.
    fn jump_to_match(
        &mut self,
        view: &dyn ViewInfo,
        query: SearchQuery,
        from: Point,
        backward: bool,
        n: u32,
        settings: &Settings,
    ) -> Result<Outcome, Outcome> {
        let Some(matches) = view.matches(&query) else {
            return Err(Outcome::Retry(query));
        };
        let snapshot = view.snapshot();
        let at = snapshot.point_to_offset(from);
        let mut actions = Vec::new();
        if let Some(start) = search::nth_match(&matches, at, backward, n, settings.wrapscan) {
            let to = self.clamp_in_mode(snapshot, snapshot.offset_to_point(start));
            if let Outcome::Handled(moved) = self.move_to_jump(to, None, true) {
                actions = moved;
            }
        }
        actions.push(Action::Highlight(query));
        Ok(Outcome::Handled(actions))
    }

    /// `Ctrl+d/u/f/b`: moves the top row, then the cursor.
    fn scroll(
        &mut self,
        view: &dyn ViewInfo,
        kind: ScrollKind,
        down: bool,
        settings: &Settings,
    ) -> Outcome {
        let mut actions = Vec::new();
        let (rows, page) = match kind {
            ScrollKind::Rows(n) => (n, false),
            ScrollKind::Page(n) => (n, true),
        };
        let top = scroll::scrolled_top(view, rows, down, page);
        if top != view.top_row() {
            actions.push(Action::ScrollTopRow(top));
        }
        let mut row = if page {
            self.cursor.row
        } else {
            motion::step_rows(view, self.cursor.row, rows, down)
        };
        if page {
            let (first, bottom) = scroll::view_span(view, top);
            row = row.clamp(first, bottom.max(first));
        }
        if let Outcome::Handled(more) = self.go_to_row(view, row, settings.startofline, false) {
            actions.extend(more);
        }
        Outcome::Handled(actions)
    }

    fn find(&mut self, view: &dyn ViewInfo, find: FindChar, n: u32, repeat: bool) -> Outcome {
        let text = line(view.snapshot(), self.cursor.row);
        match motion::find_char(&text, self.cursor.column, find, n, repeat) {
            Some(column) => self.move_horizontal(Point {
                row: self.cursor.row,
                column,
            }),
            None => Outcome::Handled(vec![]),
        }
    }

    /// `%` without a count: the bracket at or after the cursor, and its
    /// partner.
    fn matching_bracket(&mut self, view: &dyn ViewInfo) -> Outcome {
        let snapshot = view.snapshot();
        let text = line(snapshot, self.cursor.row);
        let from = self.cursor.column as usize;
        // The cursor may be past the end of the truncated line.
        let Some(at) = text
            .get(from..)
            .and_then(|rest| rest.find(['(', ')', '[', ']', '{', '}']))
        else {
            return Outcome::Handled(vec![]);
        };
        let offset = snapshot.point_to_offset(Point {
            row: self.cursor.row,
            column: (from + at) as u32,
        });
        match view.matching_bracket(offset) {
            Some(partner) => {
                let to = snapshot.offset_to_point(partner);
                self.move_to_jump(to, None, true)
            }
            None => Outcome::Handled(vec![]),
        }
    }

    /// Jumps to `row` (clamped): the first non-blank with `startofline`,
    /// otherwise the goal column.
    pub(crate) fn go_to_row(
        &mut self,
        view: &dyn ViewInfo,
        row: u32,
        startofline: bool,
        jump: bool,
    ) -> Outcome {
        let snapshot = view.snapshot();
        let row = row.min(snapshot.line_count().saturating_sub(1));
        if startofline {
            let column = motion::first_non_blank(&line(snapshot, row));
            self.move_to_jump(Point { row, column }, None, jump)
        } else {
            let (column, goal) = self.goal_column(view, row);
            self.move_to_jump(Point { row, column }, Some(goal), jump)
        }
    }

    fn word(&mut self, view: &dyn ViewInfo, kind: WordMotion, big: bool, n: u32) -> Outcome {
        let to = motion::word_motion(view.snapshot(), self.cursor, kind, big, n);
        self.move_horizontal(to)
    }

    /// Vim's `+`: `n` rows down, to the first non-blank.
    fn next_line_start(&mut self, view: &dyn ViewInfo, n: u32) -> Outcome {
        let row = motion::step_rows(view, self.cursor.row, n, true);
        if row == self.cursor.row {
            return Outcome::Handled(vec![]);
        }
        let column = motion::first_non_blank(&line(view.snapshot(), row));
        self.move_to(Point { row, column }, None)
    }

    fn move_vertical(&mut self, view: &dyn ViewInfo, n: u32, down: bool) -> Outcome {
        let row = motion::step_rows(view, self.cursor.row, n, down);
        if row == self.cursor.row {
            return Outcome::Handled(vec![]);
        }
        let (column, goal) = self.goal_column(view, row);
        self.move_to(Point { row, column }, Some(goal))
    }

    /// The column on `row` that keeps the goal column, and that goal.
    fn goal_column(&self, view: &dyn ViewInfo, row: u32) -> (u32, Goal) {
        let tab = view.tab_size();
        let snapshot = view.snapshot();
        let goal = self.goal.unwrap_or_else(|| {
            let here = line(snapshot, self.cursor.row);
            Goal::Display(display_col(&here, self.cursor.column, tab))
        });
        let column = match goal {
            Goal::Eol => self.eol_col(snapshot, row),
            Goal::Display(_) => motion::col_for_goal(&line(snapshot, row), goal, tab),
        };
        (column, goal)
    }

    fn move_horizontal(&mut self, to: Point) -> Outcome {
        self.move_to(to, None)
    }

    fn move_to(&mut self, to: Point, goal: Option<Goal>) -> Outcome {
        self.move_to_jump(to, goal, false)
    }

    fn move_to_jump(&mut self, to: Point, goal: Option<Goal>, jump: bool) -> Outcome {
        self.goal = goal;
        if to == self.cursor {
            return Outcome::Handled(vec![]);
        }
        self.cursor = to;
        Outcome::Handled(vec![Action::CursorMoved { jump }])
    }

    /// The count typed so far (1 if none), clearing the pending keys.
    fn take_count(&mut self) -> u32 {
        let n = self.count.unwrap_or(1);
        self.clear_pending();
        n
    }

    pub(crate) fn clear_pending(&mut self) {
        self.pending.clear();
        self.count = None;
        self.prefix = None;
        self.register = None;
        self.op = None;
    }

    /// The indicator text: `"x`, then the count, then `extra`; or the
    /// pending operator's keys, then the count after it, then `extra`.
    fn set_pending(&mut self, extra: &str) {
        let head = match &self.op {
            Some(p) => p.text.clone(),
            None => self.register.map_or(String::new(), |r| format!("\"{r}")),
        };
        self.pending = format!(
            "{head}{}{extra}",
            self.count.map_or(String::new(), |n| n.to_string()),
        );
    }
}

/// The first and last positions of a text object's range: for a linewise
/// one, column 0 of its first and last rows; for a charwise one, its first
/// char and its last (a row's length when that is the line break).
fn object_bounds(s: &BufferSnapshot, r: &OpRange) -> (Point, Point) {
    let start = s.offset_to_point(r.range.start);
    if r.kind == MotionKind::Linewise {
        let row = s.offset_to_point(r.range.end).row;
        return (
            Point {
                row: start.row,
                column: 0,
            },
            Point { row, column: 0 },
        );
    }
    let last = s.offset_to_point(s.floor_char_boundary(r.range.end.saturating_sub(1)));
    let column = last.column.min(visual::line_len(s, last.row));
    (
        start,
        Point {
            row: last.row,
            column,
        },
    )
}

/// `p` moved onto a char: the row into the buffer, the column onto a char
/// boundary at or before the last char of the (capped) line. A point
/// already on the empty row after a final line break stays there (at
/// column 0): `o` on the last line opens it. Motions never go there.
fn clamp_point(snapshot: &BufferSnapshot, p: Point) -> Point {
    if visual::phantom_row(snapshot) == Some(p.row) {
        return Point {
            row: p.row,
            column: 0,
        };
    }
    let row = p.row.min(snapshot.line_count().saturating_sub(1));
    let text = line(snapshot, row);
    let mut column = p.column.min(last_char_col(&text)) as usize;
    while !text.is_char_boundary(column) {
        column -= 1;
    }
    Point {
        row,
        column: column as u32,
    }
}

/// The most bytes the copies of a counted insert may type; past it the
/// copies are dropped (`99999ifoo<Esc>` types `foo` once).
const MAX_INSERT_COPY_BYTES: usize = 16 << 20;

/// The most line breaks the copies of a counted insert type through Enter
/// (one edit each, keeping the indentation); past it the copies are one
/// literal insert.
const MAX_INSERT_COPY_BREAKS: usize = 10_000;

/// The typing steps of `copies` more copies of the Insert session `typed`
/// (each after a line break when `lines`, for `o` and `O`), or `None` when
/// they would type more than [`MAX_INSERT_COPY_BYTES`].
///
/// A session of plain text and Enter (the usual one) is typed as few
/// steps as possible, so the cost does not grow with the square of the
/// count: adjacent text joins into one step, typed literally (a step of
/// more than one char gets no auto-close, as in Vim), and each Enter stays
/// its own step to keep the indentation, or, past
/// [`MAX_INSERT_COPY_BREAKS`] of them, becomes a plain `\n` in one literal
/// step. A session with Backspace or Tab replays step by step.
fn insert_copies(typed: &[Typed], lines: bool, copies: usize) -> Option<Vec<ReplayStep>> {
    let copy_bytes = usize::from(lines)
        + typed
            .iter()
            .map(|t| match t {
                Typed::Text(text) => text.len(),
                Typed::Newline | Typed::Tab => 1,
                Typed::DeleteLeft => 0,
            })
            .sum::<usize>();
    if copy_bytes.saturating_mul(copies) > MAX_INSERT_COPY_BYTES {
        return None;
    }
    let plain = typed
        .iter()
        .all(|t| matches!(t, Typed::Text(_) | Typed::Newline));
    if !plain {
        let mut steps = Vec::new();
        for _ in 0..copies {
            if lines {
                steps.push(ReplayStep::Type(Typed::Newline));
            }
            steps.extend(typed.iter().cloned().map(ReplayStep::Type));
        }
        return Some(steps);
    }
    let breaks = usize::from(lines) + typed.iter().filter(|t| matches!(t, Typed::Newline)).count();
    let mut copy = String::new();
    if lines {
        copy.push('\n');
    }
    for t in typed {
        match t {
            Typed::Text(text) => copy.push_str(text),
            _ => copy.push('\n'),
        }
    }
    if breaks.saturating_mul(copies) > MAX_INSERT_COPY_BREAKS || breaks == 0 {
        return Some(vec![ReplayStep::Type(Typed::Text(copy.repeat(copies)))]);
    }
    // Text runs between the Enters, joined across copies.
    let mut steps = Vec::new();
    let mut run = String::new();
    for _ in 0..copies {
        for part in copy.split_inclusive('\n') {
            match part.strip_suffix('\n') {
                Some(text) => {
                    run.push_str(text);
                    if !run.is_empty() {
                        steps.push(ReplayStep::Type(Typed::Text(std::mem::take(&mut run))));
                    }
                    steps.push(ReplayStep::Type(Typed::Newline));
                }
                None => run.push_str(part),
            }
        }
    }
    if !run.is_empty() {
        steps.push(ReplayStep::Type(Typed::Text(run)));
    }
    Some(steps)
}

#[cfg(test)]
mod tests {
    use lucerna_text::Point;

    use crate::test_util::{TestView, assert_keys, feed_keys};
    use crate::{Action, Mode, Outcome, Vim};

    #[test]
    fn unbound_char_is_handled_and_does_nothing() {
        let view = TestView::new("a|bc");
        let mut vim = Vim::new(view.cursor);
        assert_eq!(
            feed_keys(&mut vim, &view, "Q"),
            vec![Outcome::Handled(vec![])]
        );
        assert_eq!(vim.cursor(), Point { row: 0, column: 1 });
    }

    #[test]
    fn unbound_char_clears_pending_count() {
        let view = TestView::new("|a\nb\nc\nd");
        let mut vim = Vim::new(view.cursor);
        feed_keys(&mut vim, &view, "3");
        assert_eq!(vim.pending(), "3");
        feed_keys(&mut vim, &view, "Qj");
        assert_eq!(vim.pending(), "");
        assert_eq!(vim.cursor(), Point { row: 1, column: 0 });
    }

    fn runs(keys: &str, text: &str) -> Vec<Outcome> {
        let view = TestView::new(text);
        let mut vim = Vim::new(view.cursor);
        feed_keys(&mut vim, &view, keys)
    }

    fn run_command(id: &'static str) -> Outcome {
        Outcome::Handled(vec![Action::RunCommand(id)])
    }

    #[test]
    fn gd_runs_reveal_definition() {
        let out = runs("gd", "|abc");
        assert_eq!(out[1], run_command("editor.action.revealDefinition"));
        let out = runs("gD", "|abc");
        assert_eq!(out[1], run_command("editor.action.revealDeclaration"));
        let out = runs("gh", "|abc");
        assert_eq!(out[1], run_command("editor.action.showHover"));
    }

    #[test]
    fn bracket_d_runs_marker_next() {
        assert_eq!(
            runs("]d", "|abc")[1],
            run_command("editor.action.marker.next")
        );
        assert_eq!(
            runs("[d", "|abc")[1],
            run_command("editor.action.marker.prev")
        );
        // Other keys after the bracket stay unbound and clear the prefix.
        assert_eq!(runs("]x", "|abc")[1], Outcome::Handled(vec![]));
    }

    #[test]
    fn bracket_c_runs_next_change() {
        assert_eq!(
            runs("]c", "|abc")[1],
            run_command("workbench.action.editor.nextChange")
        );
        assert_eq!(
            runs("[c", "|abc")[1],
            run_command("workbench.action.editor.previousChange")
        );
    }

    #[test]
    fn ctrl_o_runs_navigate_back() {
        assert_eq!(
            runs("<C-o>", "|abc")[0],
            run_command("workbench.action.navigateBack")
        );
        assert_eq!(
            runs("<C-i>", "|abc")[0],
            run_command("workbench.action.navigateForward")
        );
    }

    #[test]
    fn counts_are_ignored_and_cleared() {
        let view = TestView::new("|a\nb\nc");
        let mut vim = Vim::new(view.cursor);
        let out = feed_keys(&mut vim, &view, "3gdj");
        assert_eq!(out[2], run_command("editor.action.revealDefinition"));
        assert_eq!(vim.cursor(), Point { row: 1, column: 0 });
    }

    #[test]
    fn lsp_keys_are_normal_mode_only() {
        let out = runs("vgd", "|abc");
        assert_eq!(out[2], Outcome::Handled(vec![]));
        let out = runs("v<C-o>", "|abc");
        assert_eq!(out[1], Outcome::Unhandled);
    }

    #[test]
    fn unbound_ctrl_is_unhandled() {
        let view = TestView::new("|abc");
        let mut vim = Vim::new(view.cursor);
        assert_eq!(
            feed_keys(&mut vim, &view, "<C-g>"),
            vec![Outcome::Unhandled]
        );
    }

    #[test]
    fn escape_in_normal_with_nothing_pending_is_unhandled() {
        let view = TestView::new("|abc");
        let mut vim = Vim::new(view.cursor);
        assert_eq!(
            feed_keys(&mut vim, &view, "<Esc>"),
            vec![Outcome::Unhandled]
        );
        assert_eq!(vim.mode(), Mode::Normal);
    }

    #[test]
    fn escape_clears_pending_count() {
        let view = TestView::new("|a\nb\nc\nd\ne\nf\ng");
        let mut vim = Vim::new(view.cursor);
        let out = feed_keys(&mut vim, &view, "5<Esc>");
        assert_eq!(out[1], Outcome::Handled(vec![]));
        assert_eq!(vim.pending(), "");
        assert_keys("|a\nb\nc\nd\ne\nf\ng", "5<Esc>j", "a\n|b\nc\nd\ne\nf\ng");
    }

    #[test]
    fn count_is_shown_as_pending_and_capped() {
        let view = TestView::new("|a");
        let mut vim = Vim::new(view.cursor);
        feed_keys(&mut vim, &view, "12");
        assert_eq!(vim.pending(), "12");
        feed_keys(&mut vim, &view, "<Esc>1234567");
        assert_eq!(vim.pending(), "99999");
    }

    #[test]
    fn motion_reports_cursor_moved() {
        let view = TestView::new("|ab");
        let mut vim = Vim::new(view.cursor);
        assert_eq!(
            feed_keys(&mut vim, &view, "lh"),
            vec![
                Outcome::Handled(vec![Action::CursorMoved { jump: false }]),
                Outcome::Handled(vec![Action::CursorMoved { jump: false }]),
            ]
        );
        assert_eq!(
            feed_keys(&mut vim, &view, "h"),
            vec![Outcome::Handled(vec![])]
        );
    }

    #[test]
    fn set_cursor_and_reset_are_clamped_on_next_key() {
        let view = TestView::new("|abc\nxy");
        let mut vim = Vim::new(view.cursor);
        vim.set_cursor(Point { row: 9, column: 9 });
        feed_keys(&mut vim, &view, "Q");
        assert_eq!(vim.cursor(), Point { row: 1, column: 1 });
        feed_keys(&mut vim, &view, "3");
        vim.reset(Point { row: 0, column: 7 });
        assert_eq!(vim.pending(), "");
        assert_eq!(vim.mode(), Mode::Normal);
        feed_keys(&mut vim, &view, "Q");
        assert_eq!(vim.cursor(), Point { row: 0, column: 2 });
    }

    #[test]
    fn clamp_after_reload() {
        let mut vim = Vim::new(Point { row: 1, column: 2 });
        let shorter = TestView::new("|ab");
        vim.clamp(&shorter.snapshot);
        assert_eq!(vim.cursor(), Point { row: 0, column: 1 });
    }

    #[test]
    fn clamp_never_splits_a_char() {
        let view = TestView::new("|é😀b");
        let mut vim = Vim::new(Point { row: 0, column: 4 });
        vim.clamp(&view.snapshot);
        assert_eq!(vim.cursor(), Point { row: 0, column: 2 });
    }

    #[test]
    fn remap_rows_moves_both_ends_and_keeps_the_mode() {
        let view = TestView::new("|0\n1\n2\n3\n4\n5\n6\n7\n8");
        let mut vim = Vim::new(view.cursor);
        feed_keys(&mut vim, &view, "5jVj");
        vim.remap_rows(|r| r + 2);
        assert_eq!(vim.mode(), Mode::VisualLine);
        assert_eq!(vim.cursor(), Point { row: 8, column: 0 });
        assert_eq!(
            vim.selection(&view.snapshot, 4),
            Some(crate::VimSelection::Line {
                start_row: 7,
                end_row: 8
            })
        );
    }

    #[test]
    fn percent_after_visual_dollar_on_a_huge_line_does_not_panic() {
        let long = format!("|({})", "x".repeat(20_000));
        let view = TestView::new(&long);
        let mut vim = Vim::new(view.cursor);
        feed_keys(&mut vim, &view, "v$%");
        assert_eq!(vim.mode(), Mode::Visual);
    }

    /// The first outcome of `key` from `text`, and the cursor after it.
    fn insert_key(text: &str, key: &str) -> (Outcome, Mode, Point) {
        let view = TestView::new(text);
        let mut vim = Vim::new(view.cursor);
        let out = feed_keys(&mut vim, &view, key).remove(0);
        (out, vim.mode(), vim.cursor())
    }

    #[test]
    fn insert_keys_yield_insert_at() {
        use crate::InsertAt;
        let cases = [
            ("ab|cd", "i", InsertAt::Before, pt(0, 2)),
            ("ab|cd", "a", InsertAt::After, pt(0, 3)),
            ("  ab|cd", "I", InsertAt::LineStart, pt(0, 2)),
            ("ab|cd", "A", InsertAt::LineEnd, pt(0, 4)),
            ("ab|cd\nx", "o", InsertAt::LineBelow, pt(0, 2)),
            ("x\nab|cd", "O", InsertAt::LineAbove, pt(1, 2)),
        ];
        for (text, key, at, cursor) in cases {
            let (out, mode, got) = insert_key(text, key);
            assert_eq!(
                out,
                Outcome::Handled(vec![
                    Action::EnterInsert(at),
                    Action::ModeChanged(Mode::Insert)
                ]),
                "{key}"
            );
            assert_eq!(mode, Mode::Insert, "{key}");
            assert_eq!(got, cursor, "{key}");
        }
    }

    #[test]
    fn a_appends_after_the_last_char_and_on_empty_lines() {
        assert_eq!(insert_key("ab|c", "a").2, pt(0, 3));
        assert_eq!(insert_key("a|é", "a").2, pt(0, 3));
        // `e` + U+0301 COMBINING ACUTE ACCENT: one cluster of 3 bytes.
        assert_eq!(insert_key("a|e\u{301}b", "a").2, pt(0, 4));
        assert_eq!(insert_key("|\nx", "a").2, pt(0, 0));
        // `I` on a blank line types after its whitespace.
        assert_eq!(insert_key("  |  ", "I").2, pt(0, 4));
    }

    #[test]
    fn insert_keys_are_not_vims_in_visual_mode() {
        let view = TestView::new("|abc");
        let mut vim = Vim::new(view.cursor);
        feed_keys(&mut vim, &view, "vi");
        assert_eq!(vim.mode(), Mode::Visual);
    }

    #[test]
    fn insert_mode_passes_keys_on_and_escape_returns_normal() {
        let view = TestView::new("ab|c");
        let mut vim = Vim::new(view.cursor);
        feed_keys(&mut vim, &view, "A");
        assert_eq!(
            feed_keys(&mut vim, &view, "jx<C-d><CR>"),
            vec![Outcome::Unhandled; 4]
        );
        assert_eq!(vim.mode(), Mode::Insert);
        assert_eq!(
            feed_keys(&mut vim, &view, "<Esc>"),
            vec![Outcome::Handled(vec![
                Action::ModeChanged(Mode::Normal),
                Action::CursorMoved { jump: false }
            ])]
        );
        assert_eq!(vim.mode(), Mode::Normal);
        // From the line end (after `c`) back onto `c`.
        assert_eq!(vim.cursor(), pt(0, 2));
    }

    #[test]
    fn escape_moves_left_but_not_past_column_zero() {
        let view = TestView::new("|aé");
        let mut vim = Vim::new(view.cursor);
        feed_keys(&mut vim, &view, "i<Esc>");
        assert_eq!(vim.cursor(), pt(0, 0));
        feed_keys(&mut vim, &view, "A<Esc>");
        assert_eq!(vim.cursor(), pt(0, 1));
    }

    #[test]
    fn set_cursor_keeps_insert_mode() {
        let view = TestView::new("|abc");
        let mut vim = Vim::new(view.cursor);
        feed_keys(&mut vim, &view, "i");
        // The view's caret moved while typing; the line end is allowed.
        vim.set_cursor(pt(0, 3));
        vim.clamp(&view.snapshot);
        assert_eq!(vim.mode(), Mode::Insert);
        assert_eq!(vim.cursor(), pt(0, 3));
        feed_keys(&mut vim, &view, "<Esc>");
        assert_eq!(vim.cursor(), pt(0, 2));
    }

    fn feed_with(vim: &mut Vim, view: &TestView, globals: &mut crate::VimGlobals, keys: &str) {
        feed_keys_with_view(vim, view, globals, keys);
    }

    fn feed_keys_with_view(
        vim: &mut Vim,
        view: &TestView,
        globals: &mut crate::VimGlobals,
        keys: &str,
    ) -> Vec<Outcome> {
        let settings = crate::Settings::default();
        crate::test_util::parse_keys(keys)
            .into_iter()
            .map(|k| vim.feed(k, view, globals, &settings))
            .collect()
    }

    #[test]
    fn register_prefix_shows_in_pending_and_is_used_once() {
        use crate::{RegContent, RegKind};
        let view = TestView::new("|abc def");
        let mut vim = Vim::new(view.cursor);
        let mut g = crate::VimGlobals::default();
        feed_with(&mut vim, &view, &mut g, "vl\"");
        assert_eq!(vim.pending(), "\"");
        feed_with(&mut vim, &view, &mut g, "a");
        assert_eq!(vim.pending(), "\"a");
        feed_with(&mut vim, &view, &mut g, "y");
        assert_eq!(vim.pending(), "");
        let get = |g: &crate::VimGlobals, n| {
            g.read_register(
                Some(n),
                crate::ClipboardMode::Never,
                std::path::Path::new("x"),
            )
        };
        assert_eq!(
            get(&g, 'a'),
            Some(RegContent {
                text: "ab".into(),
                kind: RegKind::Charwise
            })
        );
        assert_eq!(get(&g, '0'), None);
        // The register was spent: the next yank goes to `0`.
        feed_with(&mut vim, &view, &mut g, "vy");
        assert_eq!(get(&g, '0').map(|c| c.text), Some("a".into()));
        assert_eq!(get(&g, 'a').map(|c| c.text), Some("ab".into()));
    }

    #[test]
    fn register_prefix_cancels_on_invalid_name_and_escape() {
        let view = TestView::new("|abc");
        let mut vim = Vim::new(view.cursor);
        let mut g = crate::VimGlobals::default();
        feed_with(&mut vim, &view, &mut g, "\"!");
        assert_eq!(vim.pending(), "");
        feed_with(&mut vim, &view, &mut g, "\"a<Esc>");
        assert_eq!(vim.pending(), "");
        feed_with(&mut vim, &view, &mut g, "vy");
        assert_eq!(
            g.read_register(None, crate::ClipboardMode::Never, std::path::Path::new("x"))
                .map(|c| c.text),
            Some("a".into())
        );
        assert_eq!(
            g.read_register(
                Some('a'),
                crate::ClipboardMode::Never,
                std::path::Path::new("x")
            ),
            None
        );
    }

    #[test]
    fn percent_and_slash_registers_read_path_and_search() {
        let mut g = crate::VimGlobals::default();
        let p = std::path::Path::new("src/main.rs");
        let r = |g: &crate::VimGlobals, n| {
            g.read_register(Some(n), crate::ClipboardMode::Never, p)
                .map(|c| c.text)
        };
        assert_eq!(r(&g, '%'), Some("src/main.rs".into()));
        assert_eq!(r(&g, '/'), None);
        g.last_search = Some((
            crate::SearchQuery {
                pattern: "foo".into(),
                is_regex: false,
                case_sensitive: false,
                whole_word: false,
            },
            false,
        ));
        assert_eq!(r(&g, '/'), Some("foo".into()));
        assert_eq!(r(&g, ':'), None);
    }

    #[test]
    fn insert_keys_keep_their_count_for_the_replay() {
        for keys in ["3i", "3a", "3I", "3A", "3o", "3O"] {
            let view = TestView::new("a|b");
            let mut vim = Vim::new(view.cursor);
            feed_keys(&mut vim, &view, keys);
            assert_eq!(vim.insert_count, Some(3), "{keys}");
        }
        let view = TestView::new("a|bc");
        let mut vim = Vim::new(view.cursor);
        feed_keys(&mut vim, &view, "i");
        assert_eq!(vim.insert_count, None);
        // `s` takes its count as chars, not as repeats.
        let mut vim = Vim::new(view.cursor);
        feed_keys(&mut vim, &view, "2s");
        assert_eq!(vim.mode(), Mode::Insert);
        assert_eq!(vim.insert_count, None);
    }

    #[test]
    fn o_escape_on_the_last_line_stays_on_the_new_row() {
        use crate::test_util::{assert_edit, assert_keys};
        assert_edit("t|t", "o<Esc>", "tt\n|");
        // `k` leaves it; `j` and `G` never reach it.
        assert_edit("t|t", "o<Esc>k", "|tt\n");
        assert_keys("a\n|b\n", "j", "a\n|b\n");
        assert_keys("|a\nb\n", "G", "a\n|b\n");
        // A point past it clamps to the last real line.
        let view = TestView::new("a\n|b\n");
        let mut vim = Vim::new(pt(5, 3));
        vim.clamp(&view.snapshot);
        assert_eq!(vim.cursor(), pt(1, 0));
    }

    #[test]
    fn enter_at_the_end_then_escape_stays_on_the_new_row() {
        let mut view = TestView::new("t|t");
        let mut vim = Vim::new(view.cursor);
        let mut g = crate::VimGlobals::default();
        feed_with(&mut vim, &view, &mut g, "A");
        // The view types Enter: the caret moves to the new empty row.
        view.snapshot = lucerna_text::BufferSnapshot::from_text("tt\n");
        vim.set_cursor(pt(1, 0));
        vim.clamp(&view.snapshot);
        assert_eq!(vim.cursor(), pt(1, 0));
        feed_with(&mut vim, &view, &mut g, "<Esc>");
        assert_eq!(vim.mode(), Mode::Normal);
        assert_eq!(vim.cursor(), pt(1, 0));
    }

    #[test]
    fn dd_on_the_last_real_line_keeps_the_final_line_break() {
        use crate::test_util::assert_edit;
        assert_edit("a\n|b\n", "dd", "|a\n");
        assert_edit("|a\n", "dd", "|");
        assert_edit("a\n|b", "dd", "|a");
    }

    fn pt(row: u32, column: u32) -> Point {
        Point { row, column }
    }

    #[test]
    fn mode_labels() {
        assert_eq!(Mode::Normal.label(), "-- NORMAL --");
        assert_eq!(Mode::Visual.label(), "-- VISUAL --");
        assert_eq!(Mode::VisualLine.label(), "-- VISUAL LINE --");
        assert_eq!(Mode::VisualBlock.label(), "-- VISUAL BLOCK --");
        assert_eq!(Mode::Insert.label(), "-- INSERT --");
        assert_eq!(Mode::Insert.context_value(), "Insert");
        assert_eq!(Mode::Normal.context_value(), "Normal");
        assert_eq!(Mode::Visual.context_value(), "Visual");
        assert_eq!(Mode::VisualLine.context_value(), "VisualLine");
        assert_eq!(Mode::VisualBlock.context_value(), "VisualBlock");
    }
}
