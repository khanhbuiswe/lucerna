//! Auto-indent (spec §5, `editor.autoIndent`): the indent Enter gives the
//! new line, by the language's brackets and its `indents.scm`, and the
//! outdent of a line whose first typed char is a bracket closer.

use std::ops::Range;

use lucerna_syntax::{BracketPair, IndentDelta, Scope};

use crate::commands::{display_col, indent_string, leading_whitespace, row_start};
use crate::plan::{AutoIndent, Ctx};

/// How far back a closer looks for its opener.
const SCAN_LIMIT: usize = 1 << 20;

/// What Enter over a range inserts.
pub(crate) struct Enter {
    /// The range replaced (the selection, or a whitespace-only line).
    pub range: Range<usize>,
    pub text: String,
    /// Where the cursor goes, in bytes into `text`.
    pub cursor: usize,
}

/// `base` moved by `delta`, in visible columns, written in the configured
/// style: one unit (`tabSize` spaces, or a tab to the next stop) in, or
/// back to the previous indent stop.
fn shifted(ctx: Ctx, base: &str, delta: IndentDelta) -> String {
    let ts = ctx.config.tab_size.max(1);
    let width = display_col(base, ts);
    let cols = match delta {
        IndentDelta::Same => width,
        IndentDelta::Indent if ctx.config.insert_spaces => width + ts,
        IndentDelta::Indent => (width / ts + 1) * ts,
        IndentDelta::Outdent => width.saturating_sub(1) / ts * ts,
    };
    indent_string(ctx, cols)
}

/// A pair Enter opens: not a quote, and marked to break lines.
fn opens_lines(pair: &BracketPair) -> bool {
    pair.newline && pair.start != pair.end
}

/// The edit Enter makes over `range`. With `trim`, a whitespace-only line
/// with the cursor at its end loses that whitespace
/// (`editor.trimAutoWhitespace`).
///
/// - `keep`: the line's indent, up to the cursor.
/// - `brackets`: after an opener, one unit more; right before its closer
///   too, the closer goes on a line of its own at the opener line's indent.
/// - `advanced` / `full`: the new line takes the `indents.scm` suggestion
///   instead, when there is one.
///
/// Adapted from microsoft/vscode src/vs/editor/common/cursor/cursorTypeEditOperations.ts (MIT)
/// (`_enter`).
pub(crate) fn enter(ctx: Ctx, range: Range<usize>, trim: bool) -> Enter {
    let snap = ctx.snapshot;
    let mode = ctx.config.auto_indent;
    let p = snap.offset_to_point(range.start);
    let line = snap.line(p.row);
    let col = (p.column as usize).min(line.len());
    let keep = if mode >= AutoIndent::Keep {
        let lead = leading_whitespace(&line);
        &lead[..lead.len().min(col)]
    } else {
        ""
    };
    let mut indent = keep.to_string();
    // The opener line's indent, for a closer moved to its own line.
    let mut closer: Option<&str> = None;
    if mode >= AutoIndent::Brackets {
        let before = line[..col].trim_end_matches([' ', '\t']);
        let q = snap.offset_to_point(range.end);
        let end_line = snap.line(q.row);
        let after = &end_line[(q.column as usize).min(end_line.len())..];
        if let Some(pair) = crate::autoclose::language(&ctx)
            .brackets
            .iter()
            .find(|p| opens_lines(p) && before.ends_with(p.start.as_str()))
        {
            indent = shifted(ctx, keep, IndentDelta::Indent);
            if after.starts_with(pair.end.as_str()) {
                closer = Some(keep);
            }
        }
    }
    if mode >= AutoIndent::Advanced
        && let Some(syntax) = ctx.syntax
        && let Some((basis, delta)) =
            syntax.indent_suggestion_for_newline(snap, range.clone(), closer.is_some())
    {
        let basis_line = snap.line(basis);
        let base = leading_whitespace(&basis_line);
        indent = match delta {
            IndentDelta::Same => base.to_string(),
            delta => shifted(ctx, base, delta),
        };
    }
    let mut text = format!("\n{indent}");
    let cursor = text.len();
    if let Some(closer) = closer {
        text.push('\n');
        text.push_str(closer);
    }
    // A whitespace-only line with the cursor at its end loses that
    // whitespace; the indent is re-inserted after the break.
    let range = if trim
        && ctx.config.trim_auto_whitespace
        && range.is_empty()
        && col == line.len()
        && col > 0
        && line.bytes().all(|b| b == b' ' || b == b'\t')
    {
        row_start(snap, p.row)..range.end
    } else {
        range
    };
    Enter {
        range,
        text,
        cursor,
    }
}

