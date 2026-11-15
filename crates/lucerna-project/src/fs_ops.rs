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

/// Separators that split a multi-level name on this platform.
#[cfg(windows)]
const SEPARATORS: &[char] = &['/', '\\'];
#[cfg(not(windows))]
const SEPARATORS: &[char] = &['/'];

/// Splits `name` on `seps` and validates every component. Pure so the
/// Windows separator rules are testable on every platform.
#[doc(hidden)]
pub fn split_components<'a>(name: &'a str, seps: &[char]) -> Result<Vec<&'a str>, NameError> {
    let mut out = Vec::new();
    for c in name.split(|ch| seps.contains(&ch)) {
        if c.is_empty() {
            // "a//b": ignore empty components, like a path split would.
            continue;
        }
        if component_invalid(c) {
            return Err(NameError::Invalid(c.to_string()));
        }
        out.push(c);
    }
    Ok(out)
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
    for c in split_components(name, SEPARATORS)? {
        target.push(c);
    }
    if exists(&target) {
        let lower = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_lowercase());
        let same =
            original.is_some_and(|o| o.parent() == target.parent() && lower(o) == lower(&target));
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir(path)
}

/// True when both paths are the very same file system entry.
fn same_entry(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
    }
}

/// Renames without replacing an existing target. Atomic on macOS and Linux;
/// elsewhere it checks then renames (not atomic).
fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let f = CString::new(from.as_os_str().as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let t = CString::new(to.as_os_str().as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        #[cfg(target_os = "macos")]
        // SAFETY: both pointers are valid NUL-terminated strings.
        let rc = unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        // SAFETY: both pointers are valid NUL-terminated strings.
        let rc = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                f.as_ptr(),
                libc::AT_FDCWD,
                t.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if rc == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            // Unsupported by this file system or kernel: fall back below.
            Some(libc::EINVAL) | Some(libc::ENOSYS) | Some(libc::ENOTSUP) => {}
            _ => return Err(err),
        }
    }
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(already_exists());
    }
    std::fs::rename(from, to)
}

pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // A case-only rename targets the same entry: plain rename is safe there.
    if from != to && same_entry(from, to) {
        return std::fs::rename(from, to);
    }
    rename_no_replace(from, to)
}

pub fn trash(path: &Path) -> Result<(), TrashError> {
    trash_context()
        .delete(path)
        .map_err(|e| TrashError(e.to_string()))
}

/// On macOS the crate's default asks Finder over AppleScript, which pops an
/// automation permission prompt and fails for good if it is denied. Ask
/// NSFileManager (`trashItemAtURL`) directly instead.
#[cfg(target_os = "macos")]
fn trash_context() -> trash::TrashContext {
    use trash::macos::{DeleteMethod, TrashContextExtMacos};
    let mut ctx = trash::TrashContext::default();
    ctx.set_delete_method(DeleteMethod::NsFileManager);
    ctx
}

#[cfg(not(target_os = "macos"))]
fn trash_context() -> trash::TrashContext {
    trash::TrashContext::default()
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

/// First free target for pasting `name` into `dir`: `name` itself, then
/// `"{stem} copy{ext}"`, `"{stem} copy 2{ext}"`, ... The range is the byte
/// range of the final file name to select for renaming: the stem plus the
/// " copy N" suffix, never the extension.
pub fn paste_name(
    dir: &Path,
    name: &str,
    is_dir: bool,
    exists: impl Fn(&Path) -> bool,
) -> (PathBuf, Range<usize>) {
    let stem_len = stem_range(name, is_dir).end;
    let (stem, ext) = name.split_at(stem_len);
    let first = dir.join(name);
    if !exists(&first) {
        return (first, 0..stem_len);
    }
    let mut n = 1u32;
    loop {
        let base = if n == 1 {
            format!("{stem} copy")
        } else {
            format!("{stem} copy {n}")
        };
        let candidate = dir.join(format!("{base}{ext}"));
        if !exists(&candidate) {
            return (candidate, 0..base.len());
        }
        n += 1;
    }
}

/// Component-wise: true when `path` is `ancestor` or lies below it.
pub fn is_within(path: &Path, ancestor: &Path) -> bool {
    path.starts_with(ancestor)
}

#[cfg(unix)]
fn copy_symlink(from: &Path, to: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(std::fs::read_link(from)?, to)
}

#[cfg(windows)]
fn copy_symlink(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::fs::{symlink_dir, symlink_file};
    let target = std::fs::read_link(from)?;
    // The link's own kind decides; a dangling link falls back to a file link.
    let points_to_dir = std::fs::metadata(from).is_ok_and(|m| m.is_dir());
    if points_to_dir {
        symlink_dir(target, to)
    } else {
        symlink_file(target, to)
    }
}

/// Copies a file, directory tree or symlink from `from` to `to` without ever
/// replacing anything: an existing `to` fails with `AlreadyExists`. Symlinks
/// are recreated, not followed. Refuses (`InvalidInput`) when `to` is inside
/// `from`.
pub fn copy_entry(from: &Path, to: &Path) -> io::Result<()> {
    if is_within(to, from) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cannot copy a folder into itself",
        ));
    }
    copy_entry_inner(from, to)
}

fn copy_entry_inner(from: &Path, to: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    let ty = meta.file_type();
    if ty.is_symlink() {
        copy_symlink(from, to)
    } else if ty.is_dir() {
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_entry_inner(&entry.path(), &to.join(entry.file_name()))?;
        }
        // Directory permissions last, so a read-only source stays writable
        // while its contents are copied.
        std::fs::set_permissions(to, meta.permissions())
    } else {
        let mut src = std::fs::File::open(from)?;
        let mut dst = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(to)?;
        io::copy(&mut src, &mut dst)?;
        std::fs::set_permissions(to, meta.permissions())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn macos_trash_uses_file_manager_not_finder() {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        assert!(matches!(
            super::trash_context().delete_method(),
            DeleteMethod::NsFileManager
        ));
    }
}
