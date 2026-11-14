//! Git change bars in the gutter (spec §4.6): a bar at the gutter's left
//! edge over each added or modified hunk's display lines, a half-clipped
//! pill on the boundary where lines were deleted, and one modified bar on
//! the header of a closed fold that hides any change.

use gpui::{Bounds, Hsla, Pixels, point, px, size};
use lucerna_git::{Hunk, HunkKind};
use lucerna_theme::{Rgba, UiColors};

use super::FrameRow;
use crate::code_view::FoldState;

/// Bar width, as a share of the line height (floored).
const BAR_WIDTH: f32 = 0.275;
/// The deletion pill's visible width, as a share of the line height.
const PILL_WIDTH: f32 = 0.35;

/// One bar or pill, in window coordinates.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HunkBar {
    pub bounds: Bounds<Pixels>,
    pub color: Hsla,
    pub corner_radius: Pixels,
}

/// The bar colours, each made opaque over the editor background.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HunkColors {
    added: Hsla,
    modified: Hsla,
    deleted: Hsla,
}

impl HunkColors {
    pub fn new(ui: &UiColors) -> HunkColors {
        let bg = ui.editor_background;
        HunkColors {
            added: opaque(ui.vc_added, bg),
            modified: opaque(ui.vc_modified, bg),
            deleted: opaque(ui.vc_deleted, bg),
        }
    }
}

/// `c` composed over `bg` (itself taken as opaque).
fn opaque(c: Rgba, bg: Rgba) -> Hsla {
    let mix = |fg: f32, bg: f32| fg * c.a + bg * (1. - c.a);
    gpui::Rgba {
        r: mix(c.r, bg.r),
        g: mix(c.g, bg.g),
        b: mix(c.b, bg.b),
        a: 1.,
    }
    .into()
}

/// The width of the bars for `line_height`.
pub(crate) fn bar_width(line_height: Pixels) -> Pixels {
    (line_height * BAR_WIDTH).floor()
}

/// What a display line's bar belongs to; equal neighbours share one bar.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Hunk(usize),
    Fold(u32),
}

/// The index of the first hunk that may touch rows from `row` on.
fn first_touching(hunks: &[Hunk], row: u32) -> usize {
    hunks
        .partition_point(|h| h.rows.start < row)
        .saturating_sub(1)
}

/// Whether any hunk lies in `rows`: a changed row inside it, or a
/// deletion between two of its rows (or just before its first).
fn any_in(hunks: &[Hunk], rows: std::ops::Range<u32>) -> bool {
    hunks[first_touching(hunks, rows.start)..]
        .iter()
        .take_while(|h| h.rows.start < rows.end)
        .any(|h| match h.kind {
            HunkKind::Deleted => rows.contains(&h.rows.start),
            _ => h.rows.start < rows.end && h.rows.end > rows.start,
        })
}

/// The added or modified hunk on `row`.
fn hunk_on(hunks: &[Hunk], row: u32) -> Option<usize> {
    let from = first_touching(hunks, row);
    hunks[from..]
        .iter()
        .take_while(|h| h.rows.start <= row)
        .position(|h| h.kind != HunkKind::Deleted && h.rows.contains(&row))
        .map(|i| from + i)
}

