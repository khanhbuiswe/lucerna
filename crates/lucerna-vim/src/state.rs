//! The per-view state machine: mode, cursor, goal column, pending keys.

use lucerna_text::{BufferSnapshot, Point};

use crate::classify::{display_col, last_char_col, line};
use crate::motion::{self, FindChar, Goal, WordMotion};
use crate::scroll::{self, Screen, ScrollKind};
use crate::{Action, FoldAction, Key, Mode, Outcome, Settings, ViewInfo};

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
        if matches!(outcome, Outcome::Retry(_)) {
            *self = saved;
        }
        outcome
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn cursor(&self) -> Point {
        self.cursor
    }

    /// Moves the cursor from outside vim and forgets the goal column. The
    /// point is clamped to the buffer on the next [`Vim::feed`] or
    /// [`Vim::clamp`].
    pub fn set_cursor(&mut self, p: Point) {
        self.cursor = p;
        self.goal = None;
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
        self.cursor = clamp_point(snapshot, self.cursor);
        self.anchor = clamp_point(snapshot, self.anchor);
    }

    fn step(
        &mut self,
        key: Key,
        view: &dyn ViewInfo,
        globals: &mut VimGlobals,
        settings: &Settings,
    ) -> Outcome {
        match key {
            Key::Escape => self.escape(),
            Key::Ctrl(c) => {
                let cancelled = self.prefix.is_some();
                let explicit = self.count;
                let n = self.take_count();
                if cancelled {
                    return Outcome::Handled(vec![]);
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
                    _ => Outcome::Unhandled,
                }
            }
            Key::Enter => {
                if self.prefix.is_some() {
                    self.clear_pending();
                    return Outcome::Handled(vec![]);
                }
                let n = self.take_count();
                self.next_line_start(view, n)
            }
            Key::Char(c) => self.char(c, view, globals, settings),
        }
    }

    fn escape(&mut self) -> Outcome {
        if !self.pending.is_empty() {
            self.clear_pending();
            Outcome::Handled(vec![])
        } else if self.mode != Mode::Normal {
            self.mode = Mode::Normal;
            Outcome::Handled(vec![Action::ModeChanged(Mode::Normal)])
        } else {
            Outcome::Unhandled
        }
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
            self.pending = self.count.unwrap_or_default().to_string();
            return Outcome::Handled(vec![]);
        }
        if self.prefix.is_none() && matches!(c, 'g' | 'z' | 'f' | 'F' | 't' | 'T') {
            self.prefix = Some(c);
            self.pending = format!("{}{c}", self.count.map_or(String::new(), |n| n.to_string()));
            return Outcome::Handled(vec![]);
        }
        let prefix = self.prefix;
        let explicit = self.count;
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
        if prefix == Some('g') {
            return match c {
                'g' => {
                    let row = explicit.map_or(0, |n| n - 1);
                    self.go_to_row(view, row, settings.startofline, true)
                }
                'e' => self.word(view, WordMotion::PrevEnd, false, n),
                'E' => self.word(view, WordMotion::PrevEnd, true, n),
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
        match c {
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
                let column = last_char_col(&line(snapshot, row));
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
        let Some(at) = text[from..].find(['(', ')', '[', ']', '{', '}']) else {
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
        (motion::col_for_goal(&line(snapshot, row), goal, tab), goal)
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
    fn mode_labels() {
        assert_eq!(Mode::Normal.label(), "-- NORMAL --");
        assert_eq!(Mode::Visual.label(), "-- VISUAL --");
        assert_eq!(Mode::VisualLine.label(), "-- VISUAL LINE --");
        assert_eq!(Mode::VisualBlock.label(), "-- VISUAL BLOCK --");
        assert_eq!(Mode::Normal.context_value(), "Normal");
        assert_eq!(Mode::Visual.context_value(), "Visual");
        assert_eq!(Mode::VisualLine.context_value(), "VisualLine");
        assert_eq!(Mode::VisualBlock.context_value(), "VisualBlock");
    }
}
