//! File operations for the Explorer. Blocking: callers run them off the UI thread.

use std::fmt;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    Empty,
    LeadingSlash,
    Invalid(String),
    Exists(String),
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("A file or folder name must be provided."),
            Self::LeadingSlash => f.write_str("A file or folder name cannot start with a slash."),
            Self::Invalid(n) => write!(
                f,
                "The name {n} is not valid as a file or folder name. Please choose a different name."
            ),
            Self::Exists(n) => write!(
                f,
                "A file or folder {n} already exists at this location. Please choose a different name."
            ),
        }
    }
}

impl std::error::Error for NameError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashError(pub String);

impl fmt::Display for TrashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TrashError {}

fn component_invalid(c: &str) -> bool {
    if c == "." || c == ".." || c.contains('\0') {
        return true;
    }
    if cfg!(windows) {
        if c.chars().any(|ch| "<>:\"|?*".contains(ch)) || c.ends_with('.') {
            return true;
        }
        let stem = c.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = ["CON", "PRN", "AUX", "NUL"];
        let is_dev = |p: &str| {
            stem.strip_prefix(p)
                .is_some_and(|d| d.len() == 1 && matches!(d.as_bytes()[0], b'1'..=b'9'))
        };
        if reserved.contains(&stem.as_str()) || is_dev("COM") || is_dev("LPT") {
            return true;
        }
    }
    false
}

/// Validates `name` (possibly multi-level, split on `/`) under `parent`.
pub fn validate_new_name(
    parent: &Path,
    name: &str,
    original: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> Result<PathBuf, NameError> {
    let name = name.trim_end();
    if name.trim().is_empty() {
        return Err(NameError::Empty);
    }
    if name.starts_with('/') || name.starts_with('\\') {
        return Err(NameError::LeadingSlash);
    }
    let mut target = parent.to_path_buf();
    for c in name.split('/') {
        if c.is_empty() {
            // "a//b": ignore empty components, like a path split would.
            continue;
        }
        if component_invalid(c) {
            return Err(NameError::Invalid(c.to_string()));
        }
        target.push(c);
    }
    if exists(&target) {
        let same = original.is_some_and(|o| {
            o.to_string_lossy().to_lowercase() == target.to_string_lossy().to_lowercase()
        });
        if !same {
            let shown = target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.to_string());
            return Err(NameError::Exists(shown));
        }
    }
    Ok(target)
}

fn already_exists() -> io::Error {
    io::Error::from(io::ErrorKind::AlreadyExists)
}

pub fn create_file(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(drop)
}

pub fn create_dir(path: &Path) -> io::Result<()> {
    if path.exists() {
        return Err(already_exists());
    }
    std::fs::create_dir_all(path)
}

pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    if to.exists() {
        let case_only = from != to
            && from.to_string_lossy().to_lowercase() == to.to_string_lossy().to_lowercase();
        if !case_only {
            return Err(already_exists());
        }
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(from, to)
}

pub fn trash(path: &Path) -> Result<(), TrashError> {
    trash::delete(path).map_err(|e| TrashError(e.to_string()))
}

pub fn delete_permanently(path: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(path)?.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Byte range selected when renaming: whole name for dirs and dotfiles
/// without another dot, else up to the last dot.
pub fn stem_range(name: &str, is_dir: bool) -> Range<usize> {
    if is_dir {
        return 0..name.len();
    }
    match name.rfind('.') {
        Some(i) if i > 0 => 0..i,
        _ => 0..name.len(),
    }
}