/// The bars of `hunks` (sorted by row) on the display lines `rows`, with
/// the gutter's left edge at `left`.
pub(crate) fn layout(
    rows: &[FrameRow],
    hunks: &[Hunk],
    folds: &FoldState,
    left: Pixels,
    line_height: Pixels,
    colors: HunkColors,
) -> Vec<HunkBar> {
    let mut bars = Vec::new();
    if hunks.is_empty() || rows.is_empty() {
        return bars;
    }
    let width = bar_width(line_height);
    let mut current: Option<(Owner, Pixels, Pixels)> = None;
    let flush = |current: Option<(Owner, Pixels, Pixels)>, bars: &mut Vec<HunkBar>| {
        if let Some((owner, top, bottom)) = current {
            let color = match owner {
                Owner::Hunk(i) if hunks[i].kind == HunkKind::Added => colors.added,
                _ => colors.modified,
            };
            bars.push(HunkBar {
                bounds: Bounds::new(point(left, top), size(width, bottom - top)),
                color,
                corner_radius: px(0.),
            });
        }
    };
    for r in rows {
        let row = r.buffer_row;
        let hidden = row + 1..folds.next_visible(row + 1);
        let owner = if !hidden.is_empty() && any_in(hunks, hidden) {
            Some(Owner::Fold(row))
        } else {
            hunk_on(hunks, row).map(Owner::Hunk)
        };
        match (owner, &mut current) {
            (Some(o), Some((c, _, bottom))) if *c == o => *bottom = r.top + line_height,
            (owner, current) => {
                flush(current.take(), &mut bars);
                *current = owner.map(|o| (o, r.top, r.top + line_height));
            }
        }
    }
    flush(current, &mut bars);

    // Deletions: a pill centred on the boundary above the row after them,
    // half of it left of the gutter (clipped when painted).
    let (first, last) = (rows[0].buffer_row, rows[rows.len() - 1].buffer_row);
    let pill = line_height * PILL_WIDTH;
    for h in hunks[first_touching(hunks, first)..]
        .iter()
        .take_while(|h| h.rows.start <= last + 1)
        .filter(|h| h.kind == HunkKind::Deleted)
    {
        let at = h.rows.start;
        if folds.hiding_header(at).is_some() {
            // Inside a closed fold: its header's bar shows it.
            continue;
        }
        let y = match rows.iter().find(|r| r.buffer_row == at && r.piece == 0) {
            Some(r) => r.top,
            None if at > 0 => {
                let above = folds.prev_visible(at - 1);
                match rows.iter().rfind(|r| r.buffer_row == above) {
                    Some(r) => r.top + line_height,
                    None => continue,
                }
            }
            None => continue,
        };
        bars.push(HunkBar {
            bounds: Bounds::new(
                point(left - pill, y - line_height / 2.),
                size(pill * 2., line_height),
            ),
            color: colors.deleted,
            corner_radius: pill,
        });
    }
    bars
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(kind: HunkKind, rows: std::ops::Range<u32>) -> Hunk {
        Hunk {
            kind,
            rows,
            base_rows: 0..0,
        }
    }

    #[test]
    fn any_in_counts_changed_rows_and_inner_deletions() {
        let hunks = [
            hunk(HunkKind::Modified, 1..3),
            hunk(HunkKind::Deleted, 5..5),
            hunk(HunkKind::Added, 8..9),
        ];
        assert!(any_in(&hunks, 2..4));
        assert!(!any_in(&hunks, 3..5));
        assert!(any_in(&hunks, 4..6));
        assert!(any_in(&hunks, 5..6));
        assert!(!any_in(&hunks, 6..8));
        assert!(any_in(&hunks, 6..9));
    }

    #[test]
    fn hunk_on_skips_deletions() {
        let hunks = [
            hunk(HunkKind::Modified, 1..3),
            hunk(HunkKind::Deleted, 3..3),
            hunk(HunkKind::Added, 3..5),
        ];
        assert_eq!(hunk_on(&hunks, 0), None);
        assert_eq!(hunk_on(&hunks, 2), Some(0));
        assert_eq!(hunk_on(&hunks, 3), Some(2));
        assert_eq!(hunk_on(&hunks, 5), None);
    }

    #[test]
    fn opaque_blends_alpha_over_background() {
        let c = Rgba {
            r: 1.,
            g: 0.,
            b: 0.,
            a: 0.5,
        };
        let bg = Rgba {
            r: 0.,
            g: 0.,
            b: 1.,
            a: 1.,
        };
        let blended = gpui::Rgba::from(opaque(c, bg));
        assert!((blended.r - 0.5).abs() < 1e-3 && (blended.b - 0.5).abs() < 1e-3);
        assert_eq!(blended.a, 1.);
    }
}
