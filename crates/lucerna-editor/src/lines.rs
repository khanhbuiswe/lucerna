//! Line commands: move, copy, delete, insert, join, indent and trim.
//!
//! Every command maps the rows its selections touch (merged into blocks) to
//! one edit per block. A selection ending at column 0 does not touch its
//! last row. Row edits stop at a row's end, never at its line break, so a
//! last line with no trailing newline needs no special case.
//!
//! Adapted from microsoft/vscode src/vs/editor/contrib/linesOperations/browser/linesOperations.ts (MIT)
//! Adapted from microsoft/vscode src/vs/editor/contrib/linesOperations/browser/moveLinesCommand.ts (MIT)
//! Adapted from microsoft/vscode src/vs/editor/contrib/linesOperations/browser/copyLinesCommand.ts (MIT)

use std::ops::Range;
use std::sync::Arc;

use lucerna_text::{Bias, BufferSnapshot};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::commands::{
    Item, Place, build, build_mapped, leading_whitespace, mk_patch, row_start, shift_edits,
    touched_rows,
};
use crate::plan::{Ctx, EditPlan};
use crate::selection::{Goal, Selection, SelectionSet};
use crate::undo_stops::EditKind;

type Edits = Vec<(Range<usize>, Arc<str>)>;

/// Rows (end exclusive) and the selections (indices into the set) in them.
struct Block {
    rows: Range<u32>,
    members: Vec<usize>,
}

/// Merges per-selection row ranges. Ranges that overlap are merged; with
/// `adjacent` ranges that merely touch are too.
fn blocks(
    set: &SelectionSet,
    adjacent: bool,
    rows: impl Fn(&Selection) -> Range<u32>,
) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for (i, sel) in set.all().iter().enumerate() {
        let r = rows(sel);
        if let Some(last) = out.last_mut()
            && (r.start < last.rows.end || (adjacent && r.start == last.rows.end))
        {
            last.rows.end = last.rows.end.max(r.end);
            last.members.push(i);
            continue;
        }
        out.push(Block {
            rows: r,
            members: vec![i],
        });
    }
    out
}

fn touched_blocks(ctx: Ctx, set: &SelectionSet) -> Vec<Block> {
    blocks(set, true, |s| touched_rows(ctx.snapshot, s.range()))
}

/// The offset just past row `row`'s text, before its line break.
fn row_end(snap: &BufferSnapshot, row: u32) -> usize {
    row_start(snap, row) + snap.line(row).len()
}

fn shifted(s: &Selection, delta: isize) -> Selection {
    let by = |o: usize| (o as isize + delta) as usize;
    Selection {
        id: s.id,
        anchor: by(s.anchor),
        head: by(s.head),
        goal: Goal::None,
    }
}

fn finish(set: &SelectionSet, edits: Edits, sels: Vec<Selection>) -> EditPlan {
    EditPlan {
        edits,
        selections: set.rebuild(sels, set.primary().id),
        kind: EditKind::Other,
        stop_before: true,
        stop_after: true,
        autoclose: Vec::new(),
    }
}

/// An id for a block's merged item: the primary's when it is a member.
fn block_id(set: &SelectionSet, block: &Block) -> u64 {
    let primary = set.primary().id;
    block
        .members
        .iter()
        .map(|&i| set.all()[i].id)
        .find(|&id| id == primary)
        .unwrap_or(set.all()[block.members[0]].id)
}

