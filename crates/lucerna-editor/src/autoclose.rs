//! Smart typing around pairs (spec §5): typing over a closer the editor
//! inserted, surrounding a selection, auto-closing a pair and deleting an
//! empty auto-closed pair. The view keeps the auto-closed regions and maps
//! them through every edit ([`remap_regions`], [`retain_regions`]).

use std::borrow::Cow;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

use lucerna_syntax::{BracketPair, LanguageEditing, Scope};
use lucerna_text::{Bias, BufferSnapshot, Patch};

use crate::commands::{Item, Place, build};
use crate::plan::{AutoClosing, AutoClosingEdit, AutoSurround, Ctx, EditPlan};
use crate::selection::{Goal, Selection, SelectionSet};
use crate::undo_stops::EditKind;

/// The plan for typing `ch`, when it types over closers, surrounds the
/// selections or auto-closes a pair, checked in that order; `None` for a
/// plain insert. Each applies to all selections or to none, as in VS Code.
pub(crate) fn type_char(
    ctx: Ctx,
    set: &SelectionSet,
    ch: char,
    regions: &[Range<usize>],
    kind: EditKind,
    stop_before: bool,
) -> Option<EditPlan> {
    let mut buf = [0; 4];
    let ch: &str = ch.encode_utf8(&mut buf);
    let lang = language(&ctx);
    overtype(ctx, lang, set, ch, regions, kind, stop_before)
        .or_else(|| surround(ctx, lang, set, ch))
        .or_else(|| auto_close(ctx, lang, set, ch, kind, stop_before))
}

/// The document language's pairs, else the default brackets.
fn language<'a>(ctx: &Ctx<'a>) -> &'a LanguageEditing {
    static DEFAULT: OnceLock<LanguageEditing> = OnceLock::new();
    ctx.language
        .unwrap_or_else(|| DEFAULT.get_or_init(LanguageEditing::default))
}

/// The line holding `offset`, and `offset`'s byte column in it.
fn line_at(snapshot: &BufferSnapshot, offset: usize) -> (Cow<'_, str>, usize) {
    let p = snapshot.offset_to_point(offset);
    let line = snapshot.line(p.row);
    let col = (p.column as usize).min(line.len());
    (line, col)
}

fn is_quote(pair: &BracketPair) -> bool {
    pair.start == pair.end
}

/// Typing a closer right before the same closer moves over it: with `auto`,
/// only a closer the editor inserted (the end of a region around the
/// cursor), with `always` any closer.
///
/// Adapted from microsoft/vscode src/vs/editor/common/cursor/cursorTypeEditOperations.ts (MIT)
/// (`AutoClosingOvertypeOperation`).
fn overtype(
    ctx: Ctx,
    lang: &LanguageEditing,
    set: &SelectionSet,
    ch: &str,
    regions: &[Range<usize>],
    kind: EditKind,
    stop_before: bool,
) -> Option<EditPlan> {
    let mode = ctx.config.auto_closing_overtype;
    let pair = lang.brackets.iter().find(|p| p.end == ch)?;
    if mode == AutoClosingEdit::Never {
        return None;
    }
    for s in set.all() {
        if !s.is_empty() {
            return None;
        }
        let (line, col) = line_at(ctx.snapshot, s.head);
        if !line[col..].starts_with(ch) {
            return None;
        }
        // A quote after a backslash is an escaped one: typed, not skipped.
        if is_quote(pair) && line[..col].ends_with('\\') {
            return None;
        }
        if mode == AutoClosingEdit::Auto
            && !regions
                .iter()
                .any(|r| r.start < s.head && r.end == s.head + ch.len())
        {
            return None;
        }
    }
    let items = set
        .all()
        .iter()
        .map(|s| Item {
            id: s.id,
            range: s.head..s.head,
            text: String::new(),
            place: Place::At(ch.len()),
        })
        .collect();
    Some(build(items, set.primary().id, kind, stop_before, false))
}

