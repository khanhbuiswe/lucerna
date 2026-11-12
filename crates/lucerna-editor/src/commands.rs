//! Edit commands: typing, deleting, newline, tab and outdent. Each maps
//! every selection to an edit and returns an `EditPlan`.
//!
//! Word deletes follow microsoft/vscode `deleteWordLeft` / `deleteWordRight`
//! (WordStart / WordEnd boundaries); the rest follows `cursorTypeEditOperations.ts`
//! and `cursorDeleteOperations.ts`.
//!
//! Adapted from microsoft/vscode src/vs/editor/common/cursor/cursorDeleteOperations.ts (MIT)
//! Adapted from microsoft/vscode src/vs/editor/common/cursor/cursorTypeEditOperations.ts (MIT)
//! Adapted from microsoft/vscode src/vs/editor/contrib/wordOperations/browser/wordOperations.ts (MIT)
//! Adapted from microsoft/vscode src/vs/editor/common/commands/shiftCommand.ts (MIT)

use std::ops::Range;
use std::sync::Arc;

use lucerna_text::{Bias, BufferSnapshot, Patch, PatchEdit, Point};
use unicode_segmentation::GraphemeCursor;

use crate::plan::{Ctx, EditPlan};
use crate::selection::{Goal, Selection, SelectionSet};
use crate::undo_stops::{EditKind, should_stop_between, typing_kind};
use crate::words;

/// Where a selection's cursor goes after its edit.
#[derive(Clone, Copy)]
pub(crate) enum Place {
    /// After the inserted text.
    End,
    /// Before the inserted text.
    Start,
    /// Keep the logical position of this (old) offset, which lies at or
    /// after the edit's range.
    Keep(usize),
}

pub(crate) struct Item {
    pub id: u64,
    pub range: Range<usize>,
    pub text: String,
    pub place: Place,
}

fn mk_patch(edits: &[(Range<usize>, Arc<str>)]) -> Patch {
    let mut delta = 0isize;
    let mut out = Vec::with_capacity(edits.len());
    for (range, text) in edits {
        let start = (range.start as isize + delta) as usize;
        out.push(PatchEdit {
            old: range.clone(),
            new: start..start + text.len(),
        });
        delta += text.len() as isize - range.len() as isize;
    }
    Patch::new(out)
}

fn cursor(id: u64, offset: usize) -> Selection {
    Selection {
        id,
        anchor: offset,
        head: offset,
        goal: Goal::None,
    }
}

/// Builds a plan from one edit per selection. Edits that overlap are merged
/// (the first one's text wins); no-op edits are dropped from the plan.
pub(crate) fn build(
    mut items: Vec<Item>,
    primary: u64,
    kind: EditKind,
    stop_before: bool,
    stop_after: bool,
) -> EditPlan {
    items.sort_by_key(|i| (i.range.start, i.range.end));
    // (range, text, member item indices)
    let mut merged: Vec<(Range<usize>, String, Vec<usize>)> = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        if let Some(last) = merged.last_mut()
            && (item.range.start < last.0.end || item.range.start == last.0.start)
        {
            last.0.end = last.0.end.max(item.range.end);
            last.2.push(idx);
            continue;
        }
        merged.push((item.range.clone(), item.text.clone(), vec![idx]));
    }
    let all: Vec<(Range<usize>, Arc<str>)> = merged
        .iter()
        .map(|(r, t, _)| (r.clone(), Arc::from(t.as_str())))
        .collect();
    let patch = mk_patch(&all);
    let mut sels = Vec::with_capacity(items.len());
    for (m, (_, _, members)) in merged.iter().enumerate() {
        let new = &patch.edits()[m].new;
        for &idx in members {
            let item = &items[idx];
            let offset = match item.place {
                Place::End => new.end,
                Place::Start => new.start,
                Place::Keep(old) => new.end + (old - merged[m].0.end.min(old)),
            };
            sels.push(cursor(item.id, offset));
        }
    }
    EditPlan {
        edits: all
            .into_iter()
            .filter(|(r, t)| !(r.is_empty() && t.is_empty()))
            .collect(),
        selections: SelectionSet::from_selections(sels, primary, true),
        kind,
        stop_before,
        stop_after,
    }
}

