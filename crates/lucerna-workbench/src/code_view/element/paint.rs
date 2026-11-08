//! Painting a laid-out [`Frame`]: background, active line, selection,
//! text, whitespace marks, bracket boxes and the gutter.

use gpui::{App, Bounds, ContentMask, ShapedLine, TextAlign, Window, fill, outline, point, size};

use super::super::selection::Selection;
use super::Frame;

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
        if let Some(selection) = frame.selection.filter(|s| !s.is_empty()) {
            paint_selection(frame, selection, window);
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
    for (top, _, chevron) in &frame.chevrons {
        let origin = point(text.left() - frame.char_width * 1.5, *top);
        chevron
            .paint(origin, lh, TextAlign::Left, None, window, cx)
            .ok();
    }
}

fn paint_selection(frame: &Frame, selection: Selection, window: &mut Window) {
    let range = selection.range();
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
            window.paint_quad(fill(
                Bounds::new(
                    point(frame.text_x + x1, row.top),
                    size(x2 - x1, frame.line_height),
                ),
                frame.colors.selection,
            ));
        }
    }
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
