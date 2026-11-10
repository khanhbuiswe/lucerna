//! `H`, `M`, `L`: rows relative to the visible part of the view.

use crate::{ViewInfo, motion};

/// Rows assumed visible before the first paint.
const DEFAULT_VISIBLE_ROWS: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    Top,
    Middle,
    Bottom,
}

/// The number of rows the view shows; 30 before the first paint.
pub(crate) fn capacity(view: &dyn ViewInfo) -> u32 {
    match view.visible_rows() {
        0 => DEFAULT_VISIBLE_ROWS,
        n => n,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScrollKind {
    /// `Ctrl+d` / `Ctrl+u`: a fixed number of rows, the cursor goes along.
    Rows(u32),
    /// `Ctrl+f` / `Ctrl+b`: a page; the cursor is only clamped into view.
    Page(u32),
}

/// The new top row after scrolling `n` visible rows down or up. Scrolling
/// down never goes past the point where the last row sits at the bottom
/// (unless the view is already past it); `page` lets the top reach the last
/// visible row like vim's `Ctrl+f`.
pub(crate) fn scrolled_top(view: &dyn ViewInfo, n: u32, down: bool, page: bool) -> u32 {
    let last = view.snapshot().line_count().saturating_sub(1);
    let top = view.top_row().min(last);
    let moved = motion::step_rows(view, top, n, down);
    if !down || page {
        return moved;
    }
    let max_top = motion::step_rows(view, last, capacity(view) - 1, false);
    moved.min(max_top.max(top))
}

/// The first and last visible row when the view starts at `top`.
pub(crate) fn view_span(view: &dyn ViewInfo, top: u32) -> (u32, u32) {
    let last = view.snapshot().line_count().saturating_sub(1);
    let top = top.min(last);
    (top, motion::step_rows(view, top, capacity(view) - 1, true))
}

/// The top row that puts `row` at the top (`zt`), the middle (`zz`) or the
/// bottom (`zb`). Folded rows take no space.
pub(crate) fn top_for_cursor(view: &dyn ViewInfo, row: u32, at: Screen) -> u32 {
    let above = match at {
        Screen::Top => 0,
        Screen::Middle => capacity(view) / 2,
        Screen::Bottom => capacity(view) - 1,
    };
    motion::step_rows(view, row, above, false)
}

/// The target row of `H`, `M` or `L`. Folded rows take no screen space.
/// `count` counts from the top (`H`) or the bottom (`L`) and is ignored by
/// `M`. `scrolloff` keeps the target off the screen edges, except at the
/// top or bottom of the file.
pub(crate) fn screen_row(view: &dyn ViewInfo, which: Screen, count: u32, scrolloff: u32) -> u32 {
    let last = view.snapshot().line_count().saturating_sub(1);
    let capacity = capacity(view) as usize;
    let rows: Vec<u32> = (view.top_row().min(last)..=last)
        .filter(|&r| !view.is_row_hidden(r))
        .take(capacity)
        .collect();
    let Some(&first) = rows.first() else {
        return view.top_row().min(last);
    };
    let end = rows.len() - 1;
    let margin = scrolloff as usize;
    let index = match which {
        Screen::Top => {
            let want = (count.max(1) - 1) as usize;
            let floor = if first == 0 { 0 } else { margin };
            want.max(floor).min(end)
        }
        Screen::Middle => end / 2,
        Screen::Bottom => {
            let want = end.saturating_sub((count.max(1) - 1) as usize);
            let ceil = if rows[end] == last {
                end
            } else {
                end.saturating_sub(margin)
            };
            want.min(ceil)
        }
    };
    rows[index]
}

#[cfg(test)]
mod tests {
    use lucerna_text::Point;

    use crate::test_util::{assert_keys, assert_keys_with, run_keys};
    use crate::{Action, FoldAction, Settings};

    fn rows(n: usize) -> String {
        (0..n)
            .map(|i| format!("r{i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `n` rows with the cursor `|` at the start of row `at`.
    fn doc(n: usize, at: usize) -> String {
        rows(n).replacen(&format!("r{at}"), &format!("|r{at}"), 1)
    }

    fn scrolls(actions: &[Action]) -> Vec<u32> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::ScrollTopRow(r) => Some(*r),
                _ => None,
            })
            .collect()
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts:
    // "[count]<C-d> sets and adheres to scroll option"
    #[test]
    fn count_ctrl_d_sets_scroll() {
        assert_keys(
            "ab|c\ndef\nghi\njkl\nmno\npqr\nstu",
            "2<C-d><C-d>",
            "abc\ndef\nghi\njkl\n|mno\npqr\nstu",
        );
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts:
    // "can handle <C-u> when first line is visible and starting column is in the middle"
    #[test]
    fn ctrl_u_at_file_top_still_moves_cursor() {
        assert_keys(
            "\t hello world\nhello\nhi hello\nvery long line |at the bottom",
            "<C-u>",
            "\t |hello world\nhello\nhi hello\nvery long line at the bottom",
        );
    }

    // Adapted from VSCodeVim test/motions.test.ts:
    // "Preserves desired cursor position when pressing zz"
    #[test]
    fn zz_preserves_goal_column() {
        assert_keys(
            "very long line of text....|.\nshort line",
            "jzzk",
            "very long line of text....|.\nshort line",
        );
    }

    #[test]
    fn ctrl_d_scrolls_half_a_page() {
        let (view, vim, actions) = run_keys(
            Settings::default(),
            |v| v.visible_rows = 20,
            &doc(100, 5),
            "<C-d>",
        );
        assert_eq!(scrolls(&actions), vec![10]);
        assert_eq!(vim.cursor(), Point { row: 15, column: 0 });
        assert_eq!(view.top_row, 10);
        assert!(actions.contains(&Action::CursorMoved { jump: false }));
    }

    #[test]
    fn ctrl_d_uses_settings_scroll_then_count_overrides() {
        let settings = Settings {
            scroll: 3,
            ..Settings::default()
        };
        let (_, vim, actions) = run_keys(settings.clone(), |_| {}, &doc(100, 0), "<C-d>");
        assert_eq!(vim.cursor().row, 3);
        assert_eq!(scrolls(&actions), vec![3]);
        let (_, vim, _) = run_keys(settings, |_| {}, &doc(100, 0), "5<C-d><C-d>");
        assert_eq!(vim.cursor().row, 10);
    }

    #[test]
    fn ctrl_d_clamps_at_end_without_over_scrolling() {
        let (view, vim, _) = run_keys(
            Settings::default(),
            |v| v.visible_rows = 10,
            &doc(12, 10),
            "<C-d>",
        );
        assert_eq!(vim.cursor().row, 11);
        assert_eq!(view.top_row, 2);
    }

    #[test]
    fn ctrl_d_without_startofline_keeps_goal() {
        let settings = Settings {
            startofline: false,
            ..Settings::default()
        };
        assert_keys_with(
            settings,
            |_| {},
            "abc|def\nx\nabcdefgh",
            "2<C-d>",
            "abcdef\nx\nabc|defgh",
        );
    }

    #[test]
    fn ctrl_f_scrolls_a_page_minus_two() {
        let (view, vim, actions) = run_keys(
            Settings::default(),
            |v| v.visible_rows = 20,
            &doc(100, 0),
            "<C-f>",
        );
        assert_eq!(scrolls(&actions), vec![18]);
        assert!(vim.cursor().row >= 18);
        assert_eq!(view.top_row, 18);
    }

    #[test]
    fn ctrl_b_clamps_cursor_into_view() {
        let (view, vim, actions) = run_keys(
            Settings::default(),
            |v| {
                v.visible_rows = 20;
                v.top_row = 40;
            },
            &doc(100, 59),
            "<C-b>",
        );
        assert_eq!(scrolls(&actions), vec![22]);
        assert_eq!(view.top_row, 22);
        assert_eq!(vim.cursor().row, 41);
    }

    #[test]
    fn zt_zz_zb_only_scroll() {
        let (_, vim, actions) = run_keys(
            Settings::default(),
            |v| v.visible_rows = 20,
            &doc(100, 40),
            "zt",
        );
        assert_eq!(scrolls(&actions), vec![40]);
        assert_eq!(actions.len(), 1);
        assert_eq!(vim.cursor(), Point { row: 40, column: 0 });
        let (_, _, actions) = run_keys(
            Settings::default(),
            |v| v.visible_rows = 20,
            &doc(100, 40),
            "zz",
        );
        assert_eq!(scrolls(&actions), vec![30]);
        let (_, _, actions) = run_keys(
            Settings::default(),
            |v| v.visible_rows = 20,
            &doc(100, 40),
            "zb",
        );
        assert_eq!(scrolls(&actions), vec![21]);
    }

    #[test]
    fn zb_skips_hidden_rows_when_counting() {
        let (_, _, actions) = run_keys(
            Settings::default(),
            |v| {
                v.visible_rows = 10;
                v.hidden.push(36..40);
            },
            &doc(100, 40),
            "zb",
        );
        // Rows 36..40 are folded: the 9 rows above row 40 are 35 down to 27.
        assert_eq!(scrolls(&actions), vec![27]);
    }

    #[test]
    fn fold_keys_emit_fold_only() {
        for (keys, action) in [
            ("za", FoldAction::Toggle),
            ("zc", FoldAction::Close),
            ("zo", FoldAction::Open),
        ] {
            let (_, vim, actions) = run_keys(Settings::default(), |_| {}, &doc(10, 4), keys);
            assert_eq!(actions, vec![Action::Fold { row: 4, action }]);
            assert_eq!(vim.cursor().row, 4);
            assert_eq!(vim.pending(), "");
        }
    }

    #[test]
    fn unknown_z_key_is_swallowed_and_clears_pending() {
        let (_, vim, actions) = run_keys(Settings::default(), |_| {}, &doc(10, 4), "zq");
        assert!(actions.is_empty());
        assert_eq!(vim.pending(), "");
    }

    #[test]
    fn enter_cancels_pending_prefix() {
        for keys in ["f<CR>", "t<CR>", "g<CR>", "z<CR>"] {
            let (_, vim, actions) = run_keys(Settings::default(), |_| {}, &doc(10, 4), keys);
            assert!(actions.is_empty(), "{keys}");
            assert_eq!(vim.cursor().row, 4, "{keys}");
            assert_eq!(vim.pending(), "", "{keys}");
        }
    }
}
