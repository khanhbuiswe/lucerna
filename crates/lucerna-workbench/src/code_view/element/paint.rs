//! Painting a laid-out [`Frame`]: background, active line, selection,
//! text, whitespace marks, bracket boxes and the gutter.

use std::ops::Range;

use gpui::{
    App, Bounds, ContentMask, Pixels, ShapedLine, TextAlign, Window, fill, outline, point, px, size,
};
use lucerna_text::Point;

use super::Frame;
use crate::ui::SCROLLBAR_WIDTH;

pub(super) fn paint_frame(frame: &Frame, window: &mut Window, cx: &mut App) {
    let lh = frame.line_height;
    let colors = frame.colors;
    window.paint_quad(fill(frame.bounds, colors.background));
    let text = frame.text_bounds;

    window.with_content_mask(Some(ContentMask { bounds: text }), |window| {
        for row in &frame.rows {
            if Some(row.buffer_row) == frame.cursor_row {
                window.paint_quad(fill(
                    Bounds::new(point(text.left(), row.top), size(text.size.width, lh)),
                    colors.active_line,
                ));
            }
        }
        for (range, active) in &frame.matches {
            for bounds in range_bounds(frame, range.clone()) {
                window.paint_quad(fill(bounds, colors.search_match));
                if *active {
                    window.paint_quad(outline(
                        bounds,
                        colors.active_match,
                        gpui::BorderStyle::Solid,
                    ));
                }
            }
        }
        if let Some(selection) = frame.selection.filter(|s| !s.is_empty()) {
            for bounds in range_bounds(frame, selection.range()) {
                window.paint_quad(fill(bounds, colors.selection));
            }
        }
        // Rows do not overlap: one draw order for all their glyphs.
        window.paint_layer(text, |window| {
            for row in &frame.rows {
                let origin = point(frame.text_x, row.top);
                row.piece()
                    .line
                    .paint(origin, lh, TextAlign::Left, None, window, cx)
                    .ok();
            }
        });
        if let Some((dot, arrow)) = &frame.whitespace {
            paint_whitespace(frame, dot, arrow, window, cx);
        }
        for p in &frame.brackets {
            let Some(row) = frame
                .rows
                .iter()
                .find(|r| r.buffer_row == p.row && r.piece().map.contains(p.column))
            else {
                continue;
            };
            let x1 = row.x_for(p.column);
            let x2 = row.x_for(p.column + 1).max(x1 + frame.char_width * 0.5);
            window.paint_quad(outline(
                Bounds::new(point(frame.text_x + x1, row.top), size(x2 - x1, lh)),
                colors.bracket,
                gpui::BorderStyle::Solid,
            ));
        }
    });

    let gutter = Bounds::new(
        frame.bounds.origin,
        size(text.left() - frame.bounds.left(), frame.bounds.size.height),
    );
    window.paint_quad(fill(gutter, colors.gutter));
    let numbers_right = text.left() - frame.char_width * 2.;
    for (top, line) in &frame.numbers {
        let origin = point(numbers_right - line.width(), *top);
        line.paint(origin, lh, TextAlign::Left, None, window, cx)
            .ok();
    }
    for (top, _, chevron, _) in frame.chevrons.iter().filter(|c| c.3) {
        let origin = point(text.left() - frame.char_width * 1.5, *top);
        chevron
            .paint(origin, lh, TextAlign::Left, None, window, cx)
            .ok();
    }
    if let Some(thumb) = frame.thumb {
        window
            .paint_quad(fill(thumb, colors.scrollbar_thumb).corner_radii(px(SCROLLBAR_WIDTH / 2.)));
    }
}

/// The boxes covering `range` on the display lines on screen; a covered
/// line ending adds half a char.
fn range_bounds(frame: &Frame, range: Range<Point>) -> Vec<Bounds<Pixels>> {
    let mut boxes = Vec::new();
    for row in &frame.rows {
        let r = row.buffer_row;
        if r < range.start.row || r > range.end.row {
            continue;
        }
        let map = &row.piece().map;
        let piece_end = map.buf_start + map.buf_len;
        let start = if r == range.start.row {
            range.start.column
        } else {
            0
        };
        let end = if r == range.end.row {
            range.end.column
        } else {
            u32::MAX
        };
        if end < map.buf_start
            || start > piece_end
            || (start > map.buf_start && start == piece_end && !row.is_last_piece())
        {
            continue;
        }
        let x1 = row.x_for(start);
        let mut x2 = row.x_for(end);
        if r < range.end.row && row.is_last_piece() {
            // The selected line ending.
            x2 += frame.char_width * 0.5;
        }
        if x2 > x1 {
            boxes.push(Bounds::new(
                point(frame.text_x + x1, row.top),
                size(x2 - x1, frame.line_height),
            ));
        }
    }
    boxes
}

fn paint_whitespace(
    frame: &Frame,
    dot: &ShapedLine,
    arrow: &ShapedLine,
    window: &mut Window,
    cx: &mut App,
) {
    let lh = frame.line_height;
    for row in &frame.rows {
        let piece = row.piece();
        let line = &piece.line;
        for (i, c) in line.text.char_indices() {
            if c == ' ' && !piece.map.is_tab_fill(i) {
                let origin = point(frame.text_x + line.x_for_index(i), row.top);
                dot.paint(origin, lh, TextAlign::Left, None, window, cx)
                    .ok();
            }
        }
        for &(_, disp, _) in &piece.map.tabs {
            let origin = point(frame.text_x + line.x_for_index(disp as usize), row.top);
            arrow
                .paint(origin, lh, TextAlign::Left, None, window, cx)
                .ok();
        }
    }
}
