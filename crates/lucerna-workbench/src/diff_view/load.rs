//! Loading one file of a diff tab off the UI thread: its base text from a
//! git revision, its current text from the open buffer or the disk, and
//! its excerpts.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lucerna_git::{FileStatus, GitError, MAX_DIFF_BYTES, Repo};
use lucerna_project::Excerpt;
use lucerna_text::BufferSnapshot;

use super::build::{CONTEXT_LINES, build_file};
use crate::excerpt_list::TextSource;

/// The note of a binary file.
pub(crate) const BINARY_NOTE: &str = "Binary file";
/// The note of a file over [`MAX_DIFF_BYTES`].
pub(crate) const TOO_LARGE_NOTE: &str = "File too large to diff";

/// One side of a diff: LF text, or why it is not diffed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Side {
    Text(Arc<str>),
    Note(&'static str),
}

impl Side {
    fn empty() -> Side {
        Side::Text(Arc::from(""))
    }

    /// Decodes file bytes: too large, binary (a NUL in the first 8 KB), or
    /// LF text.
    fn from_bytes(bytes: &[u8]) -> Side {
        if bytes.len() as u64 > MAX_DIFF_BYTES {
            return Side::Note(TOO_LARGE_NOTE);
        }
        match lucerna_text::decode_file(bytes, false) {
            Ok(decoded) => Side::Text(Arc::from(decoded.text)),
            Err(_) => Side::Note(BINARY_NOTE),
        }
    }
}

/// What loading one file needs; built on the UI thread.
pub(super) struct LoadInput {
    pub repo: Repo,
    /// Absolute, for the disk read.
    pub abs_path: PathBuf,
    /// Workspace-relative, for the blob read.
    pub path: Arc<Path>,
    pub status: FileStatus,
    /// The revision of the base side; `None` (no commit yet) means empty.
    pub rev: Option<String>,
    /// The open buffer's text, which may hold unsaved changes.
    pub buffer: Option<BufferSnapshot>,
    /// The base side, when already read at `rev`.
    pub cached_base: Option<Side>,
    /// One excerpt with every line, not just the changes and their context.
    pub full_file: bool,
}

/// One loaded file.
#[derive(Debug, Default)]
pub(super) struct Loaded {
    pub excerpts: Vec<Excerpt>,
    pub added: u32,
    pub removed: u32,
    /// "Binary file" or "File too large to diff": shown instead of lines.
    pub note: Option<&'static str>,
    pub source: TextSource,
    /// The base side read, to cache for the next load at the same rev.
    pub base: Option<Side>,
}

/// Blocking: reads both sides of `input`'s file and builds its excerpts.
/// A symlink gets no excerpts: git stores the link target as its blob,
/// not the content of the file it points to (spec §6).
pub(super) fn load_file(input: LoadInput) -> Result<Loaded, GitError> {
    let is_link =
        std::fs::symlink_metadata(&input.abs_path).is_ok_and(|m| m.file_type().is_symlink());
    if is_link {
        return Ok(Loaded::default());
    }
    let base = match (input.cached_base, &input.rev, input.status) {
        (Some(side), _, _) => side,
        (None, _, FileStatus::Untracked) | (None, None, _) => Side::empty(),
        (None, Some(rev), _) => match input.repo.read_blob(rev, &input.path)? {
            Some(bytes) => Side::from_bytes(&bytes),
            None => Side::empty(),
        },
    };
    let current = if input.status == FileStatus::Deleted {
        Side::empty()
    } else if let Some(buffer) = &input.buffer {
        let text = buffer.text();
        if text.len() as u64 > MAX_DIFF_BYTES {
            Side::Note(TOO_LARGE_NOTE)
        } else {
            Side::Text(Arc::from(text))
        }
    } else {
        match std::fs::read(&input.abs_path) {
            Ok(bytes) => Side::from_bytes(&bytes),
            // Deleted since the status: nothing left on this side.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Side::empty(),
            Err(err) => return Err(GitError::Io(err)),
        }
    };
    let (base_text, current_text) = match (&base, &current) {
        (Side::Text(b), Side::Text(c)) => (b.clone(), c.clone()),
        (Side::Note(a), Side::Note(b)) if *a == TOO_LARGE_NOTE || *b == TOO_LARGE_NOTE => {
            return Ok(noted(TOO_LARGE_NOTE, base));
        }
        (Side::Note(note), _) | (_, Side::Note(note)) => return Ok(noted(note, base)),
    };
    let built = build_file(&base_text, &current_text, CONTEXT_LINES, input.full_file);
    Ok(Loaded {
        excerpts: built.excerpts,
        added: built.added,
        removed: built.removed,
        note: None,
        source: TextSource {
            current: Some(current_text),
            base: Some(base_text),
        },
        base: Some(base),
    })
}

/// A file shown as `note` only.
fn noted(note: &'static str, base: Side) -> Loaded {
    Loaded {
        note: Some(note),
        base: Some(base),
        ..Loaded::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sides_from_bytes() {
        assert_eq!(
            Side::from_bytes(b"a\r\nb\n"),
            Side::Text(Arc::from("a\nb\n"))
        );
        assert_eq!(Side::from_bytes(b"a\0b"), Side::Note(BINARY_NOTE));
        let big = vec![b'a'; MAX_DIFF_BYTES as usize + 1];
        assert_eq!(Side::from_bytes(&big), Side::Note(TOO_LARGE_NOTE));
    }
}