/// A plan from line-granular edits (sorted, non-overlapping); selections
/// keep their extent and map through the patch.
fn build_mapped(
    set: &SelectionSet,
    edits: Vec<(Range<usize>, Arc<str>)>,
    kind: EditKind,
) -> EditPlan {
    let patch = mk_patch(&edits);
    let sels = set
        .all()
        .iter()
        .map(|s| {
            let r = s.range();
            let (start, end) = if s.is_empty() {
                let o = patch.map_offset(r.start, Bias::Right);
                (o, o)
            } else {
                (
                    patch.map_offset(r.start, Bias::Left),
                    patch.map_offset(r.end, Bias::Right),
                )
            };
            let (anchor, head) = if s.reversed() {
                (end, start)
            } else {
                (start, end)
            };
            Selection {
                id: s.id,
                anchor,
                head,
                goal: Goal::None,
            }
        })
        .collect();
    EditPlan {
        edits,
        selections: SelectionSet::from_selections(sels, set.primary().id, true),
        kind,
        stop_before: true,
        stop_after: true,
    }
}

pub(crate) fn row_start(snapshot: &BufferSnapshot, row: u32) -> usize {
    snapshot.point_to_offset(Point { row, column: 0 })
}

fn leading_whitespace(line: &str) -> &str {
    let n = line
        .find(|c: char| c != ' ' && c != '\t')
        .unwrap_or(line.len());
    &line[..n]
}

/// The display column after `prefix`, with tabs jumping to the next stop.
fn display_col(prefix: &str, tab_size: u32) -> u32 {
    let ts = tab_size.max(1);
    prefix.chars().fold(0, |col, c| {
        if c == '\t' {
            (col / ts + 1) * ts
        } else {
            col + 1
        }
    })
}

pub fn insert(ctx: Ctx, set: &SelectionSet, text: &str, prev: EditKind) -> EditPlan {
    let kind = if text.chars().count() > 1 {
        EditKind::Other
    } else {
        typing_kind(text, prev)
    };
    let items = set
        .all()
        .iter()
        .map(|s| Item {
            id: s.id,
            range: s.range(),
            text: text.to_string(),
            place: Place::End,
        })
        .collect();
    let _ = ctx;
    build(
        items,
        set.primary().id,
        kind,
        should_stop_between(prev, kind),
        false,
    )
}

/// The previous grapheme boundary before `offset` (the previous `\n` counts
/// as one step). `offset` must be above 0.
fn prev_boundary(snapshot: &BufferSnapshot, offset: usize) -> usize {
    let p = snapshot.offset_to_point(offset);
    if p.column == 0 {
        return offset - 1;
    }
    let line = snapshot.line(p.row);
    let col = (p.column as usize).min(line.len());
    let mut c = GraphemeCursor::new(col, line.len(), true);
    let prev = c.prev_boundary(&line, 0).ok().flatten().unwrap_or(0);
    offset - (col - prev)
}

fn next_boundary(snapshot: &BufferSnapshot, offset: usize) -> usize {
    if offset >= snapshot.len_bytes() {
        return offset;
    }
    let p = snapshot.offset_to_point(offset);
    let line = snapshot.line(p.row);
    let col = p.column as usize;
    if col >= line.len() {
        return offset + 1;
    }
    let mut c = GraphemeCursor::new(col, line.len(), true);
    let next = c
        .next_boundary(&line, 0)
        .ok()
        .flatten()
        .unwrap_or(line.len());
    offset + (next - col)
}

fn delete_plan(
    ctx: Ctx,
    set: &SelectionSet,
    kind: EditKind,
    range_of: impl Fn(&Selection) -> Range<usize>,
) -> EditPlan {
    let items: Vec<Item> = set
        .all()
        .iter()
        .map(|s| Item {
            id: s.id,
            range: if s.is_empty() { range_of(s) } else { s.range() },
            text: String::new(),
            place: Place::End,
        })
        .collect();
    let multi_line = items
        .iter()
        .any(|i| ctx.snapshot.text_for_range(i.range.clone()).contains('\n'));
    let (before, after) = match kind {
        EditKind::DeletingLeft | EditKind::DeletingRight => (multi_line, false),
        _ => (true, true),
    };
    build(items, set.primary().id, kind, before, after)
}

