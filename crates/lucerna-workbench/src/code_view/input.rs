//! Typed text and IME composition reach the code view through GPUI's
//! platform input handler (spec §5). Offsets cross it in UTF-16, converted
//! through the snapshot; ranges given relative to the primary selection
//! apply to every selection.
//!
//! A composition (Vietnamese Telex, CJK) is written into the buffer as it
//! changes, with one marked range per selection, and pushes no undo stop
//! until it commits: the whole composition is one undo step.

use std::ops::Range;
use std::sync::Arc;

use gpui::{Bounds, Context, EntityInputHandler, Pixels, UTF16Selection, Window};
use lucerna_editor::{EditKind, EditPlan, Goal, Selection, SelectionSet};

use super::CodeView;

impl CodeView {
    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.snapshot.offset_to_utf16(range.start)..self.snapshot.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let start = self.snapshot.utf16_to_offset(range.start);
        start..self.snapshot.utf16_to_offset(range.end).max(start)
    }

    /// The index of the primary in the selections.
    fn primary_index(&self) -> usize {
        let id = self.selections.primary().id;
        self.selections
            .all()
            .iter()
            .position(|s| s.id == id)
            .unwrap_or(0)
    }

    /// The marked ranges when they still match the selections one to one.
    fn marked_ranges_checked(&self) -> Option<&[Range<usize>]> {
        self.marked
            .as_deref()
            .filter(|m| m.len() == self.selections.len())
    }

    /// The ranges the input replaces, one per selection: the marked ones
    /// (else the selections), moved by how far `range_utf16` (relative to
    /// the primary) is from the primary's. Ranges that would overlap
    /// collapse to the primary's alone.
    fn input_ranges(&self, range_utf16: Option<Range<usize>>) -> Vec<Range<usize>> {
        let base: Vec<Range<usize>> = match self.marked_ranges_checked() {
            Some(marked) => marked.to_vec(),
            None => self.selections.all().iter().map(Selection::range).collect(),
        };
        let Some(range) = range_utf16 else {
            return base;
        };
        let target = self.range_from_utf16(&range);
        let primary = &base[self.primary_index()];
        let (start_delta, end_delta) = (
            target.start as isize - primary.start as isize,
            target.end as isize - primary.end as isize,
        );
        let len = self.snapshot.len_bytes() as isize;
        let at = |offset: usize, delta: isize| {
            let moved = (offset as isize + delta).clamp(0, len) as usize;
            self.snapshot.floor_char_boundary(moved)
        };
        let moved: Vec<Range<usize>> = base
            .iter()
            .map(|r| {
                let start = at(r.start, start_delta);
                start..at(r.end, end_delta).max(start)
            })
            .collect();
        let overlap = moved
            .windows(2)
            .any(|w| w[0].end > w[1].start || w[0] == w[1]);
        if overlap { vec![target] } else { moved }
    }

    /// Replaces the input ranges for `range_utf16` (one per selection, or
    /// one for the primary alone; see `input_ranges`) with `text`. Each
    /// selection then covers `selected` (UTF-16, within the new text), or
    /// sits after the text. With `mark`, the new text is
    /// marked as the composition. Returns whether the text changed.
    fn replace_ranges(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        mark: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        // A panic here applies nothing and ends the composition (spec §16).
        let built = self.contained(
            "input",
            |v| v.input_plan(v.input_ranges(range_utf16), text, selected),
            cx,
        );
        let Some((plan, marked)) = built else {
            return false;
        };
        let applied = self.transact(plan, cx);
        self.marked = (applied && mark && !text.is_empty()).then_some(marked);
        applied
    }

    /// The edit replacing `ranges` with `text` (see `replace_ranges`), and
    /// the ranges of the new text.
    fn input_plan(
        &self,
        ranges: Vec<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
    ) -> (EditPlan, Vec<Range<usize>>) {
        let text: Arc<str> = Arc::from(text);
        // The selected part of the text, in bytes.
        let selected = selected.map(|r| {
            let byte = |utf16: usize| {
                let mut units = 0;
                for (i, c) in text.char_indices() {
                    if units >= utf16 {
                        return i;
                    }
                    units += c.len_utf16();
                }
                text.len()
            };
            let start = byte(r.start);
            start..byte(r.end).max(start)
        });
        let all = self.selections.all();
        let primary = self.selections.primary().id;
        let ids: Vec<u64> = if ranges.len() == all.len() {
            all.iter().map(|s| s.id).collect()
        } else {
            vec![primary]
        };
        let mut selections = Vec::with_capacity(ranges.len());
        let mut marked = Vec::with_capacity(ranges.len());
        let mut shift: isize = 0;
        for (range, &id) in ranges.iter().zip(&ids) {
            let start = (range.start as isize + shift) as usize;
            let (anchor, head) = match &selected {
                Some(sel) => (start + sel.start, start + sel.end),
                None => (start + text.len(), start + text.len()),
            };
            selections.push(Selection {
                id,
                anchor,
                head,
                goal: Goal::None,
            });
            marked.push(start..start + text.len());
            shift += text.len() as isize - range.len() as isize;
        }
        // Typing: one step with the typing around it, and no stop inside a
        // composition (`should_stop_between` is false).
        let plan = EditPlan::new(
            ranges.into_iter().map(|r| (r, text.clone())).collect(),
            SelectionSet::from_selections(selections, primary, false),
            EditKind::TypingOther,
        );
        (plan, marked)
    }
}

