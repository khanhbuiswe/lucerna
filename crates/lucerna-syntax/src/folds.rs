use std::ops::Range;

use lucerna_text::BufferSnapshot;

use crate::row_start;

/// Indentation-based fold ranges whose header row lies in `rows`.
///
/// A header is a non-blank line followed (after blank lines) by a more
/// indented one. Its fold runs to the last non-blank line before the indent
/// drops back to the header's level; the range is `header..last + 1`.
/// Results are sorted by header row.
pub fn indent_folds(snapshot: &BufferSnapshot, tab_size: u32, rows: Range<u32>) -> Vec<Range<u32>> {
    let tab_size = tab_size.max(1);
    let line_count = snapshot.line_count();
    let mut folds = Vec::new();
    // Open folds: (header row, header indent), indents strictly increasing.
    let mut open: Vec<(u32, u32)> = Vec::new();
    // The last non-blank line seen: (row, indent).
    let mut previous: Option<(u32, u32)> = None;
    for row in rows.start..line_count {
        let Some(indent) = indent_of(snapshot, row, tab_size) else {
            continue;
        };
        let last_row = previous.map_or(row, |(r, _)| r);
        while let Some(&(header, header_indent)) = open.last() {
            if header_indent < indent {
                break;
            }
            open.pop();
            folds.push(header..last_row + 1);
        }
        if let Some((prev_row, prev_indent)) = previous
            && indent > prev_indent
            && rows.contains(&prev_row)
        {
            open.push((prev_row, prev_indent));
        }
        previous = Some((row, indent));
        if row >= rows.end && open.is_empty() {
            break;
        }
    }
    if let Some((last_row, _)) = previous {
        folds.extend(open.drain(..).map(|(header, _)| header..last_row + 1));
    }
    folds.sort_by_key(|f| f.start);
    folds
}

/// The indent width of `row` in columns, or `None` for a blank line.
fn indent_of(snapshot: &BufferSnapshot, row: u32, tab_size: u32) -> Option<u32> {
    let line = row_start(snapshot, row)..row_start(snapshot, row + 1);
    let mut column = 0;
    for chunk in snapshot.chunks_in_range(line) {
        for byte in chunk.bytes() {
            match byte {
                b' ' => column += 1,
                b'\t' => column += tab_size - column % tab_size,
                b'\n' | b'\r' => return None,
                _ => return Some(column),
            }
        }
    }
    None
}
