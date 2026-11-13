//! The per-view state machine: mode, cursor, goal column, pending keys.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use lucerna_text::{BufferSnapshot, Point};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::classify::{self, display_col, last_char_col, line};
use crate::marks::FileMarks;
use crate::motion::{self, FindChar, Goal, WordMotion};
use crate::operator::{self, MotionKind, OpRange, Operator, PendingOp};
use crate::register::{Registers, WriteKind};
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
    #[allow(dead_code)] // used by paste
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
    mode: Mode,
    cursor: Point,
    /// `None`: derive it from the cursor on the next vertical motion.
    goal: Option<Goal>,
    /// The other end of a visual selection.
    anchor: Point,
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
        let saved = self.clone();
        let outcome = self.step(key, view, globals, settings);
        self.finish(&saved, saved.cursor, &outcome, view, globals);
        outcome
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
                Key::Escape => self.exit_insert(view),
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
    fn enter_insert(&mut self, view: &dyn ViewInfo, at: InsertAt) -> Outcome {
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
    fn exit_visual(&mut self, view: &dyn ViewInfo, globals: &mut VimGlobals) -> Vec<Action> {
        let head = self.cursor;
        self.end_visual(view, globals);
        let mut actions = vec![Action::ModeChanged(Mode::Normal)];
        if self.cursor != head {
            actions.push(Action::CursorMoved { jump: false });
        }
        actions
    }

    /// Visual `y`: copies the selection and returns to Normal at its start.
    fn yank(
        &mut self,
        view: &dyn ViewInfo,
        globals: &mut VimGlobals,
        settings: &Settings,
        register: Option<char>,
    ) -> Outcome {
        let snapshot = view.snapshot();
        let Some(sel) = self.end_visual(view, globals) else {
            return Outcome::Handled(vec![]);
        };
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
        if self.prefix.is_none()
            && matches!(
                c,
                'g' | 'z' | 'f' | 'F' | 't' | 'T' | 'm' | '\'' | '`' | ']' | '['
            )
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
            'y' if visual => self.yank(view, globals, settings, register),
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
                self.enter_insert(view, at)
            }
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
                    if view.edit_ctx().is_none() {
                        return Outcome::Handled(vec![]);
                    }
                    return self.enter_insert(view, InsertAt::Before);
                }
                Some(ch) if !ch.is_whitespace() => {
                    let to = operator::change_word_end(snapshot, from, big, n);
                    let r =
                        operator::motion_range(snapshot, from, to, MotionKind::Inclusive, p.force);
                    return match r {
                        Some(r) => self.finish_op(p, r, None, view, g, s),
                        None => Outcome::Handled(vec![]),
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
                (to, if p == '`' { Inclusive } else { Linewise })
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
        let Some((mut actions, cursor, mode)) =
            crate::edit::run_operator(p.op, r, view, g, s, p.register)
        else {
            return Outcome::Handled(vec![]);
        };
        let cursor = match (p.op, yank_cursor) {
            (Operator::Yank, Some(at)) => at,
            _ => cursor,
        };
        let edited = actions.iter().any(|a| matches!(a, Action::Edit(_)));
        if mode == Mode::Normal && (edited || cursor != self.cursor) {
            actions.push(Action::CursorMoved { jump: false });
        }
        self.mode = mode;
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
    fn go_to_row(
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

    fn clear_pending(&mut self) {
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

/// `p` moved onto a char: the row into the buffer, the column onto a char
/// boundary at or before the last char of the (capped) line.
fn clamp_point(snapshot: &BufferSnapshot, p: Point) -> Point {
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
            feed_keys(&mut vim, &view, "x"),
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
        feed_keys(&mut vim, &view, "xj");
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
        feed_keys(&mut vim, &view, "x");
        assert_eq!(vim.cursor(), Point { row: 1, column: 1 });
        feed_keys(&mut vim, &view, "3");
        vim.reset(Point { row: 0, column: 7 });
        assert_eq!(vim.pending(), "");
        assert_eq!(vim.mode(), Mode::Normal);
        feed_keys(&mut vim, &view, "x");
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