/// Swaps each block with the row above or below. Selections move with
/// their text.
pub fn move_lines(ctx: Ctx, set: &SelectionSet, up: bool) -> EditPlan {
    let snap = ctx.snapshot;
    let count = snap.line_count();
    let mut edits: Edits = Vec::new();
    let mut sels = Vec::with_capacity(set.len());
    for block in touched_blocks(ctx, set) {
        let (r0, r1) = (block.rows.start, block.rows.end - 1);
        let neighbor = if up {
            r0.checked_sub(1)
        } else {
            Some(r1 + 1).filter(|&n| n < count)
        };
        let Some(n) = neighbor else {
            sels.extend(block.members.iter().map(|&i| shifted(&set.all()[i], 0)));
            continue;
        };
        let line = snap.line(n);
        let body = snap.text_for_range(row_start(snap, r0)..row_end(snap, r1));
        let (range, text) = if up {
            (
                row_start(snap, n)..row_end(snap, r1),
                format!("{body}\n{line}"),
            )
        } else {
            (
                row_start(snap, r0)..row_end(snap, n),
                format!("{line}\n{body}"),
            )
        };
        let delta = line.len() as isize + 1;
        let delta = if up { -delta } else { delta };
        edits.push((range, Arc::from(text.as_str())));
        sels.extend(block.members.iter().map(|&i| shifted(&set.all()[i], delta)));
    }
    finish(set, edits, sels)
}

/// Duplicates each block above or below. Down puts the selection on the
/// copy; up leaves it where it was (on the copy now above the original),
/// as VS Code does.
pub fn copy_lines(ctx: Ctx, set: &SelectionSet, up: bool) -> EditPlan {
    let snap = ctx.snapshot;
    let mut edits: Edits = Vec::new();
    let mut sels = Vec::with_capacity(set.len());
    // Bytes inserted by earlier blocks.
    let mut before = 0isize;
    for block in touched_blocks(ctx, set) {
        let (r0, r1) = (block.rows.start, block.rows.end - 1);
        let body = snap.text_for_range(row_start(snap, r0)..row_end(snap, r1));
        let (at, text) = if up {
            (row_start(snap, r0), format!("{body}\n"))
        } else {
            (row_end(snap, r1), format!("\n{body}"))
        };
        let own = text.len() as isize;
        let delta = before + if up { 0 } else { own };
        before += own;
        edits.push((at..at, Arc::from(text.as_str())));
        sels.extend(block.members.iter().map(|&i| shifted(&set.all()[i], delta)));
    }
    finish(set, edits, sels)
}

/// Deletes the rows of each block with their line break. The cursor lands on
/// the following row at the same column (clamped); after the last row, on the
/// one before.
pub fn delete_lines(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    let snap = ctx.snapshot;
    let count = snap.line_count();
    let mut edits: Edits = Vec::new();
    // (id, target in old offsets, bias for mapping it)
    let mut targets: Vec<(u64, usize, Bias)> = Vec::new();
    for block in touched_blocks(ctx, set) {
        let (r0, r1) = (block.rows.start, block.rows.end - 1);
        let first = &set.all()[block.members[0]];
        // A char column, as VS Code keeps the position's column.
        let col = snap.char_column(snap.offset_to_point(first.head)) as usize;
        let id = block_id(set, &block);
        let (range, row, bias) = if r1 + 1 < count {
            (
                row_start(snap, r0)..row_start(snap, r1 + 1),
                r1 + 1,
                Bias::Right,
            )
        } else if r0 > 0 {
            // No row follows: the break before the block goes instead.
            (
                row_start(snap, r0) - 1..row_end(snap, r1),
                r0 - 1,
                Bias::Left,
            )
        } else {
            (0..snap.len_bytes(), 0, Bias::Left)
        };
        let target = if range.start == 0 && range.end == snap.len_bytes() {
            0
        } else {
            row_start(snap, row) + offset_at_char_column(&snap.line(row), col)
        };
        edits.push((range, Arc::from("")));
        targets.push((id, target, bias));
    }
    let patch = mk_patch(&edits);
    let sels = targets
        .into_iter()
        .map(|(id, target, bias)| {
            let o = patch.map_offset(target, bias);
            Selection {
                id,
                anchor: o,
                head: o,
                goal: Goal::None,
            }
        })
        .collect();
    finish(set, edits, sels)
}

/// The byte offset of char column `col` in `line` (clamped to its end),
/// moved forward to a grapheme boundary.
fn offset_at_char_column(line: &str, col: usize) -> usize {
    let mut chars = 0;
    for (at, g) in line.grapheme_indices(true) {
        if chars >= col {
            return at;
        }
        chars += g.chars().count();
    }
    line.len()
}