pub fn delete_left(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    delete_plan(ctx, set, EditKind::DeletingLeft, |s| {
        let snap = ctx.snapshot;
        let o = s.head;
        if o == 0 {
            return 0..0;
        }
        let p = snap.offset_to_point(o);
        let line = snap.line(p.row);
        let col = (p.column as usize).min(line.len());
        // Inside leading whitespace: back to the previous tab stop, by
        // visible column.
        if col > 0 && line[..col].bytes().all(|b| b == b' ' || b == b'\t') {
            let ts = ctx.config.tab_size.max(1);
            let target = ((display_col(&line[..col], ts) - 1) / ts) * ts;
            let keep = (0..=col)
                .rev()
                .find(|&q| display_col(&line[..q], ts) <= target)
                .unwrap_or(0);
            return o - (col - keep)..o;
        }
        prev_boundary(snap, o)..o
    })
}

pub fn delete_right(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    delete_plan(ctx, set, EditKind::DeletingRight, |s| {
        s.head..next_boundary(ctx.snapshot, s.head)
    })
}

pub fn delete_word_left(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    delete_plan(ctx, set, EditKind::Other, |s| {
        let snap = ctx.snapshot;
        let o = s.head;
        let p = snap.offset_to_point(o);
        if p.column == 0 {
            // Start of a line: only the line break goes.
            return o.saturating_sub(1)..o;
        }
        // Two or more blanks before the cursor: delete just that run.
        let line = snap.line(p.row);
        let before = &line[..(p.column as usize).min(line.len())];
        let blanks = before.len() - before.trim_end_matches([' ', '\t']).len();
        if blanks >= 2 {
            return o - blanks..o;
        }
        words::word_start_left(snap, o, &ctx.config.word_separators, false)..o
    })
}

pub fn delete_word_right(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    delete_plan(ctx, set, EditKind::Other, |s| {
        let snap = ctx.snapshot;
        let o = s.head;
        let p = snap.offset_to_point(o);
        if p.column as usize >= snap.line(p.row).len() {
            return o..next_boundary(snap, o);
        }
        // Two or more blanks after the cursor: delete just that run.
        let line = snap.line(p.row);
        let after = &line[p.column as usize..];
        let blanks = after.len() - after.trim_start_matches([' ', '\t']).len();
        if blanks >= 2 {
            return o..o + blanks;
        }
        o..words::word_end_right(snap, o, &ctx.config.word_separators)
    })
}

pub fn delete_all_left(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    delete_plan(ctx, set, EditKind::Other, |s| {
        let snap = ctx.snapshot;
        let o = s.head;
        let p = snap.offset_to_point(o);
        if p.column == 0 {
            return o.saturating_sub(1)..o;
        }
        row_start(snap, p.row)..o
    })
}

pub fn delete_all_right(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    delete_plan(ctx, set, EditKind::Other, |s| {
        let snap = ctx.snapshot;
        let o = s.head;
        let p = snap.offset_to_point(o);
        let len = snap.line(p.row).len();
        if p.column as usize >= len {
            return o..next_boundary(snap, o);
        }
        o..o + (len - p.column as usize)
    })
}

/// The `\n` + indent a newline at `start` inserts, and the range it replaces.
fn newline_edit(ctx: Ctx, range: Range<usize>, trim: bool) -> (Range<usize>, String) {
    let snap = ctx.snapshot;
    let p = snap.offset_to_point(range.start);
    let line = snap.line(p.row);
    let col = (p.column as usize).min(line.len());
    let indent = if ctx.config.keep_indent {
        let lead = leading_whitespace(&line);
        &lead[..lead.len().min(col)]
    } else {
        ""
    };
    let text = format!("\n{indent}");
    // A whitespace-only line with the cursor at its end loses that
    // whitespace; the indent is re-inserted after the break.
    if trim
        && ctx.config.trim_auto_whitespace
        && range.is_empty()
        && col == line.len()
        && col > 0
        && line.bytes().all(|b| b == b' ' || b == b'\t')
    {
        return (row_start(snap, p.row)..range.end, text);
    }
    (range, text)
}

