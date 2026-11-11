//! The Search tab: project search results as excerpts in an editor tab.

pub(crate) mod model;
mod view;

pub(crate) use view::register_commands;
pub use view::{SEARCH_DEBOUNCE, SearchView};
