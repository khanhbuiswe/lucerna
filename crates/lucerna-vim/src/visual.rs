//! Visual selections: their shape, their byte ranges and the text `y`
//! copies.

use std::ops::Range;

use lucerna_text::{BufferSnapshot, Point};

use crate::classify::{advance, byte_col_for_display, last_char_col, line};

/// The selection of a visual mode, read back by the workbench after every
/// key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VimSelection {
    /// `v`: from `start` to `end_inclusive`, the char under the end
    /// included. An end at the line's length (after `$`) is the line break.
    Char { start: Point, end_inclusive: Point },
    /// `V`: whole rows `start_row..=end_row`.
    Line { start_row: u32, end_row: u32 },
    /// `Ctrl+v`: `rows` by the display columns `display_cols` (tabs
    /// expanded); with `to_eol` (after `$`) every row runs to its end.
    Block {
        rows: Range<u32>,
        display_cols: Range<u32>,
        to_eol: bool,
    },
}

/// The selection as one byte-column range, end exclusive. `None` for a
/// block, whose rows come from [`block_ranges`].
///
/// `Char` runs to the byte after its end char, or past the line break when
/// the end is on it. `Line` runs from column 0 of its first row to the start
/// of the row after its last, or to the end of the buffer.
pub fn exclusive_range(sel: &VimSelection, s: &BufferSnapshot) -> Option<Range<Point>> {
    match *sel {
        VimSelection::Char {
            start,
            end_inclusive: end,
        } => {
            let at = s.point_to_offset(end);
            let to = if at >= line_end(s, end.row) {
                next_row_start(s, end.row)
            } else {
                at + utf8_len(s.byte_at(at))
            };
            Some(start..s.offset_to_point(to))
        }
        VimSelection::Line { start_row, end_row } => {
            let to = s.offset_to_point(next_row_start(s, end_row));
            Some(
                Point {
                    row: start_row,
                    column: 0,
                }..to,
            )
        }
        VimSelection::Block { .. } => None,
    }
}

/// One range per row of a block, in row order. A row's range covers every
/// char whose display span meets the block's columns, so a tab cut by the
/// block's left edge is included whole (Vim pads it with spaces instead);
/// a row shorter than the block's left edge gives an empty range at its
/// end. With `to_eol` each range runs to the row's real end, past the
/// [`MAX_LINE_CHARS`](crate::classify::MAX_LINE_CHARS) cap of horizontal
/// reads.
pub fn block_ranges(sel: &VimSelection, s: &BufferSnapshot, tab: u32) -> Vec<Range<Point>> {
    let VimSelection::Block {
        rows,
        display_cols,
        to_eol,
    } = sel
    else {
        return Vec::new();
    };
    let tab = tab.max(1);
    let right = if *to_eol { u32::MAX } else { display_cols.end };
    rows.clone()
        .map(|row| {
            let text = line(s, row);
            let mut first = None;
            let mut last = if *to_eol {
                line_len(s, row) as usize
            } else {
                text.len()
            };
            let mut col = 0;
            for (i, c) in text.char_indices() {
                let next = advance(col, c, tab);
                if col >= right {
                    last = i;
                    break;
                }
                if next > display_cols.start && first.is_none() {
                    first = Some(i);
                }
                col = next;
            }
            let first = first.unwrap_or(text.len());
            let at = |column: usize| Point {
                row,
                column: column as u32,
            };
            at(first)..at(last.max(first))
        })
        .collect()
}

