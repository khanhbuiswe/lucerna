//! The Explorer: the project's file tree in the sidebar.

mod model;
mod view;

pub use model::{CHANGE_DECORATION, ExplorerModel, Row, SelectedKind, Selection};
pub use view::ExplorerView;
