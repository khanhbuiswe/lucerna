//! Lucerna workbench UI: the GPUI shell (Zed-style chrome), command
//! registry and key dispatch.

pub mod app;
mod app_state;
pub mod code_view;
mod commands;
mod config_files;
mod documents;
mod editor_groups;
mod explorer;
pub mod extensions;
mod find_widget;
pub mod fonts;
pub mod git_branch;
pub mod icons;
pub mod keys;
mod layout;
mod open_files;
pub mod open_requests;
mod quick_open;
mod search_panel;
mod search_view;
pub mod tasks;
mod text_input;
mod theme_picker;
pub mod ui;
mod vim;
mod vim_indicator;
mod workbench;
mod workspace_state;

pub use app_state::AppState;
pub use commands::{CommandHandler, CommandRegistry};
pub use config_files::{
    CONFIG_DEBOUNCE, ConfigError, ConfigFile, ConfigPaths, ConfigState, ConfigWatcher,
    reload_config, watch_config,
};
pub use documents::{DocState, DocumentStatus, Documents};
pub use editor_groups::{
    EXTENSIONS_TAB_STATE, EditorGroups, EditorGroupsView, Group, GroupState, GroupsState, OpenMode,
    Tab, TabBodyFactory, TabId, TabItem, placeholder_body,
};
pub use explorer::{CHANGE_DECORATION, ExplorerModel, ExplorerView, Row};
pub use find_widget::{BACKGROUND_SEARCH_BYTES, FindWidget, FindWidgetEvent, find_in_snapshot};
pub use layout::StatusBarSide;
pub use layout::title_bar::window_options;
pub use open_files::OpenFileWatcher;
pub use quick_open::{
    QuickOpen, QuickOpenEvent, QuickOpenQuery, RECENT_FILES_LIMIT, RecentFiles, parse_query,
};
pub use search_panel::{SEARCH_DEBOUNCE, SearchPanel};
pub use text_input::{TEXT_INPUT_KEY_CONTEXT, TextInput};
pub use theme_picker::{ThemePicker, ThemePickerEvent};
pub use vim::VimLayer;
pub use workbench::{SidebarView, Workbench};
pub use workspace_state::{
    WorkspaceState, WorkspaceStateStore, default_data_dir, load_state, save_state, state_path,
};
