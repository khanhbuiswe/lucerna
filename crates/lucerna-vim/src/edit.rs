//! Operators turned into edit plans and register writes.

use std::ops::Range;

use lucerna_editor::case::{Case, change_case};
use lucerna_editor::edit::replace_ranges;
use lucerna_editor::{Ctx, EditKind, EditPlan, SelectionSet};
use lucerna_text::{BufferSnapshot, Point};

use unicode_segmentation::UnicodeSegmentation as _;

use crate::classify::{last_char_col, line};
use crate::motion;
use crate::operator::{MotionKind, OpRange, Operator, row_start};
use crate::register::WriteKind;
use crate::visual::{self, line_end};
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
            let mut actions: Vec<Action> = write(&r, snap, g, s, register, WriteKind::Yank)
                .into_iter()
                .collect();
            actions.extend(yanked(&r, snap, s));
            let start = snap.offset_to_point(r.range.start);
            Some((actions, start, Mode::Normal))
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
            actions.extend(yanked(&r, snap, s));
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
            match shift_rows(ctx, first, last, op == Operator::Outdent, 1) {
                Some((plan, cursor)) => {
                    edit(&mut actions, plan);
                    (actions, cursor, Mode::Normal)
                }
                None => (actions, start, Mode::Normal),
            }
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

/// The [`Action::Yanked`] for a yank of `r`, when highlighting is on.
pub(crate) fn yanked(r: &OpRange, snap: &BufferSnapshot, s: &Settings) -> Option<Action> {
    if !s.highlighted_yank {
        return None;
    }
    let mut range = r.range.clone();
    if r.kind == MotionKind::Linewise {
        let last = snap.offset_to_point(range.end).row;
        range.end = visual::next_row_start(snap, last);
    }
    (!range.is_empty()).then(|| Action::Yanked(vec![range]))
}