/// Typing `ch` at an empty cursor at `offset`: when `ch` is a bracket
/// closer (`editor.autoIndent` at least `brackets`) and the line holds only
/// blanks up to the cursor, the edit that also sets the line's indent to
/// that of the line holding the matching opener.
///
/// Adapted from microsoft/vscode src/vs/editor/common/cursor/cursorTypeEditOperations.ts (MIT)
/// (`_typeInterceptorElectricChar`).
pub(crate) fn closer_indent(ctx: Ctx, offset: usize, ch: &str) -> Option<(Range<usize>, String)> {
    if ctx.config.auto_indent < AutoIndent::Brackets {
        return None;
    }
    let pair = crate::autoclose::language(&ctx)
        .brackets
        .iter()
        .find(|p| p.start != p.end && p.end == ch)?;
    let snap = ctx.snapshot;
    let p = snap.offset_to_point(offset);
    let line = snap.line(p.row);
    let prefix = &line[..(p.column as usize).min(line.len())];
    if !prefix.bytes().all(|b| b == b' ' || b == b'\t') {
        return None;
    }
    let open = matching_open(ctx, offset, pair)?;
    let open_row = snap.offset_to_point(open).row;
    if open_row == p.row {
        return None;
    }
    let open_line = snap.line(open_row);
    let indent = shifted(ctx, leading_whitespace(&open_line), IndentDelta::Same);
    if indent == prefix {
        return None;
    }
    Some((row_start(snap, p.row)..offset, indent + ch))
}

/// The opener of `pair` a closer typed at `offset` would close: scanning
/// back and counting pairs. With a tree, a closer that matches skips its
/// whole pair, and brackets in strings and comments are ignored.
fn matching_open(ctx: Ctx, offset: usize, pair: &BracketPair) -> Option<usize> {
    let snap = ctx.snapshot;
    let syntax = ctx.syntax.filter(|s| s.tree().is_some());
    let limit = offset.saturating_sub(SCAN_LIMIT);
    let mut depth = 0usize;
    let mut pos = offset;
    loop {
        let (at, open) = prev_bracket(ctx, pos, pair, limit)?;
        pos = at;
        if let Some(syntax) = syntax {
            if !open
                && let Some(m) = syntax
                    .matching_bracket(snap, at)
                    .filter(|&m| m < at && starts_with_at(ctx, m, &pair.start))
            {
                pos = m;
                continue;
            }
            if syntax.scope_at(snap, at) != Scope::Code {
                continue;
            }
        }
        if open {
            if depth == 0 {
                return Some(at);
            }
            depth -= 1;
        } else {
            depth += 1;
        }
    }
}

fn starts_with_at(ctx: Ctx, offset: usize, s: &str) -> bool {
    let p = ctx.snapshot.offset_to_point(offset);
    let line = ctx.snapshot.line(p.row);
    line[(p.column as usize).min(line.len())..].starts_with(s)
}

