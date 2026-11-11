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
            let mut start;
            let mut end;
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
            let cluster_start = start;
            loop {
                let next_a = a.get(i).map(|e| (e.new.start, e.new.end, true));
                let next_b = b.get(j).map(|e| (e.old.start, e.old.end, false));
                let next = match (next_a, next_b) {
                    (Some(x), Some(y)) => Some(if x.0 <= y.0 { x } else { y }),
                    (x, y) => x.or(y),
                };
                let Some((s, e, from_a)) = next else { break };
                let joins = s < end || (s == end && (e == s || end == start));
                if !joins {
                    break;
                }
                end = end.max(e);
                start = start.min(s);
                if from_a {
                    da += delta(&a[i]);
                    i += 1;
                } else {
                    db += delta(&b[j]);
                    j += 1;
                }
            }
            debug_assert_eq!(start, cluster_start);
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
        let start_row = old.offset_to_point(first.old.start).row;
        let end = old.offset_to_point(last.old.end);
        // A range ending at column 0 of a row leaves that row untouched.
        let old_end_row = if end.column == 0 && last.old.end > last.old.start {
            end.row
        } else {
            end.row + 1
        }
        .max(start_row + 1)
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
        for case in 0..500 {
            let (a, mid_len) = random_patch(&mut rng, 200);
            let (b, _) = random_patch(&mut rng, mid_len);
            let c = a.compose(&b);
            for offset in 0..=200 {
                for bias in [Bias::Left, Bias::Right] {
                    let mid = a.map_offset(offset, bias);
                    let inside_later_edit = b
                        .edits()
                        .iter()
                        .any(|e| e.old.start <= mid && mid <= e.old.end);
                    // `Left` at the end of a replaced range maps to the end of
                    // the replacement, and an offset that `a` leaves alone may
                    // sit inside a range `b` replaces. Once such edits merge,
                    // the composed edit cannot tell those points apart from
                    // other interior offsets, so for them we only require
                    // landing inside the composed replacement.
                    let ambiguous = inside_later_edit
                        || (bias == Bias::Left
                            && a.edits()
                                .iter()
                                .any(|e| e.old.end == offset && e.old.start < e.old.end));
                    if ambiguous {
                        let got = c.map_offset(offset, bias);
                        assert!(
                            c.edits().iter().any(|e| {
                                e.old.start <= offset
                                    && offset <= e.old.end
                                    && e.new.start <= got
                                    && got <= e.new.end
                            }),
                            "case {case} offset {offset} {bias:?}"
                        );
                        continue;
                    }
                    let seq = b.map_offset(a.map_offset(offset, bias), bias);
                    assert_eq!(
                        c.map_offset(offset, bias),
                        seq,
                        "case {case} offset {offset} {bias:?}\na={a:?}\nb={b:?}\nc={c:?}"
                    );
                }
            }
        }
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
