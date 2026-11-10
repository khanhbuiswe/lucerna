//! Pure motions over a buffer snapshot. Columns are UTF-8 byte offsets
//! and every step is a whole char.

use crate::ViewInfo;
use lucerna_text::{BufferSnapshot, Point};

use crate::classify::{CharClass, byte_col_for_display, class, last_char_col, line};

/// The goal column kept by vertical motions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Goal {
    /// A display column, tabs expanded.
    Display(u32),
    /// The end of the line, set by `$`.
    Eol,
}

/// `count` chars left of `col`, stopping at column 0.
pub(crate) fn left(line: &str, col: u32, count: u32) -> u32 {
    line[..(col as usize).min(line.len())]
        .char_indices()
        .rev()
        .take(count as usize)
        .last()
        .map_or(col, |(i, _)| i as u32)
}

/// `count` chars right of `col`, stopping at the last char.
pub(crate) fn right(line: &str, col: u32, count: u32) -> u32 {
    let last = last_char_col(line);
    line.char_indices()
        .map(|(i, _)| i as u32)
        .filter(|&i| i > col)
        .take(count as usize)
        .last()
        .map_or(col, |i| i.min(last))
}

/// The first non-blank char; the last char on a blank line.
pub(crate) fn first_non_blank(line: &str) -> u32 {
    line.char_indices()
        .find(|(_, c)| !c.is_whitespace())
        .map_or_else(|| last_char_col(line), |(i, _)| i as u32)
}

/// The row `count` visible rows below (`down`) or above `row`, skipping
/// hidden rows and stopping at the first or last visible row.
pub(crate) fn step_rows(view: &dyn ViewInfo, row: u32, count: u32, down: bool) -> u32 {
    let last = view.snapshot().line_count().saturating_sub(1);
    let mut row = row;
    let mut left = count;
    let mut probe = row;
    while left > 0 {
        probe = match (down, probe) {
            (true, r) if r < last => r + 1,
            (false, r) if r > 0 => r - 1,
            _ => break,
        };
        if !view.is_row_hidden(probe) {
            row = probe;
            left -= 1;
        }
    }
    row
}