impl EntityInputHandler for CodeView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        *adjusted_range = Some(self.range_to_utf16(&range));
        Some(self.snapshot.text_for_range(range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let primary = self.selections.primary();
        Some(UTF16Selection {
            range: self.range_to_utf16(&primary.range()),
            reversed: primary.reversed(),
        })
    }

    /// The primary selection's marked range.
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let marked = self.marked_ranges_checked()?;
        Some(self.range_to_utf16(&marked[self.primary_index()]))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.unmark();
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }

    /// Commits text: a typed char, or the end of a composition, which
    /// replaces the marked ranges.
    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Return, Tab and the like arrive as commands; a control char here
        // is a key no binding took.
        let control = !text.is_empty() && text.chars().all(char::is_control);
        if !self.takes_text() || control {
            self.unmark();
            return;
        }
        if range_utf16.is_none() && self.marked.is_none() {
            if text.is_empty() {
                return;
            }
            self.type_text(text, cx);
        } else {
            self.replace_ranges(range_utf16, text, None, false, cx);
            self.unmark();
        }
        window.notify_text_input_changed(&self.focus);
    }

    /// Shows a composition in progress in place of the marked ranges (or
    /// the selections), marked and underlined.
    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.takes_text() {
            self.unmark();
            return;
        }
        self.replace_ranges(range_utf16, text, new_selected_range_utf16, true, cx);
    }

    /// Where the IME puts its candidate window: the start of the primary
    /// selection's range (`Frame::char_position`'s char cell).
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let start = self.range_from_utf16(&range_utf16).start;
        self.frame
            .as_ref()?
            .char_bounds(self.snapshot.offset_to_point(start))
    }

    fn character_index_for_point(
        &mut self,
        position: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let p = self.frame.as_ref()?.point_at(position)?;
        Some(self.snapshot.offset_to_utf16(self.offset_of(p)))
    }

    /// The IME selects within the text: a selection change, not an edit.
    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.range_from_utf16(&range_utf16);
        let marked = self.marked.take();
        let set = self.single(range.start, range.end);
        self.set_selections_without_stop(set, false, cx);
        // Still composing: the selection moved inside the marked text.
        self.marked = marked.filter(|m| m.len() == 1);
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.snapshot.offset_to_utf16(self.snapshot.len_bytes()))
    }
}
