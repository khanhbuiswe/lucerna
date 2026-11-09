use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use tree_sitter::Query;

use crate::wasm;

/// Why a grammar could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarError(pub String);

impl fmt::Display for GrammarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GrammarError {}

/// Identifies a WASM grammar: the extension that ships it and its name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GrammarKey {
    pub extension_id: Arc<str>,
    pub name: Arc<str>,
}

/// Where a language's grammar comes from.
pub(crate) enum GrammarSource {
    Native(tree_sitter::Language),
    Wasm { key: GrammarKey, path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageOrigin {
    Bundled,
    Extension { extension_id: Arc<str> },
}

/// A language: its grammar, highlight queries and the files it claims.
pub struct Language {
    /// VS Code language id, e.g. `typescriptreact`.
    pub id: Arc<str>,
    /// Display name, e.g. `TypeScript JSX`.
    pub name: Arc<str>,
    origin: LanguageOrigin,
    hidden: bool,
    extensions: Vec<String>,
    filenames: Vec<String>,
    source: GrammarSource,
    failed: AtomicBool,
    /// Concatenated in order into one highlights query.
    highlights: Vec<String>,
    /// Which pattern wins when several capture the same node. Upstream
    /// queries disagree: some list generic patterns first (`(identifier)
    /// @variable` then refinements), others list them last.
    precedence: Precedence,
    /// Markdown only: the inline grammar injected into `inline` nodes.
    inline: Option<InlineGrammar>,
    queries: OnceLock<Option<Queries>>,
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
    ) -> Result<HighlightQuery, String> {
        let source = sources.concat();
        let query = Query::new(grammar, &source).map_err(|e| e.to_string())?;
        let names = query
            .capture_names()
            .iter()
            .map(|&n| Arc::from(n))
            .collect();
        Ok(HighlightQuery {
            query,
            names,
            precedence,
        })
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
    /// The grammar, loaded on first use and cached. A WASM grammar that
    /// fails to load marks the language failed for the rest of the process.
    pub fn grammar(&self) -> Result<tree_sitter::Language, GrammarError> {
        let result = match &self.source {
            GrammarSource::Native(language) => Ok(language.clone()),
            GrammarSource::Wasm { key, path } => wasm::load_wasm_grammar(key, path),
        };
        if result.is_err() {
            self.failed.store(true, Ordering::Relaxed);
        }
        result
    }

    /// Whether [`Language::grammar`] returns without loading a file. A WASM
    /// grammar's first load compiles a module: do it off the UI thread.
    pub fn grammar_ready(&self) -> bool {
        match &self.source {
            GrammarSource::Native(_) => true,
            GrammarSource::Wasm { key, path } => wasm::is_loaded(key, path),
        }
    }

    pub fn origin(&self) -> &LanguageOrigin {
        &self.origin
    }

    /// Whether the grammar or highlights query of this language failed.
    pub fn is_failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    /// Hidden languages are not offered in language pickers.
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    /// A language whose grammar is the WASM file at `wasm_path`, for tests.
    #[doc(hidden)]
    pub fn from_wasm_for_test(id: &str, wasm_path: &Path, highlights_src: &str) -> Arc<Language> {
        Self::from_wasm_for_test_with(id, wasm_path, highlights_src, false, &[])
    }

    /// Like [`Language::from_wasm_for_test`], choosing the precedence.
    #[doc(hidden)]
    pub fn from_wasm_for_test_with(
        id: &str,
        wasm_path: &Path,
        highlights_src: &str,
        last_pattern_wins: bool,
        extensions: &[&str],
    ) -> Arc<Language> {
        Arc::new(Language {
            id: Arc::from(id),
            name: Arc::from(id),
            origin: LanguageOrigin::Extension {
                extension_id: Arc::from("test"),
            },
            hidden: false,
            extensions: extensions.iter().map(|e| e.to_string()).collect(),
            filenames: Vec::new(),
            source: GrammarSource::Wasm {
                // The grammar name is the file stem (`json.wasm` exports
                // `tree_sitter_json`); the id keeps each test's cache entry apart.
                key: GrammarKey {
                    extension_id: Arc::from(format!("test-{id}")),
                    name: Arc::from(wasm_path.file_stem().and_then(|s| s.to_str()).unwrap_or(id)),
                },
                path: wasm_path.to_path_buf(),
            },
            failed: AtomicBool::new(false),
            highlights: vec![highlights_src.to_string()],
            precedence: if last_pattern_wins {
                Precedence::LastPatternWins
            } else {
                Precedence::FirstPatternWins
            },
            inline: None,
            queries: OnceLock::new(),
        })
    }

    pub(crate) fn mark_failed(&self) {
        self.failed.store(true, Ordering::Relaxed);
    }

    pub(crate) fn is_wasm(&self) -> bool {
        matches!(self.source, GrammarSource::Wasm { .. })
    }

    /// Compiled on first use, so building the registry stays cheap. `None`
    /// when the grammar or query of an extension language is broken (the
    /// language is then marked failed). A broken bundled query is a bug and
    /// panics.
    pub(crate) fn queries(&self) -> Option<&Queries> {
        self.queries
            .get_or_init(|| match self.compile_queries() {
                Ok(queries) => Some(queries),
                Err(e) => {
                    if self.origin == LanguageOrigin::Bundled {
                        panic!("bundled highlights query for {} must compile: {e}", self.id);
                    }
                    tracing::warn!("language {}: {e}", self.id);
                    self.failed.store(true, Ordering::Relaxed);
                    None
                }
            })
            .as_ref()
    }

    fn compile_queries(&self) -> Result<Queries, String> {
        let grammar = self.grammar().map_err(|e| e.to_string())?;
        let sources: Vec<&str> = self.highlights.iter().map(String::as_str).collect();
        let highlights = HighlightQuery::new(&grammar, &sources, self.precedence)?;
        let inline = match &self.inline {
            None => None,
            Some(inline) => Some(InlineQueries {
                grammar: inline.grammar.clone(),
                highlights: HighlightQuery::new(
                    &inline.grammar,
                    &[inline.highlights],
                    Precedence::FirstPatternWins,
                )?,
                inline_nodes: Query::new(&grammar, "(inline) @inline")
                    .map_err(|e| e.to_string())?,
            }),
        };
        Ok(Queries { highlights, inline })
    }

    fn has_filename(&self, name: &str) -> bool {
        self.filenames.iter().any(|f| f == name)
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

    /// The bundled languages plus `extra`, for tests.
    #[doc(hidden)]
    pub fn bundled_plus_for_test(extra: Arc<Language>) -> LanguageRegistry {
        let mut registry = LanguageRegistry::bundled();
        registry.languages.push(extra);
        registry
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
        id: Arc::from(id),
        name: Arc::from(name),
        origin: LanguageOrigin::Bundled,
        hidden: false,
        extensions: extensions.iter().map(|s| s.to_string()).collect(),
        filenames: filenames.iter().map(|s| s.to_string()).collect(),
        source: GrammarSource::Native(grammar),
        failed: AtomicBool::new(false),
        highlights: highlights.iter().map(|s| s.to_string()).collect(),
        precedence,
        inline: None,
        queries: OnceLock::new(),
    }
}
