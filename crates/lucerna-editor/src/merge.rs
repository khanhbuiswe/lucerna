//! Three-way line merge (spec §7.2): bring an agent's on-disk change into a
//! buffer with unsaved edits, when the two touch different lines.

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use similar::{Algorithm, DiffTag, TextDiff};

/// How long each line diff may search for a minimal edit script before it
/// settles for a coarser one.
const DIFF_DEADLINE: Duration = Duration::from_secs(1);

/// `ours` and `theirs` changed the same or neighbouring lines of `base`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeConflict;

/// Byte-range replacements, sorted and non-overlapping.
pub type Edits = Vec<(Range<usize>, Arc<str>)>;

/// A changed run of lines: `base` lines replaced by `side` lines.
struct Hunk {
    base: Range<usize>,
    side: Range<usize>,
}

/// Edits (sorted, non-overlapping, byte ranges in `ours`) that bring
/// `theirs`' changes from `base` into `ours`.
///
/// The three-way line merge of spec §7.2: line-diff `base` against each
/// side; if any `theirs` hunk overlaps or touches an `ours` hunk the merge
/// fails whole, otherwise each `theirs` hunk becomes one edit of `ours`'
/// lines. A last line without `\n` counts as a line. Identical `ours` and
/// `theirs` are the caller's shortcut: here they usually conflict.
pub fn merge(base: &str, ours: &str, theirs: &str) -> Result<Edits, MergeConflict> {
    let ours_diff = line_diff(base, ours);
    let theirs_diff = line_diff(base, theirs);
    let ours_hunks = hunks(&ours_diff);
    let theirs_hunks = hunks(&theirs_diff);

    // Touching counts: an empty range is a point, so an insertion right at
    // the edge of the other side's change conflicts too.
    let conflicts = theirs_hunks.iter().any(|t| {
        ours_hunks
            .iter()
            .any(|o| t.base.start <= o.base.end && o.base.start <= t.base.end)
    });
    if conflicts {
        return Err(MergeConflict);
    }

    let ours_offsets = line_offsets(ours_diff.new_slices());
    let theirs_lines = theirs_diff.new_slices();
    let mut edits = Vec::with_capacity(theirs_hunks.len());
    for t in &theirs_hunks {
        // Every ours hunk is now wholly before or wholly after this one.
        let shift: isize = ours_hunks
            .iter()
            .filter(|o| o.base.end < t.base.start)
            .map(|o| o.side.len() as isize - o.base.len() as isize)
            .sum();
        let start = (t.base.start as isize + shift) as usize;
        let end = start + t.base.len();
        let text: String = theirs_lines[t.side.clone()].concat();
        edits.push((ours_offsets[start]..ours_offsets[end], Arc::from(text)));
    }
    Ok(edits)
}

fn line_diff<'a>(old: &'a str, new: &'a str) -> TextDiff<'a, 'a, 'a, str> {
    TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .deadline(Instant::now() + DIFF_DEADLINE)
        .diff_lines(old, new)
}

