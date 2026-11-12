//! Toggle line and block comments.
//!
//! Line commands decide per selection, over the rows it touches: comment
//! all of them when any non-blank row lacks the token, else uncomment;
//! the token goes at the minimum indent and blank rows are skipped. Block
//! comments wrap or unwrap the selected text.
//!
//! Adapted from microsoft/vscode src/vs/editor/contrib/comment/browser/lineCommentCommand.ts (MIT)
//! Adapted from microsoft/vscode src/vs/editor/contrib/comment/browser/blockCommentCommand.ts (MIT)

use std::ops::Range;
use std::sync::Arc;

use lucerna_text::{Bias, Patch};

use crate::commands::{leading_whitespace, mk_patch, row_start, touched_rows};
use crate::plan::{Ctx, EditPlan};
use crate::selection::{Goal, Selection, SelectionSet};
use crate::undo_stops::EditKind;

type Edits = Vec<(Range<usize>, Arc<str>)>;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Toggle,
    Add,
    Remove,
}

pub fn toggle_line(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    line(ctx, set, Mode::Toggle)
}

pub fn add_line(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    line(ctx, set, Mode::Add)
}

pub fn remove_line(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    line(ctx, set, Mode::Remove)
}

fn line(ctx: Ctx, set: &SelectionSet, mode: Mode) -> EditPlan {
    let Some(token) = ctx
        .language
        .and_then(|l| l.line_comments.first())
        .map(String::as_str)
    else {
        return plan(set, Vec::new(), default_map);
    };
    let snap = ctx.snapshot;
    let mut edits: Edits = Vec::new();
    let mut done_until = 0u32;
    for sel in set.all() {
        let rows = touched_rows(snap, sel.range());
        // Rows an earlier selection handled are not touched twice.
        let rows = rows.start.max(done_until)..rows.end;
        done_until = done_until.max(rows.end);
        if rows.is_empty() {
            continue;
        }
        let lines: Vec<(u32, String)> = rows.map(|r| (r, snap.line(r).to_string())).collect();
        let non_blank: Vec<&(u32, String)> =
            lines.iter().filter(|(_, l)| !l.trim().is_empty()).collect();
        let commented = |l: &str| starts(l, token);
        let remove = match mode {
            Mode::Add => false,
            Mode::Remove => true,
            Mode::Toggle => !non_blank.is_empty() && non_blank.iter().all(|(_, l)| commented(l)),
        };
        if remove {
            for (row, l) in &lines {
                if !commented(l) {
                    continue;
                }
                let at = row_start(snap, *row) + leading_whitespace(l).len();
                let mut end = at + token.len();
                if l[end - row_start(snap, *row)..].starts_with(' ') {
                    end += 1;
                }
                edits.push((at..end, Arc::from("")));
            }
        } else {
            // All-blank: comment them; otherwise skip the blank rows.
            let targets: Vec<&(u32, String)> = if non_blank.is_empty() {
                lines.iter().collect()
            } else {
                non_blank
            };
            let col = targets
                .iter()
                .map(|(_, l)| leading_whitespace(l).len())
                .min()
                .unwrap_or(0);
            for (row, l) in targets {
                // Blank rows shorter than the indent column keep their length.
                let col = col.min(l.len());
                let at = row_start(snap, *row) + col;
                edits.push((at..at, Arc::from(format!("{token} ").as_str())));
            }
        }
    }
    edits.sort_by_key(|(r, _)| (r.start, r.end));
    plan(set, edits, default_map)
}

fn starts(line: &str, token: &str) -> bool {
    line[leading_whitespace(line).len()..].starts_with(token)
}

