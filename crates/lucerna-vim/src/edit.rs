//! Operators turned into edit plans and register writes.

use std::ops::Range;

use lucerna_editor::case::{Case, change_case};
use lucerna_editor::edit::replace_ranges;
use lucerna_editor::lines::{indent_lines, outdent_lines};
use lucerna_editor::{Ctx, EditKind, EditPlan, Goal, Selection, SelectionSet};
use lucerna_text::{BufferSnapshot, Point};

use crate::classify::line;
use crate::motion;
use crate::operator::{MotionKind, OpRange, Operator, row_start};
use crate::register::WriteKind;
use crate::visual::line_end;
use crate::{Action, InsertAt, Mode, RegContent, RegKind, Settings, ViewInfo, VimGlobals};

/// [`apply_operator`] on the view's text, or `None` when the view is
/// read-only and `op` edits; a yank still works there.
pub(crate) fn run_operator(
    op: Operator,
    r: OpRange,
    view: &dyn ViewInfo,
    g: &mut VimGlobals,
    s: &Settings,
    register: Option<char>,
) -> Option<(Vec<Action>, Point, Mode)> {
    match view.edit_ctx() {
        Some(ctx) => Some(apply_operator(op, r, ctx, view, g, s, register)),
        None if op == Operator::Yank => {
            let snap = view.snapshot();
            let actions = write(&r, snap, g, s, register, WriteKind::Yank);
            let start = snap.offset_to_point(r.range.start);
            Some((actions.into_iter().collect(), start, Mode::Normal))
        }
        None => None,
    }
}

/// Applies `op` to `r`: edits, register writes, new cursor, next mode.
///
/// The cursor is in post-edit coordinates. `Change` ends in Insert with
/// [`Action::EnterInsert`] after the edit; a yank's cursor is the range
/// start (the caller keeps the cursor when the motion went forward).
pub(crate) fn apply_operator(
    op: Operator,
    r: OpRange,
    ctx: Ctx,
    _view: &dyn ViewInfo,
    g: &mut VimGlobals,
    s: &Settings,
    register: Option<char>,
) -> (Vec<Action>, Point, Mode) {
    let snap = ctx.snapshot;
    let start = snap.offset_to_point(r.range.start);
    let (first, last) = (start.row, snap.offset_to_point(r.range.end).row);
    let linewise = r.kind == MotionKind::Linewise;
    let mut actions: Vec<Action> = Vec::new();
    let edit = |actions: &mut Vec<Action>, plan: EditPlan| {
        if !plan.edits.is_empty() {
            actions.push(Action::Edit(plan));
        }
    };
    let delete = |del: Range<usize>, cursor: usize| {
        replace_ranges(ctx, vec![(del, String::new())], cursor, EditKind::Other)
    };
    match op {
        Operator::Yank => {
            actions.extend(write(&r, snap, g, s, register, WriteKind::Yank));
            (actions, start, Mode::Normal)
        }
        Operator::Delete => {
            actions.extend(write(&r, snap, g, s, register, WriteKind::Delete));
            let (del, cursor) = if linewise {
                linewise_delete(snap, first, last)
            } else {
                (r.range.clone(), charwise_cursor(snap, &r.range))
            };
            // The cursor's offset is the same before and after the edit
            // unless the deletion was above it.
            let pre = snap.point_to_offset(cursor);
            let post = if pre >= del.end { pre - del.len() } else { pre };
            let cursor = Point {
                row: cursor.row.min(first),
                column: cursor.column,
            };
            if !del.is_empty() {
                edit(&mut actions, delete(del, post));
            }
            (actions, cursor, Mode::Normal)
        }
        Operator::Change => {
            actions.extend(write(&r, snap, g, s, register, WriteKind::Delete));
            let (del, cursor) = if linewise {
                // Keep the indent and the final line break.
                let indent = leading_whitespace(&line(snap, first));
                let from = row_start(snap, first) + indent as usize;
                (
                    from..line_end(snap, last),
                    Point {
                        row: first,
                        column: indent,
                    },
                )
            } else {
                (r.range.clone(), start)
            };
            if !del.is_empty() {
                let at = snap.point_to_offset(cursor);
                edit(&mut actions, delete(del, at));
            }
            actions.push(Action::EnterInsert(InsertAt::Before));
            actions.push(Action::ModeChanged(Mode::Insert));
            (actions, cursor, Mode::Insert)
        }
        Operator::Indent | Operator::Outdent => {
            let at = row_start(snap, first);
            let sel = Selection {
                id: 0,
                anchor: at,
                head: line_end(snap, last),
                goal: Goal::None,
            };
            let set = SelectionSet::from_selections(vec![sel], 0, false);
            let mut plan = if op == Operator::Indent {
                indent_lines(ctx, &set)
            } else {
                outdent_lines(ctx, &set)
            };
            // The first non-blank of the first row, after the shift.
            let text = line(snap, first);
            let old = leading_whitespace(&text);
            let new = plan
                .edits
                .iter()
                .find(|(range, _)| range.start == at)
                .map_or(old, |(_, t)| t.len() as u32);
            let column = if text.len() as u32 == old {
                new.saturating_sub(1)
            } else {
                new
            };
            plan.selections = SelectionSet::cursor(at + column as usize);
            edit(&mut actions, plan);
            (actions, Point { row: first, column }, Mode::Normal)
        }
        Operator::Lower | Operator::Upper | Operator::ToggleCase => {
            let case = match op {
                Operator::Lower => Case::Lower,
                Operator::Upper => Case::Upper,
                _ => Case::Toggle,
            };
            let plan = change_case(ctx, std::slice::from_ref(&r.range), case, r.range.start);
            edit(&mut actions, plan);
            (actions, start, Mode::Normal)
        }
    }
}

