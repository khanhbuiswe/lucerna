//! Cursors and selections over LF-normalised byte offsets.

use std::ops::Range;

use lucerna_text::{Bias, BufferSnapshot, Patch};

/// The column a vertical motion tries to keep.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Goal {
    #[default]
    None,
    Column(f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    pub id: u64,
    pub anchor: usize,
    pub head: usize,
    pub goal: Goal,
}

impl Selection {
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// True when the head comes before the anchor.
    pub fn reversed(&self) -> bool {
        self.head < self.anchor
    }
}

/// One or more selections, sorted by start, with a primary.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionSet {
    selections: Vec<Selection>,
    primary: u64,
    next_id: u64,
    /// Whether `remap` and `rebuild` merge overlapping selections
    /// (`editor.multiCursorMergeOverlapping`).
    merge_overlapping: bool,
}

impl SelectionSet {
    pub fn cursor(offset: usize) -> Self {
        SelectionSet {
            selections: vec![Selection {
                id: 0,
                anchor: offset,
                head: offset,
                goal: Goal::None,
            }],
            primary: 0,
            next_id: 1,
            merge_overlapping: true,
        }
    }

    /// Sorts `selections` by start. With `merge_overlapping`, selections that
    /// overlap, share a start, or where an empty one touches the other's
    /// boundary are merged; two non-empty selections that only touch stay
    /// separate. The merged primary is the one containing the old primary.
    pub fn from_selections(
        mut selections: Vec<Selection>,
        primary: u64,
        merge_overlapping: bool,
    ) -> Self {
        assert!(!selections.is_empty(), "a selection set needs a selection");
        selections.sort_by_key(|s| (s.range().start, s.range().end));
        let next_id = selections.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        let mut primary_id = primary;
        if merge_overlapping {
            let mut merged: Vec<Selection> = Vec::with_capacity(selections.len());
            // Whether the last entry of `merged` contains the primary.
            let mut has_primary = false;
            for sel in selections {
                let sel_primary = sel.id == primary;
                if let Some(acc) = merged.last_mut() {
                    let (a, b) = (acc.range(), sel.range());
                    let touches_empty = (acc.is_empty() || sel.is_empty()) && b.start <= a.end;
                    if b.start < a.end || b.start == a.start || touches_empty {
                        let winner = if has_primary {
                            *acc
                        } else if sel_primary || (acc.is_empty() && !sel.is_empty()) {
                            sel
                        } else {
                            *acc
                        };
                        let (start, end) = (a.start.min(b.start), a.end.max(b.end));
                        let (anchor, head) = if winner.reversed() {
                            (end, start)
                        } else {
                            (start, end)
                        };
                        *acc = Selection {
                            anchor,
                            head,
                            ..winner
                        };
                        has_primary |= sel_primary;
                        if has_primary {
                            primary_id = acc.id;
                        }
                        continue;
                    }
                }
                has_primary = sel_primary;
                merged.push(sel);
            }
            selections = merged;
        }
        if !selections.iter().any(|s| s.id == primary_id) {
            primary_id = selections[0].id;
        }
        SelectionSet {
            selections,
            primary: primary_id,
            next_id,
            merge_overlapping: true,
        }
    }

    /// Whether overlapping selections are merged when the set is rebuilt.
    pub fn merge_overlapping(&self) -> bool {
        self.merge_overlapping
    }

    pub fn set_merge_overlapping(&mut self, merge: bool) {
        self.merge_overlapping = merge;
    }

    /// A set of `selections` that inherits this set's merge flag and id
    /// counter, merged as the flag says.
    pub fn rebuild(&self, selections: Vec<Selection>, primary: u64) -> Self {
        let mut set = Self::from_selections(selections, primary, self.merge_overlapping);
        set.merge_overlapping = self.merge_overlapping;
        set.next_id = set.next_id.max(self.next_id);
        set
    }

    /// The selection added last (the largest id).
    pub fn newest(&self) -> &Selection {
        self.selections
            .iter()
            .max_by_key(|s| s.id)
            .expect("a selection set needs a selection")
    }

    pub fn all(&self) -> &[Selection] {
        &self.selections
    }

    pub fn primary(&self) -> &Selection {
        self.selections
            .iter()
            .find(|s| s.id == self.primary)
            .unwrap_or(&self.selections[0])
    }

    pub fn len(&self) -> usize {
        self.selections.len()
    }

    pub fn is_empty(&self) -> bool {
        self.selections.is_empty()
    }

