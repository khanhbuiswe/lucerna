//! A glob table such as `files.associations` (glob to language) or
//! `workbench.editorAssociations` (glob to editor id).

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
    value: String,
}

/// A compiled glob table: `files.associations`, `workbench.editorAssociations`
/// or any other map of glob to value.
#[derive(Debug, Default)]
pub struct Associations {
    rules: Vec<Rule>,
}

impl Associations {
    /// Compiles `map` (glob to value, such as a language id or an editor id).
    /// Invalid globs are skipped with a warning.
    pub fn new(map: &BTreeMap<String, String>) -> Associations {
        let rules = map
            .iter()
            .filter_map(|(pattern, value)| {
                let glob = GlobBuilder::new(pattern)
                    .literal_separator(false)
                    .build()
                    .map_err(|e| tracing::warn!("associations: bad glob {pattern:?}: {e}"))
                    .ok()?;
                Some(Rule {
                    matcher: glob.compile_matcher(),
                    whole_path: pattern.contains('/'),
                    pattern_len: pattern.len(),
                    value: value.clone(),
                })
            })
            .collect();
        Associations { rules }
    }

    /// The values of the rules matching `path`, longest glob first. A glob
    /// with `/` matches the whole path, one without matches the file name.
    /// `path` should be relative to the workspace root.
    pub fn matches<'a>(&'a self, path: &Path) -> Vec<&'a str> {
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
        hits.into_iter().map(|rule| rule.value.as_str()).collect()
    }
}
