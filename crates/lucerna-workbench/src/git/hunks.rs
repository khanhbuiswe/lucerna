//! Gutter hunks of one tracked buffer, computed off the UI thread.

use std::path::Path;
use std::sync::Arc;

use lucerna_git::{GitError, Hunk, MAX_DIFF_BYTES, Repo, diff_hunks, load_base_text};
use lucerna_text::BufferSnapshot;

use super::DiffBase;

/// Inputs for diffing one tracked buffer off the UI thread.
pub(super) struct HunkWork {
    pub(super) repo: Repo,
    pub(super) path: Arc<Path>,
    pub(super) snapshot: BufferSnapshot,
    pub(super) rev: Option<String>,
    /// The base text, when cached for `rev`.
    pub(super) cached: Option<Option<Arc<str>>>,
    pub(super) diff_base: DiffBase,
}

impl HunkWork {
    /// Blocking: loads the base text when needed, then diffs.
    pub(super) fn run(self) -> Diffed {
        let text = match (self.cached, &self.rev) {
            (Some(text), _) => Ok(text),
            (None, Some(rev)) => load_base(&self.repo, rev, &self.path, self.diff_base),
            (None, None) => Ok(None),
        };
        match text {
            Ok(text) => Diffed {
                hunks: diff_buffer(text.as_ref(), &self.snapshot),
                rev: self.rev,
                text,
                failed: false,
            },
            Err(err) => {
                let path = self.path.display();
                tracing::warn!(%err, %path, "could not read the git base");
                Diffed {
                    rev: self.rev,
                    text: None,
                    hunks: None,
                    failed: true,
                }
            }
        }
    }
}

/// One buffer diffed against the base text of `rev`.
pub(super) struct Diffed {
    pub(super) rev: Option<String>,
    pub(super) text: Option<Arc<str>>,
    pub(super) hunks: Option<Arc<[Hunk]>>,
    /// Reading the base failed: the hunks shown stay, nothing is cached, and
    /// the next `track_buffer` or rescan retries.
    pub(super) failed: bool,
}

/// The base text of `path` at `rev`. `None` (no hunks) for files missing
/// there, binary or over 8 MB, except that with `"defaultBranch"` a file
/// created and committed on the branch is compared with empty text. An
/// error is not "no base": the caller keeps what it shows.
fn load_base(
    repo: &Repo,
    rev: &str,
    path: &Path,
    diff_base: DiffBase,
) -> Result<Option<Arc<str>>, GitError> {
    Ok(match repo.read_blob(rev, path)? {
        Some(bytes) => load_base_text(&bytes).map(Arc::from),
        None if diff_base == DiffBase::DefaultBranch && repo.read_blob("HEAD", path)?.is_some() => {
            Some(Arc::from(""))
        }
        None => None,
    })
}

fn diff_buffer(base: Option<&Arc<str>>, snapshot: &BufferSnapshot) -> Option<Arc<[Hunk]>> {
    let base = base?;
    let text = snapshot.text();
    if text.len() as u64 > MAX_DIFF_BYTES {
        return None;
    }
    Some(diff_hunks(base, &text).into())
}
