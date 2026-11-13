//! A generic helper: replace a list of ranges with text, leaving one cursor.

use std::ops::Range;
use std::sync::Arc;

use crate::plan::{Ctx, EditPlan};
use crate::selection::SelectionSet;
use crate::undo_stops::EditKind;

/// Replaces each range (pre-edit offsets, non-overlapping, in any order)
/// with its text. `cursor` is the post-edit offset of the single cursor the
/// plan leaves. The plan is its own undo step: it stops before and after.
pub fn replace_ranges(
    ctx: Ctx,
    mut edits: Vec<(Range<usize>, String)>,
    cursor: usize,
    kind: EditKind,
) -> EditPlan {
    edits.sort_by_key(|(r, _)| (r.start, r.end));
    debug_assert!(
        edits.windows(2).all(|w| w[0].0.end <= w[1].0.start),
        "overlapping edits: {edits:?}"
    );
    debug_assert!(edits.iter().all(|(r, _)| r.end <= ctx.snapshot.len_bytes()));
    let edits: Vec<(Range<usize>, Arc<str>)> = edits
        .into_iter()
        .map(|(r, text)| (r, Arc::from(text)))
        .collect();
    let mut plan = EditPlan::new(edits, SelectionSet::cursor(cursor), kind);
    plan.stop_before = true;
    plan.stop_after = true;
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{render_plan, test_config};
    use crate::{Ctx, EditKind};
    use lucerna_text::BufferSnapshot;

    #[test]
    fn applies_every_edit_and_places_cursor_in_post_edit_offsets() {
        let snapshot = BufferSnapshot::from_text("one two three");
        let config = test_config();
        let ctx = Ctx {
            snapshot: &snapshot,
            config: &config,
            language: None,
            syntax: None,
        };
        // Given out of order: the plan sorts them by start.
        let edits = vec![(8..13, "3".to_string()), (0..3, "uno".to_string() + "!")];
        let plan = replace_ranges(ctx, edits, 9, EditKind::DeletingRight);
        assert_eq!(plan.kind, EditKind::DeletingRight);
        assert!(plan.stop_before && plan.stop_after);
        assert_eq!(plan.edits[0].0, 0..3);
        assert_eq!(render_plan(&snapshot, &plan), "uno! two |3");
    }
}