/// Inserts an empty line after (or before) each selection's head row, with the
/// indent Enter would give, and puts the cursor there. Before row 0 the new
/// line is empty; elsewhere it is "after the row above", as in VS Code.
pub fn insert_line(ctx: Ctx, set: &SelectionSet, after: bool) -> EditPlan {
    let snap = ctx.snapshot;
    let items = set
        .all()
        .iter()
        .map(|s| {
            // The head's row (VS Code's `positionLineNumber`).
            let row = snap.offset_to_point(s.head).row;
            let (at, text, cursor) = match (after, row) {
                (false, 0) => (0, "\n".to_string(), 0),
                _ => {
                    let row = if after { row } else { row - 1 };
                    let at = row_end(snap, row);
                    let e = crate::indent::enter(ctx, at..at, false);
                    (at, e.text, e.cursor)
                }
            };
            Item {
                id: s.id,
                range: at..at,
                text,
                place: Place::At(cursor),
            }
        })
        .collect();
    build(items, set, EditKind::Other, true, true)
}

/// The 4.1 line-mode shift (indent-stop snapping), also on empty selections.
///
/// A selection within one row also indents that row when it is empty
/// (`shouldIndentEmptyLines = startLine === endLine` in VS Code's
/// shiftCommand.ts); the cursor ends after the inserted indent.
pub fn indent_lines(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    let snap = ctx.snapshot;
    let mut edits = shift_edits(ctx, set, false);
    for sel in set.all() {
        let rows = touched_rows(snap, sel.range());
        if rows.len() == 1 && snap.line(rows.start).is_empty() {
            let at = row_start(snap, rows.start);
            let indent = crate::commands::indent_string(ctx, ctx.config.tab_size.max(1));
            edits.push((at..at, Arc::from(indent.as_str())));
        }
    }
    edits.sort_by_key(|(r, _)| (r.start, r.end));
    edits.dedup_by(|a, b| a.0 == b.0);
    build_mapped(set, edits, EditKind::Other)
}

pub fn outdent_lines(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    build_mapped(set, shift_edits(ctx, set, true), EditKind::Other)
}

/// Joins each selection's row with the next (an empty selection), or all of
/// its rows. A join replaces the line break and the next line's leading
/// whitespace with one space, or nothing when either side is empty or the
/// next line starts with a closing bracket. The cursor goes to the last join.
pub fn join_lines(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    let snap = ctx.snapshot;
    let count = snap.line_count();
    let rows = |s: &Selection| {
        let r = s.range();
        let (a, b) = (
            snap.offset_to_point(r.start).row,
            snap.offset_to_point(r.end).row,
        );
        a..(if b > a { b } else { a + 1 }) + 1
    };
    let mut items = Vec::new();
    // Selections with nothing to join (on the last row): kept as they are.
    let mut skipped: Vec<Selection> = Vec::new();
    for block in blocks(set, false, rows) {
        let (r0, last) = (block.rows.start, (block.rows.end - 1).min(count - 1));
        if last <= r0 {
            skipped.extend(block.members.iter().map(|&i| set.all()[i]));
            continue;
        }
        let mut current_len = snap.line(r0).len();
        let mut text = String::new();
        let mut cursor = 0;
        for row in r0 + 1..=last {
            let line = snap.line(row);
            let next = &line[leading_whitespace(&line).len()..];
            cursor = text.len();
            let closer = next.starts_with([')', ']', '}']);
            if current_len > 0 && !next.is_empty() && !closer {
                text.push(' ');
            }
            text.push_str(next);
            current_len += text.len() - cursor;
        }
        items.push(Item {
            id: block_id(set, &block),
            range: row_end(snap, r0)..row_end(snap, last),
            text,
            place: Place::At(cursor),
        });
    }
    if items.is_empty() {
        return finish(set, Vec::new(), set.all().to_vec());
    }
    let mut plan = build(items, set, EditKind::Other, true, true);
    if !skipped.is_empty() {
        let patch = mk_patch(&plan.edits);
        let mut sels = plan.selections.all().to_vec();
        let primary = if skipped.iter().any(|s| s.id == set.primary().id) {
            set.primary().id
        } else {
            plan.selections.primary().id
        };
        sels.extend(skipped.iter().map(|s| Selection {
            anchor: patch.map_offset(s.anchor, Bias::Right),
            head: patch.map_offset(s.head, Bias::Right),
            goal: Goal::None,
            ..*s
        }));
        plan.selections = set.rebuild(sels, primary);
    }
    plan
}

