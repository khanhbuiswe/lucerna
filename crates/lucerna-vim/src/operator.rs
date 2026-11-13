//! Operators and the ranges their motions cover (spec §4.1, §4.2).

use std::ops::Range;

use lucerna_text::{BufferSnapshot, Point};

use crate::classify::line;
use crate::motion;
use crate::visual::{line_end, line_len};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    Lower,
    Upper,
    ToggleCase,
}

/// How a motion's end counts (Vim's `:h exclusive`, `:h linewise`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MotionKind {
    Exclusive,
    Inclusive,
    Linewise,
}

/// What an operator acts on. A charwise range is the exact bytes; a
/// linewise one runs from the start of its first row to the end of its
/// last row's text, before the line break.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpRange {
    pub range: Range<usize>,
    pub kind: MotionKind,
}

/// An operator waiting for its motion.
#[derive(Debug, Clone)]
pub(crate) struct PendingOp {
    pub op: Operator,
    /// The count typed before the operator.
    pub count: Option<u32>,
    pub register: Option<char>,
    /// `v` (any charwise kind: it toggles) or `V` (`Linewise`).
    pub force: Option<MotionKind>,
    /// The keys typed so far, for the indicator: `"a2d`, `gu`, `dv`.
    pub text: String,
}

/// The offset of `row`'s first byte.
pub(crate) fn row_start(s: &BufferSnapshot, row: u32) -> usize {
    s.point_to_offset(Point { row, column: 0 })
}

/// The range a motion covers from `from`, after Vim's exclusive/linewise rules (§4.2).
///
/// `force` is the `v` or `V` typed between the operator and the motion:
/// `Some(Linewise)` for `V`, any charwise kind for `v`, which turns a
/// linewise motion exclusive and toggles a charwise one. A forced motion
/// skips the exclusive rule. `None` for an empty charwise range.
pub(crate) fn motion_range(
    s: &BufferSnapshot,
    from: Point,
    to: Point,
    kind: MotionKind,
    force: Option<MotionKind>,
) -> Option<OpRange> {
    let (a, b) = if (from.row, from.column) <= (to.row, to.column) {
        (from, to)
    } else {
        (to, from)
    };
    let kind = match (force, kind) {
        (None, k) => k,
        (Some(MotionKind::Linewise), _) => MotionKind::Linewise,
        (Some(_), MotionKind::Linewise | MotionKind::Inclusive) => MotionKind::Exclusive,
        (Some(_), MotionKind::Exclusive) => MotionKind::Inclusive,
    };
    let lines = |first: u32, last: u32| OpRange {
        range: row_start(s, first)..line_end(s, last),
        kind: MotionKind::Linewise,
    };
    let at = |p: Point| s.point_to_offset(p);
    let r = match kind {
        MotionKind::Linewise => return Some(lines(a.row, b.row)),
        MotionKind::Exclusive if force.is_none() && b.column == 0 && b.row > a.row => {
            // The exclusive rule: the end goes back to the end of the
            // previous line, and from the line start it takes whole lines.
            if a.column <= motion::first_non_blank(&line(s, a.row)) {
                return Some(lines(a.row, b.row - 1));
            }
            OpRange {
                range: at(a)..line_end(s, b.row - 1),
                kind: MotionKind::Inclusive,
            }
        }
        MotionKind::Exclusive => OpRange {
            range: at(a)..at(b),
            kind,
        },
        MotionKind::Inclusive => {
            let end = at(b);
            let next = s
                .text_for_range(end..line_end(s, b.row).max(end))
                .chars()
                .next()
                .map_or(end, |c| end + c.len_utf8());
            OpRange {
                range: at(a)..next,
                kind,
            }
        }
    };
    (!r.range.is_empty()).then_some(r)
}

/// Whether a word (`big`: a WORD) starts at `p`.
fn is_word_start(s: &BufferSnapshot, p: Point, big: bool) -> bool {
    let text = line(s, p.row);
    let col = p.column as usize;
    let Some(c) = text.get(col..).and_then(|t| t.chars().next()) else {
        return false;
    };
    let class = |c| crate::classify::class(c, big);
    if class(c) == crate::classify::CharClass::Space {
        return false;
    }
    text[..col]
        .chars()
        .next_back()
        .is_none_or(|p| class(p) != class(c))
}

