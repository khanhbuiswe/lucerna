//! Read-only git integration: a hardened `git` runner and parsers for its output.
//! This crate never depends on gpui.

mod blobs;
mod command;
mod hunks;
mod repo;
mod status;

#[doc(hidden)]
pub use command::run_with_timeout;
pub use command::{GitBinary, GitError};
pub use hunks::{Hunk, HunkKind, MAX_DIFF_BYTES, diff_hunks, load_base_text};
pub use repo::Repo;
pub use status::{FileStatus, parse_status_v2};
