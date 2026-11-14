//! Builds the excerpts of a diff tab from the hunks of one file.

use lucerna_git::{Hunk, HunkKind, diff_hunks};
use lucerna_project::{Excerpt, LineKind, LineMatch, MAX_LINE_CHARS};

/// Unchanged lines shown on each side of a hunk.
pub(crate) const CONTEXT_LINES: u32 = 2;

/// The excerpts of one file plus its line counts.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct BuiltFile {
    pub excerpts: Vec<Excerpt>,
    /// Lines the current text added.
    pub added: u32,
    /// Base lines the current text removed.
    pub removed: u32,
}

/// Builds the excerpts of `current` against `base`.
///
/// With `full_file` there is one excerpt with every current line and the
/// deleted lines interleaved. Otherwise each hunk gets `context` unchanged
/// lines on each side, and two excerpts merge when their context ranges
/// overlap or touch (with 2 lines of context, hunks leaving at most 4
/// unchanged lines between them merge). A modified hunk is its base lines
/// (deleted) followed by its current lines (added).
pub(crate) fn build_file(base: &str, current: &str, context: u32, full_file: bool) -> BuiltFile {
    let hunks = diff_hunks(base, current);
    let base_lines: Vec<&str> = base.lines().collect();
    let cur_lines: Vec<&str> = current.lines().collect();
    let total = u32::try_from(cur_lines.len()).unwrap_or(u32::MAX);

    let mut built = BuiltFile::default();
    for h in &hunks {
        if h.kind != HunkKind::Deleted {
            built.added += h.rows.end - h.rows.start;
        }
        if h.kind != HunkKind::Added {
            built.removed += h.base_rows.end - h.base_rows.start;
        }
    }

    // Groups of consecutive hunks plus the row range each group shows.
    let mut groups: Vec<(u32, u32, Vec<&Hunk>)> = Vec::new();
    if full_file {
        if !(hunks.is_empty() && cur_lines.is_empty()) {
            groups.push((0, total, hunks.iter().collect()));
        }
    } else {
        for h in &hunks {
            let start = h.rows.start.saturating_sub(context);
            let end = h.rows.end.saturating_add(context).min(total);
            match groups.last_mut() {
                Some(g) if start <= g.1 => {
                    g.1 = g.1.max(end);
                    g.2.push(h);
                }
                _ => groups.push((start, end, vec![h])),
            }
        }
    }

    for (start, end, group) in groups {
        let mut lines = Vec::new();
        let mut row = start;
        for h in group {
            push_normal(&mut lines, &cur_lines, row..h.rows.start);
            if h.kind != HunkKind::Added {
                for i in h.base_rows.clone() {
                    let text = base_lines.get(i as usize).copied().unwrap_or("");
                    lines.push(line(
                        h.rows.start + 1,
                        text,
                        LineKind::Deleted { base_line: i + 1 },
                    ));
                }
            }
            if h.kind != HunkKind::Deleted {
                for r in h.rows.clone() {
                    let text = cur_lines.get(r as usize).copied().unwrap_or("");
                    lines.push(line(r + 1, text, LineKind::Added));
                }
            }
            row = h.rows.end;
        }
        push_normal(&mut lines, &cur_lines, row..end);
        built.excerpts.push(Excerpt { lines });
    }
    built
}

fn push_normal(out: &mut Vec<LineMatch>, cur: &[&str], rows: std::ops::Range<u32>) {
    for r in rows {
        let text = cur.get(r as usize).copied().unwrap_or("");
        out.push(line(r + 1, text, LineKind::Normal));
    }
}

