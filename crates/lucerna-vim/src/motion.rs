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

/// `col` moved back onto a char boundary of `line` at or before its last
/// char (0 for an empty line).
pub(crate) fn clamp_to_char(line: &str, col: u32) -> usize {
    let mut col = (col.min(last_char_col(line))) as usize;
    while !line.is_char_boundary(col) {
        col -= 1;
    }
    col
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
        let last_row = s.line_count().saturating_sub(1);
        let row = from.row.min(last_row);
        let text = line(s, row);
        let col = clamp_to_char(&text, from.column);
        Walker {
            s,
            last_row,
            big,
            row,
            col,
            text,
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

/// One of the four find-char motions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FindChar {
    pub ch: char,
    pub forward: bool,
    pub till: bool,
}

/// The column `count` matches of `find.ch` away on `line`, or `None` when
/// there are not that many. `repeat` is `;`/`,`: a `till` that would not
/// move looks one match further.
pub(crate) fn find_char(
    line: &str,
    col: u32,
    find: FindChar,
    count: u32,
    repeat: bool,
) -> Option<u32> {
    let col = clamp_to_char(line, col);
    let search = |count: u32| -> Option<u32> {
        let hit = if find.forward {
            let after = col + line[col..].chars().next().map_or(0, char::len_utf8);
            line[after..]
                .char_indices()
                .filter(|&(_, c)| c == find.ch)
                .nth(count as usize - 1)
                .map(|(i, _)| after + i)?
        } else {
            line[..col]
                .char_indices()
                .rev()
                .filter(|&(_, c)| c == find.ch)
                .nth(count as usize - 1)
                .map(|(i, _)| i)?
        };
        if !find.till {
            return Some(hit as u32);
        }
        Some(if find.forward {
            line[..hit].char_indices().next_back().map_or(0, |(i, _)| i)
        } else {
            hit + find.ch.len_utf8()
        } as u32)
    };
    let count = count.max(1);
    let to = search(count)?;
    if find.till && repeat && to as usize == col {
        return search(count.saturating_add(1));
    }
    Some(to)
}

/// `count` paragraph boundaries (empty rows) down or up from `row`; the
/// last or first row when there are fewer. Returns the row and whether it
/// is a boundary.
pub(crate) fn paragraph(s: &BufferSnapshot, row: u32, count: u32, down: bool) -> (u32, bool) {
    let last = s.line_count().saturating_sub(1);
    let empty = |r: u32| s.line(r).is_empty();
    let mut row = row.min(last);
    for _ in 0..count {
        // Leave the run of empty rows we are in.
        while empty(row) && if down { row < last } else { row > 0 } {
            row = if down { row + 1 } else { row - 1 };
        }
        loop {
            if empty(row) {
                break;
            }
            if if down { row == last } else { row == 0 } {
                return (row, false);
            }
            row = if down { row + 1 } else { row - 1 };
        }
    }
    (row, true)
}

#[cfg(test)]
mod tests {
    use lucerna_text::Point;

    use crate::test_util::{TestView, assert_keys, assert_keys_with, feed_keys};
    use crate::{Action, Outcome, Settings, Vim};

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

    /// `n` rows `x`, the cursor `|` at the start of row `at`.
    fn rows(n: usize, at: usize) -> String {
        (0..n)
            .map(|r| if r == at { "|x" } else { "x" })
            .collect::<Vec<_>>()
            .join("\n")
    }

    // The text of VSCodeVim's "paragraph motion" test.
    const PARA: &str = "this text has\n\nmany\nparagraphs\n\n\nin it.\n\nWOW";

    fn para_at(row: usize, col: usize) -> String {
        let mut out = String::new();
        for (r, l) in PARA.split('\n').enumerate() {
            if r > 0 {
                out.push('\n');
            }
            if r == row {
                out.push_str(&l[..col]);
                out.push('|');
                out.push_str(&l[col..]);
            } else {
                out.push_str(l);
            }
        }
        out
    }

    fn settings(f: impl FnOnce(&mut Settings)) -> Settings {
        let mut s = Settings::default();
        f(&mut s);
        s
    }

    // Adapted from VSCodeVim motions.test.ts: "[count]gg (startofline=true), count greater than last line"
    #[test]
    fn count_gg_clamps_to_last_row_at_first_non_blank() {
        assert_keys(
            " 123456\n 123|456\n 123456",
            "9gg",
            " 123456\n 123456\n |123456",
        );
        assert_keys(
            " 123456\n 123|456\n 123456",
            "gg",
            " |123456\n 123456\n 123456",
        );
        assert_keys("a\nb\n|c", "2gg", "a\n|b\nc");
        assert_keys("|a\nb\n c", "G", "a\nb\n |c");
        assert_keys("|a\nb\nc", "2G", "a\n|b\nc");
    }

    // Adapted from VSCodeVim motions.test.ts: "[count]G (startofline=false)"
    #[test]
    fn count_g_keeps_goal_without_startofline() {
        assert_keys_with(
            settings(|s| s.startofline = false),
            |_| {},
            " 123456\n 123|456\n 123456",
            "1G",
            " 123|456\n 123456\n 123456",
        );
        assert_keys_with(
            settings(|s| s.startofline = false),
            |_| {},
            " 123|456\n 123456\n 123456",
            "gg",
            " 123|456\n 123456\n 123456",
        );
    }

    #[test]
    fn percent_with_count_goes_to_line_percentage() {
        let start = format!("|{}", "x\n".repeat(9) + "x");
        let mut want: Vec<String> = vec!["x".into(); 10];
        want[4] = "|x".into();
        assert_keys(&start, "50%", &want.join("\n"));
        want[4] = "x".into();
        want[9] = "|x".into();
        assert_keys(&start, "100%", &want.join("\n"));
        want[9] = "x".into();
        want[0] = "|x".into();
        assert_keys(&start, "1%", &want.join("\n"));
    }

    // Adapted from VSCodeVim motions.test.ts: "Can handle 'f' then ','"
    #[test]
    fn find_and_reverse_repeat() {
        assert_keys("text tex|t", "fe,", "text t|ext");
        assert_keys("|a a a a a", "fa;;;,,,", "a |a a a a");
    }

    // Adapted from VSCodeVim motions.test.ts: "Can handle 't' twice"
    #[test]
    fn till_twice_does_not_jump() {
        assert_keys("text tex|t", "^tttt", "te|xt text");
    }

    // Adapted from VSCodeVim motions.test.ts: "Can handle 'T' twice"
    #[test]
    fn till_backward_twice_does_not_jump() {
        assert_keys("text tex|t", "$TtTt", "text t|ext");
    }

    // Adapted from VSCodeVim repeatableMovement.test.ts: "repeat t"
    #[test]
    fn semicolon_after_till_skips_adjacent_match() {
        assert_keys("|abc abc abc", "tc;", "abc a|bc abc");
        assert_keys("abc abc ab|c", "Tc;", "abc| abc abc");
    }

    #[test]
    fn find_with_count_and_no_match() {
        assert_keys("|a-a-a-a", "2f-", "a-a|-a-a");
        assert_keys("|abc", "fz", "|abc");
        assert_keys("|abc", "5fc", "|abc");
        assert_keys("a|bc", "Fa", "|abc");
    }

    #[test]
    fn find_matches_whole_chars() {
        assert_keys("|Xin chào thế", "fế", "Xin chào th|ế");
        assert_keys("Xin chào |thế", "fế", "Xin chào th|ế");
        assert_keys("Xin chào thế|", "Fà", "Xin ch|ào thế");
    }

    #[test]
    fn find_reports_no_jump_and_pending() {
        let view = TestView::new("|acbc");
        let mut vim = Vim::new(view.cursor);
        feed_keys(&mut vim, &view, "2f");
        assert_eq!(vim.pending(), "2f");
        let out = feed_keys(&mut vim, &view, "c");
        assert_eq!(
            out,
            vec![Outcome::Handled(vec![Action::CursorMoved { jump: false }])]
        );
    }

    #[test]
    fn jumps_report_jump_true() {
        let view = TestView::new("|a\nb\nc");
        let mut vim = Vim::new(view.cursor);
        assert_eq!(
            feed_keys(&mut vim, &view, "G"),
            vec![Outcome::Handled(vec![Action::CursorMoved { jump: true }])]
        );
        assert_eq!(
            feed_keys(&mut vim, &view, "gg"),
            vec![
                Outcome::Handled(vec![]),
                Outcome::Handled(vec![Action::CursorMoved { jump: true }])
            ]
        );
    }

    // Adapted from VSCodeVim matchingBracket.test.ts: "% matches the nearest bracket"
    #[test]
    fn percent_matches_brackets() {
        assert_keys("|one (two)", "%", "one (two|)");
        assert_keys("((( ))|)", "%", "|((( )))");
        assert_keys("|foo->bar(baz);", "%", "foo->bar(baz|);");
        assert_keys("|foo bar", "%", "|foo bar");
        assert_keys("|foo (bar", "%", "|foo (bar");
    }

    // Adapted from VSCodeVim motion.test.ts: "paragraph motion"
    #[test]
    fn paragraph_motion() {
        assert_keys(&para_at(2, 0), "}", &para_at(4, 0));
        assert_keys(&para_at(4, 0), "}", &para_at(7, 0));
        assert_keys(&para_at(7, 0), "}", &para_at(8, 2));
        assert_keys(&para_at(5, 0), "{", &para_at(1, 0));
        assert_keys(&para_at(2, 2), "{", &para_at(1, 0));
        assert_keys(&para_at(1, 0), "{", &para_at(0, 0));
        assert_keys(&para_at(8, 1), "2{", &para_at(5, 0));
        assert_keys(&para_at(0, 3), "2}", &para_at(4, 0));
    }

    #[test]
    fn paragraph_motion_treats_blank_rows_as_text() {
        assert_keys("|a\n  \nb\n\nc", "}", "a\n  \nb\n|\nc");
    }

    #[test]
    fn motions_survive_a_cursor_past_the_line() {
        let view = TestView::new("é|b\nxy");
        for column in [0, 1, 5, 99] {
            let mut vim = Vim::new(Point { row: 0, column });
            let mut globals = crate::VimGlobals::default();
            for k in crate::test_util::parse_keys("wbegefxtxFx%{}HML") {
                vim.feed(k, &view, &mut globals, &Settings::default());
            }
        }
        let s = lucerna_text::Buffer::from_bytes("é a".as_bytes(), false)
            .unwrap()
            .snapshot();
        let p = super::word_motion(
            &s,
            Point { row: 9, column: 1 },
            super::WordMotion::NextStart,
            false,
            1,
        );
        assert_eq!(p.row, 0);
    }

    #[test]
    fn screen_rows_in_view() {
        let setup = |top: u32, vis: u32| {
            move |v: &mut TestView| {
                v.top_row = top;
                v.visible_rows = vis;
            }
        };
        assert_keys_with(
            Settings::default(),
            setup(10, 20),
            &rows(40, 0),
            "3H",
            &rows(40, 12),
        );
        assert_keys_with(
            settings(|s| s.scrolloff = 2),
            setup(10, 20),
            &rows(40, 0),
            "L",
            &rows(40, 27),
        );
        assert_keys_with(
            settings(|s| s.scrolloff = 2),
            setup(10, 20),
            &rows(40, 0),
            "H",
            &rows(40, 12),
        );
        // No margin at the file top or bottom.
        assert_keys_with(
            settings(|s| s.scrolloff = 2),
            setup(0, 20),
            &rows(40, 30),
            "H",
            &rows(40, 0),
        );
        assert_keys_with(
            settings(|s| s.scrolloff = 2),
            setup(20, 20),
            &rows(40, 0),
            "L",
            &rows(40, 39),
        );
        assert_keys_with(
            Settings::default(),
            setup(0, 0),
            &rows(40, 39),
            "L",
            &rows(40, 29),
        );
        assert_keys_with(
            Settings::default(),
            setup(0, 30),
            &rows(5, 0),
            "M",
            &rows(5, 2),
        );
        assert_keys_with(
            Settings::default(),
            setup(0, 30),
            &rows(5, 0),
            "L",
            &rows(5, 4),
        );
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn screen_rows_skip_folded_rows() {
        let setup = |v: &mut TestView| v.hidden = vec![1..3];
        assert_keys_with(Settings::default(), setup, &rows(6, 0), "2H", &rows(6, 3));
    }

    #[test]
    fn screen_rows_honor_startofline() {
        assert_keys_with(Settings::default(), |_| {}, "|a\n  b", "L", "a\n  |b");
        assert_keys_with(
            settings(|s| s.startofline = false),
            |_| {},
            "ab|c\n  def",
            "L",
            "abc\n  |def",
        );
    }
}
