//! `H`, `M`, `L`: rows relative to the visible part of the view.

use crate::ViewInfo;

/// Rows assumed visible before the first paint.
const DEFAULT_VISIBLE_ROWS: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    Top,
    Middle,
    Bottom,
}

/// The target row of `H`, `M` or `L`. Folded rows take no screen space.
/// `count` counts from the top (`H`) or the bottom (`L`) and is ignored by
/// `M`. `scrolloff` keeps the target off the screen edges, except at the
/// top or bottom of the file.
pub(crate) fn screen_row(view: &dyn ViewInfo, which: Screen, count: u32, scrolloff: u32) -> u32 {
    let last = view.snapshot().line_count().saturating_sub(1);
    let capacity = match view.visible_rows() {
        0 => DEFAULT_VISIBLE_ROWS,
        n => n,
    } as usize;
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
