//! Per-language `config.toml` and query file discovery.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::manifest::ManifestError;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LanguageConfig {
    pub name: String,
    #[serde(default)]
    pub grammar: Option<String>,
    #[serde(default)]
    pub path_suffixes: Vec<String>,
    #[serde(default)]
    pub first_line_pattern: Option<String>,
    #[serde(default)]
    pub line_comments: Vec<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub tab_size: Option<u32>,
}

pub const QUERY_KINDS: [&str; 10] = [
    "highlights",
    "injections",
    "brackets",
    "outline",
    "indents",
    "textobjects",
    "overrides",
    "redactions",
    "runnables",
    "debugger",
];

/// Read `<dir>/config.toml`. Unknown fields are ignored.
pub fn load_language_config(dir: &Path) -> Result<LanguageConfig, ManifestError> {
    let path = dir.join("config.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| ManifestError::Io(path.clone(), e.to_string()))?;
    toml::from_str(&text).map_err(|e| ManifestError::Parse(path, e.to_string()))
}

/// Recognised `.scm` files in `dir` (sorted by file name), plus a warning
/// for each `.scm` file whose kind is not recognised.
pub fn query_files(dir: &Path) -> (Vec<(String, PathBuf)>, Vec<String>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    entries.sort();
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    for path in entries {
        if !path.is_file() || path.extension().is_none_or(|e| e != "scm") {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if QUERY_KINDS.contains(&stem) {
            files.push((stem.to_string(), path));
        } else {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            warnings.push(format!("ignoring unsupported query file {name}"));
        }
    }
    (files, warnings)
}