    /// Moves every selection through `patch`. An endpoint inside replaced
    /// text (from its start up to, not including, its end) goes to the start
    /// of the replacement. Otherwise the head sticks right when it is the
    /// later end and left otherwise, and empty selections stick right (after
    /// text inserted at them).
    pub fn remap(&mut self, patch: &Patch) {
        self.remap_with(patch, |s| {
            if s.is_empty() {
                (Bias::Right, Bias::Right)
            } else if s.reversed() {
                (Bias::Right, Bias::Left)
            } else {
                (Bias::Left, Bias::Right)
            }
        });
    }

    /// Like [`SelectionSet::remap`] for a change made outside the editor
    /// (an agent writing the file): text inserted at a selection's edge
    /// stays outside it, so the selection keeps covering the same text and
    /// the next keystroke cannot replace what was just written. Cursors
    /// still go after text inserted at them.
    pub fn remap_external(&mut self, patch: &Patch) {
        self.remap_with(patch, |s| {
            if s.is_empty() {
                (Bias::Right, Bias::Right)
            } else if s.reversed() {
                (Bias::Left, Bias::Right)
            } else {
                (Bias::Right, Bias::Left)
            }
        });
    }

    /// Moves every endpoint through `patch` with the `(anchor, head)` biases
    /// `biases` gives for its selection, then renormalises.
    fn remap_with(&mut self, patch: &Patch, biases: impl Fn(&Selection) -> (Bias, Bias)) {
        for s in &mut self.selections {
            let (anchor_bias, head_bias) = biases(s);
            s.anchor = map_endpoint(patch, s.anchor, anchor_bias);
            s.head = map_endpoint(patch, s.head, head_bias);
        }
        // Deletes can collapse or overlap selections: renormalise.
        let selections = std::mem::take(&mut self.selections);
        *self = self.rebuild(selections, self.primary);
    }

    pub fn clamp(&mut self, snapshot: &BufferSnapshot) {
        let len = snapshot.len_bytes();
        for s in &mut self.selections {
            s.anchor = snapshot.floor_char_boundary(s.anchor.min(len));
            s.head = snapshot.floor_char_boundary(s.head.min(len));
        }
    }

    pub fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// `offset` through `patch`: the start of the replacement when it lies in a
/// non-empty replaced range (end excluded), else `Patch::map_offset`.
fn map_endpoint(patch: &Patch, offset: usize, bias: Bias) -> usize {
    let edits = patch.edits();
    let i = edits.partition_point(|e| e.old.end <= offset);
    match edits.get(i) {
        Some(e) if e.old.start <= offset && offset < e.old.end => e.new.start,
        _ => patch.map_offset(offset, bias),
    }
}

#[cfg(test)]
mod tests {
    use lucerna_text::PatchEdit;

    use super::*;
    use crate::test_util::{marked, render};

    fn merged(src: &str) -> String {
        let (snap, set) = marked(src);
        let set = SelectionSet::from_selections(set.all().to_vec(), set.primary().id, true);
        render(&snap, &set)
    }

    fn insert_at(offset: usize, len: usize) -> Patch {
        Patch::new(vec![PatchEdit {
            old: offset..offset,
            new: offset..offset + len,
        }])
    }

    #[test]
    fn marked_round_trips() {
        for s in ["a|b«cd»e", "»ab«|c", "|a*|b", "|«ab»", "«ab»«cd»"] {
            let (snap, set) = marked(s);
            assert_eq!(render(&snap, &set), s);
        }
    }

    #[test]
    fn merge_rules() {
        assert_eq!(merged("«ab»|c"), "«ab»c");
        assert_eq!(merged("|«ab»c"), "«ab»c");
        assert_eq!(merged("«ab»«cd»"), "«ab»«cd»");
        assert_eq!(merged("«a«b»c»"), "«abc»");
        assert_eq!(merged("a|b|c"), "a|b|c");
    }

    #[test]
    fn merge_keeps_primary() {
        let (_, set) = marked("«ab»|*«a»");
        let m = SelectionSet::from_selections(set.all().to_vec(), set.primary().id, true);
        assert_eq!(m.len(), 2);
        assert_eq!(m.primary().range(), 2..3);
        let (_, set) = marked("*«a«b»c»");
        let m = SelectionSet::from_selections(set.all().to_vec(), set.primary().id, true);
        assert_eq!(m.len(), 1);
        assert_eq!(m.primary().range(), 0..3);
    }

