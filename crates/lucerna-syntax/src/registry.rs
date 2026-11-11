use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tree_sitter::{Query, QueryPredicateArg};

use crate::associations::Associations;
use crate::injection::InjectionQuery;
use crate::outline::OutlineQuery;
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
    /// An extension language without a grammar: shown as plain text.
    None,
    Wasm {
        key: GrammarKey,
        path: PathBuf,
    },
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
    /// Source of the injections query (run by the layer machinery).
    injections: Option<String>,
    /// Source of the brackets query, which replaces the built-in pairs.
    brackets: Option<String>,
    /// Source of the outline query (document symbols without a server).
    outline: Option<String>,
    /// Extension languages only: matched against the first line of a file.
    first_line: Option<regex::Regex>,
    /// Extension languages only: the directory whose `highlights.scm`,
    /// `injections.scm`, `brackets.scm` and `outline.scm` are read when the
    /// queries first compile (instead of `highlights`, `injections`,
    /// `brackets` and `outline`), so building a registry opens no query
    /// file.
    query_dir: Option<PathBuf>,
    queries: OnceLock<Option<LanguageQueries>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Precedence {
    FirstPatternWins,
    LastPatternWins,
}

/// A compiled highlights query with its capture names interned once, so
/// spans share an `Arc<str>` per capture instead of allocating.
pub(crate) struct HighlightQuery {
    pub query: Query,
    pub names: Vec<Arc<str>>,
    pub precedence: Precedence,
    predicates: Predicates,
}

/// Per pattern of a query: the checks beyond what tree-sitter applies
/// itself (`#has-parent?`, `#not-has-parent?`; any other general
/// predicate disables its pattern).
pub(crate) struct Predicates {
    patterns: Vec<PatternChecks>,
}

#[derive(Default)]
struct PatternChecks {
    /// A predicate Lucerna does not know: the pattern never matches.
    disabled: bool,
    parents: Vec<ParentCheck>,
}

/// `#has-parent?` / `#not-has-parent?`.
struct ParentCheck {
    capture: u32,
    kinds: Vec<Box<str>>,
    negate: bool,
}

impl Predicates {
    /// Reads the general predicates of `query`, warning once per unknown
    /// one.
    pub(crate) fn new(query: &Query, language_id: &str) -> Predicates {
        let mut warned = HashSet::new();
        let patterns = (0..query.pattern_count())
            .map(|pattern| {
                let mut checks = PatternChecks::default();
                for predicate in query.general_predicates(pattern) {
                    let negate = match &*predicate.operator {
                        "has-parent?" => false,
                        "not-has-parent?" => true,
                        other => {
                            if warned.insert(other.to_string()) {
                                tracing::warn!(
                                    "language {language_id}: unsupported predicate #{other}; \
                                     patterns using it never match"
                                );
                            }
                            checks.disabled = true;
                            continue;
                        }
                    };
                    let mut args = predicate.args.iter();
                    let Some(QueryPredicateArg::Capture(capture)) = args.next() else {
                        checks.disabled = true;
                        continue;
                    };
                    let kinds = args
                        .filter_map(|a| match a {
                            QueryPredicateArg::String(s) => Some(s.clone()),
                            QueryPredicateArg::Capture(_) => None,
                        })
                        .collect();
                    checks.parents.push(ParentCheck {
                        capture: *capture,
                        kinds,
                        negate,
                    });
                }
                checks
            })
            .collect();
        Predicates { patterns }
    }

    /// Whether the checks tree-sitter leaves to us pass for `m`.
    pub(crate) fn accepts(&self, m: &tree_sitter::QueryMatch) -> bool {
        let Some(checks) = self.patterns.get(m.pattern_index) else {
            return true;
        };
        if checks.disabled {
            return false;
        }
        checks.parents.iter().all(|check| {
            m.nodes_for_capture_index(check.capture).all(|node| {
                let in_list = node
                    .parent()
                    .is_some_and(|p| check.kinds.iter().any(|k| **k == *p.kind()));
                in_list != check.negate
            })
        })
    }
}

impl HighlightQuery {
    fn new(
        grammar: &tree_sitter::Language,
        sources: &[&str],
        precedence: Precedence,
        language_id: &str,
    ) -> Result<HighlightQuery, String> {
        let source = sources.concat();
        let query = Query::new(grammar, &source).map_err(|e| e.to_string())?;
        let names = query
            .capture_names()
            .iter()
            .map(|&n| Arc::from(n))
            .collect();
        let predicates = Predicates::new(&query, language_id);
        Ok(HighlightQuery {
            query,
            names,
            precedence,
            predicates,
        })
    }

