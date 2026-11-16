//! Hot exit: unsaved edits survive quitting the app.

pub mod store;
mod tracker;

pub use tracker::{BACKUP_DELAY, HotExit};
