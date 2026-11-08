//! Lucerna workbench UI: the GPUI shell (Zed-style chrome), command
//! registry and key dispatch.

mod app_state;
mod commands;
mod editor_groups;
mod explorer;
pub mod keys;
mod layout;
pub mod ui;
mod workbench;

pub use app_state::AppState;
pub use commands::{CommandHandler, CommandRegistry};
pub use editor_groups::{
    EditorGroups, EditorGroupsView, Group, GroupState, GroupsState, OpenMode, Tab, TabBodyFactory,
    TabId, placeholder_body,
};
pub use explorer::{CHANGE_DECORATION, ExplorerModel, ExplorerView, Row};
pub use layout::StatusBarSide;
pub use layout::title_bar::window_options;
pub use workbench::{SidebarView, Workbench};
