use similar::{Algorithm, DiffOp, TextDiff};
use std::ops::Range;
use std::time::Duration;

/// A line diff taking longer than this stops refining: the rest of the
/// text becomes coarser hunks (a mostly rewritten large file can otherwise
/// take seconds).
const DIFF_TIMEOUT: Duration = Duration::from_millis(500);

/// Files larger than this get no diff.
pub const MAX_DIFF_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub kind: HunkKind,
    /// 0-based rows of the current text. For `Deleted`: the empty range at the
    /// row that follows the deletion.
    pub rows: Range<u32>,
    /// Base rows that were removed or replaced (empty for `Added`).
    pub base_rows: Range<u32>,
}

/// Line-based diff of two LF texts, refined for at most [`DIFF_TIMEOUT`].
pub fn diff_hunks(base: &str, current: &str) -> Vec<Hunk> {
    diff_hunks_within(base, current, DIFF_TIMEOUT)
}

fn diff_hunks_within(base: &str, current: &str, timeout: Duration) -> Vec<Hunk> {
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .timeout(timeout)
        .diff_lines(base, current);
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut open: Option<(Range<usize>, Range<usize>)> = None;
    let flush = |open: &mut Option<(Range<usize>, Range<usize>)>, hunks: &mut Vec<Hunk>| {
        if let Some((old, new)) = open.take() {
            let kind = match (old.is_empty(), new.is_empty()) {
                (true, _) => HunkKind::Added,
                (_, true) => HunkKind::Deleted,
                _ => HunkKind::Modified,
            };
            hunks.push(Hunk {
                kind,
                rows: new.start as u32..new.end as u32,
                base_rows: old.start as u32..old.end as u32,
            });
        }
    };
    for op in diff.ops() {
        if let DiffOp::Equal { .. } = op {
            flush(&mut open, &mut hunks);
            continue;
        }
        let (old, new) = (op.old_range(), op.new_range());
        match &mut open {
            // Adjacent non-equal ops (delete followed by insert) form one hunk.
            Some((o, n)) => {
                o.end = old.end;
                n.end = new.end;
            }
            None => open = Some((old, new)),
        }
    }
    flush(&mut open, &mut hunks);
    hunks
}

/// Decodes base-side bytes to LF text; `None` for binary or files over 8 MB.
pub fn load_base_text(bytes: &[u8]) -> Option<String> {
    if bytes.len() as u64 > MAX_DIFF_BYTES {
        return None;
    }
    lucerna_text::decode_file(bytes, false).ok().map(|d| d.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timed_out_diff_yields_one_coarse_hunk() {
        let base = "a\nx\nb\ny\nc\n";
        let current = "a\nX\nb\nY\nc\n";
        assert_eq!(diff_hunks(base, current).len(), 2);
        // Past the deadline only the common prefix and suffix are found.
        let coarse = diff_hunks_within(base, current, Duration::ZERO);
        assert_eq!(
            coarse,
            vec![Hunk {
                kind: HunkKind::Modified,
                rows: 1..4,
                base_rows: 1..4,
            }]
        );
    }
}