/// Writes the text of `r` to `register`; the clipboard write that is due.
fn write(
    r: &OpRange,
    snap: &BufferSnapshot,
    g: &mut VimGlobals,
    s: &Settings,
    register: Option<char>,
    kind: WriteKind,
) -> Option<Action> {
    let mut text = snap.text_for_range(r.range.clone());
    let kind_reg = if r.kind == MotionKind::Linewise {
        text.push('\n');
        RegKind::Linewise
    } else {
        RegKind::Charwise
    };
    let content = RegContent {
        text,
        kind: kind_reg,
    };
    g.write_register(register, content, kind, s.use_system_clipboard)
}

/// What deleting rows `first..=last` removes, and where the cursor goes
/// (in pre-edit coordinates): the first non-blank of the row that takes
/// their place. The last rows of the buffer take the line break before
/// them.
fn linewise_delete(snap: &BufferSnapshot, first: u32, last: u32) -> (Range<usize>, Point) {
    let fnb = |row| Point {
        row,
        column: motion::first_non_blank(&line(snap, row)),
    };
    if last + 1 < snap.line_count() {
        (
            row_start(snap, first)..row_start(snap, last + 1),
            fnb(last + 1),
        )
    } else if first > 0 {
        (line_end(snap, first - 1)..snap.len_bytes(), fnb(first - 1))
    } else {
        (0..snap.len_bytes(), Point { row: 0, column: 0 })
    }
}

/// The cursor after deleting `range`: its start, or the char before it
/// when nothing is left after it on the line.
fn charwise_cursor(snap: &BufferSnapshot, range: &Range<usize>) -> Point {
    let start = snap.offset_to_point(range.start);
    let end_row = snap.offset_to_point(range.end).row;
    if range.end == line_end(snap, end_row) {
        let column = motion::left(&line(snap, start.row), start.column, 1);
        return Point {
            row: start.row,
            column,
        };
    }
    start
}

/// The byte length of `text`'s leading spaces and tabs.
fn leading_whitespace(text: &str) -> u32 {
    (text.len() - text.trim_start_matches([' ', '\t']).len()) as u32
}