/// The text `y` copies: lines each end in `\n`; block rows are joined
/// with `\n`.
pub fn yank_text(sel: &VimSelection, s: &BufferSnapshot, tab: u32) -> String {
    let text =
        |r: Range<Point>| s.text_for_range(s.point_to_offset(r.start)..s.point_to_offset(r.end));
    match sel {
        VimSelection::Char { .. } => exclusive_range(sel, s).map(text).unwrap_or_default(),
        VimSelection::Line { .. } => {
            let mut out = exclusive_range(sel, s).map(text).unwrap_or_default();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out
        }
        VimSelection::Block { .. } => block_ranges(sel, s, tab)
            .into_iter()
            .map(text)
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// The first and last positions of `sel`, for the cursor after `y` and
/// the `<` and `>` marks: a whole-line selection spans column 0 of its
/// first row to the last char of its last; a block spans its top-left and
/// bottom-right corners.
pub(crate) fn bounds(sel: &VimSelection, s: &BufferSnapshot, tab: u32) -> (Point, Point) {
    match sel {
        VimSelection::Char {
            start,
            end_inclusive,
        } => (*start, *end_inclusive),
        VimSelection::Line { start_row, end_row } => (
            Point {
                row: *start_row,
                column: 0,
            },
            Point {
                row: *end_row,
                column: last_char_col(&line(s, *end_row)),
            },
        ),
        VimSelection::Block {
            rows,
            display_cols,
            to_eol,
        } => {
            let at = |row: u32, display: u32| Point {
                row,
                column: byte_col_for_display(&line(s, row), display, tab),
            };
            let bottom = rows.end.saturating_sub(1).max(rows.start);
            let right = if *to_eol {
                u32::MAX
            } else {
                display_cols.end.saturating_sub(1)
            };
            (at(rows.start, display_cols.start), at(bottom, right))
        }
    }
}

/// The byte length of row `row` without its line break, read without
/// copying the line.
pub(crate) fn line_len(s: &BufferSnapshot, row: u32) -> u32 {
    let start = s.point_to_offset(Point { row, column: 0 });
    (line_end(s, row) - start) as u32
}

/// The buffer offset where `row`'s text ends, before its line break.
pub(crate) fn line_end(s: &BufferSnapshot, row: u32) -> usize {
    let start = s.point_to_offset(Point { row, column: 0 });
    let mut end = next_row_start(s, row);
    for eol in *b"\n\r" {
        if end > start && s.byte_at(end - 1) == eol {
            end -= 1;
        }
    }
    end
}

/// The offset of the row after `row`, or the end of the buffer.
pub(crate) fn next_row_start(s: &BufferSnapshot, row: u32) -> usize {
    if row + 1 < s.line_count() {
        s.point_to_offset(Point {
            row: row + 1,
            column: 0,
        })
    } else {
        s.len_bytes()
    }
}

/// The length of the UTF-8 char whose first byte is `b`.
fn utf8_len(b: u8) -> usize {
    match b {
        0xF0.. => 4,
        0xE0.. => 3,
        0xC0.. => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use lucerna_text::{Buffer, Point};

    use super::*;
    use crate::test_util::{TestView, assert_keys, feed_keys_with, run_keys};
    use crate::{Action, Mode, Settings, Vim, VimGlobals};

    fn p(row: u32, column: u32) -> Point {
        Point { row, column }
    }

    fn snap(text: &str) -> BufferSnapshot {
        Buffer::from_bytes(text.as_bytes(), false)
            .unwrap()
            .snapshot()
    }

    /// The yanked text and the cursor after `keys`; asserts the actions
    /// `y` emits.
    fn yank(start: &str, keys: &str) -> (String, Point) {
        let (text, _, cursor) = yank_kind(start, keys);
        (text, cursor)
    }

    /// The yanked text, whether it is linewise, and the cursor after `keys`.
    fn yank_kind(start: &str, keys: &str) -> (String, bool, Point) {
        let (_, vim, actions) = run_keys(Settings::default(), |_| {}, start, keys);
        let at = actions
            .iter()
            .position(|a| matches!(a, Action::WriteClipboard { .. }))
            .expect("no yank");
        let Action::WriteClipboard { text, linewise } = &actions[at] else {
            unreachable!()
        };
        assert_eq!(
            actions[at + 1..],
            [
                Action::ModeChanged(Mode::Normal),
                Action::CursorMoved { jump: false }
            ]
        );
        assert_eq!(vim.mode(), Mode::Normal);
        (text.clone(), *linewise, vim.cursor())
    }

    #[test]
    fn visual_yank_writes_charwise_clipboard() {
        assert_eq!(
            yank_kind("a|bc\ndef", "vjy"),
            ("bc\nde".into(), false, p(0, 1))
        );
    }

    #[test]
    fn visual_line_yank_writes_linewise_clipboard() {
        assert_eq!(
            yank_kind("a|bc\ndef", "Vjy"),
            ("abc\ndef\n".into(), true, p(0, 0))
        );
    }

    #[test]
    fn visual_block_yank_is_not_linewise() {
        assert!(!yank_kind("a|bc\ndef", "<C-v>jy").1);
    }

    // Adapted from VSCodeVim test/mode/modeVisualLine.test.ts: "Exiting via <Esc> returns cursor to original column"
    #[test]
    fn visual_line_escape_keeps_column() {
        assert_keys(
            "rocinante\nnau|voo\nanubis\ncanterbury",
            "Vj<Esc>",
            "rocinante\nnauvoo\nanu|bis\ncanterbury",
        );
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/motions.test.ts: "Jump (line) to visual start '<"
    #[test]
    fn jump_to_visual_start() {
        assert_keys(
            "one\nXx|x\ntwo\n  three",
            "vjjv'<lt>",
            "one\n|Xxx\ntwo\n  three",
        );
    }

    // Adapted from VSCodeVim test/marks.test.ts: "'> set by Visual mode (backward selection)"
    #[test]
    fn visual_end_mark_after_backward_selection() {
        assert_keys("one\nt|wo\nthree", "vjlo<Esc>gg`>", "one\ntwo\nth|ree");
    }

    #[test]
    fn char_yank() {
        assert_eq!(yank("a|bc def", "vey"), ("bc".into(), p(0, 1)));
    }

    #[test]
    fn line_yank() {
        assert_eq!(
            yank("abc\nd|ef\nghi", "Vjy"),
            ("def\nghi\n".into(), p(1, 0))
        );
    }

    #[test]
    fn block_yank_skips_short_rows() {
        assert_eq!(
            yank("ab|cd\nef\nghij", "<C-v>jjly"),
            ("cd\n\nij".into(), p(0, 2))
        );
    }

    #[test]
    fn block_yank_with_tab() {
        // The cursor on `a`, display column 4; the block covers 4..6.
        assert_eq!(
            yank("\t|ab\n    cd", "<C-v>jly"),
            ("ab\ncd".into(), p(0, 1))
        );
        let sel = VimSelection::Block {
            rows: 0..2,
            display_cols: 4..6,
            to_eol: false,
        };
        assert_eq!(yank_text(&sel, &snap("\tab\n    cd"), 4), "ab\ncd");
    }

    #[test]
    fn block_selection_columns() {
        let (view, vim, _) = run_keys(Settings::default(), |_| {}, "ab|cd\nef\nghij", "<C-v>jjl");
        assert_eq!(
            vim.selection(&view.snapshot, 4),
            Some(VimSelection::Block {
                rows: 0..3,
                display_cols: 2..4,
                to_eol: false,
            })
        );
        assert_eq!(
            block_ranges(
                vim.selection(&view.snapshot, 4).as_ref().unwrap(),
                &view.snapshot,
                4
            ),
            vec![p(0, 2)..p(0, 4), p(1, 2)..p(1, 2), p(2, 2)..p(2, 4)]
        );
    }

    #[test]
    fn block_dollar_runs_to_each_line_end() {
        let (view, vim, _) = run_keys(Settings::default(), |_| {}, "a|b\nefgh\nxyz", "<C-v>j$j");
        let sel = vim.selection(&view.snapshot, 4).unwrap();
        assert!(matches!(sel, VimSelection::Block { to_eol: true, .. }));
        assert_eq!(yank_text(&sel, &view.snapshot, 4), "b\nfgh\nyz");
        // A horizontal motion ends it.
        let (view, vim, _) = run_keys(Settings::default(), |_| {}, "a|b\nefgh\nx", "<C-v>j$h");
        let sel = vim.selection(&view.snapshot, 4).unwrap();
        assert!(matches!(sel, VimSelection::Block { to_eol: false, .. }));
    }

    #[test]
    fn mode_switching() {
        // The brief's `vV<C-v>v` ending in Normal contradicts spec §3 (and
        // Vim): `v` in VisualBlock switches to Visual. A second `v` leaves.
        let (_, vim, actions) = run_keys(Settings::default(), |_| {}, "a|bc", "vV<C-v>vv");
        let modes: Vec<_> = actions
            .iter()
            .filter_map(|a| match a {
                Action::ModeChanged(m) => Some(*m),
                _ => None,
            })
            .collect();
        assert_eq!(
            modes,
            [
                Mode::Visual,
                Mode::VisualLine,
                Mode::VisualBlock,
                Mode::Visual,
                Mode::Normal
            ]
        );
        assert_eq!(vim.mode(), Mode::Normal);
        let (_, vim, _) = run_keys(Settings::default(), |_| {}, "a|bc", "VV");
        assert_eq!(vim.mode(), Mode::Normal);
        let (_, vim, _) = run_keys(Settings::default(), |_| {}, "a|bc", "<C-v><C-v>");
        assert_eq!(vim.mode(), Mode::Normal);
    }

    #[test]
    fn no_selection_in_normal() {
        let view = TestView::new("a|bc");
        assert_eq!(Vim::new(view.cursor).selection(&view.snapshot, 4), None);
    }

    #[test]
    fn char_selection_is_ordered() {
        let (view, vim, _) = run_keys(Settings::default(), |_| {}, "abc\nd|ef", "vkh");
        assert_eq!(
            vim.selection(&view.snapshot, 4),
            Some(VimSelection::Char {
                start: p(0, 0),
                end_inclusive: p(1, 1),
            })
        );
        let (view, vim, _) = run_keys(Settings::default(), |_| {}, "abc\nd|ef", "Vk");
        assert_eq!(
            vim.selection(&view.snapshot, 4),
            Some(VimSelection::Line {
                start_row: 0,
                end_row: 1,
            })
        );
    }

    #[test]
    fn exclusive_ranges() {
        let s = snap("abc\n");
        let sel = VimSelection::Char {
            start: p(0, 0),
            end_inclusive: p(0, 2),
        };
        assert_eq!(exclusive_range(&sel, &s), Some(p(0, 0)..p(0, 3)));
        let (view, vim, _) = run_keys(Settings::default(), |_| {}, "|abc\n", "v$");
        let sel = vim.selection(&view.snapshot, 4).unwrap();
        assert_eq!(
            exclusive_range(&sel, &view.snapshot),
            Some(p(0, 0)..p(1, 0))
        );
        assert_eq!(yank_text(&sel, &view.snapshot, 4), "abc\n");
        // A multi-byte end char is taken whole.
        let sel = VimSelection::Char {
            start: p(0, 0),
            end_inclusive: p(0, 1),
        };
        assert_eq!(exclusive_range(&sel, &snap("aé\n")), Some(p(0, 0)..p(0, 3)));
        // CRLF: past the whole break.
        let sel = VimSelection::Char {
            start: p(0, 0),
            end_inclusive: p(0, 2),
        };
        assert_eq!(
            exclusive_range(&sel, &snap("ab\r\ncd")),
            Some(p(0, 0)..p(1, 0))
        );
        // The line break of the last line is the end of the buffer.
        assert_eq!(exclusive_range(&sel, &snap("ab")), Some(p(0, 0)..p(0, 2)));
        // Lines.
        let lines = |a, b| VimSelection::Line {
            start_row: a,
            end_row: b,
        };
        assert_eq!(
            exclusive_range(&lines(0, 0), &snap("ab\ncd")),
            Some(p(0, 0)..p(1, 0))
        );
        assert_eq!(
            exclusive_range(&lines(0, 1), &snap("ab\ncd")),
            Some(p(0, 0)..p(1, 2))
        );
        let block = VimSelection::Block {
            rows: 0..1,
            display_cols: 0..1,
            to_eol: false,
        };
        assert_eq!(exclusive_range(&block, &snap("ab")), None);
    }

    #[test]
    fn line_yank_adds_final_newline() {
        assert_eq!(yank("ab\nc|d", "Vky"), ("ab\ncd\n".into(), p(0, 0)));
    }

    #[test]
    fn dollar_in_visual_then_vertical_keeps_line_break() {
        let (view, vim, _) = run_keys(Settings::default(), |_| {}, "|ab\ncde\nf", "v$j");
        assert_eq!(vim.cursor(), p(1, 3));
        let sel = vim.selection(&view.snapshot, 4).unwrap();
        assert_eq!(yank_text(&sel, &view.snapshot, 4), "ab\ncde\n");
        // Esc puts the cursor back on the last char, keeping the goal.
        let (_, vim, _) = run_keys(
            Settings::default(),
            |_| {},
            "|ab\ncde\nf\nghij",
            "v$j<Esc>jj",
        );
        assert_eq!(vim.cursor(), p(3, 3));
    }

    #[test]
    fn o_swaps_anchor_and_head() {
        let (view, vim, _) = run_keys(Settings::default(), |_| {}, "a|bcd", "vllo");
        assert_eq!(vim.cursor(), p(0, 1));
        assert_eq!(
            vim.selection(&view.snapshot, 4),
            Some(VimSelection::Char {
                start: p(0, 1),
                end_inclusive: p(0, 3),
            })
        );
        // `o` does nothing in Normal mode.
        assert_keys("a|bcd", "o", "a|bcd");
    }

    #[test]
    fn y_stores_visual_marks_and_moves_to_start() {
        let mut view = TestView::new("abc\nde|f\nghi");
        let mut vim = Vim::new(view.cursor);
        let mut globals = VimGlobals::default();
        feed_keys_with(&mut vim, &mut view, &mut globals, "vkhy");
        assert_eq!(vim.cursor(), p(0, 1));
        assert_eq!(globals.marks[&view.path].visual, Some((p(0, 1), p(1, 2))));
        // Normal-mode motions then start from the new cursor.
        feed_keys_with(&mut vim, &mut view, &mut globals, "j");
        assert_eq!(vim.cursor(), p(1, 1));
    }

    #[test]
    fn set_cursor_leaves_visual_mode() {
        let view = TestView::new("a|bc\ndef");
        let mut vim = Vim::new(view.cursor);
        crate::test_util::feed_keys(&mut vim, &view, "vl");
        vim.set_cursor(p(1, 1));
        assert_eq!(vim.mode(), Mode::Normal);
        assert_eq!(vim.selection(&view.snapshot, 4), None);
        crate::test_util::feed_keys(&mut vim, &view, "v");
        assert_eq!(
            vim.selection(&view.snapshot, 4),
            Some(VimSelection::Char {
                start: p(1, 1),
                end_inclusive: p(1, 1),
            })
        );
    }

    #[test]
    fn motions_from_the_line_break_do_not_panic() {
        for keys in [
            "w", "W", "b", "e", "ge", "%", "fa", "Fa", ";", "h", "l", "0", "^", "}", "{", "G", "gg",
        ] {
            for start in ["|ab é\ncd", "|ab(é)\n", "|é", "|\n\nx"] {
                run_keys(Settings::default(), |_| {}, start, &format!("v${keys}y"));
            }
        }
    }

    #[test]
    fn h_from_the_line_break_steps_onto_the_last_char() {
        let (_, vim, _) = run_keys(Settings::default(), |_| {}, "|abé\nx", "v$h");
        assert_eq!(vim.cursor(), p(0, 2));
    }

    #[test]
    fn block_to_eol_takes_lines_past_the_cap() {
        let long = "x".repeat(12_000);
        let start = format!("|ab\n{long}\ncd");
        let (view, vim, _) = run_keys(Settings::default(), |_| {}, &start, "<C-v>j$");
        let sel = vim.selection(&view.snapshot, 4).unwrap();
        assert_eq!(yank_text(&sel, &view.snapshot, 4), format!("ab\n{long}"));
    }
}
