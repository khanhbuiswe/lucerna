//! Lucerna workbench UI: the GPUI shell (Zed-style chrome), command
//! registry and key dispatch.

pub mod app;
mod app_state;
pub mod code_view;
mod commands;
mod config_files;
mod diagnostics_status;
mod documents;
mod editing;
mod editor_groups;
mod excerpt_list;
mod explorer;
pub mod extensions;
mod find_widget;
pub mod fonts;
pub mod git_branch;
pub mod goto;
pub mod hover;
pub mod icons;
pub mod keys;
mod language_servers;
mod layout;
mod locations_picker;
mod lsp_health;
mod lsp_log;
mod lsp_status;
pub mod markdown;
mod nav_history;
mod open_files;
pub mod open_requests;
mod quick_open;
pub mod recent;
mod references_view;
mod save;
mod search_view;
mod startup;
pub mod status_message;
mod symbols;
pub mod tasks;
mod text_input;
mod theme_picker;
mod trust;
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
pub use diagnostics_status::DiagnosticsStatusItem;
pub use documents::{DocState, DocumentEvent, DocumentStatus, Documents};
pub use editor_groups::{
    EXTENSIONS_TAB_STATE, EditorGroups, EditorGroupsView, Group, GroupState, GroupsState, OpenMode,
    SEARCH_TAB_STATE, Tab, TabBodyFactory, TabId, TabItem, placeholder_body,
};
pub use excerpt_list::model::{
    FileResult as SearchFileResult, LineRef as SearchLineRef, Results as SearchResults,
    Row as SearchRow,
};
pub use explorer::{CHANGE_DECORATION, ExplorerModel, ExplorerView, Row};
pub use find_widget::{BACKGROUND_SEARCH_BYTES, FindWidget, FindWidgetEvent, find_in_snapshot};
pub use language_servers::{
    HoverInfo, LanguageServers, LanguageServersEvent, ServerStatus, SlotState,
};
pub use layout::StatusBarSide;
pub use layout::title_bar::window_options;
pub use locations_picker::{
    EXCERPT_CHARS as LOCATION_EXCERPT_CHARS, LocationsPicker, LocationsPickerEvent,
};
pub use lsp_health::LspHealthView;
pub use lsp_log::LspLogView;
pub use lsp_status::{LspStatusItem, ServerRow, StatusDot};
pub use nav_history::{NavEntry, NavHistory};
pub use open_files::OpenFileWatcher;
pub use quick_open::{
    QuickOpen, QuickOpenEvent, QuickOpenQuery, RECENT_FILES_LIMIT, RecentFiles, parse_query,
};
pub use references_view::{ReferencesView, find_references_in_tab};
pub use save::{SaveOptions, SaveReason};
pub use search_view::{SEARCH_DEBOUNCE, SearchView};
pub use startup::{Startup, startup_choice};
pub use symbols::{
    BREADCRUMBS_BATCH, BreadcrumbMode, EMPTY_SYMBOLS_TTL, FilePathMode, MAX_WORKSPACE_SYMBOLS,
    SYMBOLS_DEBOUNCE, Segment, SymbolPathMode, SymbolSource, SymbolState,
    WORKSPACE_SYMBOLS_DEBOUNCE, WorkspaceSymbolItem, breadcrumb_segments,
};
pub use text_input::{TEXT_INPUT_KEY_CONTEXT, TextInput};
pub use theme_picker::{ThemePicker, ThemePickerEvent};
pub use trust::{TrustModal, TrustModalEvent, TrustStore};
pub use vim::VimLayer;
pub use workbench::{SidebarView, Workbench};
pub use workspace_state::{
    SearchState, WorkspaceState, WorkspaceStateStore, default_data_dir, load_state, save_state,
    state_path,
};
