//! Pure display layout: which buffer rows show, as which display lines.
//! Reads only the rows asked for, and at most the first
//! [`MAX_RENDERED_LINE_CHARS`] chars of each.

use std::collections::BTreeMap;
use std::ops::Range;

use lucerna_text::{BufferSnapshot, Point, Reload};
use unicode_width::UnicodeWidthChar;

/// Chars of a line that are drawn; the rest is replaced by `…`.
pub const MAX_RENDERED_LINE_CHARS: usize = 10_000;

/// One row on screen: a whole buffer line, or one wrapped piece of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayLine {
    pub buffer_row: u32,
    /// The piece's text, without line ending; ends in `…` when the line
    /// was cut at [`MAX_RENDERED_LINE_CHARS`].
    pub text: String,
    /// The buffer line is longer than what is drawn.
    pub truncated: bool,
    /// 0 for the first piece of a line, then 1, 2, ... when wrapped.
    pub wrap_index: u16,
}

/// The folds the user has closed. Each is a range from
/// [`lucerna_syntax::indent_folds`]: the header row stays visible, the rows
/// after it up to the range end are hidden.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoldState {
    /// Header row → end row (exclusive).
    folded: BTreeMap<u32, u32>,
}

impl FoldState {
    /// Folds or unfolds the range of `available` that starts at
    /// `header_row`; rows that start no range are ignored.
    pub fn toggle(&mut self, header_row: u32, available: &[Range<u32>]) {
        if self.folded.remove(&header_row).is_some() {
            return;
        }
        if let Some(range) = available.iter().find(|r| r.start == header_row) {
            self.folded.insert(header_row, range.end);
        }
    }

    pub fn is_hidden(&self, row: u32) -> bool {
        self.hiding_header(row).is_some()
    }

    /// Whether the fold starting at `header_row` is closed.
    pub fn is_folded(&self, header_row: u32) -> bool {
        self.folded.contains_key(&header_row)
    }

    /// Forgets every closed fold.
    pub fn clear(&mut self) {
        self.folded.clear();
    }

    /// Moves closed folds to their lines in the reloaded text. A fold
    /// whose header line is in the changed region is dropped; an end inside
    /// it stretches over the new lines. [`FoldState::retain_available`]
    /// settles the ends once the new fold ranges are known.
    pub fn remap(&mut self, reload: &Reload, new_line_count: u32) {
        let changed = reload.start_row..reload.old_end_row;
        let end = |row: u32| {
            if row <= reload.start_row {
                row
            } else if row >= reload.old_end_row {
                row - reload.old_end_row + reload.new_end_row
            } else {
                reload.new_end_row
            }
        };
        self.folded = std::mem::take(&mut self.folded)
            .into_iter()
            .filter(|(header, _)| !changed.contains(header))
            .map(|(header, e)| (reload.adjust_row(header, new_line_count), end(e)))
            .filter(|&(header, e)| header < new_line_count && e > header)
            .collect();
    }

    /// Keeps only the closed folds whose header still starts one of
    /// `available`, taking that range's end.
    pub fn retain_available(&mut self, available: &[Range<u32>]) {
        self.folded = std::mem::take(&mut self.folded)
            .into_keys()
            .filter_map(|header| {
                let range = available.iter().find(|r| r.start == header)?;
                Some((header, range.end))
            })
            .collect();
    }

    /// The header of the outermost closed fold that hides `row`.
    pub fn hiding_header(&self, row: u32) -> Option<u32> {
        // Folds nest, so the outermost one hiding `row` has the smallest
        // header; few folds are ever closed, so a scan is fine.
        self.folded
            .range(..row)
            .find(|&(_, &end)| end > row)
            .map(|(&header, _)| header)
    }

    /// The first row at or after `row` that is not hidden.
    pub fn next_visible(&self, row: u32) -> u32 {
        let mut row = row;
        while let Some(header) = self.hiding_header(row) {
            row = self.folded[&header];
        }
        row
    }

    /// The last row at or before `row` that is not hidden: the header of
    /// the fold hiding it, if any.
    pub fn prev_visible(&self, row: u32) -> u32 {
        self.hiding_header(row).unwrap_or(row)
    }

