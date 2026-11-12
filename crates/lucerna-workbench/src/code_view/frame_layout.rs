//! Cursor motions as the view lays text out: Up and Down move by display
//! line (wrapped pieces count, closed folds are stepped over) and keep a
//! goal column in display columns, tabs and wide chars included.

use lucerna_editor::{Goal, Layout, PlainLayout};
use lucerna_text::BufferSnapshot;
use unicode_segmentation::UnicodeSegmentation;

use super::layout::{FoldState, char_columns, display_lines_with_tab_size, row_start, wrap_count};

/// The view's layout as of its last frame: wrap width, tab size, folds and
/// the page height.
pub(crate) struct FrameLayout<'a> {
    pub folds: &'a FoldState,
    /// Display columns per line when wrapping.
    pub wrap_chars: Option<u32>,
    pub tab_size: u32,
    pub page_rows: u32,
}

/// One display line of a buffer row: its first byte column and its text.
struct Piece {
    start: usize,
    text: String,
}

impl FrameLayout<'_> {
    fn plain(&self) -> PlainLayout {
        PlainLayout {
            tab_size: self.tab_size,
            page_rows: self.page_rows,
        }
    }

    /// The display lines of `row`, without the `…` of a cut line.
    fn pieces(&self, snapshot: &BufferSnapshot, row: u32) -> Vec<Piece> {
        let lines = display_lines_with_tab_size(
            snapshot,
            &FoldState::default(),
            self.wrap_chars,
            self.tab_size,
            row..row + 1,
        );
        let truncated = lines.first().is_some_and(|l| l.truncated);
        let mut start = 0;
        let mut pieces: Vec<Piece> = lines
            .into_iter()
            .map(|line| {
                let piece = Piece {
                    start,
                    text: line.text,
                };
                start += piece.text.len();
                piece
            })
            .collect();
        if truncated && let Some(last) = pieces.last_mut() {
            last.text.pop();
        }
        if pieces.is_empty() {
            pieces.push(Piece {
                start: 0,
                text: String::new(),
            });
        }
        pieces
    }

    fn piece_count(&self, snapshot: &BufferSnapshot, row: u32) -> usize {
        wrap_count(snapshot, row, self.wrap_chars, self.tab_size) as usize
    }

    /// The display column of byte `column` within `piece`.
    fn display_column(&self, piece: &Piece, column: usize) -> u32 {
        let end = column.saturating_sub(piece.start).min(piece.text.len());
        piece
            .text
            .char_indices()
            .take_while(|&(i, _)| i < end)
            .fold(0, |col, (_, c)| col + char_columns(c, col, self.tab_size))
    }
}

/// The index of the piece showing byte `column`: a column at a wrap point
/// shows at the start of the next piece.
fn piece_index(pieces: &[Piece], column: usize) -> usize {
    pieces.iter().rposition(|p| p.start <= column).unwrap_or(0)
}

impl Layout for FrameLayout<'_> {
    fn vertical(
        &self,
        snapshot: &BufferSnapshot,
        offset: usize,
        goal: Goal,
        rows: i32,
    ) -> (usize, Goal) {
        let p = snapshot.offset_to_point(offset);
        let pieces = self.pieces(snapshot, p.row);
        let mut index = piece_index(&pieces, p.column as usize);
        let goal_col = match goal {
            Goal::Column(c) => c,
            Goal::None => self.display_column(&pieces[index], p.column as usize) as f32,
        };
        let goal = Goal::Column(goal_col);
        let line_count = snapshot.line_count();
        let mut row = p.row;
        let mut count = pieces.len();
        if rows > 0 {
            for _ in 0..rows {
                if index + 1 < count {
                    index += 1;
                    continue;
                }
                let next = self.folds.next_visible(row + 1);
                if next >= line_count {
                    return (snapshot.len_bytes(), goal);
                }
                row = next;
                count = self.piece_count(snapshot, row);
                index = 0;
            }
        } else {
            for _ in 0..rows.unsigned_abs() {
                if index > 0 {
                    index -= 1;
                    continue;
                }
                if row == 0 {
                    return (0, goal);
                }
                row = self.folds.prev_visible(row - 1);
                count = self.piece_count(snapshot, row);
                index = count.saturating_sub(1);
            }
        }
        let pieces = self.pieces(snapshot, row);
        let index = index.min(pieces.len() - 1);
        let piece = &pieces[index];
        let last = index + 1 == pieces.len();
        let mut col = 0;
        let mut byte = 0;
        let mut prev = 0;
        for g in piece.text.graphemes(true) {
            if col as f32 >= goal_col {
                break;
            }
            for c in g.chars() {
                col += char_columns(c, col, self.tab_size);
            }
            prev = byte;
            byte += g.len();
        }
        // The end of a wrapped piece is the start of the next one: stay on
        // this display line.
        if !last && byte == piece.text.len() {
            byte = prev;
        }
        (row_start(snapshot, row) + piece.start + byte, goal)
    }

    fn page_rows(&self) -> u32 {
        self.page_rows
    }

    /// Smart home on the first display line of a row; on a wrapped piece,
    /// its start, then the row's smart home.
    fn home(&self, snapshot: &BufferSnapshot, offset: usize) -> usize {
        if self.wrap_chars.is_none() {
            return self.plain().home(snapshot, offset);
        }
        let p = snapshot.offset_to_point(offset);
        let pieces = self.pieces(snapshot, p.row);
        let index = piece_index(&pieces, p.column as usize);
        let start = row_start(snapshot, p.row) + pieces[index].start;
        if index == 0 || offset == start {
            self.plain().home(snapshot, offset)
        } else {
            start
        }
    }

    /// The end of the buffer row: the end of a wrapped piece is the start
    /// of the next, so there is no stop there.
    fn end(&self, snapshot: &BufferSnapshot, offset: usize) -> usize {
        self.plain().end(snapshot, offset)
    }
}