    /// Whether the checks tree-sitter leaves to us pass for `m`.
    pub fn accepts(&self, m: &tree_sitter::QueryMatch) -> bool {
        self.predicates.accepts(m)
    }
}

pub(crate) struct LanguageQueries {
    pub highlights: HighlightQuery,
    pub injections: Option<InjectionQuery>,
    pub brackets: Option<Query>,
    pub outline: Option<OutlineQuery>,
}

/// An extension's language as the workbench hands it over.
#[derive(Debug, Clone)]
pub struct ExtensionLanguage {
    pub extension_id: String,
    pub name: String,
    /// Holds the `*.scm` query files.
    pub dir: PathBuf,
    /// The grammar name and its `.wasm` file.
    pub grammar: Option<(String, PathBuf)>,
    pub path_suffixes: Vec<String>,
    pub first_line_pattern: Option<String>,
    pub hidden: bool,
}

impl Language {
    /// The grammar, loaded on first use and cached. A WASM grammar that
    /// fails to load marks the language failed for the rest of the process.
    pub fn grammar(&self) -> Result<tree_sitter::Language, GrammarError> {
        let result = match &self.source {
            GrammarSource::Native(language) => Ok(language.clone()),
            GrammarSource::Wasm { key, path } => wasm::load_wasm_grammar(key, path),
            GrammarSource::None => {
                return Err(GrammarError(format!("{} has no grammar", self.name)));
            }
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
            GrammarSource::Native(_) | GrammarSource::None => true,
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
            extensions: extensions.iter().map(|e| e.to_string()).collect(),
            precedence: if last_pattern_wins {
                Precedence::LastPatternWins
            } else {
                Precedence::FirstPatternWins
            },
            ..Language::new(
                id,
                id,
                LanguageOrigin::Extension {
                    extension_id: Arc::from("test"),
                },
                // The grammar name is the file stem (`json.wasm` exports
                // `tree_sitter_json`); the id keeps each test's cache entry apart.
                GrammarSource::Wasm {
                    key: GrammarKey {
                        extension_id: Arc::from(format!("test-{id}")),
                        name: Arc::from(
                            wasm_path.file_stem().and_then(|s| s.to_str()).unwrap_or(id),
                        ),
                    },
                    path: wasm_path.to_path_buf(),
                },
                vec![highlights_src.to_string()],
                Precedence::FirstPatternWins,
            )
        })
    }

    /// The one place a `Language` is assembled; callers override the rest
    /// with struct update.
    fn new(
        id: &str,
        name: &str,
        origin: LanguageOrigin,
        source: GrammarSource,
        highlights: Vec<String>,
        precedence: Precedence,
    ) -> Language {
        Language {
            id: Arc::from(id),
            name: Arc::from(name),
            origin,
            hidden: false,
            extensions: Vec::new(),
            filenames: Vec::new(),
            source,
            failed: AtomicBool::new(false),
            highlights,
            precedence,
            injections: None,
            brackets: None,
            outline: None,
            first_line: None,
            query_dir: None,
            queries: OnceLock::new(),
        }
    }

    /// An extension's language. Its query files are read, and compiled, on
    /// first use: building a registry opens none of its files.
    fn from_extension(spec: &ExtensionLanguage) -> Language {
        let id = spec.name.to_lowercase().replace(' ', "-");
        let source = match &spec.grammar {
            Some((name, path)) => GrammarSource::Wasm {
                key: GrammarKey {
                    extension_id: Arc::from(spec.extension_id.as_str()),
                    name: Arc::from(name.as_str()),
                },
                path: path.clone(),
            },
            None => GrammarSource::None,
        };
        let first_line = spec.first_line_pattern.as_deref().and_then(|pattern| {
            regex::Regex::new(pattern)
                .map_err(|e| {
                    tracing::warn!(
                        "language {}: bad first_line_pattern {pattern:?}: {e}",
                        spec.name
                    );
                })
                .ok()
        });
        Language {
            hidden: spec.hidden,
            // Every path suffix is both an exact file name and an extension.
            extensions: spec.path_suffixes.clone(),
            filenames: spec.path_suffixes.clone(),
            first_line,
            query_dir: Some(spec.dir.clone()),
            ..Language::new(
                &id,
                &spec.name,
                LanguageOrigin::Extension {
                    extension_id: Arc::from(spec.extension_id.as_str()),
                },
                source,
                Vec::new(),
                Precedence::LastPatternWins,
            )
        }
    }

    /// Whether the language has an injections query.
    #[doc(hidden)]
    pub fn has_injections(&self) -> bool {
        match &self.query_dir {
            Some(dir) => dir.join("injections.scm").is_file(),
            None => self.injections.is_some(),
        }
    }

    /// Whether the language has an outline query (`outline.scm`). For an
    /// extension language this looks for the file: one `stat`.
    pub fn has_outline(&self) -> bool {
        match &self.query_dir {
            Some(dir) => dir.join("outline.scm").is_file(),
            None => self.outline.is_some(),
        }
    }

    /// A fresh copy of the bundled language `id` with `outline` as its
    /// outline query, for tests.
    ///
    /// # Panics
    /// If no bundled language has that id.
    #[doc(hidden)]
    pub fn bundled_with_outline_for_test(id: &str, outline: &str) -> Arc<Language> {
        let language = LanguageRegistry::make_bundled_languages()
            .into_iter()
            .find(|l| &*l.id == id)
            .and_then(|l| Arc::try_unwrap(l).ok())
            .unwrap_or_else(|| panic!("no bundled language {id}"));
        Arc::new(Language {
            outline: Some(outline.to_string()),
            ..language
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
    pub(crate) fn queries(&self) -> Option<&LanguageQueries> {
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

    /// The highlights, injections, brackets and outline sources: as given,
    /// or read from [`Language::query_dir`] now.
    fn query_sources(&self) -> QuerySources {
        match &self.query_dir {
            Some(dir) => {
                let read = |file: &str| std::fs::read_to_string(dir.join(file)).ok();
                QuerySources {
                    highlights: vec![read("highlights.scm").unwrap_or_default()],
                    injections: read("injections.scm"),
                    brackets: read("brackets.scm"),
                    outline: read("outline.scm"),
                }
            }
            None => QuerySources {
                highlights: self.highlights.clone(),
                injections: self.injections.clone(),
                brackets: self.brackets.clone(),
                outline: self.outline.clone(),
            },
        }
    }

    fn compile_queries(&self) -> Result<LanguageQueries, String> {
        let grammar = self.grammar().map_err(|e| e.to_string())?;
        let QuerySources {
            highlights,
            injections,
            brackets,
            outline,
        } = self.query_sources();
        let sources: Vec<&str> = highlights.iter().map(String::as_str).collect();
        let highlights = HighlightQuery::new(&grammar, &sources, self.precedence, &self.id)?;
        let optional = |kind: &str, source: &Option<String>| {
            let source = source.as_deref()?;
            match Query::new(&grammar, source) {
                Ok(query) => Some(query),
                Err(e) if self.origin == LanguageOrigin::Bundled => {
                    panic!("bundled {kind} query for {} must compile: {e}", self.id)
                }
                // A broken side query costs that feature, not the colours.
                Err(e) => {
                    tracing::warn!("language {}: {kind} query: {e}", self.id);
                    None
                }
            }
        };
        Ok(LanguageQueries {
            highlights,
            injections: optional("injections", &injections)
                .map(|query| InjectionQuery::new(query, &self.id)),
            brackets: optional("brackets", &brackets),
            outline: optional("outline", &outline)
                .and_then(|query| OutlineQuery::new(query, &self.id)),
        })
    }

    /// Whether `name` is one of the exact file names; `ci` ignores ASCII case.
    fn has_filename(&self, name: &str, ci: bool) -> bool {
        self.filenames.iter().any(|f| {
            if ci {
                f.eq_ignore_ascii_case(name)
            } else {
                f == name
            }
        })
    }

    /// Whether `suffix` is one of the extensions; `ci` ignores ASCII case.
    fn has_extension(&self, suffix: &str, ci: bool) -> bool {
        self.extensions.iter().any(|e| {
            if ci {
                e.eq_ignore_ascii_case(suffix)
            } else {
                e == suffix
            }
        })
    }

    fn is_extension_language(&self) -> bool {
        matches!(self.origin, LanguageOrigin::Extension { .. })
    }
}

/// The query sources of a language, before they compile.
struct QuerySources {
    highlights: Vec<String>,
    injections: Option<String>,
    brackets: Option<String>,
    outline: Option<String>,
}

impl fmt::Debug for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Language")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Most bytes of the first line `first_line_pattern` looks at.
const FIRST_LINE_LIMIT: usize = 256;

#[derive(Debug)]
pub struct LanguageRegistry {
    languages: Vec<Arc<Language>>,
    associations: Associations,
    /// Ties already reported, so each is logged once.
    warned: Mutex<HashSet<String>>,
}

impl LanguageRegistry {
    /// Every language Lucerna ships. Queries compile lazily on first use.
    pub fn bundled() -> LanguageRegistry {
        Self::build(&[], Associations::default())
    }

    /// The bundled languages, then `extensions` (an extension language
    /// replaces the bundled one with the same id), and `associations` on top.
    pub fn build(extensions: &[ExtensionLanguage], associations: Associations) -> LanguageRegistry {
        let mut languages: Vec<Arc<Language>> = Self::bundled_languages().to_vec();
        for spec in extensions {
            let language = Language::from_extension(spec);
            languages.retain(|l| l.is_extension_language() || l.id != language.id);
            languages.push(Arc::new(language));
        }
        LanguageRegistry {
            languages,
            associations,
            warned: Mutex::new(HashSet::new()),
        }
    }

    /// The bundled languages, made once per process: every registry shares
    /// them, so their compiled queries survive a rebuild.
    fn bundled_languages() -> &'static [Arc<Language>] {
        static BUNDLED: OnceLock<Vec<Arc<Language>>> = OnceLock::new();
        BUNDLED.get_or_init(Self::make_bundled_languages)
    }

    fn make_bundled_languages() -> Vec<Arc<Language>> {
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
            lang(
                "markdown",
                "Markdown",
                &["md", "markdown"],
                &[],
                tree_sitter_md::LANGUAGE.into(),
                &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK],
                First,
            ),
            // Injected into Markdown's `inline` nodes; never picked by path.
            Language {
                hidden: true,
                ..lang(
                    "markdown_inline",
                    "Markdown Inline",
                    &[],
                    &[],
                    tree_sitter_md::INLINE_LANGUAGE.into(),
                    &[tree_sitter_md::HIGHLIGHT_QUERY_INLINE],
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
        languages.into_iter().map(Arc::new).collect()
    }

    /// The bundled languages plus `extra`, for tests.
    #[doc(hidden)]
    pub fn bundled_plus_for_test(extra: Arc<Language>) -> LanguageRegistry {
        let mut registry = LanguageRegistry::bundled();
        registry.languages.push(extra);
        registry
    }

    /// The same languages with other `files.associations`; nothing is
    /// rebuilt, so compiled queries and loaded grammars carry over.
    pub fn with_associations(&self, associations: Associations) -> LanguageRegistry {
        LanguageRegistry {
            languages: self.languages.clone(),
            associations,
            warned: Mutex::new(HashSet::new()),
        }
    }

    /// The WASM grammars the languages use.
    pub fn grammar_keys(&self) -> HashSet<GrammarKey> {
        self.languages
            .iter()
            .filter_map(|l| match &l.source {
                GrammarSource::Wasm { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn languages(&self) -> &[Arc<Language>] {
        &self.languages
    }

    /// Exact file names win over extensions; extension languages win over
    /// bundled ones. Ignores `files.associations` and the first line.
    pub fn language_for_path(&self, path: &Path) -> Option<Arc<Language>> {
        let name = path.file_name()?.to_str()?;
        self.by_file_name(name).or_else(|| self.by_suffix(name))
    }

    /// The language of a file, by the first rule that decides:
    /// `files.associations` (longest glob), exact file name, longest
    /// suffix, then an extension's `first_line_pattern`. `path` is
    /// relative to the workspace root, for globs containing `/`.
    pub fn language_for_file(&self, path: &Path, first_line: &str) -> Option<Arc<Language>> {
        self.associations
            .matches(path)
            .into_iter()
            .find_map(|target| self.by_id_or_name(target))
            .or_else(|| self.language_for_path(path))
            .or_else(|| self.by_first_line(first_line))
    }

    /// A language by name, then id, then path suffix, ignoring ASCII case.
    /// Only the part after the last `.` counts as the name when `name` has
    /// one (`foo.rs` is `rs`). Includes hidden languages.
    pub fn language_by_name(&self, name: &str) -> Option<Arc<Language>> {
        let short = name.rsplit_once('.').map_or(name, |(_, after)| after);
        self.by_id_or_name(name)
            .or_else(|| self.by_id_or_name(short))
            .or_else(|| {
                self.layered(|l| l.has_extension(short, true))
                    .map(|(l, _)| l.clone())
            })
    }

    fn by_id_or_name(&self, key: &str) -> Option<Arc<Language>> {
        let ordered = || self.extension_first();
        ordered()
            .find(|l| l.name.eq_ignore_ascii_case(key))
            .or_else(|| ordered().find(|l| l.id.eq_ignore_ascii_case(key)))
            .cloned()
    }

    /// Extension languages (highest id first, which breaks ties), then bundled.
    fn extension_first(&self) -> impl Iterator<Item = &Arc<Language>> {
        let mut ext: Vec<&Arc<Language>> = self
            .languages
            .iter()
            .filter(|l| l.is_extension_language())
            .collect();
        ext.sort_by(|a, b| extension_key(b).cmp(&extension_key(a)));
        ext.into_iter()
            .chain(self.languages.iter().filter(|l| !l.is_extension_language()))
    }

    /// The first non-hidden language satisfying `matches`, in layer order.
    /// The second element is whether another extension language of the same
    /// layer matched too.
    fn layered(&self, matches: impl Fn(&Language) -> bool) -> Option<(&Arc<Language>, bool)> {
        let mut first: Option<&Arc<Language>> = None;
        for language in self.extension_first().filter(|l| matches(l)) {
            match first {
                None if language.is_extension_language() => first = Some(language),
                None => return Some((language, false)),
                Some(first) => return Some((first, language.is_extension_language())),
            }
        }
        first.map(|l| (l, false))
    }

    fn by_file_name(&self, name: &str) -> Option<Arc<Language>> {
        [false, true]
            .into_iter()
            .find_map(|ci| self.visible(|l| l.has_filename(name, ci)))
    }

    /// Tries `name`'s `.`-suffixes from the longest to the shortest.
    fn by_suffix(&self, name: &str) -> Option<Arc<Language>> {
        for (dot, _) in name.match_indices('.') {
            let suffix = &name[dot + 1..];
            if suffix.is_empty() {
                continue;
            }
            for ci in [false, true] {
                if let Some(found) = self.visible(|l| l.has_extension(suffix, ci)) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Like [`Self::layered`] over visible languages, reporting a tie.
    fn visible(&self, matches: impl Fn(&Language) -> bool) -> Option<Arc<Language>> {
        let (language, tie) = self.layered(|l| !l.hidden && matches(l))?;
        if tie {
            let key = format!("tie:{}", language.id);
            if self.warned.lock().is_ok_and(|mut w| w.insert(key)) {
                tracing::warn!(
                    "several extensions claim the same files; using language {}",
                    language.name
                );
            }
        }
        Some(language.clone())
    }

    fn by_first_line(&self, first_line: &str) -> Option<Arc<Language>> {
        let mut end = first_line.len().min(FIRST_LINE_LIMIT);
        while !first_line.is_char_boundary(end) {
            end -= 1;
        }
        let line = &first_line[..end];
        self.extension_first()
            .find(|l| !l.hidden && l.first_line.as_ref().is_some_and(|re| re.is_match(line)))
            .cloned()
    }
}

/// Orders extension languages: by extension id, then language id.
fn extension_key(language: &Language) -> (&str, &str) {
    match &language.origin {
        LanguageOrigin::Extension { extension_id } => (extension_id, &language.id),
        LanguageOrigin::Bundled => ("", &language.id),
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
        extensions: extensions.iter().map(|s| s.to_string()).collect(),
        filenames: filenames.iter().map(|s| s.to_string()).collect(),
        injections: match id {
            "markdown" => Some(tree_sitter_md::INJECTION_QUERY_BLOCK.to_string()),
            "markdown_inline" => Some(tree_sitter_md::INJECTION_QUERY_INLINE.to_string()),
            "html" => Some(tree_sitter_html::INJECTIONS_QUERY.to_string()),
            _ => None,
        },
        ..Language::new(
            id,
            name,
            LanguageOrigin::Bundled,
            GrammarSource::Native(grammar),
            highlights.iter().map(|s| s.to_string()).collect(),
            precedence,
        )
    }
}
