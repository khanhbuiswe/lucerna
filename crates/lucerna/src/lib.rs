//! Lucerna (`luc`): what the command line opens, and logging.

pub mod ext_cli;
pub mod logging;
mod target;

pub use target::{Target, resolve_target, resolve_target_with_home};
