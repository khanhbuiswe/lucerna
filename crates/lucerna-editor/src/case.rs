//! Changing the case of text ranges (vim's `gu`, `gU`, `g~`, `~`).

use std::ops::Range;

use crate::edit::replace_ranges;
use crate::plan::{Ctx, EditPlan};
use crate::undo_stops::EditKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    Lower,
    Upper,
    /// Lower-case chars become upper case and the other way round.
    Toggle,
}

/// Changes the case of the text in `ranges` (pre-edit offsets on char
/// boundaries, non-overlapping, in any order). `cursor` is the post-edit
/// offset of the single cursor the plan leaves. Ranges whose text does not
/// change produce no edit.
pub fn change_case(ctx: Ctx, ranges: &[Range<usize>], case: Case, cursor: usize) -> EditPlan {
    let edits = ranges
        .iter()
        .filter_map(|r| {
            let old = ctx.snapshot.text_for_range(r.clone());
            let new = convert(&old, case);
            (new != old).then(|| (r.clone(), new))
        })
        .collect();
    replace_ranges(ctx, edits, cursor, EditKind::Other)
}

fn convert(text: &str, case: Case) -> String {
    match case {
        Case::Lower => text.to_lowercase(),
        Case::Upper => text.to_uppercase(),
        Case::Toggle => text
            .chars()
            .flat_map(|c| -> Box<dyn Iterator<Item = char>> {
                if c.is_lowercase() {
                    Box::new(c.to_uppercase())
                } else if c.is_uppercase() {
                    Box::new(c.to_lowercase())
                } else {
                    Box::new(std::iter::once(c))
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::test_config;
    use crate::{Ctx, EditKind};
    use lucerna_text::BufferSnapshot;

    fn run(text: &str, ranges: &[std::ops::Range<usize>], case: Case, cursor: usize) -> String {
        let snapshot = BufferSnapshot::from_text(text);
        let config = test_config();
        let ctx = Ctx {
            snapshot: &snapshot,
            config: &config,
            language: None,
            syntax: None,
        };
        let plan = change_case(ctx, ranges, case, cursor);
        assert_eq!(plan.kind, EditKind::Other);
        assert!(plan.stop_before && plan.stop_after);
        crate::test_util::render_plan(&snapshot, &plan)
    }

    // A one-range slice, not a range wrapped by mistake.
    #[allow(clippy::single_range_in_vec_init)]
    #[test]
    fn upper_handles_non_ascii() {
        assert_eq!(run("abc déf", &[0..8], Case::Upper, 0), "|ABC DÉF");
    }

    #[allow(clippy::single_range_in_vec_init)]
    #[test]
    fn lower() {
        assert_eq!(run("ABC DÉF", &[4..8], Case::Lower, 4), "ABC |déf");
    }

    #[allow(clippy::single_range_in_vec_init)]
    #[test]
    fn toggle_swaps_each_char() {
        assert_eq!(run("aB", &[0..2], Case::Toggle, 0), "|Ab");
    }

    #[test]
    fn cursor_at_given_offset_over_several_ranges() {
        assert_eq!(run("ab cd ef", &[6..8, 0..2], Case::Upper, 3), "AB |cd EF");
    }
}
