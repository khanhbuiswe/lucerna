use std::fmt;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use tree_sitter::Query;

/// A bundled language: its grammar, highlight queries and the files it claims.
pub struct Language {
    /// VS Code language id, e.g. `typescriptreact`.
    pub id: &'static str,
    /// Display name, e.g. `TypeScript JSX`.
    pub name: &'static str,
    extensions: &'static [&'static str],
    filenames: &'static [&'static str],
    grammar: tree_sitter::Language,
    /// Concatenated in order into one highlights query.
    highlights: &'static [&'static str],
    /// Which pattern wins when several capture the same node. Upstream
    /// queries disagree: some list generic patterns first (`(identifier)
    /// @variable` then refinements), others list them last.
    precedence: Precedence,
    /// Markdown only: the inline grammar injected into `inline` nodes.
    inline: Option<InlineGrammar>,
    queries: OnceLock<Queries>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Precedence {
    FirstPatternWins,
    LastPatternWins,
}

struct InlineGrammar {
    grammar: tree_sitter::Language,
    highlights: &'static str,
}

/// A compiled highlights query with its capture names interned once, so
/// spans share an `Arc<str>` per capture instead of allocating.
pub(crate) struct HighlightQuery {
    pub query: Query,
    pub names: Vec<Arc<str>>,
    pub precedence: Precedence,
}

impl HighlightQuery {
    fn new(
        grammar: &tree_sitter::Language,
        sources: &[&str],
        precedence: Precedence,
        id: &str,
    ) -> HighlightQuery {
        let source = sources.concat();
        let query = Query::new(grammar, &source)
            .unwrap_or_else(|e| panic!("bundled highlights query for {id} must compile: {e}"));
        let names = query
            .capture_names()
            .iter()
            .map(|&n| Arc::from(n))
            .collect();
        HighlightQuery {
            query,
            names,
            precedence,
        }
    }
}

pub(crate) struct Queries {
    pub highlights: HighlightQuery,
    pub inline: Option<InlineQueries>,
}

pub(crate) struct InlineQueries {
    pub grammar: tree_sitter::Language,
    pub highlights: HighlightQuery,
    /// `(inline) @inline` over the block tree.
    pub inline_nodes: Query,
}

impl Language {
    pub(crate) fn grammar(&self) -> &tree_sitter::Language {
        &self.grammar
    }

    /// Compiled on first use, so building the registry stays cheap.
    pub(crate) fn queries(&self) -> &Queries {
        self.queries.get_or_init(|| Queries {
            highlights: HighlightQuery::new(
                &self.grammar,
                self.highlights,
                self.precedence,
                self.id,
            ),
            inline: self.inline.as_ref().map(|inline| InlineQueries {
                grammar: inline.grammar.clone(),
                highlights: HighlightQuery::new(
                    &inline.grammar,
                    &[inline.highlights],
                    Precedence::FirstPatternWins,
                    self.id,
                ),
                inline_nodes: Query::new(&self.grammar, "(inline) @inline")
                    .expect("markdown block grammar has `inline` nodes"),
            }),
        })
    }

    fn has_filename(&self, name: &str) -> bool {
        self.filenames.contains(&name)
    }

    fn has_extension(&self, ext: &str) -> bool {
        self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
    }
}

impl fmt::Debug for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Language")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct LanguageRegistry {
    languages: Vec<Arc<Language>>,
}