    #[test]
    fn remap_after_insert() {
        let (_, mut set) = marked("ab|cd");
        set.remap(&insert_at(0, 1));
        let (snap, _) = marked("Xabcd|");
        assert_eq!(render(&snap, &set), "Xab|cd");
    }

    #[test]
    fn clamp_snaps_to_char_boundary() {
        let (snap, _) = marked("é|");
        let mut set = SelectionSet::cursor(1);
        set.clamp(&snap);
        assert_eq!(set.primary().head, 0);
    }

    #[test]
    fn remap_remerges_collapsed_cursors() {
        let (_, set) = marked("a|bc*|d");
        assert_eq!(set.primary().id, 1);
        let mut set = set;
        // Delete "bc" (1..3).
        set.remap(&Patch::new(vec![PatchEdit {
            old: 1..3,
            new: 1..1,
        }]));
        assert_eq!(set.len(), 1);
        assert_eq!(set.primary().head, 1);
        assert_eq!(set.primary().id, 1);
    }

    #[test]
    fn remap_at_insert_point_moves_right() {
        let mut set = SelectionSet::cursor(2);
        set.remap(&insert_at(2, 3));
        assert_eq!(set.primary().head, 5);
    }

    fn replace(old: std::ops::Range<usize>, new_len: usize) -> Patch {
        Patch::new(vec![PatchEdit {
            new: old.start..old.start + new_len,
            old,
        }])
    }

    #[test]
    fn remap_inside_replaced_text_goes_to_its_start() {
        // "ab\ncd\nef": line 1 ("cd\n", 3..6) becomes "XYZW\n" (3..8).
        for at in [3, 4, 5] {
            let mut set = SelectionSet::cursor(at);
            set.remap(&replace(3..6, 5));
            assert_eq!(set.primary().head, 3, "cursor at {at}");
        }
        // The end of the replaced range follows the replacement's end.
        let mut set = SelectionSet::cursor(6);
        set.remap(&replace(3..6, 5));
        assert_eq!(set.primary().head, 8);
        // A selection inside the hunk does not grow to all of it.
        let (_, mut set) = marked("ab\nc«d»\nef");
        set.remap(&replace(3..6, 5));
        assert_eq!(set.primary().range(), 3..3);
        // One reaching out of it keeps its outside end.
        let (_, mut set) = marked("a«b\ncd\ne»f");
        set.remap(&replace(3..6, 5));
        assert_eq!(set.primary().range(), 1..9);
    }

    #[test]
    fn remap_selection_edges_stick_outward() {
        let (_, mut set) = marked("«ab»");
        set.remap(&insert_at(0, 1));
        set.remap(&insert_at(3, 1));
        assert_eq!(set.primary().range(), 0..4);
    }

    #[test]
    fn remap_external_edges_stay_inside() {
        // An agent's text inserted at either edge stays outside the
        // selection; a cursor at the insert point goes after the text.
        let (_, mut set) = marked("«ab»");
        set.remap_external(&insert_at(0, 1));
        set.remap_external(&insert_at(3, 1));
        assert_eq!(set.primary().range(), 1..3);
        let (_, mut set) = marked("»ab«");
        set.remap_external(&insert_at(0, 1));
        set.remap_external(&insert_at(3, 1));
        assert_eq!(set.primary().range(), 1..3);
        assert!(set.primary().reversed());
        let mut set = SelectionSet::cursor(2);
        set.remap_external(&insert_at(2, 3));
        assert_eq!(set.primary().head, 5);
    }

    #[test]
    fn merge_flag_off_keeps_touching() {
        let (_, mut set) = marked("a|bc*|d");
        set.set_merge_overlapping(false);
        assert!(!set.merge_overlapping());
        set.remap(&Patch::new(vec![PatchEdit {
            old: 1..3,
            new: 1..1,
        }]));
        assert_eq!(set.len(), 2);
        assert!(!set.merge_overlapping());
    }

    #[test]
    fn newest_is_largest_id() {
        let (_, mut set) = marked("|a|b");
        let id = set.next_id();
        let mut sels = set.all().to_vec();
        sels[0].id = id;
        let set = set.rebuild(sels, id);
        assert_eq!(set.newest().id, id);
    }

    #[test]
    fn clamp_limits_to_buffer_end() {
        let (snap, _) = marked("ab|");
        let mut set = SelectionSet::cursor(10);
        set.clamp(&snap);
        assert_eq!(set.primary().head, 2);
    }
}
