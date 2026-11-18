//! Find in a buffer, shared by the Find bar, vim's `/` and the
//! add-selection commands (`⌘D`).
//!
//! Adapted from microsoft/vscode src/vs/editor/common/model/textModelSearch.ts (MIT)
//! (`isValidMatch`: a whole-word match is bounded by the editor's word
//! separators, not by the regex `\b`).

use std::ops::Range;

use lucerna_text::BufferSnapshot;
use regex::{Regex, RegexBuilder};

use lucerna_text::words::is_word_bounded;

/// Bytes [`find_next`] copies out of the buffer per window.
const WINDOW_BYTES: usize = 256 * 1024;

/// What to look for.
#[derive(Debug, Clone, Copy)]
pub struct FindQuery<'a> {
    pub pattern: &'a str,
    pub is_regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// `editor.wordSeparators`; decides what `whole_word` accepts.
    pub word_separators: &'a str,
}

/// The regex for `pattern`: escaped unless `is_regex`, case folding unless
/// `case_sensitive`, and `^`/`$` at line ends. Whole-word is not part of the
/// regex; see [`is_whole_word`].
pub fn build_regex(pattern: &str, is_regex: bool, case_sensitive: bool) -> Result<Regex, String> {
    let source = if is_regex {
        pattern.to_owned()
    } else {
        regex::escape(pattern)
    };
    RegexBuilder::new(&source)
        .case_insensitive(!case_sensitive)
        .multi_line(true)
        .build()
        .map_err(|err| err.to_string())
}

/// The byte ranges of every non-empty match, in order. An invalid regex
/// gives its error message; an empty pattern finds nothing. Copies the
/// whole buffer once.
pub fn find_all(snap: &BufferSnapshot, q: &FindQuery) -> Result<Vec<Range<usize>>, String> {
    if q.pattern.is_empty() {
        return Ok(Vec::new());
    }
    let regex = build_regex(q.pattern, q.is_regex, q.case_sensitive)?;
    let text = snap.text_for_range(0..snap.len_bytes());
    Ok(regex
        .find_iter(&text)
        .filter(|m| !m.is_empty())
        .map(|m| m.range())
        .filter(|r| !q.whole_word || is_whole_word(snap, r.clone(), q.word_separators))
        .collect())
}

/// The first match starting at or after `from` (or, `backwards`, the last
/// one starting before it), wrapping around the buffer once. For literal
/// patterns only. Scans in windows, so it never copies the whole buffer.
pub fn find_next(
    snap: &BufferSnapshot,
    q: &FindQuery,
    from: usize,
    backwards: bool,
) -> Option<Range<usize>> {
    debug_assert!(!q.is_regex, "find_next is for literal patterns");
    if q.pattern.is_empty() {
        return None;
    }
    let regex = build_regex(q.pattern, false, q.case_sensitive).ok()?;
    let len = snap.len_bytes();
    let from = from.min(len);
    // A literal may match more bytes than it has through case folding.
    let overlap = q.pattern.len() * 4;
    let scan = |range: Range<usize>| scan_range(snap, &regex, q, range, overlap, backwards);
    if backwards {
        scan(0..from).or_else(|| scan(from..len))
    } else {
        scan(from..len).or_else(|| scan(0..from))
    }
}

/// The first (or last) valid match whose start lies in `range`.
fn scan_range(
    snap: &BufferSnapshot,
    regex: &Regex,
    q: &FindQuery,
    range: Range<usize>,
    overlap: usize,
    backwards: bool,
) -> Option<Range<usize>> {
    let len = snap.len_bytes();
    let mut lo = range.start;
    let mut hi = range.end;
    while lo < hi {
        let (chunk_lo, chunk_hi) = if backwards {
            (hi.saturating_sub(WINDOW_BYTES).max(lo), hi)
        } else {
            (lo, (lo + WINDOW_BYTES).min(hi))
        };
        let start = snap.floor_char_boundary(chunk_lo);
        let end = snap.floor_char_boundary((chunk_hi + overlap).min(len));
        let text = snap.text_for_range(start..end);
        let valid = regex
            .find_iter(&text)
            .map(|m| m.range())
            .map(|r| r.start + start..r.end + start)
            .filter(|r| {
                r.start >= chunk_lo
                    && r.start < chunk_hi
                    && !r.is_empty()
                    && (!q.whole_word || is_whole_word(snap, r.clone(), q.word_separators))
            });
        let found = if backwards {
            valid.last()
        } else {
            valid.into_iter().next()
        };
        if found.is_some() {
            return found;
        }
        if backwards {
            hi = chunk_lo;
        } else {
            lo = chunk_hi;
        }
    }
    None
}

