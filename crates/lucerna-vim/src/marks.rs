//! Marks: `m{a-z}`, `'x`, `` `x ``, `''` and `` `` ``, kept per file.

use lucerna_text::{Bias, BufferSnapshot, Patch, Point, Reload};
use std::path::Path;

use crate::VimGlobals;
use crate::classify::{last_char_col, line};

/// The marks of one file.
#[derive(Debug, Default, Clone)]
pub struct FileMarks {
    pub(crate) letters: [Option<Point>; 26],
    /// Where the cursor was before the latest jump (`''`).
    pub(crate) last_jump: Option<Point>,
    /// The last visual selection (`<` and `>`), written by visual mode.
    pub(crate) visual: Option<(Point, Point)>,
}

fn key(p: Point) -> (u32, u32) {
    (p.row, p.column)
}

impl FileMarks {
    /// The position a mark name refers to, if set.
    pub(crate) fn get(&self, name: char) -> Option<Point> {
        match name {
            'a'..='z' => self.letters[name as usize - 'a' as usize],
            '\'' | '`' => self.last_jump,
            '<' => self
                .visual
                .map(|(a, b)| if key(a) <= key(b) { a } else { b }),
            '>' => self
                .visual
                .map(|(a, b)| if key(a) <= key(b) { b } else { a }),
            _ => None,
        }
    }
}

impl VimGlobals {
    /// Moves the marks of `path` after the file was reloaded as `snapshot`.
    pub fn remap_marks(&mut self, path: &Path, reload: &Reload, snapshot: &BufferSnapshot) {
        let Some(marks) = self.marks.get_mut(path) else {
            return;
        };
        let lines = snapshot.line_count();
        let map = |p: Point| {
            let row = reload.adjust_row(p.row, lines);
            let text = line(snapshot, row);
            let mut column = p.column.min(last_char_col(&text)) as usize;
            while !text.is_char_boundary(column) {
                column -= 1;
            }
            Point {
                row,
                column: column as u32,
            }
        };
        for mark in marks.letters.iter_mut().chain([&mut marks.last_jump]) {
            *mark = mark.map(map);
        }
        marks.visual = marks.visual.map(|(a, b)| (map(a), map(b)));
    }

    /// Moves the marks of `path` through an edit: `patch` turned `old`
    /// into `snapshot`. Each mark keeps its place in the text
    /// ([`Patch::map_offset`], left bias), so `dd` above a mark pulls it
    /// up a row; a mark inside deleted text goes to where it was.
    pub fn remap_marks_patch(
        &mut self,
        path: &Path,
        old: &BufferSnapshot,
        patch: &Patch,
        snapshot: &BufferSnapshot,
    ) {
        if patch.is_empty() {
            return;
        }
        let Some(marks) = self.marks.get_mut(path) else {
            return;
        };
        let map = |p: Point| {
            let p = map_point(old, patch, snapshot, p, Bias::Left);
            let text = line(snapshot, p.row);
            let mut column = p.column.min(last_char_col(&text)) as usize;
            while !text.is_char_boundary(column) {
                column -= 1;
            }
            Point {
                row: p.row,
                column: column as u32,
            }
        };
        for mark in marks.letters.iter_mut().chain([&mut marks.last_jump]) {
            *mark = mark.map(map);
        }
        marks.visual = marks.visual.map(|(a, b)| (map(a), map(b)));
    }
}

/// `p` in `old` moved through `patch` into `snapshot`: to the start of
/// the replacement when it lies in replaced text, else by
/// [`Patch::map_offset`] with `bias` (which side of text inserted right at
/// it it ends up on). The column is not clamped to a char: callers clamp
/// for their mode.
pub fn map_point(
    old: &BufferSnapshot,
    patch: &Patch,
    snapshot: &BufferSnapshot,
    p: Point,
    bias: Bias,
) -> Point {
    let row = p.row.min(old.line_count().saturating_sub(1));
    let column = p.column.min(old.line(row).len() as u32);
    let offset = old.point_to_offset(Point { row, column });
    let edits = patch.edits();
    let i = edits.partition_point(|e| e.old.end <= offset);
    let new = match edits.get(i) {
        Some(e) if e.old.start <= offset && offset < e.old.end => e.new.start,
        _ => patch.map_offset(offset, bias),
    };
    snapshot.offset_to_point(new)
}

