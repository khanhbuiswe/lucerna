//! Cursor motions: arrows, words, line and buffer boundaries, pages.

use lucerna_text::{BufferSnapshot, Point};
use unicode_segmentation::GraphemeCursor;

use crate::selection::{Goal, Selection, SelectionSet};
use crate::words;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordStartLeft,
    WordEndRight,
    Home,
    End,
    LineStart,
    LineEnd,
    Top,
    Bottom,
    PageUp,
    PageDown,
}

/// What depends on how lines are displayed (wrapping, tab width).
pub trait Layout {
    /// Moves `rows` display rows from `offset`, trying to keep `goal`.
    fn vertical(
        &self,
        snapshot: &BufferSnapshot,
        offset: usize,
        goal: Goal,
        rows: i32,
    ) -> (usize, Goal);
    fn page_rows(&self) -> u32;
    /// Smart home: the first non-blank char of the display line, or its
    /// start when already there.
    fn home(&self, snapshot: &BufferSnapshot, offset: usize) -> usize;
    fn end(&self, snapshot: &BufferSnapshot, offset: usize) -> usize;
}

/// A layout without wrapping.
pub struct PlainLayout {
    pub tab_size: u32,
    pub page_rows: u32,
}

fn row_start(snapshot: &BufferSnapshot, row: u32) -> usize {
    snapshot.point_to_offset(Point { row, column: 0 })
}

fn row_len(snapshot: &BufferSnapshot, row: u32) -> usize {
    snapshot.line(row).len()
}

fn last_row(snapshot: &BufferSnapshot) -> u32 {
    snapshot.offset_to_point(snapshot.len_bytes()).row
}

/// Display width of `s` in chars, with tabs advancing to the next stop.
fn advance(col: u32, c: char, tab_size: u32) -> u32 {
    if c == '\t' {
        let tab = tab_size.max(1);
        col + tab - col % tab
    } else {
        col + 1
    }
}

impl Layout for PlainLayout {
    fn vertical(
        &self,
        snapshot: &BufferSnapshot,
        offset: usize,
        goal: Goal,
        rows: i32,
    ) -> (usize, Goal) {
        let p = snapshot.offset_to_point(offset);
        let line = snapshot.line(p.row);
        let column = (p.column as usize).min(line.len());
        let goal_col = match goal {
            Goal::Column(c) => c,
            Goal::None => line[..column]
                .chars()
                .fold(0, |col, c| advance(col, c, self.tab_size)) as f32,
        };
        let goal = Goal::Column(goal_col);
        let target = i64::from(p.row) + i64::from(rows);
        if target < 0 {
            return (0, goal);
        }
        if target > i64::from(last_row(snapshot)) {
            return (snapshot.len_bytes(), goal);
        }
        let target = target as u32;
        let text = snapshot.line(target);
        let mut col = 0;
        let mut byte = 0;
        for g in unicode_segmentation::UnicodeSegmentation::graphemes(&*text, true) {
            if col as f32 >= goal_col {
                break;
            }
            for c in g.chars() {
                col = advance(col, c, self.tab_size);
            }
            byte += g.len();
        }
        (row_start(snapshot, target) + byte, goal)
    }

    fn page_rows(&self) -> u32 {
        self.page_rows
    }

    fn home(&self, snapshot: &BufferSnapshot, offset: usize) -> usize {
        let row = snapshot.offset_to_point(offset).row;
        let start = row_start(snapshot, row);
        let line = snapshot.line(row);
        let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
        if offset == start + indent {
            start
        } else {
            start + indent
        }
    }

    fn end(&self, snapshot: &BufferSnapshot, offset: usize) -> usize {
        let row = snapshot.offset_to_point(offset).row;
        row_start(snapshot, row) + row_len(snapshot, row)
    }
}

/// The offset one grapheme cluster before or after `offset`; line breaks
/// count as one step.
fn step(snapshot: &BufferSnapshot, offset: usize, forward: bool) -> usize {
    let p = snapshot.offset_to_point(offset);
    let line = snapshot.line(p.row);
    let col = p.column as usize;
    let mut cursor = GraphemeCursor::new(col, line.len(), true);
    if forward {
        if col >= line.len() {
            return (offset + 1).min(snapshot.len_bytes());
        }
        match cursor.next_boundary(&line, 0) {
            Ok(Some(b)) => offset - col + b,
            _ => offset + 1,
        }
    } else {
        if col == 0 {
            return offset.saturating_sub(1);
        }
        match cursor.prev_boundary(&line, 0) {
            Ok(Some(b)) => offset - col + b,
            _ => offset - 1,
        }
    }
}

