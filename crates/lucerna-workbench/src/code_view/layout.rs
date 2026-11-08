//! Pure display layout: which buffer rows show, as which display lines.
//! Reads only the rows asked for, and at most the first
//! [`MAX_RENDERED_LINE_CHARS`] chars of each.

use std::collections::BTreeMap;
use std::ops::Range;

use lucerna_text::{BufferSnapshot, Point};

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
}

/// The display lines of the visible rows in `rows`, clamped to the buffer.
/// `wrap_width_chars` splits lines into pieces of that many chars.
pub fn display_lines(
    snapshot: &BufferSnapshot,
    folds: &FoldState,
    wrap_width_chars: Option<u32>,
    rows: Range<u32>,
) -> Vec<DisplayLine> {
    let end = rows.end.min(snapshot.line_count());
    let mut lines = Vec::new();
    let mut row = folds.next_visible(rows.start);
    while row < end {
        let (text, truncated) = line_prefix(snapshot, row, MAX_RENDERED_LINE_CHARS);
        push_pieces(&mut lines, row, text, truncated, wrap_width_chars);
        row = folds.next_visible(row + 1);
    }
    lines
}

/// How many display lines `row` takes with `wrap_width_chars` (at least 1).
pub fn wrap_count(snapshot: &BufferSnapshot, row: u32, wrap_width_chars: Option<u32>) -> u32 {
    let Some(width) = wrap_width_chars else {
        return 1;
    };
    let (text, truncated) = line_prefix(snapshot, row, MAX_RENDERED_LINE_CHARS);
    let chars = text.chars().count() + usize::from(truncated);
    chars.div_ceil(width.max(1) as usize).max(1) as u32
}

fn push_pieces(
    lines: &mut Vec<DisplayLine>,
    buffer_row: u32,
    mut text: String,
    truncated: bool,
    wrap_width_chars: Option<u32>,
) {
    if truncated {
        text.push('…');
    }
    let width = match wrap_width_chars {
        Some(w) => w.max(1) as usize,
        None => usize::MAX,
    };
    let mut rest = text.as_str();
    let mut wrap_index = 0u16;
    loop {
        let split = rest
            .char_indices()
            .nth(width)
            .map_or(rest.len(), |(i, _)| i);
        lines.push(DisplayLine {
            buffer_row,
            text: rest[..split].to_string(),
            truncated,
            wrap_index,
        });
        rest = &rest[split..];
        if rest.is_empty() {
            break;
        }
        wrap_index = wrap_index.saturating_add(1);
    }
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
