//! A `Patch` describes a batch of edits and maps offsets of the old text to
//! offsets of the new text. Selections, undo, syntax and LSP all go through it.

use std::ops::Range;

use crate::buffer::BufferSnapshot;
use crate::reload::Reload;

/// Which side of an edit an offset sticks to when text is inserted exactly at
/// it, or when it lies inside a replaced range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bias {
    Left,
    Right,
}

/// One replaced range: `old` is in pre-edit coordinates, `new` in post-edit
/// coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchEdit {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Patch {
    edits: Vec<PatchEdit>,
}

impl Patch {
    /// `edits` must be sorted by `old.start` and must not overlap.
    pub fn new(edits: Vec<PatchEdit>) -> Patch {
        debug_assert!(
            edits.windows(2).all(|w| w[0].old.end <= w[1].old.start),
            "patch edits must be sorted and non-overlapping"
        );
        Patch { edits }
    }

    pub fn edits(&self) -> &[PatchEdit] {
        &self.edits
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Maps an old offset to the new text. An offset inside a replaced range
    /// clamps to the start (`Left`) or end (`Right`) of the replacement; the
    /// end of a non-empty old range maps to the end of the replacement.
    pub fn map_offset(&self, offset: usize, bias: Bias) -> usize {
        // Left picks the first edit reaching the offset, Right the last edit
        // starting at or before it, so touching edits resolve to the side the
        // bias asks for.
        let idx = match bias {
            Bias::Left => self.edits.partition_point(|e| e.old.end < offset),
            Bias::Right => self.edits.partition_point(|e| e.old.start <= offset),
        };
        let edit = match bias {
            Bias::Left => self.edits.get(idx).filter(|e| e.old.start <= offset),
            Bias::Right => idx
                .checked_sub(1)
                .map(|i| &self.edits[i])
                .filter(|e| offset <= e.old.end),
        };
        if let Some(e) = edit {
            return match bias {
                Bias::Left if offset == e.old.end && e.old.start < e.old.end => e.new.end,
                Bias::Left => e.new.start,
                Bias::Right => e.new.end,
            };
        }
        // Not touching any edit: shift by every edit before it.
        match self.edits[..idx].last() {
            Some(e) => offset - e.old.end + e.new.end,
            None => offset,
        }
    }

    /// The patch equivalent to applying `self`, then `later` (whose old
    /// coordinates are `self`'s new coordinates).
    pub fn compose(&self, later: &Patch) -> Patch {
        let (a, b) = (&self.edits, &later.edits);
        let (mut i, mut j) = (0, 0);
        let (mut da, mut db) = (0isize, 0isize);
        let mut out = Vec::new();
        while i < a.len() || j < b.len() {
            // Ranges in the middle (post-`self`, pre-`later`) coordinate space.
            let take_a = j >= b.len() || (i < a.len() && a[i].new.start <= b[j].old.start);
            let (da_before, db_before) = (da, db);
            let start;
            let mut end;
            let mut end_from_a = take_a;
            if take_a {
                start = a[i].new.start;
                end = a[i].new.end;
                da += delta(&a[i]);
                i += 1;
            } else {
                start = b[j].old.start;
                end = b[j].old.end;
                db += delta(&b[j]);
                j += 1;
            }
            loop {
                let next_a = a.get(i).map(|e| (e.new.start, e.new.end, true));
                let next_b = b.get(j).map(|e| (e.old.start, e.old.end, false));
                let next = match (next_a, next_b) {
                    (Some(x), Some(y)) => Some(if x.0 <= y.0 { x } else { y }),
                    (x, y) => x.or(y),
                };
                let Some((s, e, from_a)) = next else { break };
                // Edits of one patch never merge with each other; edits of
                // different patches merge when they overlap or touch.
                let joins = s < end || (s == end && from_a != end_from_a);
                if !joins {
                    break;
                }
                if e >= end {
                    end_from_a = from_a;
                }
                end = end.max(e);
                if from_a {
                    da += delta(&a[i]);
                    i += 1;
                } else {
                    db += delta(&b[j]);
                    j += 1;
                }
            }
            out.push(PatchEdit {
                old: (start as isize - da_before) as usize..(end as isize - da) as usize,
                new: (start as isize + db_before) as usize..(end as isize + db) as usize,
            });
        }
        Patch { edits: out }
    }

    /// A coarse row summary of this patch for folds, marks and scroll.
    pub fn reload(&self, old: &BufferSnapshot, new: &BufferSnapshot) -> Reload {
        let (Some(first), Some(last)) = (self.edits.first(), self.edits.last()) else {
            return Reload {
                start_row: 0,
                old_end_row: 0,
                new_end_row: 0,
            };
        };
        let start = old.offset_to_point(first.old.start);
        let start_row = start.row;
        let end = old.offset_to_point(last.old.end);
        let line_start =
            |s: &BufferSnapshot, offset: usize| offset == 0 || s.byte_at(offset - 1) == b'\n';
        // Whole lines replaced by whole lines (a line diff, or Enter at the
        // start of a line) touch no row outside them: the row at the end
        // only moves.
        let whole_lines = start.column == 0 && end.column == 0 && line_start(new, last.new.end);
        let old_end_row = if whole_lines {
            end.row
        } else if end.column == 0 && last.old.end > last.old.start {
            // A range ending at column 0 of a row leaves that row untouched.
            end.row.max(start_row + 1)
        } else {
            end.row + 1
        }
        .min(old.line_count());
        // Rows after the change shift by exactly the line-count difference.
        let delta = new.line_count() as i64 - old.line_count() as i64;
        let new_end_row = (old_end_row as i64 + delta).max(start_row as i64) as u32;
        Reload {
            start_row,
            old_end_row,
            new_end_row,
        }
    }
}

fn delta(e: &PatchEdit) -> isize {
    e.new.len() as isize - e.old.len() as isize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Buffer;

    fn patch(old: Range<usize>, new: Range<usize>) -> Patch {
        Patch::new(vec![PatchEdit { old, new }])
    }

    #[test]
    fn map_before_inside_after() {
        let p = patch(2..4, 2..7);
        assert_eq!(p.map_offset(1, Bias::Left), 1);
        assert_eq!(p.map_offset(5, Bias::Left), 8);
        assert_eq!(p.map_offset(3, Bias::Left), 2);
        assert_eq!(p.map_offset(3, Bias::Right), 7);
        assert_eq!(p.map_offset(2, Bias::Right), 7);
        assert_eq!(p.map_offset(4, Bias::Left), 7);
    }

    #[test]
    fn insertion_bias() {
        let p = patch(2..2, 2..5);
        assert_eq!(p.map_offset(2, Bias::Left), 2);
        assert_eq!(p.map_offset(2, Bias::Right), 5);
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Random sorted non-overlapping edits over a text of `len` bytes.
    fn random_patch(rng: &mut Rng, len: usize) -> (Patch, usize) {
        let mut edits = Vec::new();
        let (mut old_pos, mut delta) = (0usize, 0isize);
        for _ in 0..rng.below(6) {
            if old_pos > len {
                break;
            }
            let start = old_pos + rng.below(len - old_pos + 1).min(30);
            let end = (start + rng.below(8)).min(len);
            let new_len = rng.below(8);
            let new_start = (start as isize + delta) as usize;
            edits.push(PatchEdit {
                old: start..end,
                new: new_start..new_start + new_len,
            });
            delta += new_len as isize - (end - start) as isize;
            old_pos = end;
        }
        (Patch::new(edits), (len as isize + delta) as usize)
    }

    #[test]
    fn compose_equals_sequential() {
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        let mut skipped = 0;
        for case in 0..500 {
            let (a, mid_len) = random_patch(&mut rng, 200);
            let (b, _) = random_patch(&mut rng, mid_len);
            let c = a.compose(&b);
            // A composed edit that swallowed edits from both `a` and `b` is
            // the only place where the old coordinates lose information.
            let merged: Vec<&PatchEdit> = c
                .edits()
                .iter()
                .filter(|e| {
                    let has_a = a
                        .edits()
                        .iter()
                        .any(|x| e.old.start <= x.old.start && x.old.end <= e.old.end);
                    let has_b = b
                        .edits()
                        .iter()
                        .any(|x| e.new.start <= x.new.start && x.new.end <= e.new.end);
                    has_a && has_b
                })
                .collect();
            for offset in 0..=200 {
                for bias in [Bias::Left, Bias::Right] {
                    let got = c.map_offset(offset, bias);
                    let mid = a.map_offset(offset, bias);
                    let seq = b.map_offset(mid, bias);
                    // Left at the end of a replaced range maps to the end of
                    // the replacement, and an offset `a` leaves alone may sit
                    // in a range `b` replaces. Inside a merged replacement the
                    // composed edit cannot tell those apart from other interior
                    // offsets, so there we only require landing in it.
                    if let Some(e) = merged
                        .iter()
                        .find(|e| e.old.start <= offset && offset <= e.old.end)
                    {
                        if got != seq {
                            skipped += 1;
                            assert!(
                                e.new.start <= got && got <= e.new.end,
                                "case {case} offset {offset} {bias:?}"
                            );
                        }
                        continue;
                    }
                    assert_eq!(
                        got, seq,
                        "case {case} offset {offset} {bias:?}\na={a:?}\nb={b:?}\nc={c:?}"
                    );
                }
            }
        }
        assert!(skipped < 1000, "too many lossy cases: {skipped}");
    }

    /// Applies `p` to `text`, taking replacement slices from `new_text`.
    fn apply(p: &Patch, text: &str, new_text: &str) -> String {
        let mut out = String::new();
        let mut pos = 0;
        for e in p.edits() {
            out.push_str(&text[pos..e.old.start]);
            out.push_str(&new_text[e.new.clone()]);
            pos = e.old.end;
        }
        out.push_str(&text[pos..]);
        out
    }

    /// Applies the edits of `p` (with random replacement text) to `text`.
    fn edit_text(rng: &mut Rng, p: &Patch, text: &str) -> String {
        let mut out = String::new();
        let mut pos = 0;
        for e in p.edits() {
            out.push_str(&text[pos..e.old.start]);
            for _ in 0..e.new.len() {
                out.push((b'A' + rng.below(26) as u8) as char);
            }
            pos = e.old.end;
        }
        out.push_str(&text[pos..]);
        out
    }

    #[test]
    fn compose_replays_to_the_same_text() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for case in 0..500 {
            let text: String = (0..200).map(|i| (b'a' + (i % 26) as u8) as char).collect();
            let (a, mid_len) = random_patch(&mut rng, 200);
            let mid = edit_text(&mut rng, &a, &text);
            assert_eq!(mid.len(), mid_len);
            let (b, _) = random_patch(&mut rng, mid_len);
            let fin = edit_text(&mut rng, &b, &mid);
            let c = a.compose(&b);
            assert_eq!(
                apply(&c, &text, &fin),
                fin,
                "case {case}\n{a:?}\n{b:?}\n{c:?}"
            );
        }
    }

    #[test]
    fn whole_line_insertions_and_deletions_touch_no_row() {
        let snap = |t: &str| Buffer::from_bytes(t.as_bytes(), false).unwrap().snapshot();
        let rows = |r: Reload| (r.start_row, r.old_end_row, r.new_end_row);
        // Two lines inserted above row 0.
        let (old, new) = (snap("a\nb\n"), snap("x\ny\na\nb\n"));
        assert_eq!(rows(patch(0..0, 0..4).reload(&old, &new)), (0, 0, 2));
        // Row 1 deleted.
        let (old, new) = (snap("a\nb\nc\n"), snap("a\nc\n"));
        assert_eq!(rows(patch(2..4, 2..2).reload(&old, &new)), (1, 2, 1));
        // Enter at the start of row 1.
        let (old, new) = (snap("a\nb\n"), snap("a\n\nb\n"));
        assert_eq!(rows(patch(2..2, 2..3).reload(&old, &new)), (1, 1, 2));
        // A char typed at the start of row 1 changes that row.
        let (old, new) = (snap("a\nb\n"), snap("a\nxb\n"));
        assert_eq!(rows(patch(2..2, 2..3).reload(&old, &new)), (1, 2, 2));
    }

    #[test]
    fn reload_summary_rows() {
        let old_text: String = (0..10).map(|i| format!("line {i}\n")).collect();
        let old = Buffer::from_bytes(old_text.as_bytes(), false)
            .unwrap()
            .snapshot();
        let start = old.point_to_offset(crate::Point { row: 3, column: 0 });
        let end = old.point_to_offset(crate::Point { row: 3, column: 6 });
        let new_text = old_text.replacen("line 3", "changed", 1);
        let new = Buffer::from_bytes(new_text.as_bytes(), false)
            .unwrap()
            .snapshot();
        let p = patch(start..end, start..start + 7);
        assert_eq!(
            p.reload(&old, &new),
            Reload {
                start_row: 3,
                old_end_row: 4,
                new_end_row: 4
            }
        );
    }
}
