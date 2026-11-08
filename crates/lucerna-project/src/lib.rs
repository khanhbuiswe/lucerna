//! Project model: background, gitignore-aware worktree scanning.

mod snapshot;
mod worktree;

pub use snapshot::{Entry, EntryKind, WorktreeSnapshot};
pub use worktree::{Change, ChangeKind, ScanOptions, Worktree, WorktreeEvent};