/// Typing a pair's opener with every selection non-empty wraps each in the
/// pair and keeps it selected inside (`editor.autoSurround`).
///
/// Adapted from microsoft/vscode src/vs/editor/common/cursor/cursorTypeEditOperations.ts (MIT)
/// (`SurroundSelectionOperation`).
fn surround(ctx: Ctx, lang: &LanguageEditing, set: &SelectionSet, ch: &str) -> Option<EditPlan> {
    let pair = lang.brackets.iter().find(|p| p.start == ch)?;
    let allowed = match ctx.config.auto_surround {
        AutoSurround::LanguageDefined => true,
        AutoSurround::Quotes => is_quote(pair),
        AutoSurround::Brackets => !is_quote(pair),
        AutoSurround::Never => false,
    };
    if !allowed {
        return None;
    }
    for s in set.all() {
        if s.is_empty() {
            return None;
        }
        let text = ctx.snapshot.text_for_range(s.range());
        if text.chars().all(char::is_whitespace) {
            return None;
        }
        // A quote typed over a selected quote replaces it.
        if is_quote(pair) && matches!(text.as_str(), "'" | "\"" | "`") {
            return None;
        }
    }
    let mut sorted: Vec<&Selection> = set.all().iter().collect();
    sorted.sort_by_key(|s| s.range().start);
    let (open, close): (Arc<str>, Arc<str>) =
        (pair.start.as_str().into(), pair.end.as_str().into());
    let mut edits = Vec::with_capacity(sorted.len() * 2);
    let mut selections = Vec::with_capacity(sorted.len());
    let mut delta = 0;
    for s in sorted {
        let r = s.range();
        edits.push((r.start..r.start, open.clone()));
        edits.push((r.end..r.end, close.clone()));
        let shift = delta + open.len();
        selections.push(Selection {
            id: s.id,
            anchor: r.start + shift,
            head: r.end + shift,
            goal: Goal::None,
        });
        delta += open.len() + close.len();
    }
    Some(EditPlan {
        edits,
        selections: SelectionSet::from_selections(selections, set.primary().id, true),
        kind: EditKind::Other,
        stop_before: true,
        stop_after: true,
        autoclose: Vec::new(),
    })
}

/// A word char for the quote rule: neither a word separator nor a blank
/// (VS Code's `WordCharacterClass.Regular`).
fn is_word_char(c: char, separators: &str) -> bool {
    c != ' ' && c != '\t' && !separators.contains(c)
}

/// The text after the cursor starts with a bracket closer (VS Code's
/// `_isBeforeClosingBrace`; quotes are never closers here).
fn before_closer(lang: &LanguageEditing, after: &str) -> bool {
    lang.brackets
        .iter()
        .any(|p| !is_quote(p) && after.starts_with(p.end.as_str()))
}