/// Whether `range` is bounded by word separators or whitespace on both
/// sides, as VS Code's `isValidMatch` decides it: a side is a boundary at
/// the buffer edge, when the neighbouring char is not a word char, or when
/// the match's own edge char is not one.
pub fn is_whole_word(snap: &BufferSnapshot, range: Range<usize>, separators: &str) -> bool {
    let len = snap.len_bytes();
    let before = |at: usize| {
        let lo = snap.floor_char_boundary(at.saturating_sub(4));
        snap.text_for_range(lo..at).chars().next_back()
    };
    let after = |at: usize| {
        let hi = snap.floor_char_boundary((at + 4).min(len));
        snap.text_for_range(at..hi).chars().next()
    };
    // VS Code matches line by line, so a line break is a boundary too.
    is_word_bounded(
        before(range.start),
        after(range.start),
        before(range.end),
        after(range.end),
        separators,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::test_config;

    fn query<'a>(pattern: &'a str, whole_word: bool, separators: &'a str) -> FindQuery<'a> {
        FindQuery {
            pattern,
            is_regex: false,
            case_sensitive: true,
            whole_word,
            word_separators: separators,
        }
    }

    #[test]
    fn whole_word_uses_separators() {
        let snap = BufferSnapshot::from_text("$foo foo-bar $foo");
        let separators: String = test_config()
            .word_separators
            .chars()
            .filter(|c| !matches!(c, '$' | '-'))
            .collect();
        assert_eq!(
            find_all(&snap, &query("$foo", true, &separators)).unwrap(),
            vec![0..4, 13..17]
        );
        assert_eq!(
            find_all(&snap, &query("foo-bar", true, &separators)).unwrap(),
            vec![5..12]
        );
    }

    #[test]
    fn whole_word_default_separators() {
        let snap = BufferSnapshot::from_text("foo foobar (foo)");
        let separators = test_config().word_separators;
        assert_eq!(
            find_all(&snap, &query("foo", true, &separators)).unwrap(),
            vec![0..3, 12..15]
        );
    }

    #[test]
    fn find_next_wraps_and_backwards() {
        let snap = BufferSnapshot::from_text("a x a x a");
        let q = query("a", false, "");
        assert_eq!(find_next(&snap, &q, 3, false), Some(4..5));
        assert_eq!(find_next(&snap, &q, 9, false), Some(0..1));
        assert_eq!(find_next(&snap, &q, 4, true), Some(0..1));
    }

    #[test]
    fn find_next_across_window_boundary() {
        let mut text = "x".repeat(600 * 1024);
        text.replace_range(262_140..262_146, "needle");
        let snap = BufferSnapshot::from_text(&text);
        let q = query("needle", false, "");
        assert_eq!(find_next(&snap, &q, 0, false), Some(262_140..262_146));
        assert_eq!(
            find_next(&snap, &q, text.len(), true),
            Some(262_140..262_146)
        );
    }

    #[test]
    fn find_next_case_folding_multibyte() {
        let snap = BufferSnapshot::from_text("CHÀO chào");
        let q = FindQuery {
            case_sensitive: false,
            ..query("chào", false, "")
        };
        assert_eq!(find_next(&snap, &q, 0, false), Some(0..5));
        assert_eq!(find_next(&snap, &q, 5, false), Some(6..11));
    }

    #[test]
    fn invalid_regex_errors() {
        let snap = BufferSnapshot::from_text("a (b)");
        let q = FindQuery {
            is_regex: true,
            ..query("(", false, "")
        };
        assert!(find_all(&snap, &q).is_err());
    }
}
