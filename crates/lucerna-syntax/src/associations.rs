//! `files.associations`: glob patterns that pin a file to a language.

use std::collections::BTreeMap;
use std::path::Path;

use globset::{GlobBuilder, GlobMatcher};

#[derive(Debug)]
struct Rule {
    matcher: GlobMatcher,
    /// A pattern with `/` matches the (root-relative) path; one without
    /// matches the file name.
    whole_path: bool,
    pattern_len: usize,
    language: String,
}

/// The compiled `files.associations` setting.
#[derive(Debug, Default)]
pub struct Associations {
    rules: Vec<Rule>,
}

impl Associations {
    /// Compiles `map` (glob to language id or name). Invalid globs are
    /// skipped with a warning.
    pub fn new(map: &BTreeMap<String, String>) -> Associations {
        let rules = map
            .iter()
            .filter_map(|(pattern, language)| {
                let glob = GlobBuilder::new(pattern)
                    .literal_separator(false)
                    .build()
                    .map_err(|e| tracing::warn!("files.associations: bad glob {pattern:?}: {e}"))
                    .ok()?;
                Some(Rule {
                    matcher: glob.compile_matcher(),
                    whole_path: pattern.contains('/'),
                    pattern_len: pattern.len(),
                    language: language.clone(),
                })
            })
            .collect();
        Associations { rules }
    }

    /// The languages named by the rules matching `path`, longest glob first.
    /// `path` should be relative to the workspace root.
    pub(crate) fn matches<'a>(&'a self, path: &Path) -> Vec<&'a str> {
        let name = path.file_name().map(Path::new);
        let mut hits: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|rule| {
                let target = if rule.whole_path { Some(path) } else { name };
                target.is_some_and(|t| rule.matcher.is_match(t))
            })
            .collect();
        // Stable: equal lengths keep the map's (alphabetical) order.
        hits.sort_by_key(|rule| std::cmp::Reverse(rule.pattern_len));
        hits.into_iter()
            .map(|rule| rule.language.as_str())
            .collect()
    }
}