/// The non-equal ops of `diff` in line indices, a delete next to an insert
/// joined into one replacement.
///
/// Positions come from walking the op lengths, not from each op's own
/// indices: `similar`'s Myers ops can report the wrong index on the side an
/// insert or delete doesn't cover (a delete after an insert, say), and the
/// lengths are always right.
fn hunks(diff: &TextDiff<'_, '_, '_, str>) -> Vec<Hunk> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let (mut base_at, mut side_at) = (0, 0);
    for op in diff.ops() {
        let (tag, base_lines, side_lines) = op.as_tag_tuple();
        let base = base_at..base_at + base_lines.len();
        let side = side_at..side_at + side_lines.len();
        base_at = base.end;
        side_at = side.end;
        if tag == DiffTag::Equal {
            continue;
        }
        match hunks.last_mut() {
            Some(h) if h.base.end == base.start && h.side.end == side.start => {
                h.base.end = base.end;
                h.side.end = side.end;
            }
            _ => hunks.push(Hunk { base, side }),
        }
    }
    hunks
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

    fn apply(text: &str, edits: &[(Range<usize>, Arc<str>)]) -> String {
        let mut out = text.to_string();
        for (range, new) in edits.iter().rev() {
            out.replace_range(range.clone(), new);
        }
        out
    }

    fn merged(base: &str, ours: &str, theirs: &str) -> Result<String, MergeConflict> {
        let edits = merge(base, ours, theirs)?;
        for pair in edits.windows(2) {
            assert!(
                pair[0].0.end <= pair[1].0.start,
                "edits overlap or unsorted"
            );
        }
        Ok(apply(ours, &edits))
    }

    #[test]
    fn disjoint_changes_merge() {
        assert_eq!(
            merged("a\nb\nc\nd\ne\n", "A\nb\nc\nd\ne\n", "a\nb\nc\nd\nE\n").as_deref(),
            Ok("A\nb\nc\nd\nE\n")
        );
    }

    #[test]
    fn same_line_conflicts() {
        assert_eq!(merged("a\nb\n", "x\nb\n", "y\nb\n"), Err(MergeConflict));
    }

    #[test]
    fn adjacent_lines_conflict() {
        assert_eq!(
            merged("a\nb\nc\n", "A\nb\nc\n", "a\nB\nc\n"),
            Err(MergeConflict)
        );
    }

    #[test]
    fn one_line_apart_merges() {
        assert_eq!(
            merged("a\nb\nc\n", "A\nb\nc\n", "a\nb\nC\n").as_deref(),
            Ok("A\nb\nC\n")
        );
    }

    #[test]
    fn theirs_appends_at_end() {
        assert_eq!(
            merged("a\nb\nc\n", "A\nb\nc\n", "a\nb\nc\nd\n").as_deref(),
            Ok("A\nb\nc\nd\n")
        );
    }

    #[test]
    fn theirs_deletes_at_end() {
        assert_eq!(
            merged("a\nb\nc\nd\n", "A\nb\nc\nd\n", "a\nb\nc\n").as_deref(),
            Ok("A\nb\nc\n")
        );
    }

    #[test]
    fn insert_after_ours_shifts() {
        assert_eq!(
            merged("a\nb\nc\nd\n", "a\nx\ny\nb\nc\nd\n", "a\nb\nc\nD\n").as_deref(),
            Ok("a\nx\ny\nb\nc\nD\n")
        );
    }

    #[test]
    fn both_insert_at_start_conflict() {
        assert_eq!(merged("", "x\n", "y\n"), Err(MergeConflict));
    }

    #[test]
    fn no_final_newline() {
        assert_eq!(
            merged("a\nb\nc", "A\nb\nc", "a\nb\nc\nd").as_deref(),
            Ok("A\nb\nc\nd")
        );
    }

    #[test]
    fn unchanged_ours_takes_theirs() {
        assert_eq!(merged("a\n", "a\n", "b\n").as_deref(), Ok("b\n"));
    }

    #[test]
    fn multibyte() {
        assert_eq!(
            merged(
                "xin chào\nb\nđ\n",
                "xin chào!\nb\nđ\n",
                "xin chào\nb\nđường\n"
            )
            .as_deref(),
            Ok("xin chào!\nb\nđường\n")
        );
    }

    /// `similar`'s Myers reports this delete at new line 5 and the insert at
    /// old line 6 (both off by one); positions must come from op lengths.
    #[test]
    fn myers_op_indices_not_trusted() {
        let base = "d\nb\nd\nc\nb\nc\nc\na\nb\nd\nb\nb\n";
        let ours = "d\nd\nb\nd\nc\nb\nc\nc\na\nb\nd\nb\nb\n";
        let theirs = "d\nb\nd\nc\nc\nc\nd\nc\na\nb\nd\nb\nb\nd\n";
        assert_eq!(
            merged(base, ours, theirs).as_deref(),
            Ok("d\nd\nb\nd\nc\nc\nc\nd\nc\na\nb\nd\nb\nb\nd\n")
        );
    }

    /// A small linear congruential generator: deterministic cases without a
    /// dependency.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) % n as u64) as usize
        }
    }

    const WORDS: [&str; 4] = ["a", "b", "c", "d"];

    fn mutate(rng: &mut Lcg, base: &[&'static str]) -> Vec<&'static str> {
        let mut lines = base.to_vec();
        for _ in 0..rng.below(4) {
            let word = WORDS[rng.below(WORDS.len())];
            match rng.below(3) {
                0 if !lines.is_empty() => {
                    let at = rng.below(lines.len());
                    lines[at] = word;
                }
                1 if !lines.is_empty() => {
                    let at = rng.below(lines.len());
                    lines.remove(at);
                }
                _ => {
                    let at = rng.below(lines.len() + 1);
                    lines.insert(at, word);
                }
            }
        }
        lines
    }

    /// The lines with a `\n` each, but now and then none after the last.
    fn join(rng: &mut Lcg, lines: &[&str]) -> String {
        let mut text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        if rng.below(4) == 0 {
            text.pop();
        }
        text
    }

    #[test]
    fn merge_keeps_both_sides() {
        use similar::ChangeTag;

        /// Each run of changed lines of `side` against `base`, found by
        /// walking the diff line by line (not through `hunks`): the base
        /// lines it replaces and the lines it puts there.
        fn changes(base: &str, side: &str) -> Vec<(Range<usize>, Vec<String>)> {
            let diff = TextDiff::configure()
                .algorithm(Algorithm::Myers)
                .diff_lines(base, side);
            let mut runs: Vec<(Range<usize>, Vec<String>)> = Vec::new();
            let (mut at, mut open) = (0, false);
            for change in diff.iter_all_changes() {
                if change.tag() == ChangeTag::Equal {
                    at += 1;
                    open = false;
                    continue;
                }
                if !open {
                    runs.push((at..at, Vec::new()));
                    open = true;
                }
                let run = runs.last_mut().expect("just pushed");
                match change.tag() {
                    ChangeTag::Delete => {
                        at += 1;
                        run.0.end = at;
                    }
                    _ => run.1.push(change.value().to_string()),
                }
            }
            runs
        }

        /// `base` with both sides' changes, which a clean merge keeps
        /// apart: the result the merge must give.
        fn oracle(base: &str, ours: &str, theirs: &str) -> String {
            let mut lines: Vec<String> = base.split_inclusive('\n').map(String::from).collect();
            let mut all = changes(base, ours);
            all.extend(changes(base, theirs));
            all.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
            for (range, new) in all {
                lines.splice(range, new);
            }
            lines.concat()
        }

        let mut rng = Lcg(0x5eed);
        let mut merged_cases = 0;
        let mut open_ended = 0;
        for case in 0..2000 {
            let base: Vec<&str> = (0..rng.below(13))
                .map(|_| WORDS[rng.below(WORDS.len())])
                .collect();
            let ours = mutate(&mut rng, &base);
            let ours = join(&mut rng, &ours);
            let theirs = mutate(&mut rng, &base);
            let theirs = join(&mut rng, &theirs);
            let base = join(&mut rng, &base);
            let forward = merge(&base, &ours, &theirs);
            let backward = merge(&base, &theirs, &ours);
            let ctx = format!("case {case}: base {base:?} ours {ours:?} theirs {theirs:?}");
            assert_eq!(forward.is_ok(), backward.is_ok(), "{ctx}");
            if let (Ok(f), Ok(b)) = (&forward, &backward) {
                assert_eq!(apply(&ours, f), apply(&theirs, b), "{ctx}");
                assert_eq!(apply(&ours, f), oracle(&base, &ours, &theirs), "{ctx}");
                merged_cases += 1;
                if [&base, &ours, &theirs]
                    .iter()
                    .any(|t| !t.is_empty() && !t.ends_with('\n'))
                {
                    open_ended += 1;
                }
            }
            if ours == base {
                assert_eq!(
                    forward.map(|e| apply(&ours, &e)),
                    Ok(theirs.clone()),
                    "{ctx}"
                );
            } else if theirs == base {
                assert_eq!(forward.map(|e| apply(&ours, &e)), Ok(ours.clone()), "{ctx}");
            }
        }
        // The generator must reach the cases the oracle checks.
        assert!(merged_cases > 200, "only {merged_cases} cases merged");
        assert!(
            open_ended > 50,
            "only {open_ended} merged cases lack a final \\n"
        );
    }
}