#[cfg(test)]
mod tests {
    use lucerna_text::{Buffer, Point};

    use super::*;

    fn snapshot(text: &str) -> BufferSnapshot {
        Buffer::from_bytes(text.as_bytes(), false)
            .unwrap()
            .snapshot()
    }

    fn layout(folds: &FoldState, wrap_chars: Option<u32>) -> FrameLayout<'_> {
        FrameLayout {
            folds,
            wrap_chars,
            tab_size: 4,
            page_rows: 2,
        }
    }

    fn at(s: &BufferSnapshot, row: u32, column: u32) -> usize {
        s.point_to_offset(Point { row, column })
    }

    #[test]
    fn down_steps_through_wrapped_pieces() {
        // Wrapped at 4 columns: "abcd" "efgh" "ij", then "xy".
        let s = snapshot("abcdefghij\nxy");
        let folds = FoldState::default();
        let l = layout(&folds, Some(4));
        let (o, goal) = l.vertical(&s, at(&s, 0, 1), Goal::None, 1);
        assert_eq!(o, at(&s, 0, 5));
        let (o, goal) = l.vertical(&s, o, goal, 1);
        assert_eq!(o, at(&s, 0, 9));
        let (o, goal) = l.vertical(&s, o, goal, 1);
        assert_eq!(o, at(&s, 1, 1));
        let (o, _) = l.vertical(&s, o, goal, -2);
        assert_eq!(o, at(&s, 0, 5));
    }

    #[test]
    fn a_long_goal_stays_on_the_wrapped_line() {
        let s = snapshot("abcdefgh\nxyz");
        let folds = FoldState::default();
        let l = layout(&folds, Some(4));
        // From the end of "xyz" (column 3) up to "efgh": column 3, not the
        // wrap point.
        let (o, _) = l.vertical(&s, at(&s, 1, 3), Goal::None, -1);
        assert_eq!(o, at(&s, 0, 7));
        let (o, _) = l.vertical(&s, at(&s, 1, 3), Goal::Column(10.), -2);
        assert_eq!(o, at(&s, 0, 3), "before the wrap point");
    }

    #[test]
    fn closed_folds_are_stepped_over() {
        let s = snapshot("a {\n  b\n  c\n}\nd");
        let mut folds = FoldState::default();
        let available = [0..3, 5..6];
        folds.toggle(0, &available);
        let l = layout(&folds, None);
        let (o, goal) = l.vertical(&s, at(&s, 0, 0), Goal::None, 1);
        assert_eq!(o, at(&s, 3, 0));
        let (o, _) = l.vertical(&s, o, goal, -1);
        assert_eq!(o, at(&s, 0, 0));
    }

    #[test]
    fn goal_counts_tab_stops() {
        let s = snapshot("\tx\nabcdefg");
        let folds = FoldState::default();
        let l = layout(&folds, None);
        // After the tab is display column 4.
        let (o, _) = l.vertical(&s, at(&s, 0, 1), Goal::None, 1);
        assert_eq!(o, at(&s, 1, 4));
    }

    #[test]
    fn home_on_a_wrapped_piece_goes_to_its_start_first() {
        let s = snapshot("  abcdefgh");
        let folds = FoldState::default();
        let l = layout(&folds, Some(4));
        assert_eq!(l.home(&s, at(&s, 0, 6)), at(&s, 0, 4));
        assert_eq!(l.home(&s, at(&s, 0, 4)), at(&s, 0, 2));
        assert_eq!(l.end(&s, at(&s, 0, 4)), at(&s, 0, 10));
    }
}
