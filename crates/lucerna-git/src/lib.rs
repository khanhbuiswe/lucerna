//! Read-only git integration: a hardened `git` runner and parsers for its output.
//! This crate never depends on gpui.

mod command;
mod status;

#[doc(hidden)]
pub use command::run_with_timeout;
pub use command::{GitBinary, GitError};
pub use status::{FileStatus, parse_status_v2};