    /// The buffer row of the `n`th row not hidden by a closed fold (the
    /// inverse of `row - hidden_before(row)`); past the end it counts on as
    /// if the buffer were longer.
    pub fn nth_visible(&self, n: u32) -> u32 {
        let mut row = n;
        let mut covered = 0;
        for (&header, &end) in &self.folded {
            if header < covered {
                // Inside a fold already counted.
                continue;
            }
            if header >= row {
                break;
            }
            row += end - (header + 1);
            covered = end;
        }
        row
    }

    /// How many rows before `row` closed folds hide.
    pub fn hidden_before(&self, row: u32) -> u32 {
        let mut hidden = 0;
        let mut covered = 0;
        for (&header, &end) in &self.folded {
            let start = (header + 1).max(covered);
            hidden += end.min(row).saturating_sub(start);
            covered = covered.max(end);
        }
        hidden
    }
}

/// The display lines of the visible rows in `rows`, clamped to the buffer.
/// `wrap_width_chars` splits lines into pieces of at most that many
/// display columns, with a tab one column wide; see
/// [`display_lines_with_tab_size`].
pub fn display_lines(
    snapshot: &BufferSnapshot,
    folds: &FoldState,
    wrap_width_chars: Option<u32>,
    rows: Range<u32>,
) -> Vec<DisplayLine> {
    display_lines_with_tab_size(snapshot, folds, wrap_width_chars, 1, rows)
}

/// Like [`display_lines`], wrapping on display columns as the view draws
/// them: a tab advances to the next multiple of `tab_size` (counted from
/// the start of its piece) and wide chars such as CJK take two columns.
pub fn display_lines_with_tab_size(
    snapshot: &BufferSnapshot,
    folds: &FoldState,
    wrap_width_chars: Option<u32>,
    tab_size: u32,
    rows: Range<u32>,
) -> Vec<DisplayLine> {
    let end = rows.end.min(snapshot.line_count());
    let mut lines = Vec::new();
    let mut row = folds.next_visible(rows.start);
    while row < end {
        let (mut text, truncated) = line_prefix(snapshot, row, MAX_RENDERED_LINE_CHARS);
        if truncated {
            text.push('…');
        }
        let mut start = 0;
        let splits = wrap_splits(&text, wrap_width_chars, tab_size);
        for (wrap_index, &split) in splits.iter().chain([&text.len()]).enumerate() {
            lines.push(DisplayLine {
                buffer_row: row,
                text: text[start..split].to_string(),
                truncated,
                wrap_index: wrap_index.min(u16::MAX as usize) as u16,
            });
            start = split;
        }
        row = folds.next_visible(row + 1);
    }
    lines
}

/// How many display lines `row` takes (at least 1); agrees with
/// [`display_lines_with_tab_size`].
pub(crate) fn wrap_count(
    snapshot: &BufferSnapshot,
    row: u32,
    wrap_width_chars: Option<u32>,
    tab_size: u32,
) -> u32 {
    if wrap_width_chars.is_none() {
        return 1;
    }
    let (mut text, truncated) = line_prefix(snapshot, row, MAX_RENDERED_LINE_CHARS);
    if truncated {
        text.push('…');
    }
    wrap_splits(&text, wrap_width_chars, tab_size).len() as u32 + 1
}

/// The display columns `c` takes at `column` of its piece.
pub(crate) fn char_columns(c: char, column: u32, tab_size: u32) -> u32 {
    if c == '\t' {
        let tab_size = tab_size.max(1);
        tab_size - column % tab_size
    } else {
        // Control chars have no width but are still drawn as something.
        c.width().map_or(1, |w| w as u32)
    }
}

/// Byte offsets where `text` breaks into pieces of at most `width`
/// display columns. A char wider than the whole width gets a piece alone.
fn wrap_splits(text: &str, width: Option<u32>, tab_size: u32) -> Vec<usize> {
    let Some(width) = width else {
        return Vec::new();
    };
    let width = width.max(1);
    let mut splits = Vec::new();
    let mut column = 0;
    for (i, c) in text.char_indices() {
        let mut w = char_columns(c, column, tab_size);
        if column > 0 && column + w > width {
            splits.push(i);
            column = 0;
            w = char_columns(c, 0, tab_size);
        }
        column += w;
    }
    splits
}