/// Removes trailing spaces and tabs on every line of the document.
pub fn trim_trailing_whitespace(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    let snap = ctx.snapshot;
    let edits = (0..snap.line_count())
        .filter_map(|row| {
            let line = snap.line(row);
            let keep = line.trim_end_matches([' ', '\t']).len();
            (keep < line.len()).then(|| {
                let at = row_start(snap, row);
                (at + keep..at + line.len(), Arc::from(""))
            })
        })
        .collect();
    build_mapped(set, edits, EditKind::Other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{marked, render_plan, test_config};

    fn run(src: &str, f: impl Fn(Ctx, &SelectionSet) -> EditPlan) -> String {
        let (snapshot, set) = marked(src);
        let config = test_config();
        let ctx = Ctx {
            snapshot: &snapshot,
            config: &config,
            language: None,
            syntax: None,
        };
        let plan = f(ctx, &set);
        assert_eq!(plan.kind, EditKind::Other);
        assert!(plan.stop_before && plan.stop_after);
        render_plan(&snapshot, &plan)
    }

    fn mv(src: &str, up: bool) -> String {
        run(src, |c, s| move_lines(c, s, up))
    }

    #[test]
    fn move_down_swaps() {
        assert_eq!(mv("a|\nb\nc", false), "b\na|\nc");
        assert_eq!(mv("a\nb|\nc", true), "b|\na\nc");
    }

    #[test]
    fn move_up_first_line_is_noop() {
        assert_eq!(mv("a|\nb", true), "a|\nb");
        assert_eq!(mv("a\nb|\n", false), "a\nb|\n");
    }

    #[test]
    fn move_down_last_line_without_newline() {
        assert_eq!(mv("a\nb|", true), "b|\na");
        assert_eq!(mv("a|\nb", false), "b\na|");
        assert_eq!(mv("a|\nb\n", false), "b\na|\n");
        assert_eq!(mv("a|\nb\nc", false), "b\na|\nc");
    }

    #[test]
    fn move_block_with_selection() {
        assert_eq!(mv("«a\nb»\nc", false), "c\n«a\nb»");
        // Ending at column 0 does not touch the last row.
        assert_eq!(mv("«a\n»b\nc", false), "b\n«a\n»c");
    }

    #[test]
    fn two_cursors_same_block_moves_once() {
        assert_eq!(mv("a|b|\nc\nd", false), "c\na|b|\nd");
        assert_eq!(mv("a|\nb|\nc\nd", false), "c\na|\nb|\nd");
        assert_eq!(mv("a|\nb\nc|\nd", false), "b\na|\nd\nc|");
    }

    #[test]
    fn copy_down_and_up_selection_placement() {
        let down = |s| run(s, |c, s| copy_lines(c, s, false));
        let up = |s| run(s, |c, s| copy_lines(c, s, true));
        assert_eq!(down("a|\nb"), "a\na|\nb");
        assert_eq!(down("«a\nb»\nc"), "a\nb\n«a\nb»\nc");
        assert_eq!(down("a\nb|"), "a\nb\nb|");
        assert_eq!(up("a|\nb"), "a|\na\nb");
        assert_eq!(up("a\nb|"), "a\nb|\nb");
        assert_eq!(up("«a\nb»\nc"), "«a\nb»\na\nb\nc");
        assert_eq!(down("a|\nb\nc|"), "a\na|\nb\nc\nc|");
    }

    #[test]
    fn delete_lines_middle_and_last() {
        let del = |s| run(s, delete_lines);
        assert_eq!(del("a\nb|\nc"), "a\nc|");
        assert_eq!(del("a\nb|cd\nxyz"), "a\nx|yz");
        assert_eq!(del("a\nbcd|\nxy"), "a\nxy|");
        assert_eq!(del("a\nb|"), "a|");
        assert_eq!(del("ab\ncd|\n"), "ab|\n");
        assert_eq!(del("a|"), "|");
        assert_eq!(del("«a\nb»\nc"), "c|");
        assert_eq!(del("a|\nb\nc|\nd"), "b|\nd|");
    }

    /// Final review, finding 6: the cursor keeps its char column, not its
    /// byte column, on multibyte text.
    #[test]
    fn delete_lines_keeps_char_column_on_multibyte_text() {
        let del = |s| run(s, delete_lines);
        assert_eq!(del("ab|\nđếx"), "đế|x");
        assert_eq!(del("ẫb|\nxyz"), "xy|z");
        assert_eq!(del("x\nViệt|"), "x|");
    }

    #[test]
    fn insert_line_after_keeps_indent() {
        let after = |s| run(s, |c, s| insert_line(c, s, true));
        let before = |s| run(s, |c, s| insert_line(c, s, false));
        assert_eq!(after("    a|b"), "    ab\n    |");
        assert_eq!(after("a|\nb"), "a\n|\nb");
        assert_eq!(before("    a\n    b|"), "    a\n    |\n    b");
        assert_eq!(before("a|"), "|\na");
        // Two cursors on one row open one line, not two.
        assert_eq!(after("  a|b|"), "  ab\n  |");
        assert_eq!(before("a\nb|c|"), "a\n|\nbc");
    }

    #[test]
    fn insert_line_before() {
        let before = |s| run(s, |c, s| insert_line(c, s, false));
        assert_eq!(before("a\nb|"), "a\n|\nb");
        assert_eq!(before("a|\nb"), "|\na\nb");
    }

    /// Final review, finding 6: the head's row decides, as VS Code's
    /// `positionLineNumber` does.
    #[test]
    fn insert_line_uses_head_row() {
        let after = |s| run(s, |c, s| insert_line(c, s, true));
        let before = |s| run(s, |c, s| insert_line(c, s, false));
        assert_eq!(after("»a\nb«\nc"), "a\n|\nb\nc");
        assert_eq!(before("a\n«b\nc»"), "a\nb\n|\nc");
    }

    #[test]
    fn indent_and_outdent_lines() {
        assert_eq!(run("a|", indent_lines), "    a|");
        assert_eq!(run("      a|", outdent_lines), "    a|");
        assert_eq!(run("«a\nb»", indent_lines), "«    a\n    b»");
        // An empty row indents when it is the only row (VS Code
        // shiftCommand.ts `shouldIndentEmptyLines`), but not inside a block.
        assert_eq!(run("|", indent_lines), "    |");
        assert_eq!(run("a\n|\nb", indent_lines), "a\n    |\nb");
        assert_eq!(run("«a\n\nb»", indent_lines), "«    a\n\n    b»");
    }

    #[test]
    fn join_lines_spaces_and_brackets() {
        assert_eq!(run("a|\n   b", join_lines), "a| b");
        assert_eq!(run("f(|\n)", join_lines), "f(|)");
        assert_eq!(run("|\nb", join_lines), "|b");
        assert_eq!(run("a|\nb", join_lines), "a| b");
        assert_eq!(run("a|b", join_lines), "a|b");
        assert_eq!(run("«a\n b\n   c»", join_lines), "a b| c");
        assert_eq!(run("a\nb|", join_lines), "a\nb|");
        // A block with nothing to join keeps its selection.
        assert_eq!(run("a|\nb\nc|", join_lines), "a| b\nc|");
        assert_eq!(run("a|\nb\nc*|", join_lines), "a| b\nc*|");
    }

    #[test]
    fn trim_trailing_whitespace_all_lines() {
        assert_eq!(
            run("a  \n\t b \t\n|c\n  ", trim_trailing_whitespace),
            "a\n\t b\n|c\n"
        );
        assert_eq!(run("a  |", trim_trailing_whitespace), "a|");
    }
}