/// The last opener or closer of `pair` before `pos`, not before `limit`,
/// and whether it is the opener.
fn prev_bracket(ctx: Ctx, pos: usize, pair: &BracketPair, limit: usize) -> Option<(usize, bool)> {
    let snap = ctx.snapshot;
    let p = snap.offset_to_point(pos);
    let (mut row, mut col) = (p.row, p.column as usize);
    loop {
        let line = snap.line(row);
        let hay = &line[..col.min(line.len())];
        let start = row_start(snap, row);
        let found = match (hay.rfind(pair.start.as_str()), hay.rfind(pair.end.as_str())) {
            (Some(o), Some(c)) if c > o => Some((c, false)),
            (Some(o), _) => Some((o, true)),
            (None, Some(c)) => Some((c, false)),
            (None, None) => None,
        };
        if let Some((i, open)) = found {
            return (start + i >= limit).then_some((start + i, open));
        }
        if row == 0 || start <= limit {
            return None;
        }
        row -= 1;
        col = usize::MAX;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use lucerna_syntax::{Language, LanguageRegistry, ParseOutcome, SyntaxState};
    use lucerna_text::BufferSnapshot;

    use super::*;
    use crate::commands::{insert, line_break_insert, newline};
    use crate::plan::{EditConfig, EditPlan};
    use crate::selection::SelectionSet;
    use crate::test_util::{marked, render_plan, test_config};
    use crate::undo_stops::EditKind;

    /// Written for these tests: Rust blocks indent, `}` closes them.
    const INDENTS: &str = r#"(block "}" @end) @indent"#;

    fn plain(src: &str, config: &EditConfig, f: impl Fn(Ctx, &SelectionSet) -> EditPlan) -> String {
        let (snapshot, set) = marked(src);
        let ctx = Ctx {
            snapshot: &snapshot,
            config,
            language: None,
            syntax: None,
        };
        render_plan(&snapshot, &f(ctx, &set))
    }

    fn enter_plain(src: &str) -> String {
        plain(src, &test_config(), newline)
    }

    fn typed(ctx: Ctx, set: &SelectionSet, ch: &str) -> EditPlan {
        insert(ctx, set, ch, EditKind::TypingOther, &[])
    }

    fn parsed(language: Arc<Language>, snapshot: &BufferSnapshot) -> SyntaxState {
        let mut syntax = SyntaxState::new(language, Arc::new(LanguageRegistry::bundled())).unwrap();
        assert_eq!(syntax.parse(snapshot, None), ParseOutcome::Done);
        syntax
    }

    /// Runs `f` on `src` as Rust, parsed; `indents` adds an `indents.scm`.
    fn rust(
        src: &str,
        indents: Option<&str>,
        config: &EditConfig,
        f: impl Fn(Ctx, &SelectionSet) -> EditPlan,
    ) -> String {
        let (snapshot, set) = marked(src);
        let language = match indents {
            Some(q) => Language::bundled_with_indents_for_test("rust", q),
            None => LanguageRegistry::bundled()
                .language_by_name("Rust")
                .unwrap(),
        };
        let syntax = parsed(language.clone(), &snapshot);
        let ctx = Ctx {
            snapshot: &snapshot,
            config,
            language: Some(language.editing()),
            syntax: Some(&syntax),
        };
        render_plan(&snapshot, &f(ctx, &set))
    }

    fn with(mode: AutoIndent) -> EditConfig {
        let mut config = test_config();
        config.auto_indent = mode;
        config
    }

    #[test]
    fn enter_after_open_brace_indents() {
        assert_eq!(enter_plain("fn a() {|"), "fn a() {\n    |");
        // Blanks after the opener are ignored.
        assert_eq!(enter_plain("  f(  |"), "  f(  \n      |");
        assert_eq!(enter_plain("a [|b"), "a [\n    |b");
        // Not after other chars.
        assert_eq!(enter_plain("    a;|"), "    a;\n    |");
        // Tabs: one more tab stop.
        let mut config = test_config();
        config.insert_spaces = false;
        assert_eq!(plain("\tif {|", &config, newline), "\tif {\n\t\t|");
        assert_eq!(plain("  if {|", &config, newline), "  if {\n\t|");
    }

    #[test]
    fn enter_between_braces_splits() {
        assert_eq!(enter_plain("{|}"), "{\n    |\n}");
        // The closer goes back to the opener line's indent.
        assert_eq!(enter_plain("    f(|);"), "    f(\n        |\n    );");
        // Only right before the closer.
        assert_eq!(enter_plain("{| }"), "{\n    | }");
        // `lineBreakInsert` keeps the cursor.
        assert_eq!(
            plain("{|}", &test_config(), line_break_insert),
            "{|\n    \n}"
        );
        // `brackets` is enough; `keep` is not.
        assert_eq!(
            plain("{|}", &with(AutoIndent::Brackets), newline),
            "{\n    |\n}"
        );
        assert_eq!(plain("{|}", &with(AutoIndent::Keep), newline), "{\n|}");
    }

    #[test]
    fn typing_closer_outdents() {
        let close = |c: Ctx, s: &SelectionSet| typed(c, s, "}");
        assert_eq!(plain("{\n    |", &test_config(), close), "{\n}|");
        // The matching opener's line, skipping balanced pairs.
        assert_eq!(
            plain(
                "  a {\n    b {}\n    c {\n    }\n        |",
                &test_config(),
                close
            ),
            "  a {\n    b {}\n    c {\n    }\n  }|"
        );
        // Text before the cursor, or `keep`: typed as is.
        assert_eq!(plain("{\n    x|", &test_config(), close), "{\n    x}|");
        assert_eq!(
            plain("{\n    |", &with(AutoIndent::Keep), close),
            "{\n    }|"
        );
        // No opener: as is.
        assert_eq!(plain("    |", &test_config(), close), "    }|");
        // Other brackets have their own pairs.
        let paren = |c: Ctx, s: &SelectionSet| typed(c, s, ")");
        assert_eq!(
            plain("f(\n  {\n  }\n    |", &test_config(), paren),
            "f(\n  {\n  }\n)|"
        );
    }

    #[test]
    fn typing_closer_skips_strings_with_a_tree() {
        let close = |c: Ctx, s: &SelectionSet| typed(c, s, "}");
        assert_eq!(
            rust(
                "fn a() {\n    let s = \"{\";\n        |",
                None,
                &test_config(),
                close
            ),
            "fn a() {\n    let s = \"{\";\n}|"
        );
    }

    #[test]
    fn typing_closer_ends_the_undo_step() {
        let (snapshot, set) = marked("{\n    |");
        let config = test_config();
        let ctx = Ctx {
            snapshot: &snapshot,
            config: &config,
            language: None,
            syntax: None,
        };
        let p = typed(ctx, &set, "}");
        assert_eq!(
            (p.kind, p.stop_before, p.stop_after),
            (EditKind::TypingOther, false, true)
        );
    }

    #[test]
    fn auto_indent_none_and_keep() {
        assert_eq!(
            plain("    a|", &with(AutoIndent::None), newline),
            "    a\n|"
        );
        assert_eq!(
            plain("    a|", &with(AutoIndent::Keep), newline),
            "    a\n    |"
        );
        // `none` ignores brackets too.
        assert_eq!(plain("{|}", &with(AutoIndent::None), newline), "{\n|}");
    }

    #[test]
    fn indents_scm_suggestion() {
        let (snapshot, _) = marked("fn a() {\nx;\n\n}\n|");
        let language = Language::bundled_with_indents_for_test("rust", INDENTS);
        let syntax = parsed(language, &snapshot);
        // Inside the block: one level in from `fn a() {`.
        assert_eq!(
            syntax.indent_suggestion(&snapshot, 1),
            Some(IndentDelta::Indent)
        );
        // A blank row inside it stays at the level of `x;`.
        assert_eq!(
            syntax.indent_suggestion(&snapshot, 2),
            Some(IndentDelta::Same)
        );
        // `}` closes the level.
        assert_eq!(
            syntax.indent_suggestion(&snapshot, 3),
            Some(IndentDelta::Outdent)
        );
        assert_eq!(syntax.indent_suggestion(&snapshot, 0), None);
        // Without the query, no suggestion.
        let plain_rust = LanguageRegistry::bundled()
            .language_by_name("Rust")
            .unwrap();
        let syntax = parsed(plain_rust, &snapshot);
        assert_eq!(syntax.indent_suggestion(&snapshot, 1), None);
    }

    #[test]
    fn enter_follows_indents_scm() {
        let src = "fn a() {\n    x;|}";
        // `full`: the closer after the cursor outdents the new line.
        assert_eq!(
            rust(src, Some(INDENTS), &test_config(), newline),
            "fn a() {\n    x;\n|}"
        );
        assert_eq!(
            rust(src, Some(INDENTS), &with(AutoIndent::Advanced), newline),
            "fn a() {\n    x;\n|}"
        );
        // `brackets`, or no query: the indent is kept.
        assert_eq!(
            rust(src, Some(INDENTS), &with(AutoIndent::Brackets), newline),
            "fn a() {\n    x;\n    |}"
        );
        assert_eq!(
            rust(src, None, &test_config(), newline),
            "fn a() {\n    x;\n    |}"
        );
        // Between braces: the middle line by the query, the closer back.
        assert_eq!(
            rust("fn a() {|}", Some(INDENTS), &test_config(), newline),
            "fn a() {\n    |\n}"
        );
    }

    #[test]
    fn newline_multi_cursor_brackets() {
        assert_eq!(
            enter_plain("  a {|\nb (|\nc|"),
            "  a {\n      |\nb (\n    |\nc\n|"
        );
        assert_eq!(enter_plain("{|}\n  [|]"), "{\n    |\n}\n  [\n      |\n  ]");
    }
}