/// The first `max_chars` chars of `row` without its line ending, and
/// whether the line is longer. Reads only that prefix of the buffer.
pub fn line_prefix(snapshot: &BufferSnapshot, row: u32, max_chars: usize) -> (String, bool) {
    let range = row_range(snapshot, row);
    let mut text = String::new();
    let mut count = 0;
    let mut over = false;
    'chunks: for chunk in snapshot.chunks_in_range(range) {
        for c in chunk.chars() {
            if c == '\n' {
                break 'chunks;
            }
            // Keep one char past the limit (two, for a `\r` before `\n`)
            // to know whether the line goes on.
            if count > max_chars {
                over = true;
                break 'chunks;
            }
            text.push(c);
            count += 1;
        }
    }
    if !over && text.ends_with('\r') {
        text.pop();
        count -= 1;
    }
    let truncated = count > max_chars;
    if truncated {
        let cut = text
            .char_indices()
            .nth(max_chars)
            .map_or(text.len(), |(i, _)| i);
        text.truncate(cut);
    }
    (text, truncated)
}

/// The byte range of `row` including its line ending; empty past the end.
pub(crate) fn row_range(snapshot: &BufferSnapshot, row: u32) -> Range<usize> {
    let start = row_start(snapshot, row);
    start..row_start(snapshot, row + 1)
}

/// The byte offset where `row` starts; rows past the end give the length.
pub(crate) fn row_start(snapshot: &BufferSnapshot, row: u32) -> usize {
    if row >= snapshot.line_count() {
        snapshot.len_bytes()
    } else {
        snapshot.point_to_offset(Point { row, column: 0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_text::Buffer;

    #[test]
    fn wrap_count_agrees_with_pieces() {
        let text = "\t\tabcd\n  \tx  \ty\n日本語\nplain\n\t\n";
        let s = Buffer::from_bytes(text.as_bytes(), false)
            .unwrap()
            .snapshot();
        for row in 0..s.line_count() {
            let pieces =
                display_lines_with_tab_size(&s, &FoldState::default(), Some(6), 4, row..row + 1);
            assert_eq!(
                wrap_count(&s, row, Some(6), 4),
                pieces.len() as u32,
                "row {row}"
            );
        }
    }

    #[test]
    fn nth_visible_skips_folds() {
        let available = [2..10, 20..25];
        let mut folds = FoldState::default();
        assert_eq!(folds.nth_visible(7), 7);
        folds.toggle(2, &available);
        // Rows 3..10 are hidden.
        assert_eq!(folds.nth_visible(0), 0);
        assert_eq!(folds.nth_visible(2), 2);
        assert_eq!(folds.nth_visible(3), 10);
        assert_eq!(folds.nth_visible(5), 12);
        folds.toggle(20, &available);
        // And 21..25: row 20 is index 13, the next shown is 25.
        assert_eq!(folds.nth_visible(13), 20);
        assert_eq!(folds.nth_visible(14), 25);
        for n in 0..40 {
            let row = folds.nth_visible(n);
            assert_eq!(row - folds.hidden_before(row), n, "n {n}");
            assert_eq!(folds.prev_visible(row), row, "n {n}");
        }
    }

    #[test]
    fn hidden_before_counts_nested_folds_once() {
        let available = [1..6, 2..4, 8..10];
        let mut folds = FoldState::default();
        assert_eq!(folds.hidden_before(20), 0);
        folds.toggle(2, &available);
        folds.toggle(1, &available);
        folds.toggle(8, &available);
        // Rows 2..6 (the outer fold covers the inner) and 9..10.
        assert_eq!(folds.hidden_before(2), 0);
        assert_eq!(folds.hidden_before(4), 2);
        assert_eq!(folds.hidden_before(6), 4);
        assert_eq!(folds.hidden_before(20), 5);
    }
}