pub fn move_selections(
    snapshot: &BufferSnapshot,
    set: &SelectionSet,
    motion: Motion,
    extend: bool,
    layout: &dyn Layout,
    word_separators: &str,
) -> SelectionSet {
    let fast = set.len() == 1;
    let selections = set
        .all()
        .iter()
        .map(|s| {
            let range = s.range();
            let collapse = !extend && !s.is_empty();
            let (head, goal) = match motion {
                Motion::Left if collapse => (range.start, Goal::None),
                Motion::Right if collapse => (range.end, Goal::None),
                Motion::Left => (step(snapshot, s.head, false), Goal::None),
                Motion::Right => (step(snapshot, s.head, true), Goal::None),
                Motion::Up => layout.vertical(snapshot, s.head, s.goal, -1),
                Motion::Down => layout.vertical(snapshot, s.head, s.goal, 1),
                Motion::PageUp => {
                    layout.vertical(snapshot, s.head, s.goal, -(layout.page_rows() as i32))
                }
                Motion::PageDown => {
                    layout.vertical(snapshot, s.head, s.goal, layout.page_rows() as i32)
                }
                Motion::WordStartLeft => (
                    words::word_start_left(snapshot, s.head, word_separators, fast),
                    Goal::None,
                ),
                Motion::WordEndRight => (
                    words::word_end_right(snapshot, s.head, word_separators),
                    Goal::None,
                ),
                Motion::Home => (layout.home(snapshot, s.head), Goal::None),
                Motion::End => (layout.end(snapshot, s.head), Goal::None),
                Motion::LineStart => {
                    let row = snapshot.offset_to_point(s.head).row;
                    (row_start(snapshot, row), Goal::None)
                }
                Motion::LineEnd => {
                    let row = snapshot.offset_to_point(s.head).row;
                    (
                        row_start(snapshot, row) + row_len(snapshot, row),
                        Goal::None,
                    )
                }
                Motion::Top => (0, Goal::None),
                Motion::Bottom => (snapshot.len_bytes(), Goal::None),
            };
            Selection {
                id: s.id,
                anchor: if extend { s.anchor } else { head },
                head,
                goal,
            }
        })
        .collect();
    SelectionSet::from_selections(selections, set.primary().id, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{marked, render};

    const SEPS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

    fn layout() -> PlainLayout {
        PlainLayout {
            tab_size: 4,
            page_rows: 2,
        }
    }

    /// Applies `motions` in turn and returns the render after each.
    fn run(src: &str, motions: &[(Motion, bool)]) -> Vec<String> {
        let (snap, mut set) = marked(src);
        motions
            .iter()
            .map(|&(m, extend)| {
                set = move_selections(&snap, &set, m, extend, &layout(), SEPS);
                render(&snap, &set)
            })
            .collect()
    }

    fn one(src: &str, m: Motion) -> String {
        run(src, &[(m, false)]).remove(0)
    }

    #[test]
    fn left_right() {
        assert_eq!(one("a|bc", Motion::Left), "|abc");
        assert_eq!(one("a|bc", Motion::Right), "ab|c");
        assert_eq!(one("|abc", Motion::Left), "|abc");
        assert_eq!(one("abc|", Motion::Right), "abc|");
        assert_eq!(one("ab|\ncd", Motion::Right), "ab\n|cd");
        assert_eq!(one("ab\n|cd", Motion::Left), "ab|\ncd");
    }

    #[test]
    fn up_down() {
        assert_eq!(one("ab\nc|d\nef", Motion::Up), "a|b\ncd\nef");
        assert_eq!(one("ab\nc|d\nef", Motion::Down), "ab\ncd\ne|f");
        assert_eq!(one("a|b", Motion::Up), "|ab");
        assert_eq!(one("a|b", Motion::Down), "ab|");
    }

    #[test]
    fn vertical_keeps_goal() {
        assert_eq!(
            run(
                "abcd|e\nx\nabcdef",
                &[(Motion::Down, false), (Motion::Down, false)]
            ),
            ["abcde\nx|\nabcdef", "abcde\nx\nabcd|ef"]
        );
    }

    #[test]
    fn vertical_expands_tabs() {
        // Column 4 after the tab (tab_size 4) is column 4 of "abcdef".
        assert_eq!(one("\t|x\nabcdef", Motion::Down), "\tx\nabcd|ef");
    }

    #[test]
    fn left_collapses_selection() {
        assert_eq!(one("a«bc»d", Motion::Left), "a|bcd");
        assert_eq!(one("a«bc»d", Motion::Right), "abc|d");
    }

    #[test]
    fn extend_keeps_anchor() {
        assert_eq!(
            run("a|bcd", &[(Motion::Right, true), (Motion::Right, true)]),
            ["a«b»cd", "a«bc»d"]
        );
    }

    // Matches VS Code `cursorWordEndRight` (cursorWordOperations.ts, WordEnd).
    // The brief expected a stop at `foo.|bar`, but VS Code skips a single
    // separator that is followed by a regular char, so `foo.` + `bar` is one
    // stop.
    #[test]
    fn word_end_right_vscode() {
        let m = (Motion::WordEndRight, false);
        assert_eq!(
            run("|foo.bar baz", &[m, m, m]),
            ["foo|.bar baz", "foo.bar| baz", "foo.bar baz|"]
        );
        // A run of separators is a word.
        assert_eq!(run("|a..b", &[m, m]), ["a|..b", "a..|b"]);
    }

    // Matches VS Code `cursorWordLeft` (WordStartFast, cursorWordOperations.ts).
    // The brief expected `foo|.bar`, but the fast variant skips a lone
    // separator followed by a regular char, so from `foo.|bar` it goes to the
    // start of `foo`.
    #[test]
    fn word_start_left_vscode() {
        let m = (Motion::WordStartLeft, false);
        assert_eq!(
            run("foo.bar baz|", &[m, m, m]),
            ["foo.bar |baz", "foo.|bar baz", "|foo.bar baz"]
        );
        // With several cursors VS Code uses the plain WordStart rule.
        let (snap, set) = marked("foo.|bar x|");
        let set = move_selections(&snap, &set, Motion::WordStartLeft, false, &layout(), SEPS);
        assert_eq!(render(&snap, &set), "foo|.bar |x");
    }

    #[test]
    fn words_cross_lines() {
        assert_eq!(one("foo|\nbar", Motion::WordEndRight), "foo\nbar|");
        assert_eq!(one("foo\n|bar", Motion::WordStartLeft), "|foo\nbar");
    }

    #[test]
    fn smart_home() {
        assert_eq!(
            run("  ab|c", &[(Motion::Home, false), (Motion::Home, false)]),
            ["  |abc", "|  abc"]
        );
        assert_eq!(one("  a|bc", Motion::LineStart), "|  abc");
    }

    #[test]
    fn end_line_end() {
        assert_eq!(one("a|bc\nd", Motion::End), "abc|\nd");
        assert_eq!(one("a|bc\nd", Motion::LineEnd), "abc|\nd");
    }

    #[test]
    fn top_bottom_page() {
        assert_eq!(one("a\nb|\nc", Motion::Top), "|a\nb\nc");
        assert_eq!(one("a\nb|\nc", Motion::Bottom), "a\nb\nc|");
        assert_eq!(one("|a\nb\nc\nd", Motion::PageDown), "a\nb\n|c\nd");
        assert_eq!(one("a\nb\nc\n|d", Motion::PageUp), "a\n|b\nc\nd");
    }

    #[test]
    fn graphemes_not_split() {
        assert_eq!(one("a|e\u{301}b", Motion::Right), "ae\u{301}|b");
        assert_eq!(one("ae\u{301}|b", Motion::Left), "a|e\u{301}b");
        assert_eq!(one("a|😀b", Motion::Right), "a😀|b");
    }

    #[test]
    fn colliding_cursors_merge() {
        assert_eq!(one("|a|bc", Motion::Left), "|abc");
    }
}