/// Typing a pair's opener at empty cursors inserts the pair and puts the
/// cursor between, when the settings, the next char, the previous char
/// (quotes) and the scope allow it at every cursor.
///
/// Adapted from microsoft/vscode src/vs/editor/common/cursor/cursorTypeEditOperations.ts (MIT)
/// (`AutoClosingOpenCharTypeOperation`).
fn auto_close(
    ctx: Ctx,
    lang: &LanguageEditing,
    set: &SelectionSet,
    ch: &str,
    kind: EditKind,
    stop_before: bool,
) -> Option<EditPlan> {
    let pair = lang.brackets.iter().find(|p| p.close && p.start == ch)?;
    let quote = is_quote(pair);
    let setting = if quote {
        ctx.config.auto_closing_quotes
    } else {
        ctx.config.auto_closing_brackets
    };
    if setting == AutoClosing::Never {
        return None;
    }
    let snap = ctx.snapshot;
    for s in set.all() {
        if !s.is_empty() {
            return None;
        }
        let (line, col) = line_at(snap, s.head);
        let after = &line[col..];
        // The end of the line always allows it.
        if let Some(next) = after.chars().next() {
            let allowed = match setting {
                AutoClosing::Always => true,
                AutoClosing::BeforeWhitespace => next.is_whitespace(),
                AutoClosing::LanguageDefined => {
                    next.is_whitespace()
                        || lang.autoclose_before.contains(next)
                        || before_closer(lang, after)
                }
                AutoClosing::Never => false,
            };
            if !allowed {
                return None;
            }
        }
        let before = line[..col].chars().next_back();
        // `don't`: no `'` or `"` pair after a word char (VS Code skips this
        // with `always`, and for other quotes such as backticks).
        if matches!(ch, "'" | "\"")
            && setting != AutoClosing::Always
            && before.is_some_and(|c| is_word_char(c, &ctx.config.word_separators))
        {
            return None;
        }
        // The scope of the char before the cursor, as VS Code reads the
        // token before it: the end of a line comment is in the comment.
        if !pair.not_in.is_empty()
            && let Some(syntax) = ctx.syntax
        {
            let at = before.map_or(s.head, |c| s.head - c.len_utf8());
            let scope = match syntax.scope_at(snap, at) {
                Scope::String => "string",
                Scope::Comment => "comment",
                Scope::Code => "",
            };
            if !scope.is_empty() && pair.not_in.iter().any(|n| n == scope) {
                return None;
            }
        }
    }
    let text = format!("{}{}", pair.start, pair.end);
    let items = set
        .all()
        .iter()
        .map(|s| Item {
            id: s.id,
            range: s.head..s.head,
            text: text.clone(),
            place: Place::At(pair.start.len()),
        })
        .collect();
    let mut plan = build(items, set.primary().id, kind, stop_before, false);
    plan.autoclose = plan
        .selections
        .all()
        .iter()
        .map(|s| s.head - pair.start.len()..s.head + pair.end.len())
        .collect();
    Some(plan)
}

/// The pair whose opener ends at `offset` and whose closer starts there
/// (`(|)`), and the range of both.
fn pair_around<'a>(
    lang: &'a LanguageEditing,
    snapshot: &BufferSnapshot,
    offset: usize,
) -> Option<(&'a BracketPair, Range<usize>)> {
    let (line, col) = line_at(snapshot, offset);
    let pair = lang.brackets.iter().find(|p| {
        line[..col].ends_with(p.start.as_str()) && line[col..].starts_with(p.end.as_str())
    })?;
    Some((pair, offset - pair.start.len()..offset + pair.end.len()))
}

/// Whether `delete_left` deletes both chars of the empty pair at every
/// cursor (`editor.autoClosingDelete`): with `auto`, only a pair a region
/// says the editor inserted.
///
/// Adapted from microsoft/vscode src/vs/editor/common/cursor/cursorDeleteOperations.ts (MIT)
/// (`isAutoClosingPairDelete`).
pub(crate) fn deletes_pairs(ctx: Ctx, set: &SelectionSet, regions: &[Range<usize>]) -> bool {
    let c = ctx.config;
    if c.auto_closing_delete == AutoClosingEdit::Never
        || (c.auto_closing_brackets == AutoClosing::Never
            && c.auto_closing_quotes == AutoClosing::Never)
    {
        return false;
    }
    let lang = language(&ctx);
    set.all().iter().all(|s| {
        if !s.is_empty() {
            return false;
        }
        let Some((pair, range)) = pair_around(lang, ctx.snapshot, s.head) else {
            return false;
        };
        let setting = if is_quote(pair) {
            c.auto_closing_quotes
        } else {
            c.auto_closing_brackets
        };
        setting != AutoClosing::Never
            && (c.auto_closing_delete == AutoClosingEdit::Always
                || regions
                    .iter()
                    .any(|r| r.end == range.end && r.start <= range.start))
    })
}

/// The range of the empty pair around `offset` (`(|)`), if any.
pub(crate) fn empty_pair_at(ctx: Ctx, offset: usize) -> Option<Range<usize>> {
    pair_around(language(&ctx), ctx.snapshot, offset).map(|(_, range)| range)
}