/// The byte column on `line` for `goal`.
pub(crate) fn col_for_goal(line: &str, goal: Goal, tab: u32) -> u32 {
    match goal {
        Goal::Display(d) => byte_col_for_display(line, d, tab),
        Goal::Eol => last_char_col(line),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WordMotion {
    /// `w`, `W`
    NextStart,
    /// `e`, `E`
    NextEnd,
    /// `b`, `B`
    PrevStart,
    /// `ge`, `gE`
    PrevEnd,
}

/// Walks the buffer one char at a time. Every non-empty line has a virtual
/// position just past its last char (class blank) that stands for the line
/// break; an empty line has only that position.
struct Walker<'a> {
    s: &'a BufferSnapshot,
    last_row: u32,
    big: bool,
    row: u32,
    col: usize,
    text: String,
}

/// Class of a position: 0 blank, 1 punctuation, 2 word.
type Cls = u8;

impl<'a> Walker<'a> {
    fn new(s: &'a BufferSnapshot, from: Point, big: bool) -> Self {
        Walker {
            s,
            last_row: s.line_count().saturating_sub(1),
            big,
            row: from.row,
            col: from.column as usize,
            text: line(s, from.row),
        }
    }

    fn point(&self) -> Point {
        Point {
            row: self.row,
            column: self.col as u32,
        }
    }

    fn cls(&self) -> Cls {
        self.text[self.col..]
            .chars()
            .next()
            .map_or(0, |c| match class(c, self.big) {
                CharClass::Space => 0,
                CharClass::Punct => 1,
                CharClass::Word => 2,
            })
    }

    fn on_empty_line(&self) -> bool {
        self.text.is_empty()
    }

    /// Steps forward: `Some(false)` within a line, `Some(true)` onto the
    /// next line (or onto the line-break position), `None` at the end.
    fn inc(&mut self) -> Option<bool> {
        if let Some(c) = self.text[self.col..].chars().next() {
            self.col += c.len_utf8();
            return Some(self.col >= self.text.len());
        }
        if self.row < self.last_row {
            self.row += 1;
            self.text = line(self.s, self.row);
            self.col = 0;
            return Some(true);
        }
        None
    }

    /// Steps backward, like [`Walker::inc`].
    fn dec(&mut self) -> Option<bool> {
        if let Some(c) = self.text[..self.col].chars().next_back() {
            self.col -= c.len_utf8();
            return Some(false);
        }
        if self.row > 0 {
            self.row -= 1;
            self.text = line(self.s, self.row);
            self.col = self.text.len();
            return Some(true);
        }
        None
    }

    /// Moves forward while the class is `c`; `false` if the end was hit.
    fn skip_forward(&mut self, c: Cls) -> bool {
        while self.cls() == c {
            if self.inc().is_none() {
                return false;
            }
        }
        true
    }

    fn next_start(&mut self) {
        let sclass = self.cls();
        let last_line = self.row == self.last_row;
        let Some(_) = self.inc() else { return };
        if sclass != 0 {
            while self.cls() == sclass {
                match self.inc() {
                    None => return,
                    Some(crossed) if crossed && last_line => return,
                    Some(_) => {}
                }
            }
        }
        while self.cls() == 0 {
            if self.col == 0 && self.on_empty_line() {
                break;
            }
            match self.inc() {
                None => return,
                Some(crossed) if crossed && last_line => return,
                Some(_) => {}
            }
        }
    }

    fn prev_start(&mut self) {
        if self.dec().is_none() {
            return;
        }
        while self.cls() == 0 {
            if self.col == 0 && self.on_empty_line() {
                return;
            }
            if self.dec().is_none() {
                return;
            }
        }
        let c = self.cls();
        while self.cls() == c {
            if self.dec().is_none() {
                return;
            }
        }
        self.inc();
    }

    fn next_end(&mut self) {
        let sclass = self.cls();
        if self.inc().is_none() {
            return;
        }
        if sclass != 0 && self.cls() == sclass {
            if !self.skip_forward(sclass) {
                return;
            }
        } else {
            while self.cls() == 0 {
                if self.inc().is_none() {
                    return;
                }
            }
            let c = self.cls();
            if !self.skip_forward(c) {
                return;
            }
        }
        self.dec();
    }

    fn prev_end(&mut self) {
        let sclass = self.cls();
        if self.dec().is_none() {
            return;
        }
        if sclass != 0 {
            while self.cls() == sclass {
                if self.dec().is_none() {
                    return;
                }
            }
        }
        while self.cls() == 0 {
            if self.col == 0 && self.on_empty_line() {
                break;
            }
            if self.dec().is_none() {
                return;
            }
        }
    }
}

/// `count` word motions from `from`. The result is clamped onto a char; a
/// step that makes no progress ends the loop.
pub(crate) fn word_motion(
    s: &BufferSnapshot,
    from: Point,
    kind: WordMotion,
    big: bool,
    count: u32,
) -> Point {
    let mut cur = from;
    for _ in 0..count {
        let mut w = Walker::new(s, cur, big);
        match kind {
            WordMotion::NextStart => w.next_start(),
            WordMotion::NextEnd => w.next_end(),
            WordMotion::PrevStart => w.prev_start(),
            WordMotion::PrevEnd => w.prev_end(),
        }
        let mut next = w.point();
        next.column = next.column.min(last_char_col(&w.text));
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

#[cfg(test)]
mod tests {
    use lucerna_text::Point;

    use crate::test_util::{TestView, assert_keys, assert_keys_with, feed_keys};
    use crate::{Settings, Vim};

    #[test]
    fn l_stops_at_last_char() {
        assert_keys("a|bc\nd", "lll", "ab|c\nd");
        assert_keys("|abc", "5l", "ab|c");
    }

    #[test]
    fn h_stops_at_column_zero() {
        assert_keys("ab|c\nd", "hhhh", "|abc\nd");
        assert_keys("abc\nd|e", "h", "abc\n|de");
    }

    #[test]
    fn j_restores_goal_on_longer_line() {
        assert_keys("ab|cd\nx\nabcd", "jj", "abcd\nx\nab|cd");
        assert_keys("abcd\nx\nab|cd", "kk", "ab|cd\nx\nabcd");
    }

    #[test]
    fn goal_is_a_display_column() {
        // "\tx": the tab covers display columns 0..4.
        assert_keys("abcd|e\n\tx", "j", "abcde\n\t|x");
    }

    #[test]
    fn zero_inside_a_count_is_a_digit() {
        let start = format!("|{}", "x\n".repeat(12));
        let end = format!("{}|{}", "x\n".repeat(10), "x\n".repeat(2));
        assert_keys(&start, "10j", &end);
    }

    #[test]
    fn zero_alone_goes_to_column_zero() {
        assert_keys("  ab|c", "0", "|  abc");
    }

    #[test]
    fn caret_goes_to_first_non_blank() {
        assert_keys("\tf|oo", "^", "\t|foo");
        assert_keys("|\tfoo", "^", "\t|foo");
        assert_keys("|   ", "^", "  | ");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "Can handle $ with a count"
    #[test]
    fn dollar_with_count() {
        assert_keys("te|xt text\ntext\ntext", "3$", "text text\ntext\ntex|t");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "Can handle $ with a count at end of file"
    #[test]
    fn dollar_with_count_at_end_of_file() {
        assert_keys("te|xt text text", "3$", "text text tex|t");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "maintains column position correctly with $"
    #[test]
    fn dollar_keeps_end_of_line_goal() {
        assert_keys(
            "|one one one\ntwo\nthree",
            "$jj",
            "one one one\ntwo\nthre|e",
        );
    }

    #[test]
    fn horizontal_motion_resets_goal() {
        assert_keys("|abcd\nxy\nabcd", "$jhj", "abcd\nxy\n|abcd");
        assert_keys("ab|cd\nxyz", "0j", "abcd\n|xyz");
    }

    #[test]
    fn l_steps_by_char() {
        let view = TestView::new("|aé😀b");
        let mut vim = Vim::new(view.cursor);
        let mut cols = Vec::new();
        for _ in 0..4 {
            feed_keys(&mut vim, &view, "l");
            cols.push(vim.cursor().column);
        }
        assert_eq!(cols, vec![1, 3, 7, 7]);
        feed_keys(&mut vim, &view, "h");
        assert_eq!(vim.cursor().column, 3);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // a list of hidden row ranges
    fn j_and_k_skip_hidden_rows() {
        let hide = |v: &mut TestView| v.hidden = vec![1..2];
        assert_keys_with(Settings::default(), hide, "|a\nb\nc", "j", "a\nb\n|c");
        assert_keys_with(Settings::default(), hide, "a\nb\n|c", "k", "|a\nb\nc");
    }

    #[test]
    fn j_on_last_row_and_k_on_first_row_stay() {
        assert_keys("a\n|b", "j", "a\n|b");
        assert_keys("|a\nb", "k", "|a\nb");
        assert_keys("|a\nb\nc", "9j", "a\nb\n|c");
    }

    #[test]
    fn last_line_without_newline_and_trailing_newline() {
        assert_keys("|ab\ncd\n", "jj$", "ab\nc|d\n");
    }

    #[test]
    fn dollar_on_huge_line_stops_at_char_cap() {
        let start = format!("|{}", "a".repeat(20_000));
        let end = format!("{}|{}", "a".repeat(9_999), "a".repeat(10_001));
        assert_keys(&start, "$", &end);
    }

    #[test]
    fn empty_file_keeps_cursor_at_origin() {
        let view = TestView::new("|");
        let mut vim = Vim::new(view.cursor);
        feed_keys(&mut vim, &view, "hjkl0^$<CR>10j5k3$");
        assert_eq!(vim.cursor(), Point { row: 0, column: 0 });
    }

    #[test]
    fn enter_goes_to_next_first_non_blank() {
        assert_keys("a|bc\n  def\nx", "<CR>", "abc\n  |def\nx");
        assert_keys("|a\nb\n  c", "2<CR>", "a\nb\n  |c");
        assert_keys("a\n|b", "<CR>", "a\n|b");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "w moves to next word"
    #[test]
    fn w_moves_to_next_word() {
        assert_keys("one |two three four", "w", "one two |three four");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "w stops at punctuation"
    #[test]
    fn w_stops_at_punctuation() {
        assert_keys("one |two,.;/three four", "w", "one two|,.;/three four");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "w on punctuation jumps over punctuation"
    #[test]
    fn w_on_punctuation_jumps_over_punctuation() {
        assert_keys("one two|,.;/three four", "w", "one two,.;/|three four");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "w goes over EOL"
    #[test]
    fn w_goes_over_eol() {
        assert_keys("o|ne  \n two three", "w", "one  \n |two three");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "[count]w"
    #[test]
    fn count_w() {
        assert_keys("|one two three four", "2w", "one two |three four");
    }

    const CODE: &str = "if (true) {\n  return true;\n} else {\n\n  return false;\n  \n} // endif";

    /// Runs `keys` on CODE from (row, col) and returns the end (row, col).
    #[track_caller]
    fn from_code(start: (u32, u32), keys: &str) -> (u32, u32) {
        let view = TestView::new(&format!("|{CODE}"));
        let mut vim = Vim::new(Point {
            row: start.0,
            column: start.1,
        });
        feed_keys(&mut vim, &view, keys);
        (vim.cursor().row, vim.cursor().column)
    }

    // Adapted from VSCodeVim test/motion.test.ts: "last word should move to next line"
    #[test]
    fn w_last_word_moves_to_next_line() {
        assert_eq!(from_code((0, 10), "w"), (1, 2));
    }

    // Adapted from VSCodeVim test/motion.test.ts: "stops on empty line"
    #[test]
    fn w_stops_on_empty_line() {
        assert_eq!(from_code((2, 7), "w"), (3, 0));
    }

    // Adapted from VSCodeVim test/motion.test.ts: "skips whitespace only line"
    #[test]
    fn w_skips_whitespace_only_line() {
        assert_eq!(from_code((4, 14), "w"), (6, 0));
    }

    // Adapted from VSCodeVim test/motion.test.ts: "last word on last line should go to end of document"
    #[test]
    fn w_last_word_clamps_to_last_char() {
        assert_eq!(from_code((6, 6), "w"), (6, 9));
        assert_eq!(from_code((6, 6), "5w"), (6, 9));
    }

    #[test]
    fn b_e_big_word_on_code() {
        assert_eq!(from_code((6, 0), "b"), (4, 14));
        assert_eq!(from_code((2, 7), "e"), (4, 7));
        assert_eq!(from_code((0, 3), "W"), (0, 10));
        assert_eq!(from_code((0, 4), "E"), (0, 8));
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/commands.test.ts: "Can handle 'ge' in multiple lines case2"
    #[test]
    fn ge_across_lines_reaches_start() {
        assert_keys(
            "one two\nthree\nfour fiv|e",
            "gegegegege",
            "|one two\nthree\nfour five",
        );
    }

    #[test]
    fn ge_and_big_ge() {
        assert_keys("one two\nthr|ee", "ge", "one tw|o\nthree");
        assert_keys("a.b c|d", "gE", "a.|b cd");
        assert_keys("a.b c|d", "2ge", "a|.b cd");
        assert_keys("one two\nthr|ee", "2ge", "on|e two\nthree");
    }

    #[test]
    fn ge_stops_on_empty_line() {
        assert_keys("a\n\nb|c", "ge", "a\n|\nbc");
    }

    #[test]
    fn vietnamese_words() {
        assert_keys("|Xin chào thế giới", "w", "Xin |chào thế giới");
        assert_keys("|Xin chào thế giới", "ww", "Xin chào |thế giới");
        assert_keys("|Xin chào", "e", "Xi|n chào");
    }

    #[test]
    fn emoji_is_punctuation() {
        assert_keys("a|😀b c", "w", "a😀|b c");
    }

    #[test]
    fn big_b_treats_punctuation_as_word() {
        assert_keys("a.b |c", "B", "|a.b c");
    }

    #[test]
    fn w_stays_put_at_end_of_file() {
        assert_keys("ab\nc|d", "w", "ab\nc|d");
    }
}