pub fn newline(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    let items = set
        .all()
        .iter()
        .map(|s| {
            let (range, text) = newline_edit(ctx, s.range(), true);
            Item {
                id: s.id,
                range,
                text,
                place: Place::End,
            }
        })
        .collect();
    build(items, set.primary().id, EditKind::TypingOther, true, false)
}

/// Inserts a line break (with indent) after the cursor and leaves the cursor
/// where it was (`editor.action.lineBreakInsert`).
pub fn line_break_insert(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    let items = set
        .all()
        .iter()
        .map(|s| {
            let (range, text) = newline_edit(ctx, s.range(), false);
            Item {
                id: s.id,
                range,
                text,
                place: Place::Start,
            }
        })
        .collect();
    build(items, set.primary().id, EditKind::Other, true, true)
}

/// `cols` visible columns of indent in the configured style.
fn indent_string(ctx: Ctx, cols: u32) -> String {
    let ts = ctx.config.tab_size.max(1);
    if ctx.config.insert_spaces {
        " ".repeat(cols as usize)
    } else {
        "\t".repeat((cols / ts) as usize) + &" ".repeat((cols % ts) as usize)
    }
}

/// Snaps each touched line's indent to the next (or previous) indent stop,
/// rewritten in the configured style (like VS Code's `ShiftCommand`).
fn shift_edits(ctx: Ctx, set: &SelectionSet, outdent: bool) -> Vec<(Range<usize>, Arc<str>)> {
    let snap = ctx.snapshot;
    let ts = ctx.config.tab_size.max(1);
    rows_of(ctx, set)
        .into_iter()
        .filter_map(|row| {
            let line = snap.line(row);
            let lead = leading_whitespace(&line);
            let vis = display_col(lead, ts);
            let desired = if outdent {
                if vis == 0 {
                    return None;
                }
                ((vis - 1) / ts) * ts
            } else {
                if line.is_empty() {
                    return None;
                }
                (vis / ts + 1) * ts
            };
            let new = indent_string(ctx, desired);
            let at = row_start(snap, row);
            (new != lead).then(|| (at..at + lead.len(), Arc::from(new.as_str())))
        })
        .collect()
}

/// The rows a selection touches: a selection ending at column 0 of a later
/// row does not touch that row.
fn touched_rows(snap: &BufferSnapshot, r: Range<usize>) -> Range<u32> {
    let (a, b) = (snap.offset_to_point(r.start), snap.offset_to_point(r.end));
    let end = if b.row > a.row && b.column == 0 {
        b.row - 1
    } else {
        b.row
    };
    a.row..end + 1
}