/// The char at `offset` (`\n` at a line end).
fn char_at(snapshot: &BufferSnapshot, offset: usize) -> Option<char> {
    if offset >= snapshot.len_bytes() {
        return None;
    }
    let (line, col) = line_at(snapshot, offset);
    Some(line[col..].chars().next().unwrap_or('\n'))
}

/// The char before `offset` (`\n` at a line start).
fn char_before(snapshot: &BufferSnapshot, offset: usize) -> Option<char> {
    if offset == 0 {
        return None;
    }
    let (line, col) = line_at(snapshot, offset);
    Some(line[..col].chars().next_back().unwrap_or('\n'))
}

/// Maps auto-closed regions through an edit (`patch`) from `old` to `new`.
/// Like VS Code's `NeverGrowsWhenTypingAtEdges` decorations, text typed at
/// a region's outer edges stays outside it. A region goes when its text no
/// longer starts and ends with the chars it had.
pub fn remap_regions(
    regions: &[Range<usize>],
    old: &BufferSnapshot,
    new: &BufferSnapshot,
    patch: &Patch,
) -> Vec<Range<usize>> {
    regions
        .iter()
        .filter_map(|r| {
            let (first, last) = (char_at(old, r.start)?, char_before(old, r.end)?);
            let start = patch.map_offset(r.start, Bias::Right);
            let end = patch.map_offset(r.end, Bias::Left);
            let fits = end >= start + first.len_utf8() + last.len_utf8();
            (fits && char_at(new, start) == Some(first) && char_before(new, end) == Some(last))
                .then_some(start..end)
        })
        .collect()
}

/// Keeps the regions some selection is strictly inside: a cursor that
/// left the pair (even onto its edge) forgets it.
pub fn retain_regions(regions: &mut Vec<Range<usize>>, set: &SelectionSet) {
    regions.retain(|r| {
        set.all().iter().any(|s| {
            let sr = s.range();
            sr.start > r.start && sr.end < r.end
        })
    });
}

