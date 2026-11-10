//! The per-view state machine: mode, cursor, goal column, pending keys.

use lucerna_text::{BufferSnapshot, Point};

use crate::classify::{display_col, last_char_col, line};
use crate::motion::{self, Goal, WordMotion};
use crate::{Action, Key, Mode, Outcome, Settings, ViewInfo};

/// The largest count; more digits are ignored.
const MAX_COUNT: u32 = 99_999;

/// State shared by every view of one workbench.
#[derive(Debug, Default)]
pub struct VimGlobals {}

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
        _globals: &mut VimGlobals,
        _settings: &Settings,
    ) -> Outcome {
        match key {
            Key::Escape => self.escape(),
            Key::Ctrl(_) => {
                self.clear_pending();
                Outcome::Unhandled
            }
            Key::Enter => {
                let n = self.take_count();
                self.next_line_start(view, n)
            }
            Key::Char(c) => self.char(c, view),
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

    fn char(&mut self, c: char, view: &dyn ViewInfo) -> Outcome {
        if self.prefix.is_none()
            && let Some(d) = c.to_digit(10)
            && (d != 0 || self.count.is_some())
        {
            let count = self.count.unwrap_or(0).saturating_mul(10).saturating_add(d);
            self.count = Some(count.min(MAX_COUNT));
            self.pending = self.count.unwrap_or_default().to_string();
            return Outcome::Handled(vec![]);
        }
        if self.prefix.is_none() && c == 'g' {
            self.prefix = Some('g');
            self.pending = format!("{}g", self.count.map_or(String::new(), |n| n.to_string()));
            return Outcome::Handled(vec![]);
        }
        let prefix = self.prefix;
        let n = self.take_count();
        if prefix == Some('g') {
            return match c {
                'e' => self.word(view, WordMotion::PrevEnd, false, n),
                'E' => self.word(view, WordMotion::PrevEnd, true, n),
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
            _ => Outcome::Handled(vec![]),
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
        let tab = view.tab_size();
        let snapshot = view.snapshot();
        let goal = self.goal.unwrap_or_else(|| {
            let here = line(snapshot, self.cursor.row);
            Goal::Display(display_col(&here, self.cursor.column, tab))
        });
        let column = motion::col_for_goal(&line(snapshot, row), goal, tab);
        self.move_to(Point { row, column }, Some(goal))
    }

    fn move_horizontal(&mut self, to: Point) -> Outcome {
        self.move_to(to, None)
    }

    fn move_to(&mut self, to: Point, goal: Option<Goal>) -> Outcome {
        self.goal = goal;
        if to == self.cursor {
            return Outcome::Handled(vec![]);
        }
        self.cursor = to;
        Outcome::Handled(vec![Action::CursorMoved { jump: false }])
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
