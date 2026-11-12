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

/// How a line is commented: a line token, or (languages with only block
/// comments) the block tokens around each line, as VS Code does.
enum Style {
    Line(String),
    Block(String, String),
}

impl Style {
    fn of(ctx: &Ctx) -> Option<Style> {
        let language = ctx.language?;
        // Data may carry the token with its space (`"# "`).
        let token = language
            .line_comments
            .iter()
            .map(|t| t.trim())
            .find(|t| !t.is_empty());
        if let Some(token) = token {
            return Some(Style::Line(token.to_string()));
        }
        let (open, close) = language.block_comment.clone()?;
        Some(Style::Block(open, close))
    }

    /// Whether `line` is commented in this style.
    fn commented(&self, line: &str) -> bool {
        let body = line.trim();
        match self {
            Style::Line(token) => body.starts_with(token.as_str()),
            Style::Block(open, close) => {
                body.len() >= open.len() + close.len()
                    && body.starts_with(open.as_str())
                    && body.ends_with(close.as_str())
            }
        }
    }

    /// The edits (relative to the line start) that comment `line`.
    fn add(&self, line: &str, col: usize) -> Vec<(Range<usize>, Arc<str>)> {
        match self {
            Style::Line(token) => vec![(col..col, Arc::from(format!("{token} ").as_str()))],
            Style::Block(open, close) => {
                let end = line.trim_end().len();
                vec![
                    (col..col, Arc::from(format!("{open} ").as_str())),
                    (end..end, Arc::from(format!(" {close}").as_str())),
                ]
            }
        }
    }

    /// The edits (relative to the line start) that uncomment `line`.
    fn remove(&self, line: &str) -> Vec<(Range<usize>, Arc<str>)> {
        let at = leading_whitespace(line).len();
        let empty: Arc<str> = Arc::from("");
        match self {
            Style::Line(token) => {
                let mut end = at + token.len();
                if line[end..].starts_with(' ') {
                    end += 1;
                }
                vec![(at..end, empty)]
            }
            Style::Block(open, close) => {
                let mut a = at + open.len();
                if line[a..].starts_with(' ') {
                    a += 1;
                }
                let trimmed = line.trim_end().len();
                let mut b = trimmed - close.len();
                if b > a && line[..b].ends_with(' ') {
                    b -= 1;
                }
                vec![(at..a, empty.clone()), (b..trimmed, empty)]
            }
        }
    }
}

fn line(ctx: Ctx, set: &SelectionSet, mode: Mode) -> EditPlan {
    let Some(style) = Style::of(&ctx) else {
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
        let remove = match mode {
            Mode::Add => false,
            Mode::Remove => true,
            Mode::Toggle => {
                !non_blank.is_empty() && non_blank.iter().all(|(_, l)| style.commented(l))
            }
        };
        let mut push = |row: u32, rel: Vec<(Range<usize>, Arc<str>)>| {
            let base = row_start(snap, row);
            edits.extend(
                rel.into_iter()
                    .map(|(r, t)| (base + r.start..base + r.end, t)),
            );
        };
        if remove {
            for (row, l) in &lines {
                if style.commented(l) {
                    push(*row, style.remove(l));
                }
            }
        } else {
            // All-blank: comment them (line style); otherwise skip blank rows.
            let blank_ok = non_blank.is_empty() && matches!(style, Style::Line(_));
            let targets: Vec<&(u32, String)> = if blank_ok {
                lines.iter().collect()
            } else {
                non_blank
            };
            let tab = ctx.config.tab_size.max(1) as usize;
            let min = targets
                .iter()
                .map(|(_, l)| visible_width(leading_whitespace(l), tab))
                .min()
                .unwrap_or(0);
            // Down to a whole indent level, as VS Code does.
            let min = min / tab * tab;
            for (row, l) in targets {
                match style {
                    // At the shared visible indent column.
                    Style::Line(_) => push(*row, style.add(l, insertion_offset(l, min, tab))),
                    Style::Block(..) => push(*row, style.add(l, leading_whitespace(l).len())),
                }
            }
        }
    }
    edits.sort_by_key(|(r, _)| (r.start, r.end));
    // A wrapped line's selection end takes in the closing token.
    let end_bias = if matches!(style, Style::Block(..)) {
        Bias::Right
    } else {
        Bias::Left
    };
    plan(set, edits, move |s, p| map_with(s, p, Bias::Left, end_bias))
}

/// The visible width of `indent` (spaces and tabs) with tab stops every
/// `tab` columns.
fn visible_width(indent: &str, tab: usize) -> usize {
    indent.bytes().fold(0, |col, b| next_column(col, b, tab))
}

fn next_column(col: usize, b: u8, tab: usize) -> usize {
    if b == b'\t' {
        col + tab - col % tab
    } else {
        col + 1
    }
}