/// A row, with its text capped like search results.
fn line(line_number: u32, text: &str, kind: LineKind) -> LineMatch {
    let keep = text
        .char_indices()
        .nth(MAX_LINE_CHARS)
        .map_or(text.len(), |(i, _)| i);
    LineMatch {
        line_number,
        line_text: text[..keep].to_owned(),
        ranges: Vec::new(),
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nums(e: &Excerpt) -> Vec<(u32, LineKind)> {
        e.lines.iter().map(|l| (l.line_number, l.kind)).collect()
    }
    fn lines(n: usize) -> String {
        (1..=n).map(|i| format!("l{i}\n")).collect()
    }
    /// `lines(n)` with the given 1-based lines changed.
    fn changed(n: usize, at: &[usize]) -> String {
        (1..=n)
            .map(|i| {
                if at.contains(&i) {
                    format!("X{i}\n")
                } else {
                    format!("l{i}\n")
                }
            })
            .collect()
    }

    #[test]
    fn modified_hunk_is_deleted_then_added() {
        let b = build_file("a\nb\nc\n", "a\nB\nc\n", 0, false);
        assert_eq!(b.excerpts.len(), 1);
        let l = &b.excerpts[0].lines;
        assert_eq!(l[0].line_text, "b");
        assert_eq!(l[0].kind, LineKind::Deleted { base_line: 2 });
        assert_eq!(l[0].line_number, 2);
        assert!(l[0].ranges.is_empty());
        assert_eq!(
            (l[1].line_text.as_str(), l[1].kind, l[1].line_number),
            ("B", LineKind::Added, 2)
        );
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn context_two_lines_and_merge_touching() {
        // Single-line hunks whose rows are 5 apart: contexts touch -> merge.
        let b = build_file(&lines(20), &changed(20, &[5, 10]), CONTEXT_LINES, false);
        assert_eq!(b.excerpts.len(), 1);
        // 6 apart: one unchanged line is left out -> two excerpts.
        let b = build_file(&lines(20), &changed(20, &[5, 11]), CONTEXT_LINES, false);
        assert_eq!(b.excerpts.len(), 2);
        let normals: Vec<u32> = b.excerpts[0]
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Normal)
            .map(|l| l.line_number)
            .collect();
        assert_eq!(normals, vec![3, 4, 6, 7]);
    }

    #[test]
    fn untracked_all_added() {
        let b = build_file("", "a\nb\n", CONTEXT_LINES, false);
        assert_eq!(b.excerpts.len(), 1);
        assert_eq!(
            nums(&b.excerpts[0]),
            vec![(1, LineKind::Added), (2, LineKind::Added)]
        );
    }

    #[test]
    fn deleted_file_all_deleted() {
        let b = build_file("a\nb\n", "", CONTEXT_LINES, false);
        assert_eq!(b.excerpts.len(), 1);
        assert_eq!(
            nums(&b.excerpts[0]),
            vec![
                (1, LineKind::Deleted { base_line: 1 }),
                (1, LineKind::Deleted { base_line: 2 })
            ]
        );
        assert_eq!((b.added, b.removed), (0, 2));
    }

    #[test]
    fn full_file_includes_unchanged_lines() {
        let b = build_file("a\nb\nc\nd\n", "a\nB\nc\nd\ne\n", CONTEXT_LINES, true);
        assert_eq!(b.excerpts.len(), 1);
        assert_eq!(
            nums(&b.excerpts[0]),
            vec![
                (1, LineKind::Normal),
                (2, LineKind::Deleted { base_line: 2 }),
                (2, LineKind::Added),
                (3, LineKind::Normal),
                (4, LineKind::Normal),
                (5, LineKind::Added),
            ]
        );
    }

    #[test]
    fn deleted_at_end_of_file_and_unchanged() {
        let b = build_file("a\nb\n", "a\n", 2, false);
        assert_eq!(
            nums(&b.excerpts[0]),
            vec![
                (1, LineKind::Normal),
                (2, LineKind::Deleted { base_line: 2 })
            ]
        );
        assert!(build_file("a\n", "a\n", 2, false).excerpts.is_empty());
        assert!(build_file("a\n", "a\n", 2, true).excerpts.len() == 1);
        assert!(build_file("", "", 2, true).excerpts.is_empty());
    }

    #[test]
    fn stats_count_lines() {
        let b = build_file("a\nb\nc\n", "a\nB\nB2\nc\nd\n", CONTEXT_LINES, false);
        assert_eq!((b.added, b.removed), (3, 1));
    }

    #[test]
    fn long_lines_are_capped() {
        let long = "x".repeat(MAX_LINE_CHARS + 50);
        let b = build_file("", &format!("{long}\n"), 0, false);
        assert_eq!(
            b.excerpts[0].lines[0].line_text.chars().count(),
            MAX_LINE_CHARS
        );
    }
}
