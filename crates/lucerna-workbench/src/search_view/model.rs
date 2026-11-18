//! The Search tab's own helpers: matching a result line again, the glob
//! inputs and the status line. Pure, no gpui.

use std::ops::Range;

use lucerna_project::{MAX_RESULTS, SearchQuery, line_matches, search_regex};

use crate::excerpt_list::model::thousands;

/// The first non-empty match of `query` in `line`.
pub fn first_match(query: &SearchQuery, line: &str) -> Option<Range<usize>> {
    let regex = search_regex(query).ok()?;
    line_matches(&regex, query, line).into_iter().next()
}

/// Comma-separated globs, trimmed, empty ones dropped. As VS Code's
/// `splitGlobAware`, a comma inside a class (`[,]`) or braces (`{a,b}`)
/// does not split.
pub fn globs(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let (mut start, mut braces) = (0, 0usize);
    // Inside `[...]`, and the index where its first character sits (a `]`
    // there is literal).
    let mut class: Option<usize> = None;
    for (i, c) in text.char_indices() {
        match (c, class) {
            (']', Some(first)) if i != first => class = None,
            (_, Some(_)) => {}
            ('[', None) => class = Some(i + 1),
            ('{', None) => braces += 1,
            ('}', None) => braces = braces.saturating_sub(1),
            (',', None) if braces == 0 => {
                parts.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
        .into_iter()
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The status line: `done` is `None` while searching, else `Some(truncated)`.
pub fn summary(matches: usize, files: usize, searched: bool, done: Option<bool>) -> Option<String> {
    if !searched {
        return None;
    }
    if matches == 0 {
        return Some(match done {
            Some(_) => "No results found".into(),
            None => "Searching…".into(),
        });
    }
    let plural = |n: usize, word: &str| match n {
        1 => format!("1 {word}"),
        n => format!("{} {word}s", thousands(n)),
    };
    let mut out = format!("{} in {}", plural(matches, "result"), plural(files, "file"));
    if done == Some(true) {
        out.push_str(&format!(" (truncated at {})", thousands(MAX_RESULTS)));
    }
    Some(out)
}

/// Shown in the replace-all confirmation when the results cover only part of
/// the matches. Wording as VS Code's `buildReplaceAllConfirmationMessage`
/// (MIT, src/vs/workbench/contrib/search/browser/searchView.ts).
#[allow(dead_code)] // until the replace-all dialog uses it
pub const REPLACE_ALL_TRUNCATED_DETAIL: &str =
    "The results only contain a subset of all matches. Files not in the results are not changed.";

/// The replacement value is cut to this many lines in the confirmation.
#[allow(dead_code)] // until the replace-all dialog uses it
const CONFIRM_VALUE_LINES: usize = 10;

/// At most this many skipped files are named in `skipped_message`.
#[allow(dead_code)] // until the replace-all dialog uses it
const SKIPPED_LISTED: usize = 5;

/// `1 file`, `2 files`, `20,000 occurrences`.
#[allow(dead_code)] // until the replace-all dialog uses it
fn count(n: usize, word: &str) -> String {
    match n {
        1 => format!("1 {word}"),
        n => format!("{} {word}s", thousands(n)),
    }
}

/// ` with 'value'`, or nothing when the replacement is empty (a deletion).
#[allow(dead_code)] // until the replace-all dialog uses it
fn with_value(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        format!(" with '{value}'")
    }
}

/// The replace-all confirmation. Wording as VS Code's
/// `buildReplaceAllConfirmationMessage` (MIT, src/vs/workbench/contrib/search/browser/searchView.ts).
/// A value longer than ten lines is cut to ten and `...` is appended.
#[allow(dead_code)] // until the replace-all dialog uses it
pub fn replace_all_confirmation(occurrences: usize, files: usize, value: &str) -> String {
    let lines: Vec<&str> = value.lines().collect();
    let shown = if lines.len() > CONFIRM_VALUE_LINES {
        format!("{}\n...", lines[..CONFIRM_VALUE_LINES].join("\n"))
    } else {
        value.to_owned()
    };
    format!(
        "Replace {} across {}{}?",
        count(occurrences, "occurrence"),
        count(files, "file"),
        with_value(&shown)
    )
}

/// The message once replace-all is done. Wording as VS Code's
/// `buildAfterReplaceAllMessage` (MIT, src/vs/workbench/contrib/search/browser/searchView.ts).
/// The value is never cut here.
#[allow(dead_code)] // until the replace-all dialog uses it
pub fn replaced_message(occurrences: usize, files: usize, value: &str) -> String {
    format!(
        "Replaced {} across {}{}.",
        count(occurrences, "occurrence"),
        count(files, "file"),
        with_value(value)
    )
}

/// Names the skipped files with their reasons, at most five. `skipped` holds
/// `(file name, reason)`. `None` when nothing was skipped.
#[allow(dead_code)] // until the replace-all dialog uses it
pub fn skipped_message(skipped: &[(String, String)]) -> Option<String> {
    if skipped.is_empty() {
        return None;
    }
    let listed: Vec<String> = skipped
        .iter()
        .take(SKIPPED_LISTED)
        .map(|(name, reason)| format!("{name} ({reason})"))
        .collect();
    let mut out = format!(
        "Skipped {}: {}",
        count(skipped.len(), "file"),
        listed.join(", ")
    );
    if skipped.len() > SKIPPED_LISTED {
        out.push_str(", …");
    }
    Some(out)
}

/// The notice for files with unsaved edits that replace-all left alone.
/// `None` when there are none.
#[allow(dead_code)] // until the replace-all dialog uses it
pub fn unsaved_message(files: usize) -> Option<String> {
    match files {
        0 => None,
        1 => Some("1 file with unsaved changes was not saved.".into()),
        n => Some(format!(
            "{} files with unsaved changes were not saved.",
            thousands(n)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_strings() {
        assert_eq!(summary(0, 0, true, None).as_deref(), Some("Searching…"));
        assert_eq!(
            summary(0, 0, true, Some(false)).as_deref(),
            Some("No results found")
        );
        assert_eq!(
            summary(1, 1, true, Some(false)).as_deref(),
            Some("1 result in 1 file")
        );
        assert_eq!(
            summary(20_000, 3, true, Some(true)).as_deref(),
            Some("20,000 results in 3 files (truncated at 20,000)")
        );
        assert_eq!(summary(5, 1, false, None), None);
    }

    #[test]
    fn globs_split_on_commas() {
        assert_eq!(globs(" src/**, *.rs ,,"), vec!["src/**", "*.rs"]);
        assert!(globs("").is_empty());
    }

    /// As VS Code's `splitGlobAware`: commas in a class or braces stay.
    #[test]
    fn globs_keep_commas_in_classes_and_braces() {
        assert_eq!(globs("a[,]b/**, {x,y}.rs"), vec!["a[,]b/**", "{x,y}.rs"]);
        assert_eq!(globs("[]],[,]"), vec!["[]]", "[,]"]);
    }

    #[test]
    fn replace_all_confirmation_strings() {
        assert_eq!(
            replace_all_confirmation(1, 1, "x"),
            "Replace 1 occurrence across 1 file with 'x'?"
        );
        assert_eq!(
            replace_all_confirmation(2, 1, "x"),
            "Replace 2 occurrences across 1 file with 'x'?"
        );
        assert_eq!(
            replace_all_confirmation(1, 3, "x"),
            "Replace 1 occurrence across 3 files with 'x'?"
        );
        assert_eq!(
            replace_all_confirmation(5, 3, ""),
            "Replace 5 occurrences across 3 files?"
        );
        assert_eq!(
            replace_all_confirmation(20_000, 3, "x"),
            "Replace 20,000 occurrences across 3 files with 'x'?"
        );
    }

    #[test]
    fn replace_all_confirmation_truncates_value_to_ten_lines() {
        let long = (1..=12)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let first_ten = (1..=10)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            replace_all_confirmation(2, 2, &long),
            format!("Replace 2 occurrences across 2 files with '{first_ten}\n...'?")
        );
    }

    #[test]
    fn replaced_message_strings() {
        assert_eq!(
            replaced_message(1, 1, "x"),
            "Replaced 1 occurrence across 1 file with 'x'."
        );
        assert_eq!(
            replaced_message(4, 2, ""),
            "Replaced 4 occurrences across 2 files."
        );
    }

    /// Unlike the confirmation, the replaced message never truncates.
    #[test]
    fn replaced_message_keeps_long_value() {
        let long = (1..=12)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            replaced_message(4, 2, &long),
            format!("Replaced 4 occurrences across 2 files with '{long}'.")
        );
    }

    #[test]
    fn truncated_detail_text() {
        assert_eq!(
            REPLACE_ALL_TRUNCATED_DETAIL,
            "The results only contain a subset of all matches. Files not in the results are not changed."
        );
    }

    #[test]
    fn skipped_message_strings() {
        assert_eq!(skipped_message(&[]), None);
        assert_eq!(
            skipped_message(&[("a.rs".into(), "changed on disk".into())]).as_deref(),
            Some("Skipped 1 file: a.rs (changed on disk)")
        );
        assert_eq!(
            skipped_message(&[
                ("a.rs".into(), "changed on disk".into()),
                ("b.rs".into(), "not valid UTF-8".into()),
            ])
            .as_deref(),
            Some("Skipped 2 files: a.rs (changed on disk), b.rs (not valid UTF-8)")
        );
    }

    /// Seven files: five listed, then an ellipsis.
    #[test]
    fn skipped_message_lists_at_most_five() {
        let files: Vec<(String, String)> = (1..=7)
            .map(|i| (format!("f{i}.rs"), "changed on disk".to_string()))
            .collect();
        assert_eq!(
            skipped_message(&files).as_deref(),
            Some(
                "Skipped 7 files: f1.rs (changed on disk), f2.rs (changed on disk), \
                 f3.rs (changed on disk), f4.rs (changed on disk), \
                 f5.rs (changed on disk), …"
            )
        );
    }

    #[test]
    fn unsaved_message_strings() {
        assert_eq!(unsaved_message(0), None);
        assert_eq!(
            unsaved_message(1).as_deref(),
            Some("1 file with unsaved changes was not saved.")
        );
        assert_eq!(
            unsaved_message(2).as_deref(),
            Some("2 files with unsaved changes were not saved.")
        );
        assert_eq!(
            unsaved_message(20_000).as_deref(),
            Some("20,000 files with unsaved changes were not saved.")
        );
    }
}
