//! Zed extension manifests, language configs and snippet validation.
//!
//! Pure data parsing: no dependency on syntax, theme or UI crates.

pub mod language_config;
pub mod manifest;
pub mod snippets;

pub use language_config::{LanguageConfig, QUERY_KINDS, load_language_config, query_files};
pub use manifest::{ExtensionManifest, GrammarManifest, ManifestError, load_manifest};
pub use snippets::{SnippetFile, check_snippet_body, check_snippets};