fn rows_of(ctx: Ctx, set: &SelectionSet) -> Vec<u32> {
    let mut rows: Vec<u32> = set
        .all()
        .iter()
        .flat_map(|s| touched_rows(ctx.snapshot, s.range()))
        .collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// With an empty selection, inserts up to the next tab stop. A selection on
/// one line is replaced by the indent. When any selection spans lines, every
/// touched line is indented to the next indent stop (empty lines are skipped).
pub fn tab(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    let snap = ctx.snapshot;
    let multi = set.all().iter().any(|s| {
        let r = s.range();
        snap.offset_to_point(r.start).row != snap.offset_to_point(r.end).row
    });
    if multi {
        let edits = shift_edits(ctx, set, false);
        return build_mapped(set, edits, EditKind::Other);
    }
    let items = set
        .all()
        .iter()
        .map(|s| {
            let r = s.range();
            let p = snap.offset_to_point(r.start);
            let text = if ctx.config.insert_spaces {
                let line = snap.line(p.row);
                let prefix = &line[..(p.column as usize).min(line.len())];
                let ts = ctx.config.tab_size.max(1);
                " ".repeat((ts - display_col(prefix, ts) % ts) as usize)
            } else {
                "\t".to_string()
            };
            Item {
                id: s.id,
                range: r,
                text,
                place: Place::End,
            }
        })
        .collect();
    build(items, set.primary().id, EditKind::Other, true, true)
}

/// Moves each touched line's indent back to the previous indent stop.
pub fn outdent(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    build_mapped(set, shift_edits(ctx, set, true), EditKind::Other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{marked, render_plan, test_config};

    fn run(src: &str, f: impl Fn(Ctx, &SelectionSet) -> EditPlan) -> String {
        let (snapshot, set) = marked(src);
        let config = test_config();
        let plan = f(
            Ctx {
                snapshot: &snapshot,
                config: &config,
            },
            &set,
        );
        render_plan(&snapshot, &plan)
    }

    fn typed(src: &str, text: &str) -> String {
        run(src, |c, s| insert(c, s, text, EditKind::Other))
    }

    #[test]
    fn insert_cases() {
        assert_eq!(typed("a|b", "x"), "ax|b");
        assert_eq!(typed("a«bc»d", "x"), "ax|d");
        assert_eq!(typed("|a\n|b", "x"), "x|a\nx|b");
        assert_eq!(typed("a|b", "é"), "aé|b");
    }

    #[test]
    fn insert_kinds() {
        let (snap, set) = marked("a|");
        let cfg = test_config();
        let ctx = Ctx {
            snapshot: &snap,
            config: &cfg,
        };
        let p = insert(ctx, &set, "x", EditKind::TypingOther);
        assert_eq!(
            (p.kind, p.stop_before, p.stop_after),
            (EditKind::TypingOther, false, false)
        );
        let p = insert(ctx, &set, " ", EditKind::TypingOther);
        assert_eq!(p.kind, EditKind::TypingFirstSpace);
        assert!(p.stop_before);
        let p = insert(ctx, &set, "ab", EditKind::TypingOther);
        assert_eq!(p.kind, EditKind::Other);
        assert!(p.stop_before);
    }

    #[test]
    fn delete_left_cases() {
        assert_eq!(run("ab|c", delete_left), "a|c");
        assert_eq!(run("a«bc»d", delete_left), "a|d");
        assert_eq!(run("|ab", delete_left), "|ab");
        assert_eq!(run("a\n|b", delete_left), "a|b");
        assert_eq!(run("        |x", delete_left), "    |x");
        assert_eq!(run("      |x", delete_left), "    |x");
        assert_eq!(run("e\u{301}|x", delete_left), "|x");
        assert_eq!(run("a|b\nc|d", delete_left), "|b\n|d");
        // adjacent cursors merge instead of overlapping
        assert_eq!(run("a||b", delete_left), "|b");
        assert_eq!(run("ab|c|", delete_left), "a|");
    }

    #[test]
    fn delete_left_flags_and_noop() {
        let (snap, set) = marked("|ab");
        let cfg = test_config();
        let ctx = Ctx {
            snapshot: &snap,
            config: &cfg,
        };
        assert!(delete_left(ctx, &set).edits.is_empty());
        let (snap, set) = marked("a\n|b");
        let p = delete_left(
            Ctx {
                snapshot: &snap,
                config: &cfg,
            },
            &set,
        );
        assert_eq!(p.kind, EditKind::DeletingLeft);
        assert!(p.stop_before && !p.stop_after);
    }

    #[test]
    fn delete_right_cases() {
        assert_eq!(run("a|bc", delete_right), "a|c");
        assert_eq!(run("a|\nb", delete_right), "a|b");
        assert_eq!(run("ab|", delete_right), "ab|");
        assert_eq!(run("|a|b", delete_right), "|");
        assert_eq!(run("a|b\nc|d", delete_right), "a|\nc|");
    }

    #[test]
    fn word_deletes() {
        assert_eq!(run("foo bar|", delete_word_left), "foo |");
        assert_eq!(run("a\n|b", delete_word_left), "a|b");
        assert_eq!(run("|foo bar", delete_word_right), "| bar");
        assert_eq!(run("a|\nb", delete_word_right), "a|b");
        assert_eq!(run("foo bar|\nab cd|", delete_word_left), "foo |\nab |");
        assert_eq!(run("«foo» bar", delete_word_left), "| bar");
    }

    #[test]
    fn delete_all() {
        assert_eq!(run("ab|cd", delete_all_left), "|cd");
        assert_eq!(run("ab|cd", delete_all_right), "ab|");
        assert_eq!(run("a\n|b", delete_all_left), "a|b");
        assert_eq!(run("a|\nb", delete_all_right), "a|b");
        assert_eq!(run("ab|c\nde|f", delete_all_right), "ab|\nde|");
    }

    #[test]
    fn newline_cases() {
        assert_eq!(run("    ab|c", newline), "    ab\n    |c");
        assert_eq!(run("  ab|c", newline), "  ab\n  |c");
        assert_eq!(run(" | ab", newline), " \n | ab");
        assert_eq!(run("    |", newline), "\n    |");
        assert_eq!(run("a«bc»d", newline), "a\n|d");
        assert_eq!(run("  a|\n  b|", newline), "  a\n  |\n  b\n  |");
    }

    #[test]
    fn newline_without_keep_indent() {
        let (snap, set) = marked("    ab|c");
        let mut cfg = test_config();
        cfg.keep_indent = false;
        let p = newline(
            Ctx {
                snapshot: &snap,
                config: &cfg,
            },
            &set,
        );
        assert_eq!(render_plan(&snap, &p), "    ab\n|c");
        assert_eq!(p.kind, EditKind::TypingOther);
        assert!(p.stop_before && !p.stop_after);
    }

    #[test]
    fn line_break_insert_keeps_cursor() {
        assert_eq!(run("  a|b", line_break_insert), "  a|\n  b");
    }

    #[test]
    fn tab_cases() {
        assert_eq!(run("ab|", tab), "ab  |");
        assert_eq!(run("|", tab), "    |");
        assert_eq!(run("a|b\nabc|", tab), "a   |b\nabc |");
        assert_eq!(run("a«b»c", tab), "a   |c");
        assert_eq!(run("\t|", tab), "\t    |");
    }

    #[test]
    fn tab_with_tabs() {
        let (snap, set) = marked("ab|");
        let mut cfg = test_config();
        cfg.insert_spaces = false;
        let p = tab(
            Ctx {
                snapshot: &snap,
                config: &cfg,
            },
            &set,
        );
        assert_eq!(render_plan(&snap, &p), "ab\t|");
    }

    #[test]
    fn tab_indents_multiple_lines() {
        assert_eq!(run("a«b\n\ncd»e", tab), "    a«b\n\n    cd»e");
        assert_eq!(run("«a\nb\n»c", tab), "«    a\n    b\n»c");
    }

    #[test]
    fn outdent_cases() {
        assert_eq!(run("      a|b", outdent), "    a|b");
        assert_eq!(run("  a|b", outdent), "a|b");
        assert_eq!(run("\ta|b", outdent), "a|b");
        assert_eq!(run("a|b", outdent), "a|b");
        assert_eq!(run("    «a\n    b»", outdent), "«a\nb»");
        assert_eq!(run("    a|\n    b|", outdent), "a|\nb|");
        assert_eq!(run("  |  a", outdent), "|a");
        assert_eq!(run("        a|b", outdent), "    a|b");
    }

    #[test]
    fn shift_normalises_indent() {
        assert_eq!(run("«  a\nb»", tab), "«    a\n    b»");
        assert_eq!(run("«\ta\nb»", tab), "«        a\n    b»");
        assert_eq!(run("«  a\n\tb»", tab), "«    a\n        b»");
        assert_eq!(run("«\t  a\nb»", outdent), "«    a\nb»");
    }

    #[test]
    fn word_delete_whitespace_heuristics() {
        assert_eq!(run("foo    |", delete_word_left), "foo|");
        assert_eq!(run("|   foo", delete_word_right), "|foo");
        assert_eq!(run("foo |", delete_word_left), "|");
        assert_eq!(run("| foo", delete_word_right), "|");
        assert_eq!(run("a\t |b", delete_word_left), "a|b");
        assert_eq!(run("a|  ", delete_word_right), "a|");
    }

    #[test]
    fn delete_left_visible_tab_stop() {
        assert_eq!(run("\t  |x", delete_left), "\t|x");
        assert_eq!(run("\t|x", delete_left), "|x");
        assert_eq!(run("  \t|x", delete_left), "|x");
    }
}