pub fn toggle_block(ctx: Ctx, set: &SelectionSet) -> EditPlan {
    let Some((open, close)) = ctx.language.and_then(|l| l.block_comment.clone()) else {
        return toggle_line(ctx, set);
    };
    let snap = ctx.snapshot;
    let mut edits: Edits = Vec::new();
    // Cursors that get a fresh `open  close` pair: the cursor goes between.
    let mut fresh: Vec<(usize, usize)> = Vec::new();
    for sel in set.all() {
        let r = sel.range();
        if r.is_empty() {
            fresh.push((r.start, open.len() + 1));
            edits.push((r.clone(), Arc::from(format!("{open}  {close}").as_str())));
            continue;
        }
        let text = snap.text_for_range(r.clone());
        if let Some((a, b)) = inside(&text, &open, &close) {
            edits.push((r.start..r.start + a, Arc::from("")));
            edits.push((r.end - b..r.end, Arc::from("")));
            continue;
        }
        let line_start = row_start(snap, snap.offset_to_point(r.start).row);
        let before = snap.text_for_range(line_start..r.start);
        let end_row = snap.offset_to_point(r.end).row;
        let line_end = (row_start(snap, end_row) + snap.line(end_row).len()).max(r.end);
        let after = snap.text_for_range(r.end..line_end);
        if let (Some(a), Some(b)) = (
            outside_before(&before, &open),
            outside_after(&after, &close),
        ) {
            edits.push((r.start - a..r.start, Arc::from("")));
            edits.push((r.end..r.end + b, Arc::from("")));
            continue;
        }
        edits.push((r.start..r.start, Arc::from(format!("{open} ").as_str())));
        edits.push((r.end..r.end, Arc::from(format!(" {close}").as_str())));
    }
    edits.sort_by_key(|(r, _)| (r.start, r.end));
    plan(set, edits, move |s, patch| {
        if let Some(&(_, into)) = s
            .is_empty()
            .then(|| fresh.iter().find(|(at, _)| *at == s.range().start))
            .flatten()
        {
            let o = patch.map_offset(s.range().start, Bias::Left) + into;
            return (o, o);
        }
        map_with(s, patch, Bias::Right)
    })
}

/// Lengths of the start and end tokens (with one inner space each) when
/// `text` itself is wrapped by them.
fn inside(text: &str, open: &str, close: &str) -> Option<(usize, usize)> {
    if text.len() < open.len() + close.len() || !text.starts_with(open) || !text.ends_with(close) {
        return None;
    }
    let mut a = open.len();
    let mut b = close.len();
    if text[a..text.len() - b].starts_with(' ') {
        a += 1;
    }
    if text[a..text.len() - b].ends_with(' ') {
        b += 1;
    }
    Some((a, b))
}

/// Bytes of `open` (and one space after it) at the end of `before`.
fn outside_before(before: &str, open: &str) -> Option<usize> {
    let t = before.strip_suffix(' ').unwrap_or(before);
    t.ends_with(open)
        .then(|| open.len() + (before.len() - t.len()))
}

/// Bytes of `close` (and one space before it) at the start of `after`.
fn outside_after(after: &str, close: &str) -> Option<usize> {
    let t = after.strip_prefix(' ').unwrap_or(after);
    t.starts_with(close)
        .then(|| close.len() + (after.len() - t.len()))
}

/// Keeps a selection on its text: an insertion at a cursor pushes it
/// right; at the start of a selection it lands inside (`start_bias`
/// `Left`, a line comment's token) or before the selection (`Right`, a
/// block comment's open token); at its end it stays outside.
fn default_map(s: &Selection, patch: &Patch) -> (usize, usize) {
    map_with(s, patch, Bias::Left)
}

fn map_with(s: &Selection, patch: &Patch, start_bias: Bias) -> (usize, usize) {
    let r = s.range();
    let (start, end) = if s.is_empty() {
        let o = patch.map_offset(r.start, Bias::Right);
        (o, o)
    } else {
        (
            patch.map_offset(r.start, start_bias),
            patch.map_offset(r.end, Bias::Left),
        )
    };
    if s.reversed() {
        (end, start)
    } else {
        (start, end)
    }
}

