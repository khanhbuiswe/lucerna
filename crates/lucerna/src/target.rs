//! Which folder and file a `luc [PATH]` invocation opens.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// What `luc` opens: a canonical root folder and, for a file, the file to
/// open as a pinned tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub root: PathBuf,
    pub file: Option<PathBuf>,
    /// The file is opened alone, as in Zed and VS Code: `root` (its
    /// parent) is not scanned or watched, and the sidebar lists only the
    /// file. Always set for a file.
    pub single_file: bool,
}

impl Target {
    /// A folder, scanned as usual.
    pub fn folder(root: PathBuf) -> Target {
        Target {
            root,
            file: None,
            single_file: false,
        }
    }
}

/// Whether `luc` should ask for a folder instead of opening the current
/// directory: no path was given and the process runs in `/`, which is how
/// Finder and the Dock start the app bundle. Opening `/` would scan the
/// whole disk.
pub fn prompts_for_folder(path: Option<&Path>, cwd: &Path) -> bool {
    path.is_none() && cwd == Path::new("/")
}

/// Resolves the command-line path, with `$HOME` from the environment
/// (see [`resolve_target_with_home`]).
///
/// # Errors
/// `path not found: …` when the path does not exist.
pub fn resolve_target(path: Option<PathBuf>) -> Result<Target, String> {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from);
    resolve_target_with_home(path, home.as_deref())
}

/// Resolves the command-line path to what [`Target`] opens:
///
/// - `None`: the current directory.
/// - A directory: that directory.
/// - A file: the file alone ([`Target::single_file`]), rooted at its
///   parent, as Zed and VS Code open one. Neither its folder nor the git
///   repository holding it is scanned (see
///   `docs/superpowers/specs/2026-11-13-single-file-open-design.md`).
///
/// `home` is no longer consulted; it stays so callers need not change.
///
/// # Errors
/// `path not found: …` when the path does not exist.
pub fn resolve_target_with_home(
    path: Option<PathBuf>,
    _home: Option<&Path>,
) -> Result<Target, String> {
    let path = match path {
        Some(path) => path,
        None => std::env::current_dir().map_err(|err| format!("current directory: {err}"))?,
    };
    let canonical = std::fs::canonicalize(&path).map_err(|err| match err.kind() {
        ErrorKind::NotFound => format!("path not found: {}", path.display()),
        _ => format!("{}: {err}", path.display()),
    })?;
    if canonical.is_dir() {
        return Ok(Target::folder(canonical));
    }
    let parent = canonical.parent().unwrap_or(Path::new("/"));
    Ok(Target {
        root: parent.to_path_buf(),
        file: Some(canonical),
        single_file: true,
    })
}