#[cfg(test)]
// One-region lists are slices of regions here, not ranges to expand.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use lucerna_syntax::{LanguageRegistry, ParseOutcome, SyntaxState};

    use super::*;
    use crate::commands::{delete_left, insert};
    use crate::plan::EditConfig;
    use crate::test_util::{marked, render_plan, test_config};

    fn editing(name: &str) -> LanguageEditing {
        LanguageRegistry::bundled()
            .language_by_name(name)
            .unwrap_or_else(|| panic!("no language {name}"))
            .editing()
            .clone()
    }

    /// Types `ch` into `src` in language `lang` (parsed), with `regions`.
    /// Returns the rendered result and the plan's new regions.
    fn type_in(
        lang: &str,
        src: &str,
        ch: &str,
        regions: &[Range<usize>],
        config: &EditConfig,
    ) -> (String, Vec<Range<usize>>) {
        let (snapshot, set) = marked(src);
        let registry = Arc::new(LanguageRegistry::bundled());
        let language = registry.language_by_name(lang).unwrap();
        let mut syntax = SyntaxState::new(language.clone(), registry.clone()).unwrap();
        assert_eq!(syntax.parse(&snapshot, None), ParseOutcome::Done);
        let ctx = Ctx {
            snapshot: &snapshot,
            config,
            language: Some(language.editing()),
            syntax: Some(&syntax),
        };
        let plan = insert(ctx, &set, ch, EditKind::TypingOther, regions);
        (render_plan(&snapshot, &plan), plan.autoclose)
    }

    fn rust(src: &str, ch: &str, regions: &[Range<usize>]) -> String {
        type_in("Rust", src, ch, regions, &test_config()).0
    }

    fn deleted(src: &str, regions: &[Range<usize>], config: &EditConfig) -> String {
        let (snapshot, set) = marked(src);
        let lang = editing("Rust");
        let ctx = Ctx {
            snapshot: &snapshot,
            config,
            language: Some(&lang),
            syntax: None,
        };
        render_plan(&snapshot, &delete_left(ctx, &set, regions))
    }

    #[test]
    fn autoclose_bracket() {
        let (out, regions) = type_in("Rust", "|", "(", &[], &test_config());
        assert_eq!(out, "(|)");
        assert_eq!(regions, [0..2]);
        // `b` is not in `autoclose_before`.
        assert_eq!(rust("a|b", "(", &[]), "a(|b");
        // Before a closer, a space or the end of the line.
        assert_eq!(rust("f(|)", "[", &[]), "f([|])");
        assert_eq!(rust("a| b", "{", &[]), "a{|} b");
        assert_eq!(rust("a|\nb", "[", &[]), "a[|]\nb");
    }

    #[test]
    fn autoclose_kind_groups_with_typing() {
        let (snapshot, set) = marked("a|");
        let config = test_config();
        let lang = editing("Rust");
        let ctx = Ctx {
            snapshot: &snapshot,
            config: &config,
            language: Some(&lang),
            syntax: None,
        };
        let p = insert(ctx, &set, "(", EditKind::TypingOther, &[]);
        assert_eq!(
            (p.kind, p.stop_before, p.stop_after),
            (EditKind::TypingOther, false, false)
        );
        let p = insert(ctx, &set, ")", EditKind::TypingOther, &[]);
        assert_eq!(p.kind, EditKind::TypingOther);
    }

    #[test]
    fn quotes_do_not_close_in_strings_or_after_words() {
        assert_eq!(rust("let s = \"a|\"", "\"", &[]), "let s = \"a\"|\"");
        let ts = |src: &str, ch: &str| type_in("TypeScript", src, ch, &[], &test_config()).0;
        assert_eq!(ts("don|", "'"), "don'|");
        assert_eq!(ts("x = |", "'"), "x = '|'");
        assert_eq!(ts("// a |", "'"), "// a '|");
        // Only `'` and `"` skip the pair after a word char.
        assert_eq!(ts("html|", "`"), "html`|`");
        // Inside a string, with a non-word char before and a space after.
        assert_eq!(
            rust("let s = \"a | b\";", "\"", &[]),
            "let s = \"a \"| b\";"
        );
        // In code, a quote closes.
        assert_eq!(rust("let s = |;", "\"", &[]), "let s = \"|\";");
    }

    #[test]
    fn overtype_closer_in_region() {
        assert_eq!(rust("(|)", ")", &[0..2]), "()|");
        // Typed text inside the pair: the closer is still the region's.
        assert_eq!(rust("(ab|)", ")", &[0..4]), "(ab)|");
        // `auto`: a closer the editor did not insert is typed.
        assert_eq!(rust("(|)", ")", &[]), "()|)");
        let mut config = test_config();
        config.auto_closing_overtype = AutoClosingEdit::Always;
        assert_eq!(type_in("Rust", "(|)", ")", &[], &config).0, "()|");
        config.auto_closing_overtype = AutoClosingEdit::Never;
        assert_eq!(type_in("Rust", "(|)", ")", &[0..2], &config).0, "()|)");
    }

    #[test]
    fn autodelete_empty_pair() {
        let config = test_config();
        assert_eq!(deleted("(|)", &[0..2], &config), "|");
        assert_eq!(deleted("a[|]\nb[|]", &[1..3, 5..7], &config), "a|\nb|");
        // `auto`: only a pair the editor inserted.
        assert_eq!(deleted("(|)", &[], &config), "|)");
        let mut config = test_config();
        config.auto_closing_delete = AutoClosingEdit::Always;
        assert_eq!(deleted("(|)", &[], &config), "|");
        config.auto_closing_delete = AutoClosingEdit::Never;
        assert_eq!(deleted("(|)", &[0..2], &config), "|)");
    }

    #[test]
    fn surround_selection() {
        assert_eq!(rust("«ab»", "(", &[]), "(«ab»)");
        assert_eq!(rust("«ab»", "\"", &[]), "\"«ab»\"");
        assert_eq!(rust("«a» «b»", "[", &[]), "[«a»] [«b»]");
        // Whitespace only: replaced.
        assert_eq!(rust("a«  »b", "(", &[]), "a(|b");
        let mut config = test_config();
        config.auto_surround = AutoSurround::Quotes;
        assert_eq!(type_in("Rust", "«ab»", "(", &[], &config).0, "(|");
        assert_eq!(type_in("Rust", "«ab»", "\"", &[], &config).0, "\"«ab»\"");
    }

    #[test]
    fn surround_is_its_own_undo_step() {
        let (snapshot, set) = marked("«ab»");
        let config = test_config();
        let lang = editing("Rust");
        let ctx = Ctx {
            snapshot: &snapshot,
            config: &config,
            language: Some(&lang),
            syntax: None,
        };
        let p = insert(ctx, &set, "(", EditKind::TypingOther, &[]);
        assert_eq!(
            (p.kind, p.stop_before, p.stop_after),
            (EditKind::Other, true, true)
        );
    }

    #[test]
    fn never_setting_disables() {
        let mut config = test_config();
        config.auto_closing_brackets = AutoClosing::Never;
        assert_eq!(type_in("Rust", "|", "(", &[], &config).0, "(|");
        // Quotes have their own setting.
        assert_eq!(type_in("Rust", "|", "\"", &[], &config).0, "\"|\"");
        config.auto_surround = AutoSurround::Never;
        assert_eq!(type_in("Rust", "«ab»", "(", &[], &config).0, "(|");
    }

    #[test]
    fn closing_settings_before_text() {
        let mut config = test_config();
        config.auto_closing_brackets = AutoClosing::Always;
        assert_eq!(type_in("Rust", "a|b", "(", &[], &config).0, "a(|)b");
        config.auto_closing_brackets = AutoClosing::BeforeWhitespace;
        assert_eq!(type_in("Rust", "a|;", "(", &[], &config).0, "a(|;");
        assert_eq!(type_in("Rust", "a| ", "(", &[], &config).0, "a(|) ");
    }

    #[test]
    fn multi_cursor_autoclose() {
        let (out, regions) = type_in("Rust", "|\n|", "[", &[], &test_config());
        assert_eq!(out, "[|]\n[|]");
        assert_eq!(regions, [0..2, 3..5]);
    }

    #[test]
    fn regions_follow_edits() {
        let text = |s: &str| marked(&format!("{s}|")).0;
        let old = text("(a)");
        let new = text("x(ab)");
        let patch = Patch::new(vec![
            lucerna_text::PatchEdit {
                old: 0..0,
                new: 0..1,
            },
            lucerna_text::PatchEdit {
                old: 2..2,
                new: 3..4,
            },
        ]);
        assert_eq!(remap_regions(&[0..3], &old, &new, &patch), [1..5]);
        // Typed right after the closer: outside the region.
        let new = text("(a)x");
        let patch = Patch::new(vec![lucerna_text::PatchEdit {
            old: 3..3,
            new: 3..4,
        }]);
        assert_eq!(remap_regions(&[0..3], &old, &new, &patch), [0..3]);
        // The closer was deleted: the region goes.
        let new = text("(a");
        let patch = Patch::new(vec![lucerna_text::PatchEdit {
            old: 2..3,
            new: 2..2,
        }]);
        assert!(remap_regions(&[0..3], &old, &new, &patch).is_empty());
    }

    #[test]
    fn regions_need_a_cursor_inside() {
        let mut regions = vec![0..2, 3..7];
        retain_regions(&mut regions, &marked("(|)\n(ab)|").1);
        assert_eq!(regions, [0..2]);
        retain_regions(&mut regions, &marked("|()").1);
        assert!(regions.is_empty());
    }
}
