//! Git integration: a hardened `git` runner, parsers for its output, and
//! branch switching (the only write).
//! This crate never depends on gpui.

mod blobs;
mod branch;
mod command;
mod hunks;
mod repo;
mod status;
mod watch;

pub use branch::{BranchRef, SwitchTarget, is_valid_branch_name, sanitize_branch_name};
#[doc(hidden)]
pub use command::run_with_timeout;
pub use command::{GitBinary, GitError};
pub use hunks::{Hunk, HunkKind, MAX_DIFF_BYTES, diff_hunks, load_base_text};
pub use repo::Repo;
pub use status::{FileStatus, parse_status_v2};
pub use watch::{GitDirWatcher, is_relevant_git_path};