/// The byte offset in `line`'s indent where the visible column reaches
/// `min`; a tab that crosses it is not split, so the token goes before it.
///
/// Adapted from microsoft/vscode src/vs/editor/contrib/comment/browser/lineCommentCommand.ts (MIT)
/// (`_normalizeInsertionPoint`).
fn insertion_offset(line: &str, min: usize, tab: usize) -> usize {
    let indent = leading_whitespace(line).as_bytes();
    let (mut col, mut j) = (0, 0);
    while col < min && j < indent.len() {
        col = next_column(col, indent[j], tab);
        j += 1;
    }
    if col > min { j - 1 } else { j }
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
        map_with(s, patch, Bias::Right, Bias::Left)
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
    map_with(s, patch, Bias::Left, Bias::Left)
}

fn map_with(s: &Selection, patch: &Patch, start_bias: Bias, end_bias: Bias) -> (usize, usize) {
    let r = s.range();
    let (start, end) = if s.is_empty() {
        let o = patch.map_offset(r.start, Bias::Right);
        (o, o)
    } else {
        (
            patch.map_offset(r.start, start_bias),
            patch.map_offset(r.end, end_bias),
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
        // `marked` reads `*` as a primary marker: `~` stands for it in `src`.
        let (snapshot, set) = marked(src);
        let snapshot = Buffer::from_bytes(
            snapshot
                .text_for_range(0..snapshot.len_bytes())
                .replace('~', "*")
                .as_bytes(),
            false,
        )
        .unwrap()
        .snapshot();
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

    fn block_reversed(before: &str, sel: &str, after: &str) -> String {
        let text = format!("{before}{sel}{after}");
        let snapshot = Buffer::from_bytes(text.as_bytes(), false)
            .unwrap()
            .snapshot();
        let set = SelectionSet::from_selections(
            vec![Selection {
                id: 0,
                anchor: before.len() + sel.len(),
                head: before.len(),
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
        let ctx = Ctx {
            snapshot: &snapshot,
            config: &config,
            language: Some(&language),
            syntax: None,
        };
        render_plan(&snapshot, &toggle_block(ctx, &set))
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
            rust("«    a\n\n        // b\n    c»", toggle_line),
            "«    // a\n\n    //     // b\n    // c»"
        );
        // All commented: uncomment, blank rows untouched.
        assert_eq!(rust("«  // a\n\n  // b»", toggle_line), "«  a\n\n  b»");
    }

    /// Final review, finding 5: the token goes at the minimum visible
    /// indent (tabs count to the next tab stop), rounded down to a whole
    /// indent level, as in VS Code's `_normalizeInsertionPoint` tests.
    #[test]
    fn mixed_tab_and_space_indents_align_by_visible_column() {
        // A tab and four spaces are both one level at tab size 4.
        assert_eq!(rust("«\ta\n    b»", toggle_line), "«\t// a\n    // b»");
        assert_eq!(
            rust("«\t\ta\n    \tb\n        c»", toggle_line),
            "«\t\t// a\n    \t// b\n        // c»"
        );
        // VS Code bug 16696: two and four spaces at tab size 4 align at 0.
        assert_eq!(rust("«  a\n    b»", toggle_line), "«//   a\n//     b»");
    }

    #[test]
    fn selection_ending_at_column_zero_skips_that_line() {
        assert_eq!(rust("«a\nb\n»c", toggle_line), "«// a\n// b\n»c");
    }

    #[test]
    fn selection_stays_on_its_text() {
        assert_eq!(rust("    «a»", toggle_line), "    «// a»");
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
    fn token_with_trailing_space_round_trips() {
        let (snapshot, set) = marked("|a");
        let config = test_config();
        let mut language = LanguageRegistry::bundled()
            .language_by_name("Python")
            .unwrap()
            .editing()
            .clone();
        language.line_comments = vec!["# ".into()];
        let ctx = Ctx {
            snapshot: &snapshot,
            config: &config,
            language: Some(&language),
            syntax: None,
        };
        assert_eq!(render_plan(&snapshot, &toggle_line(ctx, &set)), "# |a");
        let (snapshot, set) = marked("# |a");
        let ctx = Ctx {
            snapshot: &snapshot,
            ..ctx
        };
        assert_eq!(render_plan(&snapshot, &toggle_line(ctx, &set)), "|a");
    }

    #[test]
    fn block_only_language_wraps_each_line() {
        let css = |src, f| run(Some("CSS"), src, f);
        assert_eq!(css("«a\n\n  b»", toggle_line), "«/* a */\n\n  /* b */»");
        assert_eq!(css("«/~ a ~/\n  /~ b ~/»", toggle_line), "«a\n  b»");
        // Mixed: comment all, including the already wrapped one.
        assert_eq!(css("«a\n/~ b ~/»", toggle_line), "«/* a */\n/* /* b */ */»");
        assert_eq!(css("|a", add_line), "/* |a */");
        assert_eq!(css("«/~ a ~/\nb»", remove_line), "«a\nb»");
        assert_eq!(css("|/~ a ~/", toggle_line), "|a");
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

    #[test]
    fn block_toggle_multi_cursor_and_reversed() {
        assert_eq!(rust("«a» b «c»", toggle_block), "/* «a» */ b /* «c» */");
        assert_eq!(rust("|a |b", toggle_block), "/* | */a /* | */b");
        assert_eq!(rust("»ab«", toggle_block), "/* »ab« */");
        assert_eq!(block_reversed("/* ", "ab", " */"), "»ab«");
    }
}
