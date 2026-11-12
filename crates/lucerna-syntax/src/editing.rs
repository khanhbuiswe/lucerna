//! Per-language editing data: auto-closing pairs and comment tokens.

/// One auto-closing / surrounding pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BracketPair {
    pub start: String,
    pub end: String,
    /// Insert `end` when `start` is typed.
    pub close: bool,
    /// Pressing Enter between the pair puts the closer on its own line.
    pub newline: bool,
    /// Scopes (`string`, `comment`) where the pair does not auto-close.
    pub not_in: Vec<String>,
}

impl BracketPair {
    fn new(start: &str, end: &str, newline: bool, not_in: &[&str]) -> BracketPair {
        BracketPair {
            start: start.to_string(),
            end: end.to_string(),
            close: true,
            newline,
            not_in: not_in.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// How a language is edited: its pairs and comment tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageEditing {
    pub brackets: Vec<BracketPair>,
    /// Auto-closing only happens before one of these characters.
    pub autoclose_before: String,
    pub line_comments: Vec<String>,
    pub block_comment: Option<(String, String)>,
}

impl Default for LanguageEditing {
    fn default() -> LanguageEditing {
        LanguageEditing {
            brackets: vec![
                BracketPair::new("(", ")", true, &[]),
                BracketPair::new("[", "]", true, &[]),
                BracketPair::new("{", "}", true, &[]),
            ],
            autoclose_before: ";:.,=}])> \n\t".to_string(),
            line_comments: Vec::new(),
            block_comment: None,
        }
    }
}

/// Line comments, block comment and quote pairs with their `not_in` scopes.
type Facts = (
    &'static [&'static str],
    Option<(&'static str, &'static str)>,
    &'static [(&'static str, &'static [&'static str])],
);

impl LanguageEditing {
    /// The editing data of a bundled language, written by hand from the
    /// languages' syntax.
    pub(crate) fn bundled(id: &str) -> LanguageEditing {
        const STRING: &[&str] = &["string"];
        const STRING_COMMENT: &[&str] = &["string", "comment"];
        const SLASH: (&str, &str) = ("/*", "*/");
        // (line comment, block comment, quote pairs)
        let (line, block, quotes): Facts = match id {
            "rust" => (&["//"], Some(SLASH), &[("\"", STRING)]),
            "typescript" | "typescriptreact" | "javascript" => (
                &["//"],
                Some(SLASH),
                &[
                    ("\"", STRING_COMMENT),
                    ("'", STRING_COMMENT),
                    ("`", STRING_COMMENT),
                ],
            ),
            "go" => (
                &["//"],
                Some(SLASH),
                &[("\"", STRING_COMMENT), ("`", STRING_COMMENT)],
            ),
            "python" | "toml" | "yaml" | "bash" => (
                &["#"],
                None,
                &[("\"", STRING_COMMENT), ("'", STRING_COMMENT)],
            ),
            "json" => (&[], None, &[("\"", STRING)]),
            "css" => (
                &[],
                Some(SLASH),
                &[("\"", STRING_COMMENT), ("'", STRING_COMMENT)],
            ),
            "html" => (
                &[],
                Some(("<!--", "-->")),
                &[("\"", STRING_COMMENT), ("'", STRING_COMMENT)],
            ),
            "markdown" => (&[], Some(("<!--", "-->")), &[("`", STRING)]),
            _ => return LanguageEditing::default(),
        };
        let mut editing = LanguageEditing::default();
        for (quote, not_in) in quotes {
            editing
                .brackets
                .push(BracketPair::new(quote, quote, false, not_in));
        }
        editing.line_comments = line.iter().map(|s| s.to_string()).collect();
        editing.block_comment = block.map(|(a, b)| (a.to_string(), b.to_string()));
        editing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_three_bracket_pairs() {
        let editing = LanguageEditing::default();
        let starts: Vec<&str> = editing.brackets.iter().map(|b| b.start.as_str()).collect();
        assert_eq!(starts, ["(", "[", "{"]);
        assert!(editing.brackets.iter().all(|b| b.close && b.newline));
        assert_eq!(editing.autoclose_before, ";:.,=}])> \n\t");
        assert!(editing.line_comments.is_empty() && editing.block_comment.is_none());
    }

    #[test]
    fn bundled_rust_has_comment_tokens() {
        let registry = crate::LanguageRegistry::bundled();
        let rust = registry.language_by_name("Rust").unwrap();
        let editing = rust.editing();
        assert_eq!(editing.line_comments, ["//"]);
        assert_eq!(
            editing.block_comment,
            Some(("/*".to_string(), "*/".to_string()))
        );
        assert!(editing.brackets.iter().any(|b| b.start == "\""));
        assert!(rust.indents_query().is_none());
    }

    #[test]
    fn quote_pairs_do_not_break_lines() {
        let editing = LanguageEditing::bundled("javascript");
        let quotes: Vec<&BracketPair> = editing.brackets.iter().skip(3).collect();
        assert_eq!(quotes.len(), 3);
        assert!(quotes.iter().all(|q| !q.newline && q.not_in.len() == 2));
    }
}
