//! Indentation detection (`editor.detectIndentation`): guesses a file's
//! indent style from the leading whitespace of its lines.

// Adapted from microsoft/vscode src/vs/editor/common/model/indentationGuesser.ts (MIT)

use lucerna_text::BufferSnapshot;

/// How a file is indented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indentation {
    pub insert_spaces: bool,
    pub tab_size: u32,
}

/// Lines looked at, from the top.
const MAX_LINES: u32 = 10_000;
/// Candidate tab sizes, in order of preference (even sizes first).
const TAB_SIZE_GUESSES: [usize; 7] = [2, 4, 6, 8, 3, 5, 7];
const MAX_TAB_SIZE_GUESS: usize = 8;

/// The difference in indentation of two lines, in spaces.
#[derive(Default)]
struct SpacesDiff {
    diff: usize,
    looks_like_alignment: bool,
}

/// The diff in spaces between the indentation of line `a` (whose indent is
/// `a_len` bytes) and of line `b`.
fn spaces_diff(a: &[u8], a_len: usize, b: &[u8], b_len: usize) -> SpacesDiff {
    let mut result = SpacesDiff::default();
    // Either can be the deeper one: `\t` against `\t    ` is one tab and
    // four spaces.
    let common = a[..a_len]
        .iter()
        .zip(&b[..b_len])
        .take_while(|(x, y)| x == y)
        .count();
    let count = |s: &[u8], len: usize| {
        let spaces = s[common..len].iter().filter(|&&c| c == b' ').count();
        (spaces, len - common - spaces)
    };
    let (a_spaces, a_tabs) = count(a, a_len);
    let (b_spaces, b_tabs) = count(b, b_len);
    if (a_spaces > 0 && a_tabs > 0) || (b_spaces > 0 && b_tabs > 0) {
        return result;
    }
    let tabs_diff = a_tabs.abs_diff(b_tabs);
    let spaces = a_spaces.abs_diff(b_spaces);
    if tabs_diff == 0 {
        // Aligned continuation lines (`const a = 1,` over `      b = 2`)
        // say nothing of the indent.
        result.diff = spaces;
        if spaces > 0
            && b_spaces >= 1
            && b_spaces - 1 < a.len()
            && b_spaces < b.len()
            && b[b_spaces] != b' '
            && a[b_spaces - 1] == b' '
            && a.last() == Some(&b',')
        {
            result.looks_like_alignment = true;
        }
    } else if spaces % tabs_diff == 0 {
        result.diff = spaces / tabs_diff;
    }
    result
}

/// Guesses the indentation of `snapshot` from its first 10 000 lines.
/// Falls back to `default` when too few lines are indented to tell: a tab
/// size is only guessed from 2 to 8, never from noise.
pub fn guess(snapshot: &BufferSnapshot, default: Indentation) -> Indentation {
    let lines = snapshot.line_count().min(MAX_LINES);
    let mut with_tabs = 0u32;
    let mut with_spaces = 0u32;
    let mut indented = 0u32;
    let mut diff_counts = [0u32; MAX_TAB_SIZE_GUESS + 1];
    let mut previous = String::new();
    let mut previous_indent = 0;
    for row in 0..lines {
        let line = snapshot.line(row);
        let bytes = line.as_bytes();
        let indent = bytes
            .iter()
            .take_while(|&&c| c == b' ' || c == b'\t')
            .count();
        // Blank and whitespace-only lines say nothing.
        if indent == bytes.len() {
            continue;
        }
        let tabs = bytes[..indent].iter().filter(|&&c| c == b'\t').count();
        if tabs > 0 {
            with_tabs += 1;
        } else if indent > 1 {
            with_spaces += 1;
        }
        if indent > 0 {
            indented += 1;
        }
        let diff = spaces_diff(previous.as_bytes(), previous_indent, bytes, indent);
        if diff.looks_like_alignment
            && !(default.insert_spaces && default.tab_size as usize == diff.diff)
        {
            continue;
        }
        if diff.diff <= MAX_TAB_SIZE_GUESS {
            diff_counts[diff.diff] += 1;
        }
        previous.clear();
        previous.push_str(&line);
        previous_indent = indent;
    }
    if indented < 2 {
        return default;
    }
    let insert_spaces = if with_tabs == with_spaces {
        default.insert_spaces
    } else {
        with_tabs < with_spaces
    };
    let mut tab_size = default.tab_size as usize;
    if insert_spaces {
        let mut best = 0;
        for size in TAB_SIZE_GUESSES {
            if diff_counts[size] > best {
                best = diff_counts[size];
                tab_size = size;
            }
        }
        // 2 wins over 4 with at least 2/3 of its occurrences (nested YAML).
        if tab_size == 4
            && diff_counts[4] > 0
            && diff_counts[2] > 0
            && diff_counts[2] * 3 >= diff_counts[4] * 2
        {
            tab_size = 2;
        }
    }
    Indentation {
        insert_spaces,
        tab_size: tab_size as u32,
    }
}

