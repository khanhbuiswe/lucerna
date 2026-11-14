//! The Explorer: the project's file tree in the sidebar.

mod commands;
mod menu;
mod model;
mod view;

pub use model::{
    CHANGE_DECORATION, EditKind, EditState, ExplorerModel, Row, SelectedKind, Selection,
};
pub use view::ExplorerView;

pub(crate) use commands::{editing, register_commands, show_explorer};
