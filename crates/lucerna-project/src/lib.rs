//! Project model: background, gitignore-aware worktree scanning.

mod coalesce;
mod ignored;
mod snapshot;
mod watch;
mod worktree;

pub use coalesce::{Coalesced, RESCAN_THRESHOLD, RawEvent, coalesce};
pub use watch::DEBOUNCE;

pub use snapshot::{Entry, EntryKind, WorktreeSnapshot};
pub use worktree::{Change, ChangeKind, ScanOptions, Worktree, WorktreeEvent};
