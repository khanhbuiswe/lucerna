//! Smart select: grow each selection to the next larger word, syntax node,
//! line or the whole buffer (`editor.action.smartSelect.expand`).
//!
//! Behaviour modelled on microsoft/vscode src/vs/editor/contrib/smartSelect/browser/smartSelect.ts
//! and wordSelections.ts (MIT): the word at the cursor comes first, then the
//! syntax ranges, then the line.

use std::ops::Range;

use lucerna_syntax::SyntaxState;
use lucerna_text::BufferSnapshot;

use crate::selection::{Selection, SelectionSet};
use crate::words::{CharClass, classify};

/// Grows every selection to the smallest candidate that strictly contains
/// it. A selection with no larger candidate stays as it is.
pub fn expand(
    snapshot: &BufferSnapshot,
    syntax: Option<&SyntaxState>,
    set: &SelectionSet,
    word_separators: &str,
) -> SelectionSet {
    let selections = set
        .all()
        .iter()
        .map(|s| {
            let range = s.range();
            match next_range(snapshot, syntax, &range, word_separators) {
                Some(next) => {
                    let (anchor, head) = if s.reversed() {
                        (next.end, next.start)
                    } else {
                        (next.start, next.end)
                    };
                    Selection {
                        anchor,
                        head,
                        goal: crate::Goal::None,
                        ..*s
                    }
                }
                None => *s,
            }
        })
        .collect();
    set.rebuild(selections, set.primary().id)
}

fn next_range(
    snapshot: &BufferSnapshot,
    syntax: Option<&SyntaxState>,
    range: &Range<usize>,
    separators: &str,
) -> Option<Range<usize>> {
    let mut candidates: Vec<Range<usize>> = Vec::new();
    candidates.extend(word_around(snapshot, range, separators));
    if let Some(syntax) = syntax {
        candidates.extend(syntax.enclosing_nodes(range.clone()));
    }
    candidates.extend(line_ranges(snapshot, range));
    candidates.push(0..snapshot.len_bytes());
    candidates
        .into_iter()
        .filter(|c| c.start <= range.start && c.end >= range.end && c != range)
        .min_by_key(|c| c.end - c.start)
}

/// The word touching `range.start` when `range` lies inside it.
pub(crate) fn word_around(
    snapshot: &BufferSnapshot,
    range: &Range<usize>,
    separators: &str,
) -> Option<Range<usize>> {
    let p = snapshot.offset_to_point(range.start);
    let line = snapshot.line(p.row);
    let base = range.start - (p.column as usize).min(line.len());
    let col = (p.column as usize).min(line.len());
    let is_word = |c: char| classify(c, separators) == CharClass::Regular;
    let mut start = col;
    for (i, c) in line[..col].char_indices().rev() {
        if !is_word(c) {
            break;
        }
        start = i;
    }
    let mut end = col;
    for (i, c) in line[col..].char_indices() {
        if !is_word(c) {
            break;
        }
        end = col + i + c.len_utf8();
    }
    (start < end && base + end >= range.end).then_some(base + start..base + end)
}

/// The text of the lines `range` touches, without and with their
/// indentation and trailing blanks.
fn line_ranges(snapshot: &BufferSnapshot, range: &Range<usize>) -> Vec<Range<usize>> {
    let first = snapshot.offset_to_point(range.start).row;
    let last = snapshot.offset_to_point(range.end).row;
    let line_start = |row: u32| snapshot.point_to_offset(lucerna_text::Point { row, column: 0 });
    let start = line_start(first);
    let end = line_start(last) + snapshot.line(last).len();
    let first_line = snapshot.line(first);
    let last_line = snapshot.line(last);
    let lead = first_line.len() - first_line.trim_start_matches([' ', '\t']).len();
    let trail = last_line.len() - last_line.trim_end_matches([' ', '\t']).len();
    let mut out = Vec::new();
    // A line that is all blanks has no content range.
    if start + lead < end - trail {
        out.push(start + lead..end - trail);
    }
    out.push(start..end);
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use lucerna_syntax::{LanguageRegistry, ParseOutcome};

    use super::*;
    use crate::test_util::{marked, render, test_config};

    fn syntax_for(lang: &str, snapshot: &BufferSnapshot) -> SyntaxState {
        let registry = Arc::new(LanguageRegistry::bundled());
        let language = registry.language_by_name(lang).unwrap();
        let mut syntax = SyntaxState::new(language, registry).unwrap();
        assert_eq!(syntax.parse(snapshot, None), ParseOutcome::Done);
        syntax
    }

    fn step(
        snapshot: &BufferSnapshot,
        syntax: Option<&SyntaxState>,
        set: &SelectionSet,
    ) -> SelectionSet {
        expand(snapshot, syntax, set, &test_config().word_separators)
    }

    #[test]
    fn expand_word_then_node_then_line() {
        let (snapshot, mut set) = marked("fn a() { let x = foo(b|ar); }");
        let syntax = syntax_for("rust", &snapshot);
        let mut seen = Vec::new();
        for _ in 0..12 {
            set = step(&snapshot, Some(&syntax), &set);
            let shown = render(&snapshot, &set);
            if seen.last() == Some(&shown) {
                break;
            }
            seen.push(shown);
        }
        assert_eq!(seen[0], "fn a() { let x = foo(«bar»); }");
        assert_eq!(seen[1], "fn a() { let x = foo«(bar)»; }");
        assert_eq!(seen[2], "fn a() { let x = «foo(bar)»; }");
        assert_eq!(seen[3], "fn a() { «let x = foo(bar);» }");
        assert_eq!(seen[4], "fn a() «{ let x = foo(bar); }»");
        assert_eq!(seen.last().unwrap(), "«fn a() { let x = foo(bar); }»");
    }

    #[test]
    fn expand_without_tree_uses_word_line_all() {
        let (snapshot, mut set) = marked("one\n  two th|ree\nfour");
        set = step(&snapshot, None, &set);
        assert_eq!(render(&snapshot, &set), "one\n  two «three»\nfour");
        set = step(&snapshot, None, &set);
        assert_eq!(render(&snapshot, &set), "one\n  «two three»\nfour");
        set = step(&snapshot, None, &set);
        assert_eq!(render(&snapshot, &set), "one\n«  two three»\nfour");
        set = step(&snapshot, None, &set);
        assert_eq!(render(&snapshot, &set), "«one\n  two three\nfour»");
        set = step(&snapshot, None, &set);
        assert_eq!(render(&snapshot, &set), "«one\n  two three\nfour»");
    }

    #[test]
    fn expand_keeps_reversed_direction_and_each_cursor() {
        let (snapshot, set) = marked("ab| cd|");
        let set = step(&snapshot, None, &set);
        assert_eq!(render(&snapshot, &set), "«ab» «cd»");
        let (snapshot, set) = marked("x »ab«");
        let set = step(&snapshot, None, &set);
        assert_eq!(render(&snapshot, &set), "»x ab«");
    }
}
