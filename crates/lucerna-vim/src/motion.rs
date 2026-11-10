//! Pure motions over a buffer snapshot. Columns are UTF-8 byte offsets
//! and every step is a whole char.

use crate::ViewInfo;
use crate::classify::{byte_col_for_display, last_char_col};

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
}
