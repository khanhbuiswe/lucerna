//! Zed extension manifests, language configs and snippet validation.
//!
//! Pure data parsing: no dependency on syntax, theme or UI crates.

pub mod builder;
pub mod disk;
pub mod git;
pub mod index;
pub mod language_config;
pub mod lock;
pub mod manifest;
pub mod process;
pub mod registry;
pub mod snippets;
pub mod store;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod toolchain;
pub mod validate;

pub use language_config::{LanguageConfig, QUERY_KINDS, load_language_config, query_files};
pub use manifest::{ExtensionManifest, GrammarManifest, ManifestError, load_manifest};
pub use snippets::{SnippetFile, check_snippet_body, check_snippets};
