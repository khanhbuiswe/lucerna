//! The edits that turn a snapshot into new text, for reloads: a line diff,
//! so unchanged lines (and the cursors, folds and marks on them) stay put.

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use similar::{Algorithm, DiffTag, TextDiff};

use crate::buffer::BufferSnapshot;

/// How long the diff may search for a minimal edit script before it settles
/// for a coarser one.
const DIFF_DEADLINE: Duration = Duration::from_secs(1);

/// Sorted, non-overlapping edits against `old` that produce `new_text`
/// (LF-normalised, like the buffer). Each edit covers whole lines: a Myers
/// line diff, cut short after one second.
pub fn diff_edits(old: &BufferSnapshot, new_text: &str) -> Vec<(Range<usize>, Arc<str>)> {
    let old_text = old.text_for_range(0..old.len_bytes());
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .deadline(Instant::now() + DIFF_DEADLINE)
        .diff_lines(old_text.as_str(), new_text);
    let old_offsets = line_offsets(diff.old_slices());
    let new_offsets = line_offsets(diff.new_slices());
    let mut ranges: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    for op in diff.ops() {
        let (tag, old_lines, new_lines) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            continue;
        }
        let old_range = old_offsets[old_lines.start]..old_offsets[old_lines.end];
        let new_range = new_offsets[new_lines.start]..new_offsets[new_lines.end];
        match ranges.last_mut() {
            // A delete next to an insert is one replacement.
            Some((o, n)) if o.end == old_range.start && n.end == new_range.start => {
                o.end = old_range.end;
                n.end = new_range.end;
            }
            _ => ranges.push((old_range, new_range)),
        }
    }
    ranges
        .into_iter()
        .map(|(o, n)| (o, Arc::from(&new_text[n])))
        .collect()
}

/// The byte offset of each line start, plus the end.
fn line_offsets(lines: &[&str]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(lines.len() + 1);
    let mut at = 0;
    offsets.push(0);
    for line in lines {
        at += line.len();
        offsets.push(at);
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    type Edits = Vec<(Range<usize>, Arc<str>)>;

    fn apply(old: &str, new: &str) -> (Edits, String) {
        let mut buffer = Buffer::from_bytes(old.as_bytes(), false).unwrap();
        let edits = diff_edits(&buffer.snapshot(), new);
        buffer.edit(edits.clone());
        let s = buffer.snapshot();
        (edits, s.text_for_range(0..s.len_bytes()))
    }

    #[test]
    fn identical_text_has_no_edits() {
        assert!(apply("a\nb\n", "a\nb\n").0.is_empty());
    }

    #[test]
    fn inserted_line_is_one_insertion() {
        let (edits, text) = apply("a\nb\nc\n", "a\nX\nb\nc\n");
        assert_eq!(edits, [(2..2, Arc::from("X\n"))]);
        assert_eq!(text, "a\nX\nb\nc\n");
    }

    #[test]
    fn changed_line_is_one_replacement() {
        let (edits, text) = apply("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(edits, [(2..4, Arc::from("B\n"))]);
        assert_eq!(text, "a\nB\nc\n");
    }

    #[test]
    fn separate_changes_stay_separate() {
        let old: String = (0..10).map(|i| format!("{i}\n")).collect();
        let new = old.replace("1\n", "one\n").replace("8\n", "eight\n");
        let (edits, text) = apply(&old, &new);
        assert_eq!(edits.len(), 2);
        assert_eq!(text, new);
    }

    #[test]
    fn final_newline_and_empty_text() {
        assert_eq!(apply("a\nb", "a\nb\n").1, "a\nb\n");
        assert_eq!(apply("a\nb\n", "a\nb").1, "a\nb");
        assert_eq!(apply("a\nb\n", "").1, "");
        assert_eq!(apply("", "x\n").1, "x\n");
    }

    #[test]
    fn multibyte_lines() {
        let (_, text) = apply("xin chào\nđ\n", "xin chào\nđường\n");
        assert_eq!(text, "xin chào\nđường\n");
    }
}