/// Where `w`/`W` end under an operator (`:h WORD` special cases): a word
/// that ends a line ends the motion at that line's end, not on the next
/// line, and running out of words reaches the end of the buffer.
pub(crate) fn word_end_for_operator(s: &BufferSnapshot, from: Point, big: bool, n: u32) -> Point {
    let next = |p, n| motion::word_motion(s, p, motion::WordMotion::NextStart, big, n);
    let target = next(from, n);
    let last = if n > 1 { next(from, n - 1) } else { from };
    let last_row = s.line_count().saturating_sub(1);
    if target.row > last.row {
        let eol = Point {
            row: last.row,
            column: line_len(s, last.row),
        };
        if eol != from {
            return eol;
        }
    }
    let at_end = target.row == last_row
        && target.column == crate::classify::last_char_col(&line(s, last_row));
    if at_end && (target == from || !is_word_start(s, target, big)) {
        return Point {
            row: target.row,
            column: line_len(s, target.row),
        };
    }
    target
}

/// Where `cw`/`cW` end on a non-blank: like `e`/`E`, but a cursor already
/// on the end of a word counts that word as the first (inclusive).
pub(crate) fn change_word_end(s: &BufferSnapshot, from: Point, big: bool, n: u32) -> Point {
    let text = line(s, from.row);
    let col = from.column as usize;
    let class = |c| crate::classify::class(c, big);
    let mut rest = text[col.min(text.len())..].chars();
    let at_word_end = match (rest.next(), rest.next()) {
        (Some(c), Some(d)) => class(c) != class(d),
        _ => true,
    };
    let n = if at_word_end { n - 1 } else { n };
    motion::word_motion(s, from, motion::WordMotion::NextEnd, big, n)
}

