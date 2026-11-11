//! The default server table names languages that exist, or that extensions
//! provide.

use lucerna_lsp::table::ServerTable;
use lucerna_syntax::LanguageRegistry;

/// Ids bundled in Lucerna that the table must resolve.
const BUNDLED_IN_TABLE: &[&str] = &[
    "rust",
    "go",
    "python",
    "typescript",
    "typescriptreact",
    "javascript",
    "json",
    "markdown",
    "toml",
    "yaml",
    "shellscript",
    "html",
    "css",
];

#[test]
fn table_languages_resolve() {
    let registry = LanguageRegistry::bundled();
    let bundled: Vec<&str> = registry.languages().iter().map(|l| &*l.id).collect();
    let table = ServerTable::build(None);
    let named: std::collections::BTreeSet<&str> = table
        .servers
        .values()
        .flat_map(|d| d.languages.iter().map(String::as_str))
        .collect();
    // Every bundled id the table names must exist in the registry (a typo in
    // either side shows here); ids from extensions are allowed to be absent.
    for id in BUNDLED_IN_TABLE {
        assert!(
            bundled.contains(id),
            "bundled language {id} missing from the registry"
        );
        assert!(
            named.contains(id),
            "table does not name bundled language {id}"
        );
    }
    for id in &named {
        assert!(
            id.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-.#+_".contains(c)),
            "language id {id} is not in extension form"
        );
    }
}
