//! Syntax colors for search-result excerpts. Pure and blocking: it parses
//! the result file with tree-sitter, so it runs on a background thread and
//! never touches gpui.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use lucerna_project::{Excerpt, LineMatch};
use lucerna_syntax::{LanguageRegistry, SyntaxState};
use lucerna_text::{Buffer, Point};

use crate::tasks::catch_panic;

/// Files above this size are shown plain.
pub const MAX_HIGHLIGHT_BYTES: u64 = 2 * 1024 * 1024;

/// Byte ranges within `LineMatch::line_text`, with capture names.
pub type LineSpans = Vec<(Range<usize>, Arc<str>)>;
/// Keyed by 1-based `line_number`.
pub type FileHighlights = HashMap<u32, LineSpans>;

/// Blocking; call on a background thread. Empty when the file is too big,
/// cannot be loaded, has no language or its grammar fails. A line whose text
/// no longer starts with the excerpt's `line_text` (the file changed since
/// the search) is left out and shows plain.
pub fn highlight_file(
    abs_path: &Path,
    rel_path: &Path,
    excerpts: &[Excerpt],
    registry: &Arc<LanguageRegistry>,
    styled: &dyn Fn(&str) -> bool,
) -> FileHighlights {
    let too_big = std::fs::metadata(abs_path).is_ok_and(|m| m.len() > MAX_HIGHLIGHT_BYTES);
    if too_big {
        return FileHighlights::new();
    }
    catch_panic("search highlight", || {
        highlight_loaded(abs_path, rel_path, excerpts, registry, styled)
    })
    .unwrap_or_default()
}

fn highlight_loaded(
    abs_path: &Path,
    rel_path: &Path,
    excerpts: &[Excerpt],
    registry: &Arc<LanguageRegistry>,
    styled: &dyn Fn(&str) -> bool,
) -> FileHighlights {
    let mut out = FileHighlights::new();
    let Ok(buffer) = Buffer::load(abs_path) else {
        return out;
    };
    let snapshot = buffer.snapshot();
    let Some(language) = registry.language_for_file(rel_path, &snapshot.line(0)) else {
        return out;
    };
    let Ok(mut syntax) = SyntaxState::new(language, registry.clone()) else {
        return out;
    };
    syntax.parse(&snapshot, None);
    let line_count = snapshot.line_count();

    for excerpt in excerpts {
        let rows: Vec<(u32, &LineMatch)> = excerpt
            .lines
            .iter()
            .filter(|l| l.line_number >= 1 && l.line_number <= line_count)
            .map(|l| (l.line_number - 1, l))
            .collect();
        let (Some(first), Some(last)) = (
            rows.iter().map(|(r, _)| *r).min(),
            rows.iter().map(|(r, _)| *r).max(),
        ) else {
            continue;
        };
        let spans = syntax.highlights(&snapshot, first..last + 1, styled);
        for (row, line) in rows {
            if !snapshot.line(row).starts_with(line.line_text.as_str()) {
                continue;
            }
            let start = snapshot.point_to_offset(Point { row, column: 0 });
            let len = line.line_text.len();
            let mut line_spans = LineSpans::new();
            for span in &spans {
                let from = span.range.start.max(start) - start;
                let to = span.range.end.saturating_sub(start).min(len);
                if span.range.end <= start || from >= to {
                    continue;
                }
                // Row text starts with line_text, so these are boundaries
                // unless the cut fell inside a char; line_text is a prefix
                // ending on a boundary, but spans may not be on one.
                if line.line_text.is_char_boundary(from) && line.line_text.is_char_boundary(to) {
                    line_spans.push((from..to, span.capture.clone()));
                }
            }
            out.insert(line.line_number, line_spans);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(n: u32, text: &str) -> LineMatch {
        LineMatch {
            line_number: n,
            line_text: text.into(),
            ranges: Vec::new(),
        }
    }

    fn run(name: &str, content: &[u8], lines: Vec<LineMatch>) -> FileHighlights {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().join(name);
        std::fs::write(&abs, content).unwrap();
        let registry = Arc::new(LanguageRegistry::bundled());
        highlight_file(
            &abs,
            Path::new(name),
            &[Excerpt { lines }],
            &registry,
            &|_| true,
        )
    }

    #[test]
    fn rust_keyword_gets_a_capture() {
        let h = run("a.rs", b"fn main() {}\n", vec![line(1, "fn main() {}")]);
        let spans = &h[&1];
        assert!(
            spans
                .iter()
                .any(|(r, c)| *r == (0..2) && c.starts_with("keyword")),
            "{spans:?}"
        );
    }

    #[test]
    fn multi_line_comment_colors_inner_line() {
        let h = run("a.rs", b"/*\nneedle\n*/\n", vec![line(2, "needle")]);
        let spans = &h[&2];
        assert!(
            spans.iter().any(|(_, c)| c.starts_with("comment")),
            "{spans:?}"
        );
    }

    #[test]
    fn changed_line_is_left_plain() {
        let h = run("a.rs", b"fn new() {}\n", vec![line(1, "old")]);
        assert!(!h.contains_key(&1));
    }

    #[test]
    fn unknown_language_is_empty() {
        let h = run(
            "x.unknownext",
            b"fn main() {}\n",
            vec![line(1, "fn main() {}")],
        );
        assert!(h.is_empty());
    }

    #[test]
    fn oversized_file_is_empty() {
        let mut content = b"fn main() {}\n".to_vec();
        content.resize(MAX_HIGHLIGHT_BYTES as usize + 1, b' ');
        let h = run("a.rs", &content, vec![line(1, "fn main() {}")]);
        assert!(h.is_empty());
    }

    #[test]
    fn spans_are_clipped_to_cut_line() {
        let h = run(
            "a.rs",
            b"fn main() { let value = 1; }\n",
            vec![line(1, "fn main() ")],
        );
        let spans = &h[&1];
        assert!(!spans.is_empty());
        assert!(spans.iter().all(|(r, _)| r.end <= 10), "{spans:?}");
    }
}
