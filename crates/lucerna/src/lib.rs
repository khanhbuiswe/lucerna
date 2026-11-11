//! Lucerna (`luc`): what the command line opens, logging and the `luc` subcommands.

pub mod ext_cli;
pub mod logging;
pub mod lsp_cli;
mod target;

pub use target::{Target, prompts_for_folder, resolve_target, resolve_target_with_home};