impl LanguageRegistry {
    /// Every language Lucerna ships. Queries compile lazily on first use.
    pub fn bundled() -> LanguageRegistry {
        use Precedence::{FirstPatternWins as First, LastPatternWins as Last};
        const JS: &str = tree_sitter_javascript::HIGHLIGHT_QUERY;
        const JSX: &str = tree_sitter_javascript::JSX_HIGHLIGHT_QUERY;
        const TS: &str = tree_sitter_typescript::HIGHLIGHTS_QUERY;
        let languages = vec![
            lang(
                "rust",
                "Rust",
                &["rs"],
                &[],
                tree_sitter_rust::LANGUAGE.into(),
                &[tree_sitter_rust::HIGHLIGHTS_QUERY],
                First,
            ),
            // The TypeScript query extends the JavaScript one, as upstream's
            // own configuration does.
            lang(
                "typescript",
                "TypeScript",
                &["ts", "mts", "cts"],
                &[],
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                &[JS, TS],
                Last,
            ),
            lang(
                "typescriptreact",
                "TypeScript JSX",
                &["tsx"],
                &[],
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                &[JS, JSX, TS],
                Last,
            ),
            lang(
                "javascript",
                "JavaScript",
                &["js", "mjs", "cjs", "jsx"],
                &[],
                tree_sitter_javascript::LANGUAGE.into(),
                &[JS, JSX],
                Last,
            ),
            lang(
                "python",
                "Python",
                &["py", "pyi"],
                &[],
                tree_sitter_python::LANGUAGE.into(),
                &[tree_sitter_python::HIGHLIGHTS_QUERY],
                Last,
            ),
            lang(
                "go",
                "Go",
                &["go"],
                &[],
                tree_sitter_go::LANGUAGE.into(),
                &[tree_sitter_go::HIGHLIGHTS_QUERY],
                First,
            ),
            lang(
                "json",
                "JSON",
                &["json", "jsonc"],
                &["tsconfig.json", "settings.json"],
                tree_sitter_json::LANGUAGE.into(),
                &[tree_sitter_json::HIGHLIGHTS_QUERY],
                First,
            ),
            Language {
                inline: Some(InlineGrammar {
                    grammar: tree_sitter_md::INLINE_LANGUAGE.into(),
                    highlights: tree_sitter_md::HIGHLIGHT_QUERY_INLINE,
                }),
                ..lang(
                    "markdown",
                    "Markdown",
                    &["md", "markdown"],
                    &[],
                    tree_sitter_md::LANGUAGE.into(),
                    &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK],
                    First,
                )
            },
            lang(
                "toml",
                "TOML",
                &["toml"],
                &["Cargo.lock"],
                tree_sitter_toml_ng::LANGUAGE.into(),
                &[tree_sitter_toml_ng::HIGHLIGHTS_QUERY],
                First,
            ),
            lang(
                "yaml",
                "YAML",
                &["yml", "yaml"],
                &[],
                tree_sitter_yaml::LANGUAGE.into(),
                &[tree_sitter_yaml::HIGHLIGHTS_QUERY],
                First,
            ),
            lang(
                "shellscript",
                "Shell",
                &["sh", "bash", "zsh"],
                &[".zshrc", ".bashrc"],
                tree_sitter_bash::LANGUAGE.into(),
                &[tree_sitter_bash::HIGHLIGHT_QUERY],
                First,
            ),
            lang(
                "html",
                "HTML",
                &["html", "htm"],
                &[],
                tree_sitter_html::LANGUAGE.into(),
                &[tree_sitter_html::HIGHLIGHTS_QUERY],
                First,
            ),
            lang(
                "css",
                "CSS",
                &["css"],
                &[],
                tree_sitter_css::LANGUAGE.into(),
                &[tree_sitter_css::HIGHLIGHTS_QUERY],
                First,
            ),
        ];
        LanguageRegistry {
            languages: languages.into_iter().map(Arc::new).collect(),
        }
    }

    pub fn languages(&self) -> &[Arc<Language>] {
        &self.languages
    }

    /// Exact file names win over extensions; extensions ignore ASCII case.
    pub fn language_for_path(&self, path: &Path) -> Option<Arc<Language>> {
        let by_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| self.languages.iter().find(|l| l.has_filename(n)));
        let by_ext = || {
            let ext = path.extension()?.to_str()?;
            self.languages.iter().find(|l| l.has_extension(ext))
        };
        by_name.or_else(by_ext).cloned()
    }
}

fn lang(
    id: &'static str,
    name: &'static str,
    extensions: &'static [&'static str],
    filenames: &'static [&'static str],
    grammar: tree_sitter::Language,
    highlights: &'static [&'static str],
    precedence: Precedence,
) -> Language {
    Language {
        id,
        name,
        extensions,
        filenames,
        grammar,
        highlights,
        precedence,
        inline: None,
        queries: OnceLock::new(),
    }
}