#[cfg(test)]
mod tests {
    use lucerna_text::{Point, Reload};

    use crate::marks::FileMarks;
    use crate::test_util::{TestView, assert_keys, feed_keys_with, parse_keys};
    use crate::{Outcome, Settings, Vim, VimGlobals};

    fn p(row: u32, column: u32) -> Point {
        Point { row, column }
    }

    // Adapted from VSCodeVim test/marks.test.ts: "Can jump to lowercase mark"
    #[test]
    fn jump_to_lowercase_mark() {
        assert_keys("|hello world and mars", "wma2w`a", "hello |world and mars");
    }

    #[test]
    fn apostrophe_goes_to_first_non_blank() {
        assert_keys("a\n  b|cd\ne", "majgg'a", "a\n  |bcd\ne");
    }

    #[test]
    fn backtick_goes_to_exact_position_clamped() {
        assert_keys("a\nbc|d\ne", "majgg`a", "a\nbc|d\ne");
    }

    #[test]
    fn double_apostrophe_returns_after_jump() {
        assert_keys("|a\nb\nc", "G''", "|a\nb\nc");
        assert_keys("a\nb|b\nc", "G``", "a\nb|b\nc");
    }

    #[test]
    fn unset_mark_does_not_move() {
        let mut view = TestView::new("a|bc\ndef");
        let mut vim = Vim::new(view.cursor);
        let mut globals = VimGlobals::default();
        let out = feed_keys_with(&mut vim, &mut view, &mut globals, "`z'z`<lt>");
        assert!(out.iter().all(|o| *o == Outcome::Handled(vec![])));
        assert_eq!(vim.cursor(), p(0, 1));
        assert_eq!(vim.pending(), "");
    }

    #[test]
    fn m_with_non_letter_cancels() {
        let mut view = TestView::new("a|bc");
        let mut vim = Vim::new(view.cursor);
        let mut globals = VimGlobals::default();
        feed_keys_with(&mut vim, &mut view, &mut globals, "m1");
        assert_eq!(vim.pending(), "");
        assert!(
            globals
                .marks
                .values()
                .all(|m| m.letters.iter().all(|l| l.is_none()))
        );
    }

    #[test]
    fn mark_jump_is_a_jump() {
        let mut view = TestView::new("|a\nb\nc");
        let mut vim = Vim::new(view.cursor);
        let mut globals = VimGlobals::default();
        feed_keys_with(&mut vim, &mut view, &mut globals, "majj");
        let out = feed_keys_with(&mut vim, &mut view, &mut globals, "'a");
        assert_eq!(
            out[1],
            Outcome::Handled(vec![crate::Action::CursorMoved { jump: true }])
        );
        // and the cursor before it is the new `''`.
        feed_keys_with(&mut vim, &mut view, &mut globals, "''");
        assert_eq!(vim.cursor(), p(2, 0));
    }

    #[test]
    fn visual_marks_are_read() {
        let mut view = TestView::new("ab\ncd\n|ef");
        let mut vim = Vim::new(view.cursor);
        let mut globals = VimGlobals::default();
        globals.marks.insert(
            view.path.clone(),
            FileMarks {
                visual: Some((p(1, 1), p(0, 0))),
                ..Default::default()
            },
        );
        feed_keys_with(&mut vim, &mut view, &mut globals, "`<lt>");
        assert_eq!(vim.cursor(), p(0, 0));
        feed_keys_with(&mut vim, &mut view, &mut globals, "`>");
        assert_eq!(vim.cursor(), p(1, 1));
        feed_keys_with(&mut vim, &mut view, &mut globals, "gg'>");
        assert_eq!(vim.cursor(), p(1, 0));
    }

    #[test]
    fn marks_are_per_path() {
        let mut a = TestView::new("|x\ny\nz");
        let mut b = TestView::new("|x\ny\nz");
        b.path = std::sync::Arc::from(std::path::Path::new("other.rs"));
        let mut globals = VimGlobals::default();
        let mut va = Vim::new(a.cursor);
        let mut vb = Vim::new(b.cursor);
        feed_keys_with(&mut va, &mut a, &mut globals, "jma");
        feed_keys_with(&mut vb, &mut b, &mut globals, "G`a");
        assert_eq!(vb.cursor(), p(2, 0), "b has no mark a");
        feed_keys_with(&mut va, &mut a, &mut globals, "gg`a");
        assert_eq!(va.cursor(), p(1, 0));
    }

