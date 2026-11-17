//! Project model: background, gitignore-aware worktree scanning.

mod coalesce;
pub mod fs_ops;
mod fuzzy;
mod ignored;
pub mod replace;
mod search;
mod snapshot;
mod watch;
mod worktree;

pub use coalesce::{Coalesced, RESCAN_THRESHOLD, RawEvent, coalesce};
pub use fuzzy::{FileMatch, QUICK_OPEN_LIMIT, match_files};
pub use search::{
    Excerpt, LineKind, LineMatch, MAX_LINE_CHARS, MAX_RESULTS, SearchEvent, SearchQuery,
    build_pattern, search,
};
pub use watch::{DEBOUNCE, watch_error_reason};

pub use snapshot::{Entry, EntryKind, RootStatus, WorktreeSnapshot};
pub use worktree::{Change, ChangeKind, ScanOptions, Worktree, WorktreeEvent};
