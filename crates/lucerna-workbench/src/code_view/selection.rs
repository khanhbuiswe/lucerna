//! The selection, kept in buffer [`Point`]s, and the char-boundary-safe
//! text operations on it.

use std::ops::Range;

use lucerna_text::{BufferSnapshot, Point};

use super::layout::{MAX_RENDERED_LINE_CHARS, line_prefix, row_range};

/// Where a selection started and where it ends; `head` is the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Selection {
    pub anchor: Point,
    pub head: Point,
}

impl Selection {
    pub fn cursor(at: Point) -> Selection {
        Selection {
            anchor: at,
            head: at,
        }
    }

    /// Start to end, whichever way it was made.
    pub fn range(&self) -> Range<Point> {
        if cmp(self.anchor, self.head).is_le() {
            self.anchor..self.head
        } else {
            self.head..self.anchor
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

pub(crate) fn cmp(a: Point, b: Point) -> std::cmp::Ordering {
    (a.row, a.column).cmp(&(b.row, b.column))
}

/// The text of `range`, with both ends moved back to char boundaries.
pub(crate) fn text_for(snapshot: &BufferSnapshot, range: Range<Point>) -> String {
    let start = boundary_offset(snapshot, range.start);
    let end = boundary_offset(snapshot, range.end).max(start);
    snapshot.text_for_range(start..end)
}

/// The byte offset of `p`, moved back to the nearest char boundary and
/// kept within its line (a column past the line end gives the line end).
pub(crate) fn boundary_offset(snapshot: &BufferSnapshot, p: Point) -> usize {
    if p.row >= snapshot.line_count() {
        return snapshot.len_bytes();
    }
    let line = row_range(snapshot, p.row);
    let target = line.start + p.column as usize;
    let mut offset = line.start;
    'chunks: for chunk in snapshot.chunks_in_range(line) {
        for (i, c) in chunk.char_indices() {
            let at = offset + i;
            if at + c.len_utf8() > target || c == '\n' || c == '\r' {
                offset = at;
                break 'chunks;
            }
        }
        offset += chunk.len();
    }
    offset
}

/// The word around `p` (letters, digits and `_`), or the single other
/// char under it. Looks only at the drawn part of the line.
pub(crate) fn word_at(snapshot: &BufferSnapshot, p: Point) -> Range<Point> {
    let (text, _) = line_prefix(snapshot, p.row, MAX_RENDERED_LINE_CHARS);
    let range = word_range(&text, p.column as usize);
    let at = |column: usize| Point {
        row: p.row,
        column: column as u32,
    };
    at(range.start)..at(range.end)
}

/// The whole of `row`, through its line ending.
pub(crate) fn line_at(snapshot: &BufferSnapshot, row: u32) -> Range<Point> {
    let start = Point { row, column: 0 };
    let end = if row + 1 < snapshot.line_count() {
        Point {
            row: row + 1,
            column: 0,
        }
    } else {
        snapshot.offset_to_point(snapshot.len_bytes())
    };
    start..end
}

/// The byte range of the word in `text` at byte `column`.
fn word_range(text: &str, column: usize) -> Range<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let column = column.min(text.len());
    let column = (0..=column)
        .rev()
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(0);
    let Some(c) = text[column..].chars().next() else {
        return column..column;
    };
    if !is_word(c) {
        return column..column + c.len_utf8();
    }
    let start = text[..column]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(column, |(i, _)| i);
    let end = text[column..]
        .char_indices()
        .find(|&(_, c)| !is_word(c))
        .map_or(text.len(), |(i, _)| column + i);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_text::Buffer;

    fn snapshot(text: &str) -> BufferSnapshot {
        Buffer::from_bytes(text.as_bytes(), false)
            .unwrap()
            .snapshot()
    }

    fn pt(row: u32, column: u32) -> Point {
        Point { row, column }
    }

    #[test]
    fn word_range_finds_words_and_punctuation() {
        assert_eq!(word_range("let foo_bar = 1;", 6), 4..11);
        assert_eq!(word_range("let foo_bar = 1;", 4), 4..11);
        assert_eq!(word_range("let foo_bar = 1;", 12), 12..13);
        assert_eq!(word_range("héllo wörld", 2), 0..6);
        assert_eq!(word_range("abc", 10), 3..3);
    }

    #[test]
    fn word_and_line_selection() {
        let s = snapshot("fn main() {\n    call(x);\n}");
        assert_eq!(word_at(&s, pt(1, 6)), pt(1, 4)..pt(1, 8));
        assert_eq!(line_at(&s, 0), pt(0, 0)..pt(1, 0));
        assert_eq!(line_at(&s, 2), pt(2, 0)..pt(2, 1));
    }

    #[test]
    fn offsets_snap_to_char_boundaries_within_the_line() {
        let s = snapshot("héllo\r\nnext\n");
        assert_eq!(boundary_offset(&s, pt(0, 2)), 1);
        assert_eq!(boundary_offset(&s, pt(0, 3)), 3);
        // Past the end of the line stops before its `\r\n`.
        assert_eq!(boundary_offset(&s, pt(0, 99)), 6);
        assert_eq!(boundary_offset(&s, pt(9, 0)), s.len_bytes());
        assert_eq!(text_for(&s, pt(0, 2)..pt(1, 2)), "éllo\r\nne");
    }

    #[test]
    fn selection_range_is_ordered() {
        let sel = Selection {
            anchor: pt(3, 1),
            head: pt(1, 4),
        };
        assert_eq!(sel.range(), pt(1, 4)..pt(3, 1));
        assert!(!sel.is_empty());
        assert!(Selection::cursor(pt(2, 2)).is_empty());
    }
}