fn plan(
    set: &SelectionSet,
    edits: Edits,
    map: impl Fn(&Selection, &Patch) -> (usize, usize),
) -> EditPlan {
    let patch = mk_patch(&edits);
    let sels = set
        .all()
        .iter()
        .map(|s| {
            let (anchor, head) = map(s, &patch);
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
        kind: EditKind::Other,
        stop_before: true,
        stop_after: true,
        autoclose: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use lucerna_syntax::LanguageRegistry;
    use lucerna_text::Buffer;

    use super::*;
    use crate::test_util::{marked, render_plan, test_config};

    fn run(lang: Option<&str>, src: &str, f: fn(Ctx, &SelectionSet) -> EditPlan) -> String {
        let (snapshot, set) = marked(src);
        let config = test_config();
        let language = lang.map(|n| {
            LanguageRegistry::bundled()
                .language_by_name(n)
                .unwrap_or_else(|| panic!("no language {n}"))
                .editing()
                .clone()
        });
        let plan = f(
            Ctx {
                snapshot: &snapshot,
                config: &config,
                language: language.as_ref(),
                syntax: None,
            },
            &set,
        );
        render_plan(&snapshot, &plan)
    }

    fn rust(src: &str, f: fn(Ctx, &SelectionSet) -> EditPlan) -> String {
        run(Some("Rust"), src, f)
    }

    /// Toggles the block comment of `sel` in `before + sel + after` (Rust).
    fn block(before: &str, sel: &str, after: &str) -> String {
        let text = format!("{before}{sel}{after}");
        let snapshot = Buffer::from_bytes(text.as_bytes(), false)
            .unwrap()
            .snapshot();
        let set = SelectionSet::from_selections(
            vec![Selection {
                id: 0,
                anchor: before.len(),
                head: before.len() + sel.len(),
                goal: Goal::None,
            }],
            0,
            false,
        );
        let config = test_config();
        let language = LanguageRegistry::bundled()
            .language_by_name("Rust")
            .unwrap()
            .editing()
            .clone();
        let plan = toggle_block(
            Ctx {
                snapshot: &snapshot,
                config: &config,
                language: Some(&language),
                syntax: None,
            },
            &set,
        );
        render_plan(&snapshot, &plan)
    }

    #[test]
    fn toggle_comments_and_uncomments() {
        assert_eq!(rust("|a\nb", toggle_line), "// |a\nb");
        assert_eq!(rust("// |a\nb", toggle_line), "|a\nb");
        assert_eq!(rust("//|a", toggle_line), "|a");
    }

    #[test]
    fn toggle_mixed_block_comments_all_at_min_indent() {
        assert_eq!(
            rust("«  a\n\n    // b\n  c»", toggle_line),
            "«  // a\n\n  //   // b\n  // c»"
        );
        // All commented: uncomment, blank rows untouched.
        assert_eq!(rust("«  // a\n\n  // b»", toggle_line), "«  a\n\n  b»");
    }

    #[test]
    fn selection_ending_at_column_zero_skips_that_line() {
        assert_eq!(rust("«a\nb\n»c", toggle_line), "«// a\n// b\n»c");
    }

    #[test]
    fn selection_stays_on_its_text() {
        assert_eq!(rust("  «a»", toggle_line), "  «// a»");
        assert_eq!(rust("»a b«", toggle_line), "»// a b«");
    }

    #[test]
    fn remove_line_only_removes() {
        assert_eq!(rust("«// a\nb»", remove_line), "«a\nb»");
        assert_eq!(rust("|a", remove_line), "|a");
    }

    #[test]
    fn add_line_on_commented_adds_again() {
        assert_eq!(rust("// |a", add_line), "// // |a");
    }

    #[test]
    fn blank_only_lines_are_commented() {
        assert_eq!(rust("|", toggle_line), "// |");
    }

    #[test]
    fn block_comment_wraps_and_unwraps() {
        assert_eq!(rust("«ab»", toggle_block), "/* «ab» */");
        // `marked` reads `*` as a primary marker, so build these by hand.
        assert_eq!(block("/* ", "ab", " */"), "«ab»");
        assert_eq!(block("/*", "ab", "*/"), "«ab»");
        assert_eq!(block("", "/* ab */", ""), "«ab»");
        assert_eq!(block("", "/*ab*/", ""), "«ab»");
        assert_eq!(block("x /* ", "a\nb", " */ y"), "x «a\nb» y");
        assert_eq!(rust("x «a\nb» y", toggle_block), "x /* «a\nb» */ y");
    }

    #[test]
    fn block_comment_at_empty_cursor() {
        assert_eq!(rust("a |b", toggle_block), "a /* | */b");
    }

    #[test]
    fn python_has_no_block_comment_falls_back_to_line() {
        assert_eq!(run(Some("Python"), "«ab»", toggle_block), "«# ab»");
    }

    #[test]
    fn no_comment_data_is_an_empty_plan() {
        let (snapshot, set) = marked("|a");
        let config = test_config();
        let ctx = Ctx {
            snapshot: &snapshot,
            config: &config,
            language: None,
            syntax: None,
        };
        for f in [toggle_line, add_line, remove_line, toggle_block] {
            let p = f(ctx, &set);
            assert!(p.edits.is_empty());
            assert_eq!(render_plan(&snapshot, &p), "|a");
        }
    }

    #[test]
    fn multi_cursor_two_regions() {
        assert_eq!(rust("|a\nb\n|c", toggle_line), "// |a\nb\n// |c");
        // Two cursors on one line comment it once.
        assert_eq!(rust("|a |b", toggle_line), "// |a |b");
    }
}
