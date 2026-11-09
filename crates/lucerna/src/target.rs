//! Which folder and file a `luc [PATH]` invocation opens.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Resolves the command-line path to a canonical root folder and, for a
/// file, the file to open as a pinned tab.
///
/// - `None`: the current directory.
/// - A directory: that directory.
/// - A file: the nearest ancestor holding `.git`, else the file's parent.
///
/// # Errors
/// `path not found: …` when the path does not exist.
pub fn resolve_target(path: Option<PathBuf>) -> Result<(PathBuf, Option<PathBuf>), String> {
    let path = match path {
        Some(path) => path,
        None => std::env::current_dir().map_err(|err| format!("current directory: {err}"))?,
    };
    let canonical = std::fs::canonicalize(&path).map_err(|err| match err.kind() {
        ErrorKind::NotFound => format!("path not found: {}", path.display()),
        _ => format!("{}: {err}", path.display()),
    })?;
    if canonical.is_dir() {
        return Ok((canonical, None));
    }
    let parent = canonical.parent().unwrap_or(Path::new("/"));
    let root = parent
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(parent)
        .to_path_buf();
    Ok((root, Some(canonical)))
}
