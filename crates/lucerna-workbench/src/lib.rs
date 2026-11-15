//! Lucerna workbench UI: the GPUI shell (Zed-style chrome), command
//! registry and key dispatch.

mod about;
pub mod app;
pub mod app_menu;
mod app_state;
// Used by the app router in a later onboarding task.
#[allow(dead_code)]
mod app_state_file;
pub mod code_view;
mod commands;
mod config_files;
mod conflict_bar;
mod context_menu;
mod diagnostics_status;
mod diff_view;
mod documents;
mod editing;
mod editor_groups;
mod excerpt_list;
mod explorer;
pub mod extensions;
mod find_widget;
mod font_picker;
pub mod fonts;
pub mod git;
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
mod markdown_preview;
mod nav_history;
mod onboarding;
mod open_commands;
mod open_files;
pub mod open_requests;
pub mod process;
mod quick_open;
pub mod recent;
mod recent_picker;
mod references_view;
mod save;
mod scrollbar;
mod search_view;
mod settings_edit;
mod startup;
pub mod status_message;
mod symbols;
pub mod system_shell;
pub mod tasks;
mod terminal;
mod text_input;
mod theme_picker;
mod trust;
pub mod ui;
mod vim;
mod vim_indicator;
mod welcome;
mod workbench;
mod workspace_state;

pub use app_state::AppState;
pub use commands::{CommandHandler, CommandRegistry};
pub use config_files::{
    CONFIG_DEBOUNCE, ConfigError, ConfigFile, ConfigPaths, ConfigState, ConfigWatcher,
    reload_config, set_system_appearance, watch_config,
};
pub use diagnostics_status::DiagnosticsStatusItem;
pub use diff_view::branch_picker::{BranchPicker, BranchPickerEvent};
pub use diff_view::{DiffKind, DiffView};
pub use documents::{DiskNotice, DocState, DocumentEvent, DocumentStatus, Documents};
pub use editor_groups::{
    BRANCH_CHANGES_TAB_STATE, EXTENSIONS_TAB_STATE, EditorGroups, EditorGroupsView, FileView,
    GIT_CHANGES_TAB_STATE, Group, GroupState, GroupsState, MARKDOWN_PREVIEW_DYNAMIC_STATE_PREFIX,
    MARKDOWN_PREVIEW_STATE_PREFIX, OpenMode, SEARCH_TAB_STATE, Tab, TabBodyFactory, TabId, TabItem,
    placeholder_body,
};
pub use excerpt_list::model::{
    FileMeta, FileResult as SearchFileResult, LineRef as SearchLineRef, Results as SearchResults,
    Row as SearchRow,
};
pub use excerpt_list::{ExcerptList, ExcerptListEvent, TextSource};
pub use explorer::{
    CHANGE_DECORATION, EditKind, EditState, ExplorerModel, ExplorerView, Row, SelectedKind,
    Selection,
};
pub use find_widget::{BACKGROUND_SEARCH_BYTES, FindWidget, FindWidgetEvent, find_in_snapshot};
pub use font_picker::{FontPicker, FontPickerEvent, font_names};
pub use git::{GitSnapshot, GitState, GitStore, GitStoreEvent};
pub use language_servers::{
    HoverInfo, LanguageServers, LanguageServersEvent, ServerStatus, SlotState,
};
pub use layout::StatusBarSide;
pub use layout::panel::{PanelPosition, PanelState};
pub use layout::title_bar::window_options;
pub use locations_picker::{
    EXCERPT_CHARS as LOCATION_EXCERPT_CHARS, LocationsPicker, LocationsPickerEvent,
};
pub use lsp_health::LspHealthView;
pub use lsp_log::LspLogView;
pub use lsp_status::{LspStatusItem, ServerRow, StatusDot};
pub use markdown_preview::{
    ImageContext, ImageSrc, LinkAction, MAX_IMAGE_BYTES, MAX_PREVIEW_BYTES, MarkdownPreview,
    MarkdownPreviewEvent, PREVIEW_EDITOR_ID, REFRESH_PREVIEW, REPARSE_DELAY, SHOW_PREVIEW,
    SHOW_PREVIEW_TO_SIDE, SHOW_SOURCE, TOGGLE_PREVIEW, editor_for, heading_slugs, item_to_line,
    line_to_item, resolve_image, resolve_link,
};
pub use nav_history::{NavEntry, NavHistory};
pub use onboarding::{Host, ONBOARDING_COMMANDS, OnboardingEvent, OnboardingPage};
pub use open_commands::{OPEN_IN_BROWSER, WorkbenchEvent, browser_url};
pub use open_files::OpenFileWatcher;
pub use quick_open::{
    QuickOpen, QuickOpenEvent, QuickOpenQuery, RECENT_FILES_LIMIT, RecentFiles, parse_query,
};
pub use recent_picker::{RecentPicker, RecentPickerEvent};
pub use references_view::{ReferencesView, find_references_in_tab};
pub use save::{SaveOptions, SaveReason};
pub use search_view::{SEARCH_DEBOUNCE, SearchView};
pub use startup::{Startup, last_workspace, restores_last_workspace, startup_choice};
pub use symbols::{
    BREADCRUMBS_BATCH, BreadcrumbMode, EMPTY_SYMBOLS_TTL, FilePathMode, MAX_WORKSPACE_SYMBOLS,
    SYMBOLS_DEBOUNCE, Segment, SymbolPathMode, SymbolSource, SymbolState,
    WORKSPACE_SYMBOLS_DEBOUNCE, WorkspaceSymbolItem, breadcrumb_segments,
};
pub use terminal::{
    DEFAULT_COMMANDS_TO_SKIP_SHELL, PanelEvent, TabState, TerminalFont, TerminalPanel,
    TerminalSpawner, TerminalTab, TerminalView, TerminalViewEvent, set_default_terminal_spawner,
    skips_shell, tab_title as terminal_tab_title, terminal_font,
};
pub use text_input::{TEXT_INPUT_KEY_CONTEXT, TextInput};
pub use theme_picker::{ThemePicker, ThemePickerEvent};
pub use trust::{TrustModal, TrustModalEvent, TrustStore};
pub use vim::VimLayer;
pub use welcome::{WELCOME_COMMANDS, WelcomeEvent, WelcomeView};
pub use workbench::{SidebarView, Workbench};
pub use workspace_state::{
    SearchState, WorkspaceState, WorkspaceStateStore, default_data_dir, load_state, save_state,
    state_path,
};