#[cfg(test)]
mod tests {
    use crate::test_util::{assert_edit, assert_edit_with, run_edit, run_edit_with};
    use crate::{Action, ClipboardMode, InsertAt, Mode, Settings};

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle dw across lines (1)"
    #[test]
    fn dw_across_lines() {
        assert_edit("one |two\n  three", "dw", "one| \n  three");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'cw' without deleting following white spaces"
    #[test]
    fn cw_keeps_following_whitespace() {
        assert_edit("|const a = 1;", "cw", "| a = 1;");
        let (_, actions) = run_edit("|const a = 1;", "cw");
        assert!(matches!(
            actions.as_slice(),
            [
                Action::Edit(_),
                Action::EnterInsert(InsertAt::Before),
                Action::ModeChanged(Mode::Insert)
            ]
        ));
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'c2w'"
    #[test]
    fn c2w() {
        assert_edit("|const a = 1;", "c2w", "| = 1;");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'cw' on white space"
    #[test]
    fn cw_on_whitespace() {
        assert_edit("| const a = 1;", "0cw", "|const a = 1;");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'cw' without removing EOL"
    #[test]
    fn cw_keeps_eol() {
        assert_edit("|text;\ntext", "llllcw", "text|\ntext");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'de'"
    #[test]
    fn de() {
        assert_edit("text tex|t", "^de", "| text");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'dj'"
    #[test]
    fn dj() {
        assert_edit("|11\n22\n\t33\n44\n55\n66", "dj", "\t|33\n44\n55\n66");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle d} at beginning of line"
    #[test]
    fn d_close_brace_at_line_start() {
        assert_edit("|foo\nbar\n\nfun", "d}", "|\nfun");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 3dd off end of document"
    #[test]
    fn three_dd_off_end() {
        assert_edit("one\ntwo\nthree\n|four\nfive", "3dd", "one\ntwo\n|three");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "can delete linewise with d2gg backwards"
    #[test]
    fn d2gg_backwards() {
        assert_edit("one\ntwo\nthr|ee\nfour", "d2gg", "one\n|four");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'dl' at end of line"
    #[test]
    fn dl_at_end_of_line() {
        assert_edit("bla|h", "$dldldl", "|b");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle guw"
    #[test]
    fn guw() {
        assert_edit("|ABC DEF", "guw", "|abc DEF");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can handle 'g~~'"
    #[test]
    fn g_tilde_tilde() {
        assert_edit("oNe\nTw|O\ntHrEe", "g~~", "oNe\n|tWo\ntHrEe");
    }

    #[test]
    fn doubled_case_operators() {
        assert_edit("a|Bc\nx", "gugu", "|abc\nx");
        assert_edit("a|Bc\nx", "guu", "|abc\nx");
        assert_edit("a|bc\nx", "gUU", "|ABC\nx");
        assert_edit("a|bc\nx", "gUgU", "|ABC\nx");
        assert_edit("a|Bc\nx", "g~g~", "|AbC\nx");
    }

    #[test]
    fn dd_last_line_without_newline() {
        assert_edit("a\n|b", "dd", "|a");
    }

    #[test]
    fn dd_on_empty_file() {
        assert_edit("|", "dd", "|");
    }

    #[test]
    fn counts_multiply() {
        assert_edit("|a b c d e f g h", "2d3w", "|g h");
    }

    #[test]
    fn dvj_is_charwise() {
        assert_edit("a|bc\ndef", "dvj", "a|ef");
        // `V` makes a charwise motion linewise.
        assert_edit("a|bc\ndef\nx", "dVl", "|def\nx");
    }

    #[test]
    fn indent_doubled_uses_tab_size() {
        assert_edit("|abc", ">>", "    |abc");
        assert_edit("a\n    |b\nc", "<lt><lt>", "a\n|b\nc");
        assert_edit("|a\nb\nc", "2>>", "    |a\n    b\nc");
    }

    #[test]
    fn yw_keeps_cursor_and_fills_register_zero() {
        let mut g = crate::VimGlobals::default();
        let mut view = crate::test_util::TestView::new("|one two");
        let mut vim = crate::Vim::new(view.cursor);
        let s = Settings::default();
        for k in crate::test_util::parse_keys("yw") {
            if let crate::Outcome::Handled(a) = vim.feed(k, &view, &mut g, &s) {
                assert!(!a.iter().any(|a| matches!(a, Action::Edit(_))));
                view.apply(&mut vim, &a);
            }
        }
        assert_eq!(view.marked(vim.cursor()), "|one two");
        let reg = g.read_register(Some('0'), ClipboardMode::Never, std::path::Path::new("x"));
        assert_eq!(reg.map(|r| r.text), Some("one ".to_string()));
    }

    #[test]
    fn failed_motion_makes_no_edit() {
        let (text, actions) = run_edit("|abc", "dfz");
        assert_eq!(text, "|abc");
        assert!(!actions.iter().any(|a| matches!(a, Action::Edit(_))));
        // `dj` on the last line fails too.
        assert_edit("a\n|b", "dj", "a\n|b");
    }

    #[test]
    fn readonly_view_gets_no_edit() {
        let (text, actions) =
            run_edit_with(Settings::default(), |v| v.readonly = true, "|a\nb", "dd");
        assert_eq!(text, "|a\nb");
        assert!(!actions.iter().any(|a| matches!(a, Action::Edit(_))));
    }

    #[test]
    fn escape_and_invalid_keys_cancel() {
        assert_edit("|abc def", "d<Esc>w", "abc |def");
        assert_edit("|abc def", "dxw", "abc |def");
        // `/` comes later; for now it cancels. An unknown text object
        // cancels too.
        assert_edit("|abc def", "d/", "|abc def");
        assert_edit("|abc def", "diqw", "abc |def");
    }

    #[test]
    fn pending_shows_operator_and_counts() {
        let view = crate::test_util::TestView::new("|abc");
        let mut vim = crate::Vim::new(view.cursor);
        crate::test_util::feed_keys(&mut vim, &view, "\"a2d3");
        assert_eq!(vim.pending(), "\"a2d3");
    }

    #[test]
    fn dw_on_empty_line_is_linewise_and_cw_is_not() {
        assert_edit("a\n|\nb", "dw", "a\n|b");
        let (text, actions) = run_edit("a\n|\nb", "cw");
        assert_eq!(text, "a\n|\nb");
        assert!(!actions.iter().any(|a| matches!(a, Action::Edit(_))));
    }

    #[test]
    fn dw_on_last_word_deletes_to_the_end() {
        assert_edit("abc |def", "dw", "abc| ");
        assert_edit("a|bc", "dw", "|a");
    }

    #[test]
    fn cw_with_change_word_includes_whitespace() {
        let s = Settings {
            change_word_includes_whitespace: true,
            ..Settings::default()
        };
        assert_edit_with(s, |_| {}, "|const a = 1;", "cw", "|a = 1;");
    }

    #[test]
    fn cc_keeps_indent_and_line_break() {
        assert_edit("a\n  b|c\nd", "cc", "a\n  |\nd");
    }

    #[test]
    fn yank_cursor_stays_forward_and_moves_backward() {
        assert_edit("a\nb|c\nd", "yy", "a\nb|c\nd");
        assert_edit("a\nb|c\nd", "yk", "|a\nbc\nd");
    }

    #[test]
    fn find_and_inclusive_motions() {
        assert_edit("|abc.def", "dt.", "|.def");
        assert_edit("|abc.def", "df.", "|def");
        assert_edit("abc.d|ef", "dF.", "abc|ef");
        assert_edit("|abc def", "d$", "|");
        assert_edit("|(a) b", "d%", "| b");
    }

    #[test]
    fn search_motion_retries_then_runs() {
        let mut view = crate::test_util::TestView::new("|ab x ab");
        let mut vim = crate::Vim::new(view.cursor);
        let mut g = crate::VimGlobals::default();
        let s = Settings::default();
        let feed = |vim: &mut crate::Vim, view: &crate::test_util::TestView, g: &mut _, k| {
            vim.feed(crate::Key::Char(k), view, g, &s)
        };
        assert_eq!(
            feed(&mut vim, &view, &mut g, 'd'),
            crate::Outcome::Handled(vec![])
        );
        view.searching = true;
        assert!(matches!(
            feed(&mut vim, &view, &mut g, '*'),
            crate::Outcome::Retry(_)
        ));
        assert_eq!(vim.pending(), "d");
        view.searching = false;
        if let crate::Outcome::Handled(a) = feed(&mut vim, &view, &mut g, '*') {
            view.apply(&mut vim, &a);
        }
        assert_eq!(view.marked(vim.cursor()), "|ab");
    }

    #[test]
    fn readonly_view_still_yanks() {
        let mut g = crate::VimGlobals::default();
        let mut view = crate::test_util::TestView::new("|a\nb");
        view.readonly = true;
        let mut vim = crate::Vim::new(view.cursor);
        for k in crate::test_util::parse_keys("yy") {
            vim.feed(k, &view, &mut g, &Settings::default());
        }
        let reg = g.read_register(None, ClipboardMode::Never, std::path::Path::new("x"));
        assert_eq!(reg.map(|r| r.text), Some("a\n".to_string()));
    }

    #[test]
    fn change_over_an_empty_range_still_inserts() {
        for (start, keys) in [("a\n|\nb", "c$"), ("a\n|\nb", "cl"), ("|abc", "c0")] {
            let (text, actions) = run_edit(start, keys);
            assert_eq!(text, start, "{keys}");
            assert_eq!(
                actions,
                vec![
                    Action::EnterInsert(InsertAt::Before),
                    Action::ModeChanged(Mode::Insert)
                ],
                "{keys}"
            );
        }
    }

    #[test]
    fn linewise_delete_keeps_column_without_startofline() {
        let s = Settings {
            startofline: false,
            ..Settings::default()
        };
        assert_edit_with(s, |_| {}, "abc|d\n  efgh\nx", "dd", "  e|fgh\nx");
        assert_edit("abc|d\n  efgh\nx", "dd", "  |efgh\nx");
    }

    #[test]
    fn backtick_mark_is_exclusive() {
        assert_edit("ab|cdef", "ma0d`a", "|cdef");
        assert_edit("ab|cdef", "mal$d`a", "ab|f");
    }

    /// Feeds `keys` from `start` and returns the globals.
    fn registers_after(start: &str, keys: &str) -> crate::VimGlobals {
        let mut g = crate::VimGlobals::default();
        let mut view = crate::test_util::TestView::new(start);
        let mut vim = crate::Vim::new(view.cursor);
        for k in crate::test_util::parse_keys(keys) {
            if let crate::Outcome::Handled(a) = vim.feed(k, &view, &mut g, &Settings::default()) {
                view.apply(&mut vim, &a);
            }
        }
        g
    }

    fn reg(g: &crate::VimGlobals, name: char) -> Option<String> {
        g.read_register(Some(name), ClipboardMode::Never, std::path::Path::new("x"))
            .map(|r| r.text)
    }

    #[test]
    fn operators_write_registers() {
        // `dd` fills `1` and shifts the ring.
        let g = registers_after("|a\nb\nc", "dddd");
        assert_eq!(reg(&g, '1').as_deref(), Some("b\n"));
        assert_eq!(reg(&g, '2').as_deref(), Some("a\n"));
        assert_eq!(reg(&g, '-'), None);
        // A small delete goes to `-`, not the ring.
        let g = registers_after("|one two", "dw");
        assert_eq!(reg(&g, '-').as_deref(), Some("one "));
        assert_eq!(reg(&g, '1'), None);
        // A named register skips the ring and `-`.
        let g = registers_after("|a\nb", "\"add");
        assert_eq!(reg(&g, 'a').as_deref(), Some("a\n"));
        assert_eq!(reg(&g, '1'), None);
        assert_eq!(reg(&g, '-'), None);
        // The black hole writes nothing.
        let g = registers_after("|a\nb", "\"_dd");
        assert_eq!(reg(&g, '"'), None);
        assert_eq!(reg(&g, '1'), None);
        // `c` writes like a delete, not a yank.
        let g = registers_after("|one two", "cw");
        assert_eq!(reg(&g, '-').as_deref(), Some("one"));
        assert_eq!(reg(&g, '0'), None);
    }
}