    fn reloaded(mark: Point, old: &str, new: &str, reload: Reload) -> Point {
        let mut view = TestView::new(&format!("|{old}"));
        let mut globals = VimGlobals::default();
        let path = view.path.clone();
        let mut m = FileMarks::default();
        m.letters[0] = Some(mark);
        globals.marks.insert(path.clone(), m);
        view.snapshot = lucerna_text::Buffer::from_bytes(new.as_bytes(), false)
            .unwrap()
            .snapshot();
        globals.remap_marks(&path, &reload, &view.snapshot);
        globals.marks[&path].letters[0].unwrap()
    }

    #[test]
    fn remap_shifts_mark_down_for_rows_inserted_above() {
        let reload = Reload {
            start_row: 0,
            old_end_row: 0,
            new_end_row: 3,
        };
        assert_eq!(
            reloaded(p(1, 1), "aa\nbb\ncc", "x\ny\nz\naa\nbb\ncc", reload),
            p(4, 1)
        );
    }

    #[test]
    fn remap_clamps_mark_when_its_row_is_deleted() {
        let reload = Reload {
            start_row: 1,
            old_end_row: 3,
            new_end_row: 1,
        };
        assert_eq!(
            reloaded(p(2, 1), "aa\nbb\ncc\ndd", "aa\ndd", reload),
            p(1, 1)
        );
        // Column clamps to the last char of the new row.
        let reload = Reload {
            start_row: 1,
            old_end_row: 2,
            new_end_row: 2,
        };
        assert_eq!(reloaded(p(1, 5), "aa\nbbbbbb", "aa\nq", reload), p(1, 0));
    }

    /// Mark `a` at `mark` in `old`, after the edit `edits` (pre-edit
    /// offsets).
    fn edited(mark: Point, old: &str, edits: Vec<(std::ops::Range<usize>, &str)>) -> Point {
        let mut buffer = lucerna_text::Buffer::from_bytes(old.as_bytes(), false).unwrap();
        let before = buffer.snapshot();
        let patch = buffer.edit(
            edits
                .into_iter()
                .map(|(r, t)| (r, std::sync::Arc::from(t)))
                .collect::<Vec<_>>(),
        );
        let path: std::sync::Arc<std::path::Path> = std::sync::Arc::from(std::path::Path::new("a"));
        let mut globals = VimGlobals::default();
        let mut m = FileMarks::default();
        m.letters[0] = Some(mark);
        globals.marks.insert(path.clone(), m);
        globals.remap_marks_patch(&path, &before, &patch, &buffer.snapshot());
        globals.marks[&path].letters[0].unwrap()
    }

    #[test]
    fn patch_remap_pulls_mark_up_for_deleted_line_above() {
        // `dd` on row 1 of five rows: the mark on row 3 moves to row 2.
        assert_eq!(
            edited(p(3, 1), "a\nb\ncc\ndd\ne\n", vec![(2..4, "")]),
            p(2, 1)
        );
    }

    #[test]
    fn patch_remap_keeps_column_within_line() {
        // An insertion before the mark on its row shifts its column.
        assert_eq!(edited(p(0, 2), "abcd\n", vec![(0..0, "xy")]), p(0, 4));
        // A mark inside deleted text goes to the deletion point, clamped
        // onto a char.
        assert_eq!(edited(p(0, 3), "abcd\nx\n", vec![(1..4, "")]), p(0, 0));
    }

    #[test]
    fn ctrl_unbound_during_prefix_is_unhandled() {
        let mut view = TestView::new("|abc");
        let mut vim = Vim::new(view.cursor);
        let mut globals = VimGlobals::default();
        let settings = Settings::default();
        for prefix in ["g", "m", "'", "f"] {
            for k in parse_keys(prefix) {
                vim.feed(k, &view, &mut globals, &settings);
            }
            assert_ne!(vim.pending(), "", "{prefix}");
            let out = vim.feed(crate::Key::Ctrl('g'), &view, &mut globals, &settings);
            assert_eq!(out, Outcome::Unhandled, "{prefix}");
            assert_eq!(vim.pending(), "");
        }
        view.top_row = 0;
    }
}
