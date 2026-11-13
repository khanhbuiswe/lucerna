//! The Search tab's own helpers: matching a result line again, the glob
//! inputs and the status line. Pure, no gpui.

use std::ops::Range;

use lucerna_project::{MAX_RESULTS, SearchQuery};

use crate::excerpt_list::model::thousands;
use crate::find_widget::build_regex;

/// The first non-empty match of `query` in `line`.
pub fn first_match(query: &SearchQuery, line: &str) -> Option<Range<usize>> {
    let regex = build_regex(
        &query.pattern,
        query.is_regex,
        query.case_sensitive,
        query.whole_word,
    )
    .ok()?;
    regex
        .find_iter(line)
        .find(|m| !m.is_empty())
        .map(|m| m.range())
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
}