/// Writes the text of `r` to `register`; the clipboard write that is due.
pub(crate) fn write(
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

/// `>` and `<`: shifts rows `first..=last` by `levels` indent levels, as
/// the editor's indent and outdent commands do once per level (each
/// non-empty row snaps to the next or previous tab stop, written in the
/// configured style; an empty row is indented only when it is the only
/// row), computed in one pass so a huge count stays linear. The cursor
/// goes to the first non-blank of the first row (onto the last char of
/// indent when the row is blank).
///
/// `None`, with no edit, when a row's new indent would pass
/// [`MAX_INDENT_COLUMNS`] or all of them [`MAX_EDIT_BYTES`]
/// (`ggVG99999>`).
pub(crate) fn shift_rows(
    ctx: Ctx,
    first: u32,
    last: u32,
    outdent: bool,
    levels: u32,
) -> Option<(EditPlan, Point)> {
    let snap = ctx.snapshot;
    let ts = ctx.config.tab_size.max(1);
    let levels = levels.max(1);
    let indent = |cols: u32| {
        if ctx.config.insert_spaces {
            " ".repeat(cols as usize)
        } else {
            "\t".repeat((cols / ts) as usize) + &" ".repeat((cols % ts) as usize)
        }
    };
    let mut edits = Vec::new();
    let mut first_lead = None;
    let mut total = 0usize;
    for row in first..=last {
        let text = line(snap, row);
        let lead = &text[..leading_whitespace(&text) as usize];
        let vis = lead.chars().fold(0u32, |col, c| {
            if c == '\t' {
                (col / ts + 1) * ts
            } else {
                col + 1
            }
        });
        let cols = if outdent {
            (vis.div_ceil(ts).saturating_sub(levels)).saturating_mul(ts)
        } else if text.is_empty() && first != last {
            0
        } else {
            (vis / ts).saturating_add(levels).saturating_mul(ts)
        };
        total = total.saturating_add(cols as usize);
        if cols > MAX_INDENT_COLUMNS || total > MAX_EDIT_BYTES {
            return None;
        }
        let new = if outdent && vis == 0 {
            lead.to_string()
        } else {
            indent(cols)
        };
        if row == first {
            first_lead = Some((text.len() as u32, lead.len() as u32, new.len() as u32));
        }
        if new != lead {
            let start = row_start(snap, row);
            edits.push((start..start + lead.len(), new));
        }
    }
    let (len, old, new) = first_lead.unwrap_or((0, 0, 0));
    let column = if len == old {
        new.saturating_sub(1)
    } else {
        new
    };
    let at = row_start(snap, first);
    let plan = replace_ranges(ctx, edits, at + column as usize, EditKind::Other);
    Some((plan, Point { row: first, column }))
}

/// The most bytes one command may insert; a count past it (`99999p` of a
/// big register) does nothing rather than run out of memory.
const MAX_EDIT_BYTES: usize = 64 << 20;

/// The widest indent `>` may give a row, in columns.
const MAX_INDENT_COLUMNS: u32 = 10_000;

/// `n` copies of `text` (plus the line break a linewise put may add to
/// each) would pass [`MAX_EDIT_BYTES`].
fn too_many_copies(text: &str, n: usize) -> bool {
    (text.len() + 1).saturating_mul(n) > MAX_EDIT_BYTES
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
    } else if visual::phantom_row(snap) == Some(last + 1) {
        // The last real rows of a buffer that ends in a line break: the
        // break before them stays, and the cursor goes to the new last
        // real row rather than the empty row after it.
        let cursor = if first > 0 {
            fnb(first - 1)
        } else {
            Point { row: 0, column: 0 }
        };
        (row_start(snap, first)..snap.len_bytes(), cursor)
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

/// `r{c}`: replaces `n` chars from `at` with `ch`, or with one line break
/// for `\n`. `None` when fewer than `n` chars are left on the line. The
/// cursor is on the last replaced char (the start of the next line for a
/// line break).
pub(crate) fn replace_chars(ctx: Ctx, at: Point, ch: char, n: u32) -> Option<(EditPlan, Point)> {
    let snap = ctx.snapshot;
    let text = line(snap, at.row);
    let col = at.column as usize;
    let rest = text.get(col..)?;
    let chars: Vec<char> = rest.chars().take(n as usize).collect();
    if chars.len() < n as usize || n == 0 {
        return None;
    }
    let len: usize = chars.iter().map(|c| c.len_utf8()).sum();
    let start = snap.point_to_offset(at);
    let (new, cursor) = if ch == '\n' {
        let cursor = Point {
            row: at.row + 1,
            column: 0,
        };
        ("\n".to_string(), cursor)
    } else {
        let last = (n as usize - 1) * ch.len_utf8();
        let cursor = Point {
            row: at.row,
            column: at.column + last as u32,
        };
        (ch.to_string().repeat(n as usize), cursor)
    };
    let post = start
        + if ch == '\n' {
            1
        } else {
            new.len() - ch.len_utf8()
        };
    let plan = replace_ranges(ctx, vec![(start..start + len, new)], post, EditKind::Other);
    Some((plan, cursor))
}

/// `~` (without `tildeop`): toggles the case of `n` chars from `at`, not
/// past the line end. The cursor goes right past them, staying on the
/// line. `None` on an empty line.
pub(crate) fn toggle_chars(ctx: Ctx, at: Point, n: u32) -> Option<(EditPlan, Point)> {
    let snap = ctx.snapshot;
    let text = line(snap, at.row);
    let col = at.column as usize;
    let len: usize = text
        .get(col..)?
        .chars()
        .take(n as usize)
        .map(char::len_utf8)
        .sum();
    if len == 0 {
        return None;
    }
    let start = snap.point_to_offset(at);
    let range = start..start + len;
    let mut plan = change_case(ctx, std::slice::from_ref(&range), Case::Toggle, start);
    let new = plan
        .edits
        .first()
        .map_or_else(|| text[col..col + len].to_string(), |(_, t)| t.to_string());
    let new_line = format!("{}{new}{}", &text[..col], &text[col + len..]);
    let column = ((col + new.len()) as u32).min(last_char_col(&new_line));
    plan.selections = SelectionSet::cursor(start - col + column as usize);
    Some((
        plan,
        Point {
            row: at.row,
            column,
        },
    ))
}

/// `J` (`spaces`) and `gJ`: joins `count` lines (at least two) from
/// `row`. `J` drops the next line's indent and puts one space between
/// (two after `.!?` with `joinspaces`), none when the text so far is empty
/// or ends in whitespace, the next line is blank or starts with `)`. `gJ`
/// keeps everything and adds nothing. The cursor goes to the last join.
/// `None` on the last line.
pub(crate) fn join_lines(
    ctx: Ctx,
    row: u32,
    count: u32,
    spaces: bool,
    joinspaces: bool,
) -> Option<(EditPlan, Point)> {
    let snap = ctx.snapshot;
    let last = row
        .saturating_add(count.max(2) - 1)
        .min(snap.line_count().saturating_sub(1));
    if last <= row {
        return None;
    }
    let first_line = snap.text_for_range(row_start(snap, row)..line_end(snap, row));
    let mut len = first_line.len();
    let mut tail = first_line.chars().next_back();
    let mut cursor = len;
    let mut edits = Vec::new();
    for r in row + 1..=last {
        let start = row_start(snap, r);
        let text = snap.text_for_range(start..line_end(snap, r));
        let skip = if spaces {
            leading_whitespace(&text) as usize
        } else {
            0
        };
        let content = &text[skip..];
        let sep = if !spaces
            || content.is_empty()
            || tail.is_none_or(char::is_whitespace)
            || content.starts_with(')')
        {
            ""
        } else if joinspaces && matches!(tail, Some('.' | '!' | '?')) {
            "  "
        } else {
            " "
        };
        edits.push((line_end(snap, r - 1)..start + skip, sep.to_string()));
        cursor = len;
        len += sep.len() + content.len();
        tail = content.chars().next_back().or(tail);
    }
    let at = row_start(snap, row);
    let plan = replace_ranges(ctx, edits, at + cursor, EditKind::Other);
    Some((
        plan,
        Point {
            row,
            column: cursor as u32,
        },
    ))
}

/// Visual `r{c}`: replaces every char of `range` but the line breaks with
/// `ch`. The cursor stays at the range start. `None` when nothing changes.
pub(crate) fn replace_selection(
    ctx: Ctx,
    range: Range<usize>,
    ch: char,
) -> Option<(EditPlan, Point)> {
    let snap = ctx.snapshot;
    let text = snap.text_for_range(range.clone());
    let mut edits = Vec::new();
    let mut at = range.start;
    for part in text.split_inclusive('\n') {
        let body = part.trim_end_matches(['\n', '\r']);
        let new: String = body.graphemes(true).map(|_| ch).collect();
        if new != body {
            edits.push((at..at + body.len(), new));
        }
        at += part.len();
    }
    if edits.is_empty() {
        return None;
    }
    let plan = replace_ranges(ctx, edits, range.start, EditKind::Other);
    Some((plan, snap.offset_to_point(range.start)))
}

/// How `p` and its variants put text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Put {
    /// `P`/`gP`: before the cursor (above the line for lines).
    pub before: bool,
    /// `gp`/`gP`: the cursor goes just after the pasted text.
    pub after: bool,
}

/// `p P gp gP`: puts `n` copies of `content` at `at` (spec §6.3). The
/// cursor: charwise one-line on the last pasted char, multi-line at the
/// start; linewise on the first non-blank of the first pasted line; with
/// `after`, just past the text (column 0 of the next line for lines).
pub(crate) fn put(
    ctx: Ctx,
    at: Point,
    content: &RegContent,
    n: u32,
    how: Put,
) -> Option<(EditPlan, Point)> {
    let snap = ctx.snapshot;
    let n = n.max(1) as usize;
    if content.text.is_empty() || too_many_copies(&content.text, n) {
        return None;
    }
    let (offset, text, cursor_in) = match content.kind {
        RegKind::Charwise => {
            let here = snap.point_to_offset(at);
            let offset = if how.before {
                here
            } else {
                let text = line(snap, at.row);
                here + text
                    .get(at.column as usize..)
                    .and_then(|rest| rest.graphemes(true).next())
                    .map_or(0, str::len)
            };
            let text = content.text.repeat(n);
            let cursor_in = if how.after {
                text.len()
            } else if text.contains('\n') {
                0
            } else {
                text.len() - text.chars().next_back().map_or(0, char::len_utf8)
            };
            (offset, text, cursor_in)
        }
        RegKind::Linewise => {
            let mut body = content.text.clone();
            if !body.ends_with('\n') {
                body.push('\n');
            }
            let body = body.repeat(n);
            let fnb = motion::first_non_blank(body.lines().next().unwrap_or("")) as usize;
            if how.before || at.row + 1 < snap.line_count() {
                let row = if how.before { at.row } else { at.row + 1 };
                let cursor_in = if how.after { body.len() } else { fnb };
                (row_start(snap, row), body, cursor_in)
            } else {
                // The last line has no line break to paste after: add one
                // before the text instead of leaving a blank line after it.
                let text = format!("\n{}", &body[..body.len() - 1]);
                let cursor_in = if how.after {
                    text.rfind('\n').map_or(0, |i| i + 1)
                } else {
                    1 + fnb
                };
                (line_end(snap, at.row), text, cursor_in)
            }
        }
    };
    let cursor = advance(snap.offset_to_point(offset), &text[..cursor_in]);
    let plan = replace_ranges(
        ctx,
        vec![(offset..offset, text)],
        offset + cursor_in,
        EditKind::Other,
    );
    Some((plan, cursor))
}

/// Visual `p`/`P`: puts `n` copies of `content` in place of `sel`. Lines
/// over a charwise selection go on lines of their own; charwise text over
/// lines takes their place as a line. The cursor follows `p` (§6.3): the
/// first non-blank of the first pasted line for lines, else the last
/// pasted char, or the start of multi-line text.
pub(crate) fn put_over(
    ctx: Ctx,
    sel: &OpRange,
    content: &RegContent,
    n: u32,
) -> Option<(EditPlan, Point)> {
    let snap = ctx.snapshot;
    let n = n.max(1) as usize;
    if content.text.is_empty() || too_many_copies(&content.text, n) {
        return None;
    }
    let lines = || {
        let mut body = content.text.clone();
        if !body.ends_with('\n') {
            body.push('\n');
        }
        body.repeat(n)
    };
    let fnb = |body: &str| motion::first_non_blank(body.lines().next().unwrap_or("")) as usize;
    let charwise_cursor = |text: &str| {
        if text.contains('\n') {
            0
        } else {
            text.len() - text.chars().next_back().map_or(0, char::len_utf8)
        }
    };
    let (del, text, cursor_in) = if sel.kind == MotionKind::Linewise {
        let first = snap.offset_to_point(sel.range.start).row;
        let last = snap.offset_to_point(sel.range.end).row;
        let del = row_start(snap, first)..visual::next_row_start(snap, last);
        let (mut body, cursor_in) = match content.kind {
            RegKind::Linewise => {
                let body = lines();
                let cursor_in = fnb(&body);
                (body, cursor_in)
            }
            RegKind::Charwise => {
                let text = content.text.repeat(n);
                let cursor_in = charwise_cursor(&text);
                (text + "\n", cursor_in)
            }
        };
        // The last row of a buffer without a final line break.
        if del.end == sel.range.end {
            body.pop();
        }
        (del, body, cursor_in)
    } else {
        match content.kind {
            RegKind::Charwise => {
                let text = content.text.repeat(n);
                let cursor_in = charwise_cursor(&text);
                (sel.range.clone(), text, cursor_in)
            }
            RegKind::Linewise => {
                let body = lines();
                let cursor_in = 1 + fnb(&body);
                (sel.range.clone(), format!("\n{body}"), cursor_in)
            }
        }
    };
    let start = snap.offset_to_point(del.start);
    let cursor = advance(start, &text[..cursor_in]);
    let plan = replace_ranges(
        ctx,
        vec![(del.clone(), text)],
        del.start + cursor_in,
        EditKind::Other,
    );
    Some((plan, cursor))
}

/// `p` moved past `text`.
fn advance(p: Point, text: &str) -> Point {
    match text.rfind('\n') {
        Some(i) => Point {
            row: p.row + text.matches('\n').count() as u32,
            column: (text.len() - i - 1) as u32,
        },
        None => Point {
            row: p.row,
            column: p.column + text.len() as u32,
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::test_util::{assert_edit, assert_edit_with, run_edit, run_edit_with, run_keys};
    use crate::{Action, InsertAt, Mode, Settings};
    use std::ops::Range;

    // Adapted from VSCodeVim test/mode/normalModeTests/commands.test.ts: "Can handle 'D' with count 3"
    #[test]
    fn counted_d_capital() {
        assert_edit("o|ne\ntwo\nthree\nfour", "3D", "|o\nfour");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/commands.test.ts: "Can handle '<Count>r'"
    #[test]
    fn r_with_count() {
        assert_edit("123|456\n789", "2ra", "123a|a6\n789");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/commands.test.ts: "Can handle '<Count>r'"
    #[test]
    fn r_count_past_line_end_does_nothing() {
        assert_edit("123|456\n789", "4ra", "123|456\n789");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/commands.test.ts: "Can handle '<Count>r\n'"
    #[test]
    fn r_enter_with_count() {
        assert_edit("abc|defg\n12345", "3r<CR>", "abc\n|g\n12345");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/commands.test.ts: "Can handle 'Nx' and paste"
    #[test]
    fn x_then_paste_on_new_line() {
        assert_edit("t|ext", "2xo<Esc>p", "tt\ne|x");
        assert_edit("t|ext\nend", "2xo<Esc>p", "tt\ne|x\nend");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/commands.test.ts: "'x' with count does not go over EOL"
    #[test]
    fn x_count_stops_at_line_end() {
        assert_edit("one t|wo\nthree four", "1000x", "one |t\nthree four");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/dot.test.ts: "Can repeat '~' with <num>"
    #[test]
    fn tilde_with_count() {
        assert_edit("|teXt", "4~", "TEx|T");
    }

    // Adapted from VSCodeVim test/mode/modeNormal.test.ts: "Can do S"
    #[test]
    fn s_capital_with_count() {
        assert_edit(" one\n tw|o\n three", "2S", " one\n |");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/commands.test.ts: "Can handle 'J' with count"
    #[test]
    fn j_capital_with_count() {
        assert_edit("|one\ntwo\nthree\nfour", "3J", "one two| three\nfour");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/commands.test.ts: "Can handle 'J' with line ending with '.'"
    #[test]
    fn j_capital_joinspaces() {
        assert_edit("one.\ntw|o", "kJ", "one.|  two");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/commands.test.ts: "Can handle 'J' with ')' first character on next line"
    #[test]
    fn j_capital_before_close_paren() {
        assert_edit("one(\n)tw|o", "kJ", "one(|)two");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/commands.test.ts: "Can handle 'J' with only white space on next line"
    #[test]
    fn j_capital_whitespace_only_next_line() {
        assert_edit("on|e\n ", "J", "on|e");
    }

    // Adapted from VSCodeVim test/mode/normalModeTests/commands.test.ts: "Can handle 'gJ' with count when line has whitespaces"
    #[test]
    fn g_j_capital_with_count() {
        assert_edit("|one\n two \nthree\nfour", "3gJ", "one two |three\nfour");
    }

    #[test]
    fn j_capital_without_joinspaces() {
        let s = Settings {
            joinspaces: false,
            ..Settings::default()
        };
        assert_edit_with(s, |_| {}, "one.\ntw|o", "kJ", "one.| two");
    }

    #[test]
    fn tildeop_makes_tilde_an_operator() {
        let s = Settings {
            tildeop: true,
            ..Settings::default()
        };
        assert_edit_with(s, |_| {}, "|ab cd", "~w", "|AB cd");
    }

    // Adapted from VSCodeVim test/register/register.test.ts: "Register '1'-'9' stores delete content"
    #[test]
    fn numbered_registers_put() {
        assert_edit(
            "|test1\ntest2\ntest3\n",
            "dddddd\"1p\"2p\"3p",
            "\ntest3\ntest2\n|test1",
        );
    }

    // Adapted from VSCodeVim test/register/register.test.ts: "`x` sets small delete register"
    #[test]
    fn small_delete_register_put() {
        assert_edit("|test1\ntest2\ntest3", "2xj\"-p", "st1\ntt|eest2\ntest3");
    }

    // Adapted from VSCodeVim test/register/register.test.ts: "Yank stores text in Register '0'"
    #[test]
    fn yank_register_put() {
        assert_edit(
            "|test1\ntest2\ntest3",
            "yyjyyggdd\"0P",
            "|test2\ntest2\ntest3",
        );
    }

    // Adapted from VSCodeVim test/register/register.test.ts: "Can use black hole register"
    #[test]
    fn black_hole_keeps_unnamed() {
        assert_edit("|asdf\nqwer", "yyj\"_ddkp", "asdf\n|asdf");
    }

    // Adapted from VSCodeVim test/operator/put.test.ts: "Yank character-wise, <count>p"
    #[test]
    fn p_charwise_with_count() {
        assert_edit("|XYZone two three", "d3lw2p", "one tXYZXY|Zwo three");
    }

    // Adapted from VSCodeVim test/operator/put.test.ts: "Yank line-wise, <count>gp"
    #[test]
    fn gp_linewise_with_count() {
        assert_edit(
            "|abc\none\ntwo\nthree",
            "ddj2gp",
            "one\ntwo\nabc\nabc\n|three",
        );
    }

    fn no_edit(start: &str, keys: &str) {
        let (end, actions) = run_edit(start, keys);
        assert_eq!(end.replace('|', ""), start.replace('|', ""), "{keys}");
        assert!(
            !actions.iter().any(|a| matches!(a, Action::Edit(_))),
            "{keys} edited"
        );
    }

    #[test]
    fn put_past_the_size_cap_does_nothing() {
        // 1001 bytes x 99_999 copies is over 64 MiB.
        let start = format!("|{}\nb", "x".repeat(1000));
        no_edit(&start, "yy99999p");
        no_edit(&start, "y$99999P");
        no_edit(&start, "yyjV99999p");
        // Under the cap the copies go in.
        let (end, _) = run_edit(&start, "yy9p");
        assert_eq!(end.matches('x').count(), 10 * 1000);
    }

    #[test]
    fn shift_past_the_size_cap_does_nothing() {
        let start = format!("|{}", "a\n".repeat(20));
        // 99_999 levels is over 10_000 columns on one row.
        no_edit(&start, "ggVG99999>");
        // 2_500 levels of 4 columns over 7_000 rows is over 64 MiB.
        let start = format!("|{}", "a\n".repeat(7000));
        no_edit(&start, "ggVG2500>");
        // Under the caps it shifts.
        let (end, _) = run_edit("|a\nb", "ggVG3>");
        assert_eq!(end, "            |a\n            b");
    }

    #[test]
    fn p_linewise_on_last_line_without_newline() {
        assert_edit("a\n|b", "yyp", "a\nb\n|b");
    }

    #[test]
    fn p_charwise_multiline_puts_cursor_at_start() {
        assert_edit("|ab\ncd", "vjyGp", "ab\nc|ab\ncd");
    }

    #[test]
    fn capital_p_linewise_cursor_on_first_non_blank() {
        assert_edit("  |ab\ncd", "yyjP", "  ab\n  |ab\ncd");
    }

    #[test]
    fn g_capital_p_charwise_cursor_after_paste() {
        assert_edit("|ab cd", "yiwwgP", "ab ab|cd");
    }

    #[test]
    fn x_and_capital_x_write_small_delete() {
        assert_edit("ab|cd", "X$p", "acd|b");
        assert_edit("ab|cd", "x$p", "abd|c");
        assert_edit("|abc", "X", "|abc");
    }

    #[test]
    fn s_and_c_capital_enter_insert() {
        let (text, actions) = run_edit("a|bcd", "2s");
        assert_eq!(text, "a|d");
        assert!(matches!(
            actions.as_slice(),
            [
                Action::Edit(_),
                Action::EnterInsert(InsertAt::Before),
                Action::ModeChanged(Mode::Insert)
            ]
        ));
        let (text, _) = run_edit("a|bcd", "C");
        assert_eq!(text, "a|");
        let (text, _) = run_edit("|", "s");
        assert_eq!(text, "|");
    }

    #[test]
    fn y_capital_yanks_the_line() {
        assert_edit("a|b\ncd", "Yjp", "ab\ncd\n|ab");
    }

    #[test]
    fn paste_from_an_empty_register_does_nothing() {
        assert_edit("a|b", "\"qp", "a|b");
    }

    // On the last line there is no next line to land on: `gp` leaves the
    // cursor at the start of the last pasted line.
    #[test]
    fn gp_linewise_on_last_line_cursor_on_last_pasted_line() {
        assert_edit("a\n|b", "yygp", "a\nb\n|b");
        assert_edit("a\n|b", "yy2gp", "a\nb\nb\n|b");
    }

    #[test]
    fn readonly_s_and_c_stay_in_normal() {
        for keys in ["s", "S", "C", "2s"] {
            let (_, actions) =
                run_edit_with(Settings::default(), |v| v.readonly = true, "a|b\ncd", keys);
            assert!(
                !actions.iter().any(|a| matches!(
                    a,
                    Action::EnterInsert(_) | Action::ModeChanged(Mode::Insert) | Action::Edit(_)
                )),
                "{keys}: {actions:?}"
            );
        }
    }

    // Adapted from VSCodeVim test/mode/modeVisualLine.test.ts: "Vp updates register content"
    #[test]
    fn visual_line_p_updates_register() {
        assert_edit("|hello\nworld", "ddVpP", "|world\nhello");
    }

    // Adapted from VSCodeVim test/mode/modeVisualLine.test.ts: "Vp places the cursor on first non-whitespace character on line"
    #[test]
    fn visual_line_p_cursor_on_first_non_blank() {
        assert_edit("begin\n| middle\nend", "yyjVp", "begin\n middle\n |middle");
    }

    #[test]
    fn visual_d_charwise_across_lines() {
        assert_edit("a|bc\ndef", "vjd", "a|f");
        assert_edit("a|bc\ndef", "vjx", "a|f");
        // The deleted text is in `"`.
        assert_edit("a|bc\ndef", "vjd$p", "af|bc\nde");
    }

    #[test]
    fn visual_line_d_puts_the_cursor_on_the_next_line() {
        assert_edit("a\n|b\nc\n  d", "Vjd", "a\n  |d");
    }

    #[test]
    fn visual_capital_d_and_x_delete_lines() {
        assert_edit("a\nb|c\nde\nf", "vjD", "a\n|f");
        assert_edit("a\nb|c\nde\nf", "vX", "a\n|de\nf");
    }

    #[test]
    fn visual_line_indent_two_lines() {
        assert_edit("|a\nb\nc", "Vj>", "    |a\n    b\nc");
        assert_edit("    |a\n    b\nc", "Vj<lt>", "|a\nb\nc");
    }

    #[test]
    fn visual_indent_with_a_count_shifts_that_many_levels() {
        assert_edit("|a\nb\nc", "vj2>", "        |a\n        b\nc");
        assert_edit("          |a\nb", "V3<lt>", "|a\nb");
    }

    #[test]
    fn visual_case_operators() {
        assert_edit("|ab cd", "veU", "|AB cd");
        assert_edit("|AB cd", "veu", "|ab cd");
        assert_edit("a|Bc\ndE", "vj~", "a|bC\nDe");
        assert_edit("ab\n|cd", "VU", "ab\n|CD");
    }

    #[test]
    fn visual_r_replaces_every_char_but_line_breaks() {
        assert_edit("|abc", "vlr*", "|**c");
        assert_edit("a|bc\nde", "vjr-", "a|--\n--");
        assert_edit("ab\n|cd\nef", "Vjrx", "ab\n|xx\nxx");
    }

    #[test]
    fn visual_join() {
        assert_edit("|a\nb\nc", "VjJ", "a| b\nc");
        assert_edit("|a\n b\nc", "vjgJ", "a| b\nc");
        // One row joins with the next.
        assert_edit("|a\nb\nc", "VJ", "a| b\nc");
    }

    #[test]
    fn visual_change_enters_insert() {
        let (text, actions) = run_edit("a|bc d", "vec");
        assert_eq!(text, "a| d");
        assert!(matches!(
            actions[actions.len() - 3..],
            [
                Action::Edit(_),
                Action::EnterInsert(InsertAt::Before),
                Action::ModeChanged(Mode::Insert)
            ]
        ));
        assert_edit("a|bc d", "vls", "a| d");
        // Linewise: the indent stays.
        assert_edit("ab\n  c|d\nef", "vS", "ab\n  |\nef");
        assert_edit("ab\n  c|d\nef", "vjC", "ab\n  |");
        assert_edit("ab\n  c|d\nef", "vR", "ab\n  |\nef");
        assert_edit("ab\n  c|d\nef", "Vc", "ab\n  |\nef");
    }

    #[test]
    fn visual_capital_y_yanks_lines() {
        assert_edit("a|b\ncd", "vYjp", "ab\ncd\n|ab");
    }

    #[test]
    fn visual_p_writes_the_old_selection_and_capital_p_does_not() {
        assert_edit("|ab cd", "yiwwvep0P", "c|dab ab");
        assert_edit("|ab cd", "yiwwveP0P", "a|bab ab");
    }

    #[test]
    fn visual_p_mixed_kinds() {
        // Lines over a charwise selection go on lines of their own.
        assert_edit("|ab\ncdef", "yyjlvlp", "ab\nc\n|ab\nf");
        // Charwise text over lines takes their place.
        assert_edit("|ab\ncd", "yiwjVp", "ab\na|b");
        // A count repeats the text.
        assert_edit("|ab cd", "yiwwve2p", "ab aba|b");
    }

    #[test]
    fn visual_operators_update_the_marks() {
        assert_edit("|abcd", "vlU$`<lt>", "|ABcd");
        assert_edit("|abcd", "vlU$`>", "A|Bcd");
    }

    #[test]
    fn visual_block_operators_do_nothing() {
        let (view, before, _) = run_keys(Settings::default(), |_| {}, "|ab\ncd", "yl<C-v>j");
        let selection = before.selection(&view.snapshot, view.tab_size);
        assert!(selection.is_some());
        for key in ["d", "x", "c", ">", "U", "rx", "p", "D", "J", "Y", "~", "u"] {
            let keys = format!("yl<C-v>j{key}");
            let (view, vim, actions) = run_keys(Settings::default(), |_| {}, "|ab\ncd", &keys);
            assert!(
                !actions.iter().any(|a| matches!(a, Action::Edit(_))),
                "{keys}: {actions:?}"
            );
            assert_eq!(vim.mode(), Mode::VisualBlock, "{keys}");
            assert_eq!(
                vim.selection(&view.snapshot, view.tab_size),
                selection,
                "{keys}"
            );
        }
    }

    #[test]
    fn visual_dollar_takes_the_line_break() {
        assert_edit("|ab\ncd", "v$d", "|cd");
        let (text, actions) = run_edit("|ab\ncd", "v$c");
        assert_eq!(text, "|cd");
        assert!(actions.contains(&Action::ModeChanged(Mode::Insert)));
        // The last line has no line break: only its text goes, and the
        // cursor stays on the row that is left.
        assert_edit("ab\n|cd", "v$d", "ab\n|");
        assert_edit("ab\n|cd", "vld", "ab\n|");
    }

    #[test]
    fn visual_multibyte_last_char() {
        assert_edit("|aé b", "vld", "| b");
        assert_edit("|aé b", "vlU", "|AÉ b");
        assert_edit("|aé b", "vlr*", "|** b");
        assert_edit("|aé b", "vlc", "| b");
    }

    #[test]
    fn visual_backward_selection() {
        assert_edit("ab|cd", "vhd", "a|d");
        assert_edit("ab\nc|d", "vkd", "|a");
        assert_edit("ab|cd", "vhU", "a|BCd");
    }

    #[test]
    fn visual_on_the_phantom_row() {
        let (text, actions) = run_edit("ab\n|", "vd");
        assert_eq!(text, "ab\n|");
        assert!(!actions.iter().any(|a| matches!(a, Action::Edit(_))));
        let (text, actions) = run_edit("ab\n|", "vc");
        assert_eq!(text, "ab\n|");
        assert!(!actions.iter().any(|a| matches!(a, Action::Edit(_))));
        assert!(actions.contains(&Action::ModeChanged(Mode::Insert)));
        assert_edit("ab\n|", "Vd", "|ab");
    }

    #[test]
    fn visual_operators_use_the_register_prefix() {
        assert_edit("|ab cd", "v\"ad$\"ap", "b cd|a");
        // `"_` leaves `"` alone.
        assert_edit("|ab cd", "yiwwv\"_d0P", "a|bab d");
        assert_edit("|ab cd", "yiwwvd0P", "|cab d");
        // A named linewise register with a count over lines.
        assert_edit("|ab\ncd\nef", "\"ayyjV\"a2p", "ab\n|ab\nab\nef");
    }

    #[test]
    fn visual_shift_with_a_huge_count_is_direct() {
        // Past 10_000 columns the shift does nothing.
        no_edit("|a\nb\nc", "Vjj99999>");
        let (text, _) = run_edit("|a\nb\nc", "Vjj2500>");
        let width = 2_500 * 4;
        for (row, line) in text.replace('|', "").lines().enumerate() {
            assert_eq!(line.len(), width + 1, "row {row}");
            assert!(line[..width].bytes().all(|b| b == b' '));
        }
        assert_edit("    |a\n        b", "Vj99999<lt>", "|a\nb");
    }

    #[test]
    fn shift_keeps_the_editor_rules() {
        // Empty rows of several are skipped; a lone empty row is indented.
        assert_edit("|a\n\nb", "Vjj>", "    |a\n\n    b");
        assert_edit("a\n|\nb", "V2>", "a\n       | \nb");
        // Rows snap to the tab stops.
        assert_edit("  |a\n     b", "Vj>", "    |a\n        b");
        assert_edit("  |a\n     b", "Vj<lt>", "|a\n    b");
        let s = Settings::default();
        assert_edit_with(
            s,
            |v| v.config.insert_spaces = false,
            "|a\n  b",
            "Vj2>",
            "\t\t|a\n\t\tb",
        );
    }

    #[test]
    fn readonly_visual_operators_leave_visual_without_editing() {
        for keys in [
            "vd", "vx", "vD", "vc", "vs", "vS", "vR", "v>", "vU", "v~", "vrx", "vJ", "Vp", "VP",
            "vlc",
        ] {
            let (_, vim, actions) = run_keys(
                Settings::default(),
                |v| v.readonly = true,
                "a|b\ncd",
                &format!("yl{keys}"),
            );
            assert!(
                !actions.iter().any(|a| matches!(
                    a,
                    Action::Edit(_) | Action::EnterInsert(_) | Action::ModeChanged(Mode::Insert)
                )),
                "{keys}: {actions:?}"
            );
            assert_eq!(vim.mode(), Mode::Normal, "{keys}");
            // At the selection start: column 0 for whole lines.
            let column = if keys.starts_with('V') { 0 } else { 1 };
            assert_eq!(vim.cursor().column, column, "{keys}");
        }
        // `y` still yanks.
        let (_, vim, actions) =
            run_keys(Settings::default(), |v| v.readonly = true, "a|b\ncd", "vy");
        assert_eq!(vim.mode(), Mode::Normal);
        assert!(actions.contains(&Action::ModeChanged(Mode::Normal)));
    }

    #[test]
    fn readonly_view_does_not_edit() {
        for keys in [
            "x", "X", "s", "S", "C", "J", "gJ", "~", "rz", "r<CR>", "yyp", "yyP", "yygp", "D",
        ] {
            assert_edit_with(
                Settings::default(),
                |v| v.readonly = true,
                "a|b\ncd",
                keys,
                "a|b\ncd",
            );
        }
    }

    fn yanked(text: &str, keys: &str) -> Vec<Vec<Range<usize>>> {
        let settings = Settings {
            highlighted_yank: true,
            ..Settings::default()
        };
        let (_, actions) = run_edit_with(settings, |_| {}, text, keys);
        actions
            .into_iter()
            .filter_map(|a| match a {
                Action::Yanked(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn yanked_ranges_for_motions_lines_and_visual() {
        assert_eq!(yanked("one t|wo", "yiw"), vec![vec![4..7]]);
        assert_eq!(yanked("a\n|bc", "yy"), vec![vec![2..4]]);
        assert_eq!(yanked("|a\nbc", "Y"), vec![vec![0..2]]);
        assert_eq!(yanked("|a\nbc", "2yy"), vec![vec![0..4]]);
        assert_eq!(yanked("|abc def", "vey"), vec![vec![0..3]]);
        assert_eq!(yanked("|ab\ncd", "Vjy"), vec![vec![0..5]]);
        assert_eq!(yanked("|abc\ndef", "<C-v>jly"), vec![vec![0..2, 4..6]]);
        assert_eq!(yanked("|one\ntwo", ":y<CR>"), vec![vec![0..4]]);
    }

    #[test]
    fn yanked_is_off_by_default_and_never_for_delete() {
        let (_, _, actions) =
            crate::test_util::run_keys(Settings::default(), |_| {}, "one t|wo", "yiw");
        assert!(!actions.iter().any(|a| matches!(a, Action::Yanked(_))));
        assert_eq!(yanked("one t|wo", "diw"), Vec::<Vec<Range<usize>>>::new());
        assert_eq!(yanked("a\n|bc", "dd"), Vec::<Vec<Range<usize>>>::new());
    }
}