#[cfg(test)]
mod tests {
    use lucerna_text::Buffer;

    use super::*;

    const DEFAULT: Indentation = Indentation {
        insert_spaces: true,
        tab_size: 4,
    };

    fn guess_text(text: &str) -> Indentation {
        let snapshot = Buffer::from_bytes(text.as_bytes(), false)
            .unwrap()
            .snapshot();
        guess(&snapshot, DEFAULT)
    }

    #[test]
    fn guess_two_spaces() {
        let text = "fn a() {\n  b();\n  if c {\n    d();\n  }\n}\n";
        assert_eq!(
            guess_text(text),
            Indentation {
                insert_spaces: true,
                tab_size: 2
            }
        );
    }

    #[test]
    fn guess_tabs() {
        let text = "fn a() {\n\tb();\n\tif c {\n\t\td();\n\t}\n}\n";
        assert!(!guess_text(text).insert_spaces);
    }

    #[test]
    fn guess_falls_back_when_unclear() {
        assert_eq!(guess_text("a\nb\nc\n"), DEFAULT);
        assert_eq!(guess_text("a\nb\n    c\nd\n"), DEFAULT);
        assert_eq!(guess_text(""), DEFAULT);
        // One tab-indented line among many 4-space ones.
        let text = "a\n    b\n    c\n\td\n    e\n        f\n";
        let two = Indentation {
            insert_spaces: true,
            tab_size: 2,
        };
        let snapshot = Buffer::from_bytes(text.as_bytes(), false)
            .unwrap()
            .snapshot();
        assert_eq!(guess(&snapshot, two), spaces(4));
        let text = "a\n  b\n  c\n\td\n  e\n    f\n";
        assert_eq!(
            guess_text(text),
            Indentation {
                insert_spaces: true,
                tab_size: 2
            }
        );
        // A one-space indent is noise, never a tab size of 1.
        assert_eq!(guess_text(" a\n b\n  c\n d\n"), DEFAULT);
    }

    /// A file whose spaces diffs are `n2` twos then `n4` fours (even
    /// counts: each block goes out to its width and back).
    fn diffs(n2: usize, n4: usize) -> String {
        let block = |width: usize, n: usize| {
            (0..n)
                .map(|i| {
                    if i % 2 == 0 {
                        " ".repeat(width)
                    } else {
                        String::new()
                    }
                })
                .map(|pad| format!("{pad}x\n"))
                .collect::<String>()
                + "y\n"
        };
        // The closing `y` of a block is one more diff of its width.
        block(2, n2 - 1) + &block(4, n4 - 1)
    }

    fn spaces(tab_size: u32) -> Indentation {
        Indentation {
            insert_spaces: true,
            tab_size,
        }
    }

    #[test]
    fn two_needs_two_thirds_of_fours_to_win() {
        // VS Code: `diff[2] >= diff[4] * 2 / 3`.
        assert_eq!(guess_text(&diffs(6, 10)), spaces(4));
        assert_eq!(guess_text(&diffs(8, 10)), spaces(2));
        assert_eq!(guess_text(&diffs(10, 6)), spaces(2));
    }

    #[test]
    fn guess_seven() {
        let text = "a\n       b\n       c\nd\n       e\nf\n";
        assert_eq!(guess_text(text), spaces(7));
    }

    #[test]
    fn alignment_does_not_count() {
        // Default (true, 4): the aligned `b` (6 spaces) is skipped, so the
        // real 2-space nesting decides.
        let aligned = "const a = 1,\n      b = 2;\n".repeat(3);
        let text = format!("{aligned}fn f() {{\n  g();\n  h();\n}}\n");
        assert_eq!(guess_text(&text), spaces(2));
    }
}
