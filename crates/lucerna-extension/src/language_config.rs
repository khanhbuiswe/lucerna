//! Per-language `config.toml` and query file discovery.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
    /// Auto-closing and surrounding pairs; empty means the editor default.
    #[serde(default)]
    pub brackets: Vec<BracketPairConfig>,
    /// Characters before which the editor may auto-close.
    #[serde(default)]
    pub autoclose_before: Option<String>,
    #[serde(default)]
    pub block_comment: Option<BlockComment>,
}

fn yes() -> bool {
    true
}

/// One `brackets` entry of `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BracketPairConfig {
    pub start: String,
    pub end: String,
    #[serde(default = "yes")]
    pub close: bool,
    #[serde(default = "yes")]
    pub newline: bool,
    #[serde(default)]
    pub not_in: Vec<String>,
}

/// A block comment's delimiters, trimmed of surrounding spaces. Written
/// either `["/* ", " */"]` or `{ start = "/*", end = "*/", .. }`; other
/// keys of the table are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "BlockCommentRaw")]
pub struct BlockComment {
    pub start: String,
    pub end: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BlockCommentRaw {
    Pair(String, String),
    Table { start: String, end: String },
}

impl From<BlockCommentRaw> for BlockComment {
    fn from(raw: BlockCommentRaw) -> BlockComment {
        let (start, end) = match raw {
            BlockCommentRaw::Pair(start, end) | BlockCommentRaw::Table { start, end } => {
                (start, end)
            }
        };
        BlockComment {
            start: start.trim().to_string(),
            end: end.trim().to_string(),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_toml_brackets_and_block_comment_both_shapes() {
        let cfg: LanguageConfig = toml::from_str(
            r#"
name = "X"
brackets = [
  { start = "{", end = "}", close = true, newline = true },
  { start = "\"", end = "\"", close = true, newline = false, not_in = ["string"] },
]
autoclose_before = ";:.,=}])>"
block_comment = ["/* ", " */"]
"#,
        )
        .unwrap();
        assert_eq!(cfg.brackets.len(), 2);
        assert_eq!(cfg.brackets[0].start, "{");
        assert!(cfg.brackets[0].close && cfg.brackets[0].newline);
        assert_eq!(cfg.brackets[1].start, "\"");
        assert!(!cfg.brackets[1].newline);
        assert_eq!(cfg.brackets[1].not_in, ["string"]);
        assert_eq!(cfg.autoclose_before.as_deref(), Some(";:.,=}])>"));
        let block = cfg.block_comment.unwrap();
        assert_eq!((block.start.as_str(), block.end.as_str()), ("/*", "*/"));

        let cfg: LanguageConfig = toml::from_str(
            r#"
name = "X"
block_comment = { start = "/* ", end = " */", prefix = "* ", tab_size = 1 }
brackets = [{ start = "(", end = ")" }]
"#,
        )
        .unwrap();
        let block = cfg.block_comment.unwrap();
        assert_eq!((block.start.as_str(), block.end.as_str()), ("/*", "*/"));
        assert!(cfg.brackets[0].close && cfg.brackets[0].newline);
    }
}
