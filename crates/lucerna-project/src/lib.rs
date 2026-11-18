//! Project model: background, gitignore-aware worktree scanning.

mod coalesce;
pub mod fs_ops;
mod fuzzy;
mod ignored;
pub mod replace;
mod replace_files;
mod search;
mod snapshot;
mod watch;
mod worktree;

pub use coalesce::{Coalesced, RESCAN_THRESHOLD, RawEvent, coalesce};
pub use fuzzy::{FileMatch, QUICK_OPEN_LIMIT, match_files};
pub use search::{
    Excerpt, LineKind, LineMatch, MAX_LINE_CHARS, MAX_RESULTS, SearchEvent, SearchQuery,
    WHOLE_READ_BYTES, build_pattern, line_matches, search, search_regex, search_text,
};
pub use watch::{DEBOUNCE, watch_error_reason};

pub use replace_files::{
    FileOutcome, ReplaceProgress, ReplaceSpec, ShownLine, SkipReason, edits_for_text, replace_file,
    replace_file_with_hook, replace_file_with_limit, replace_files,
};
pub use snapshot::{Entry, EntryKind, RootStatus, WorktreeSnapshot};
pub use worktree::{Change, ChangeKind, ScanOptions, Worktree, WorktreeEvent};
