//! The root view: shell layout, view slots and VS Code–style key dispatch.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::Either;
use futures::{FutureExt as _, StreamExt};
use gpui::{
    AnyView, App, Bounds, Context, DragMoveEvent, Entity, EntityId, FocusHandle, Focusable,
    IntoElement, KeyDownEvent, ModifiersChangedEvent, ParentElement, Pixels, Render, Styled,
    Subscription, Task, WeakFocusHandle, Window, WindowHandle, div, prelude::*, px,
};
use lucerna_project::{Change, ChangeKind, ScanOptions, Worktree, WorktreeEvent, WorktreeSnapshot};
use lucerna_settings::{Context as WhenContext, ContextValue, Keystroke, Resolution, SettingsView};
use lucerna_text::Point;
use serde_json::Value;

use crate::code_view::{self, CodeView, CodeViewEvent, LargeFileStatus};
use crate::config_files::{self, ConfigErrorItem, ConfigState};
use crate::diagnostics_status::{self, DiagnosticsStatusItem};
use crate::diff_view::{BranchBase, DiffKind, DiffView};
use crate::documents::{DocumentEvent, DocumentStatusItem, Documents, GoTo};
use crate::editor_groups::{
    self, EditorGroups, EditorGroupsView, FileView, GroupId, OpenMode, TabId, TabItem,
};
use crate::explorer::{ClipOp, ExplorerClipboard, ExplorerView, SelectedKind};
use crate::extensions::{ExtensionService, ExtensionsPage, Toast, installable_ids};
use crate::git::branch_picker::ShownBranchPicker;
use crate::git::{GitStore, GitStoreEvent};
use crate::hot_exit::HotExit;
use crate::hot_exit::store::WorkspaceKey;
use crate::hover::SHOW_HOVER;
use crate::language_servers::LanguageServers;
use crate::layout::panel::{self, EDITOR_MIN_SIZE, PanelPosition, PanelSash, PanelState};
use crate::layout::sidebar::SidebarSash;
use crate::layout::{
    SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, StatusBarSide, color,
    panel_toggles, sidebar, status_bar, title_bar,
};
use crate::locations_picker::ShownLocationsPicker;
use crate::lsp_health::LspHealthView;
use crate::lsp_log::LspLogs;
use crate::lsp_status::{self, LspStatusItem};
use crate::markdown_preview::{self, MarkdownPreview, ScrollSync};
use crate::nav_history::{NavEntry, NavHistory};
use crate::onboarding::{self, TabPage};
use crate::open_commands::{self, ShownRecentPicker};
use crate::open_files::{OpenFileWatcher, matching_files};
use crate::quick_open::{self, QuickOpen, RecentFiles};
use crate::references_view::{self, ReferencesView};
use crate::save::{self, SaveChoice};
use crate::search_view::{self, SearchView};
use crate::symbols::SymbolSource;
use crate::symbols::breadcrumbs::Breadcrumbs;
use crate::tasks::try_spawn_guarded;
use crate::terminal::commands::{allow_chords, key_bytes, option_is_meta, skips_shell};
use crate::terminal::settings::{ConfirmOnExit, confirm_on_exit};
use crate::terminal::view::{default_spawner, spawn_view};
use crate::terminal::{LinkTarget, PanelEvent, TerminalPanel, TerminalSpawner, TerminalView};
use crate::text_input::TEXT_INPUT_KEY_CONTEXT;
use crate::theme_picker::{self, ShownThemePicker};
use crate::trust::{self, Trust};
use crate::ui::{UI_FONT_FAMILY, UI_FONT_SIZE, popover_overlay};
use crate::vim::{self, VimLayer};
use crate::vim_indicator::ModeIndicator;
use crate::watch_status::WatchStatusItem;
use crate::workspace_state::{WorkspaceState, WorkspaceStateStore, absolute_path};
use crate::{AppState, CommandRegistry, WindowSettings, find_widget, keys};

/// What the sidebar shows. Only the Explorer: Search and Extensions are
/// editor tabs. A saved `"search"` or `"extensions"` restores the Explorer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarView {
    Explorer,
}

impl SidebarView {
    /// The name the workspace state file uses.
    fn state_name(self) -> &'static str {
        match self {
            SidebarView::Explorer => "explorer",
        }
    }
}

/// How long after a layout change the workspace state is written.
const SAVE_DELAY: Duration = Duration::from_secs(2);

/// Saving and restoring the layout; present only when a state store is
/// configured.
struct Persistence {
    store: WorkspaceStateStore,
    /// What the file holds (or would after a restore), so unchanged state
    /// is not written again.
    last_saved: Option<WorkspaceState>,
    /// A save is scheduled.
    save_pending: bool,
    /// Scroll rows of open tabs that have no live view to ask, from the
    /// last restore or save.
    scroll_rows: HashMap<PathBuf, u32>,
    _subscriptions: Vec<Subscription>,
}

/// How long a path from [`Workbench::expect_own_changes`] is remembered.
const OWN_CHANGE_TTL: Duration = Duration::from_secs(5);

pub struct Workbench {
    root: PathBuf,
    /// The one background scan of `root`; every view reads the tree from it.
    worktree: Worktree,
    /// When the scan started, and whether its first progress was logged.
    scan_started: std::time::Instant,
    scan_progress_logged: bool,
    scan_complete_logged: bool,
    /// Root-relative paths Lucerna itself just created, renamed or deleted,
    /// and when; the Explorer does not mark them as changed by another program.
    own_changes: HashMap<Arc<Path>, Instant>,
    /// Absolute paths a Delete is asking about or deleting, or a Paste is
    /// moving; another Delete or move of them (or of a path inside or
    /// around them) is ignored meanwhile.
    pub(crate) deletes_in_flight: Rc<std::cell::RefCell<HashSet<PathBuf>>>,
    /// How many Explorer file operations (Paste, Duplicate, Delete) are
    /// writing to disk; a quit or close asks first while any is.
    file_operations: Rc<Cell<usize>>,
    /// What the Explorer's Cut or Copy last took.
    explorer_clipboard: Option<ExplorerClipboard>,
    /// The options of the scan, from `files.exclude` and `files.scanDepth`.
    scan_options: ScanOptions,
    /// This window, for its status bar messages.
    window_id: gpui::WindowId,
    /// The message that folders past `files.scanDepth` load when expanded
    /// was shown; it shows once per window.
    unloaded_notice_shown: bool,
    /// Re-themes and rescans when [`AppState`] changes.
    _settings: Subscription,
    /// [`ConfigState::keymap_generation`] as last seen.
    keymap_generation: u64,
    /// [`WindowSettings::languages_generation`] as last seen.
    languages_generation: u64,
    explorer: Entity<ExplorerView>,
    /// The body of the Search tab when it is open. Kept while the tab is
    /// closed, so it reopens with its query and results.
    search_view: Entity<SearchView>,
    /// The body of the References tab, kept while the tab is closed.
    references_view: Entity<ReferencesView>,
    /// The References tab is the results tab used last: its list had the
    /// focus at the last key press, or it opened a line since the Search
    /// tab did. `F4` walks its references then (`referencesViewFocus`).
    references_used_last: bool,
    /// The Extensions page, the body of the Extensions tab when it is
    /// open. Kept while the tab is closed, so it reopens as it was.
    extensions_page: Entity<ExtensionsPage>,
    /// Every open file; tab bodies are its code views.
    documents: Entity<Documents>,
    /// The language servers of the open documents.
    language_servers: Entity<LanguageServers>,
    /// The symbols of the open documents.
    pub(crate) symbols: SymbolSource,
    /// The rows under the tab bars following the cursor.
    pub(crate) breadcrumbs: Breadcrumbs,
    /// The ⚡ status bar item.
    pub(crate) lsp_status: Entity<LspStatusItem>,
    /// The body of the Health tab, kept (with its last check) while the
    /// tab is closed.
    pub(crate) lsp_health: Entity<LspHealthView>,
    /// The body of the Get Started tab, made when the tab first opens and
    /// kept while it is closed.
    pub(crate) onboarding: TabPage,
    /// The diagnostics counts and message in the status bar.
    pub(crate) diagnostics_status: Entity<DiagnosticsStatusItem>,
    /// The status bar's warning when a file watcher could not start.
    watch_status: Entity<WatchStatusItem>,
    /// The F8 lookup in flight.
    pub(crate) marker_task: Option<Task<()>>,
    /// Where an F8 jump into a tab not drawn yet shows its hover.
    pub(crate) marker_hover: Option<(Arc<Path>, Point)>,
    /// The go to definition / references lookup in flight.
    pub(crate) goto_task: Option<Task<()>>,
    /// The bodies of the server log tabs.
    pub(crate) lsp_logs: LspLogs,
    /// Forwards worktree events to [`Workbench::on_worktree_event`].
    _worktree_events: Task<()>,
    /// `false` in single-file mode: the worktree neither scans nor
    /// watches, so every open file is followed by the open-file watcher.
    scanning: bool,
    /// The file of single-file mode, absolute; `None` for a folder.
    single_file: Option<PathBuf>,
    /// Follows open files the worktree does not; `None` on the test
    /// scheduler or when it could not start.
    open_file_watcher: Option<OpenFileWatcher>,
    /// What it follows: open files outside the root or not in the tree.
    watched_open_files: Vec<PathBuf>,
    /// Forwards its batches to [`Workbench::on_open_file_changes`].
    _open_file_events: Task<()>,
    /// The repository's `HEAD` file, also followed by the open-file
    /// watcher so a checkout updates the title bar.
    git_head: Option<PathBuf>,
    /// What `HEAD` names: the branch, or a short SHA when detached.
    pub(crate) git_branch: Option<String>,
    /// The repository: statuses and gutter hunks. Runs `git` only once
    /// the folder is trusted.
    pub(crate) git: Entity<GitStore>,
    /// The [`ConfigState`] batch last forwarded to the documents.
    config_batch: u64,
    /// This window's settings: the folder's `.lucerna/settings.json` over
    /// the user's.
    settings: Entity<WindowSettings>,
    /// [`WindowSettings::generation`] last forwarded to the documents.
    settings_generation: u64,
    _subscriptions: Vec<Subscription>,
    commands: Rc<CommandRegistry>,
    focus: FocusHandle,
    sidebar_focus: FocusHandle,
    editor_focus: FocusHandle,
    /// The view (code view or Markdown preview) last made active, to
    /// focus a newly active one when focus was on the editor area or on
    /// that view.
    active_view: Option<(EntityId, WeakFocusHandle)>,
    sidebar_visible: bool,
    sidebar_width: Pixels,
    /// Where the bottom / right panel is, its sizes and whether it shows.
    panel: PanelState,
    /// The panel's terminals; kept while the panel is hidden.
    terminal_panel: Entity<TerminalPanel>,
    /// Starts the panel's terminals.
    terminal_spawner: TerminalSpawner,
    /// The panel was restored visible: its fresh terminal starts once the
    /// folder's trust is known, so it gets the Workspace shell settings of
    /// a trusted folder.
    restore_terminal: bool,
    /// The focused group and its active tab as last seen: when they
    /// change under a maximized panel, the panel goes back to its size so
    /// the editor shows (as VS Code does).
    shown_editor: (usize, Option<TabId>),
    explorer_view: Option<AnyView>,
    editor_groups: Entity<EditorGroupsView>,
    /// Scrolls each shown Markdown preview with the shown editors of its
    /// file in other groups.
    scroll_sync: ScrollSync,
    editor_area: Option<AnyView>,
    status_left: Vec<AnyView>,
    status_right: Vec<AnyView>,
    /// Keystrokes of a chord in progress, e.g. `[cmd+k]`.
    pending_chord: Vec<Keystroke>,
    /// Vim navigation in code views; sees keys before the keymap.
    vim: Entity<VimLayer>,
    /// The left button went down on the title bar and the window has not
    /// started moving yet. Only acted on while the button is still held.
    pub(crate) title_bar_pressed: bool,
    /// The title bar's branch button as last laid out: the Checkout-mode
    /// picker opened from it shows under it.
    pub(crate) branch_button_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// A press on the branch button closed the picker it opens.
    pub(crate) branch_button_press_closed: bool,
    /// A `git switch` is running; the title bar says "Switching…".
    pub(crate) switching_branch: bool,
    /// Switches started, for tests.
    pub(crate) switches_started: usize,
    /// The shown Quick Open and the subscription to its events.
    pub(crate) quick_open: Option<(Entity<QuickOpen>, Subscription)>,
    /// Where focus goes back to when Quick Open closes.
    pub(crate) focus_before_quick_open: Option<FocusHandle>,
    /// Every opened file, newest first; Quick Open ranks them first.
    pub(crate) recent_files: RecentFiles,
    /// Go Back / Go Forward, per editor group index.
    /// Go Back / Go Forward history per editor group, by stable group id.
    nav_history: HashMap<GroupId, NavHistory>,
    /// The shown theme picker.
    pub(crate) theme_picker: Option<ShownThemePicker>,
    /// The shown Open Recent picker.
    pub(crate) recent_picker: Option<ShownRecentPicker>,
    /// Loads the recent list for the Open Recent picker about to show.
    pub(crate) recent_picker_loading: Option<Task<()>>,
    /// The shown locations picker (several definitions, references…).
    pub(crate) locations_picker: Option<ShownLocationsPicker>,
    /// The shown branch picker of the Changes since tab.
    pub(crate) branch_picker: Option<ShownBranchPicker>,
    /// Lists the branches for the branch picker about to show.
    pub(crate) branch_picker_loading: Option<Task<()>>,
    /// The ref the Changes since tab compares with, as picked (`None`: the
    /// default branch); saved with the workspace.
    pub(crate) branch_base: Entity<BranchBase>,
    /// Folder trust: the trust store's answer and the trust modal.
    pub(crate) trust: Trust,
    /// `files.autoSave`.
    pub(crate) auto_save: crate::save::AutoSave,
    /// The notification at the bottom right.
    pub(crate) toast: Option<Toast>,
    /// Toasts shown so far; tells a toast's timer whether it is current.
    pub(crate) toast_count: u64,
    /// How many of [`ExtensionService::fresh_installs`] were looked at for
    /// a theme to offer.
    pub(crate) fresh_installs_seen: usize,
    /// Fresh installs whose index entry had not arrived when looked at.
    pub(crate) fresh_installs_waiting: Vec<String>,
    persistence: Option<Persistence>,
    /// Backs up unsaved documents; `None` without a data dir (hot exit
    /// off).
    hot_exit: Option<Entity<HotExit>>,
    /// A quit or window close is waiting on the save prompt.
    confirming_quit: bool,
}

impl Workbench {
    /// Focuses the editor area, so key dispatch and `editorFocus`
    /// bindings work from the start.
    ///
    /// # Panics
    /// If the [`AppState`] global is not set: rendering and key dispatch
    /// read the theme and keymap from it.
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(root, None, None, window, cx)
    }

    /// Like [`Workbench::new`], and keeps the layout of `root` in
    /// `data_dir` (the app's is [`crate::default_data_dir`]): restores it
    /// now, saves it after layout changes and when the window closes.
    pub fn with_data_dir(
        root: PathBuf,
        data_dir: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = WorkspaceStateStore::new(data_dir, root.clone());
        Self::build(root, None, Some(store), window, cx)
    }

    /// Single-file mode, for a file opened alone as Zed opens one: `root`
    /// (its folder) is never scanned or watched, the tree and Quick Open
    /// list only `file`, and open files are followed one by one. Keeps the
    /// layout in `data_dir`, keyed by `file`, when given. Does not open
    /// `file`.
    pub fn single_file(
        root: PathBuf,
        file: &Path,
        data_dir: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let rel: Arc<Path> = match file.strip_prefix(&root) {
            Ok(rel) => Arc::from(rel),
            Err(_) => Arc::from(Path::new(file.file_name().unwrap_or(file.as_os_str()))),
        };
        // Keyed by the file, not its folder: lone files beside each other,
        // and the folder opened as a project, keep their own tabs.
        let store = data_dir.map(|dir| WorkspaceStateStore::new(dir, file.to_path_buf()));
        Self::build(root, Some(rel), store, window, cx)
    }

    /// `single_file`: the root-relative file of single-file mode; the
    /// folder is scanned when it is `None`.
    fn build(
        root: PathBuf,
        single_file: Option<Arc<Path>>,
        store: Option<WorkspaceStateStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        assert!(
            cx.has_global::<AppState>(),
            "set the AppState global before creating the Workbench"
        );
        crate::config_files::follow_system_appearance(window, cx);
        // Before every other entity: they read settings through it.
        let settings = {
            let folder = single_file.is_none().then_some(root.as_path());
            cx.new(|cx| WindowSettings::new(folder, cx))
        };
        let focus = cx.focus_handle();
        let editor_focus = cx.focus_handle();
        window.focus(&editor_focus, cx);
        let commands = default_registry();
        let documents = cx.new(|_| {
            let mut documents = Documents::new(root.clone(), settings.clone());
            documents.set_window(window.window_handle().window_id());
            documents
        });
        let language_servers = {
            let root = root.clone();
            // Servers' `{data_dir}`: the workspace's data dir, else the
            // app's (a temporary folder when there is none).
            let data_dir = store
                .as_ref()
                .map(|store| store.data_dir().to_path_buf())
                .or_else(crate::default_data_dir)
                .unwrap_or_else(|| std::env::temp_dir().join("Lucerna"));
            let settings = settings.clone();
            cx.new(|cx| LanguageServers::new(root, &documents, settings, data_dir, cx))
        };
        let symbols = SymbolSource::new(root.clone(), &documents, &language_servers, cx);
        // Servers end with the window (without waiting for them) or the
        // app (waiting at most 5 s across all windows, then killed; see the
        // `on_app_quit` below).
        cx.on_release(|wb, cx| {
            wb.release_theme_picker(cx);
            wb.language_servers
                .update(cx, |servers, _| servers.shutdown_all_in_background())
        })
        .detach();
        let extensions_page = {
            let workbench = cx.weak_entity();
            cx.new(|cx| ExtensionsPage::new(workbench, cx))
        };
        // Read once: the scan, search and git discovery share its `$HOME`.
        let scan_options = scan_options(settings.read(cx).view(cx));
        let search_view = {
            let (root, workbench) = (root.clone(), cx.weak_entity());
            let searchable = single_file.is_none();
            let home = scan_options.home.clone();
            let (settings, documents) = (settings.clone(), documents.clone());
            cx.new(|cx| {
                let mut view =
                    SearchView::new(root, workbench, documents, searchable, settings, window, cx);
                view.set_home(home);
                view
            })
        };
        let references_view = {
            let (root, workbench) = (root.clone(), cx.weak_entity());
            let servers = language_servers.clone();
            let docs = documents.clone();
            cx.new(|cx| ReferencesView::new(root, workbench, servers, docs, window, cx))
        };
        let lsp_logs = LspLogs::default();
        let lsp_health = {
            // Probes run in the folder once it is trusted, as servers
            // start there; a loose file's folder is not a project.
            let probe_dir = single_file.is_none().then(|| root.clone());
            let servers = language_servers.clone();
            cx.new(|cx| LspHealthView::new(servers, probe_dir, cx))
        };
        let lsp_status = {
            let (workbench, servers) = (cx.weak_entity(), language_servers.clone());
            cx.new(|cx| LspStatusItem::new(workbench, servers, cx))
        };
        // Before the editor groups: the diff tabs read them.
        let git = {
            let settings = settings.clone();
            let id = window.window_handle().window_id();
            let home = scan_options.home.clone();
            cx.new(|cx| {
                let mut store = GitStore::new(root.clone(), settings, cx);
                store.set_window(id);
                store.set_home(home);
                store
            })
        };
        let branch_base = cx.new(|_| BranchBase(None));
        let onboarding = TabPage::default();
        let workbench = cx.weak_entity();
        let breadcrumbs_target = cx.weak_entity();
        let command_target = cx.weak_entity();
        let editor_groups = cx.new(|cx| {
            let mut groups = EditorGroupsView::new(settings.clone());
            let docs = documents.clone();
            let page = extensions_page.clone();
            let search = search_view.clone();
            let references = references_view.clone();
            let health = lsp_health.clone();
            let onboarding = onboarding.clone();
            let git = git.clone();
            let branch_base = branch_base.clone();
            let (logs, servers) = (lsp_logs.clone(), language_servers.clone());
            let preview_root: Arc<Path> = Arc::from(root.as_path());
            let preview_settings = settings.clone();
            let onboarding_settings = settings.clone();
            groups.set_tab_body_factory(
                Rc::new(move |tab, window, cx| {
                    let path = match &tab.item {
                        TabItem::File(path) => path.clone(),
                        TabItem::MarkdownPreview(path) | TabItem::MarkdownPreviewDynamic(path) => {
                            let dynamic = matches!(tab.item, TabItem::MarkdownPreviewDynamic(_));
                            let (path, docs) = (path.clone(), docs.clone());
                            let (workbench, root) = (workbench.clone(), preview_root.clone());
                            let settings = preview_settings.clone();
                            return cx
                                .new(|cx| {
                                    MarkdownPreview::new(
                                        path, dynamic, root, docs, workbench, settings, cx,
                                    )
                                })
                                .into();
                        }
                        TabItem::Extensions => {
                            page.update(cx, |page, cx| page.shown(cx));
                            return page.clone().into();
                        }
                        TabItem::Search => return search.clone().into(),
                        TabItem::References => return references.clone().into(),
                        TabItem::LspLog(server) => return logs.view(server, &servers, cx).into(),
                        TabItem::LspHealth => {
                            health.update(cx, |health, cx| health.shown(cx));
                            return health.clone().into();
                        }
                        TabItem::Onboarding => {
                            return onboarding::tab_page(
                                &onboarding,
                                &workbench,
                                &onboarding_settings,
                                window,
                                cx,
                            )
                            .into();
                        }
                        TabItem::GitChanges | TabItem::FileDiff(_) | TabItem::BranchChanges => {
                            let kind = match &tab.item {
                                TabItem::FileDiff(path) => DiffKind::File(path.clone()),
                                TabItem::BranchChanges => DiffKind::Branch {
                                    base_ref: branch_base.read(cx).0.clone(),
                                },
                                _ => DiffKind::Uncommitted,
                            };
                            let branch = matches!(kind, DiffKind::Branch { .. });
                            let (git, docs) = (git.clone(), docs.clone());
                            let (root, workbench) = (preview_root.to_path_buf(), workbench.clone());
                            let base = branch_base.clone();
                            return cx
                                .new(|cx| {
                                    let mut view =
                                        DiffView::new(kind, git, docs, root, workbench, cx);
                                    view.set_window(window.window_handle().window_id());
                                    if branch {
                                        view.follow_branch_base(base, cx);
                                    }
                                    view
                                })
                                .into();
                        }
                    };
                    let view = docs.update(cx, |docs, cx| docs.open(path, cx));
                    view.update(cx, |v, _| {
                        v.set_hover_sources(workbench.clone(), servers.downgrade())
                    });
                    // A click on a change bar opens the file's diff there.
                    window
                        .subscribe(&view, cx, {
                            let workbench = workbench.clone();
                            move |view, event: &CodeViewEvent, window, cx| {
                                if let CodeViewEvent::DiffHunkClicked { row } = *event {
                                    let path = view.read(cx).path().clone();
                                    workbench
                                        .update(cx, |wb, cx| {
                                            wb.open_file_diff(path, Some(row), window, cx)
                                        })
                                        .ok();
                                }
                            }
                        })
                        .detach();
                    // An F8 jump into this file shows its hover once the
                    // view exists.
                    cx.defer({
                        let (workbench, view) = (workbench.clone(), view.clone());
                        move |cx| {
                            workbench
                                .update(cx, |wb, cx| wb.show_marker_hover_in(&view, cx))
                                .ok();
                        }
                    });
                    // A scroll settles into a save; the breadcrumbs follow
                    // the cursor.
                    let workbench = workbench.clone();
                    cx.observe(&view, move |_, cx| {
                        workbench
                            .update(cx, |wb, cx| {
                                wb.layout_changed(cx);
                                wb.breadcrumbs_changed(cx);
                            })
                            .ok();
                    })
                    .detach();
                    view.into()
                }),
                cx,
            );
            groups.set_documents(documents.clone());
            groups.set_root(root.clone());
            groups.set_on_breadcrumbs_click(Rc::new(move |group, window, cx| {
                breadcrumbs_target
                    .update(cx, |wb, cx| wb.open_breadcrumbs(group, window, cx))
                    .ok();
            }));
            groups.set_workbench(command_target.clone());
            groups.set_on_command(Rc::new(move |id, window, cx| {
                command_target
                    .update(cx, |wb, cx| wb.run_command(id, None, window, cx))
                    .ok();
            }));
            groups
        });
        let large_file_status = {
            let groups = editor_groups.clone();
            cx.new(|_| LargeFileStatus::new(groups))
        };
        let document_status = {
            let (groups, docs) = (editor_groups.clone(), documents.clone());
            cx.new(|_| DocumentStatusItem::new(groups, docs))
        };
        let scan_started = std::time::Instant::now();
        let scanning = single_file.is_none();
        let single_file_path = single_file.as_ref().map(|rel| root.join(rel));
        let worktree = match single_file {
            Some(file) => Worktree::without_scan(root.clone(), vec![file]),
            None => Worktree::start(root.clone(), scan_options.clone()),
        };
        let trust = Trust::new(
            store.as_ref().map(|s| s.data_dir().to_path_buf()),
            window.window_handle().window_id(),
        );
        let hot_exit = store.as_ref().map(|store| {
            let key = match &single_file_path {
                Some(file) => WorkspaceKey::File(file.clone()),
                None => WorkspaceKey::Folder(root.clone()),
            };
            HotExit::new(documents.clone(), store.data_dir().to_path_buf(), key, cx)
        });
        let restored = store
            .as_ref()
            .and_then(WorkspaceStateStore::load)
            .map(|state| state.without_missing_files(&root));
        if let Some(state) = &restored {
            editor_groups.update(cx, |view, cx| {
                view.restore(&state.groups, &state.group_widths, cx)
            });
            let open: Vec<PathBuf> = editor_groups
                .read(cx)
                .model()
                .groups()
                .iter()
                .flat_map(|g| g.tabs.iter().filter_map(|t| t.path()))
                .map(|p| p.to_path_buf())
                .collect();
            documents.update(cx, |docs, _| {
                for (path, row) in state.scroll.iter().filter(|(p, _)| open.contains(p)) {
                    docs.scroll_when_opened(path, *row);
                }
            });
        }
        // After the saved tabs: a backup of a file without one opens a tab.
        if let Some(hot_exit) = hot_exit.as_ref().filter(|_| HotExit::enabled(cx)) {
            for backup in hot_exit.read(cx).read_backups() {
                let path = documents.update(cx, |docs, cx| docs.open_from_backup(backup, cx));
                editor_groups.update(cx, |view, cx| view.open_restored(path, window, cx));
            }
        }
        if let Some(state) = &restored {
            branch_base.update(cx, |base, _| base.0 = state.branch_diff_base.clone());
            search_view.update(cx, |view, cx| {
                view.restore(state.search.clone(), window, cx)
            });
            let has_tab = editor_groups.read(cx).model().search_tab().is_some();
            if has_tab {
                search_view.update(cx, |view, cx| view.run_search(cx));
            }
        }
        let panel_state = match &restored {
            Some(state) => state.panel.clone().sanitized(),
            None => PanelState {
                position: PanelPosition::from_setting(settings.read(cx).view(cx)),
                ..PanelState::default()
            },
        };
        let terminal_panel = cx.new(|cx| TerminalPanel::new(settings.clone(), window, cx));
        let sidebar_focus = cx.focus_handle();
        let explorer = {
            let (root, snapshot) = (root.clone(), worktree.snapshot());
            let lone = single_file_path.is_some();
            let (workbench, groups) = (cx.weak_entity(), editor_groups.clone());
            let focus = sidebar_focus.clone();
            let (docs, settings) = (documents.downgrade(), settings.clone());
            cx.new(|cx| {
                let mut explorer = ExplorerView::new(
                    root, lone, snapshot, workbench, &groups, settings, focus, cx,
                );
                explorer.set_documents(docs);
                explorer
            })
        };
        let worktree_events = pump_worktree_events(&worktree, cx);
        let (open_file_watcher, open_file_events) = pump_open_file_changes(cx);
        let mut subscriptions = vec![
            cx.observe(&editor_groups, |wb, _, cx| {
                wb.breadcrumbs_changed(cx);
                wb.refresh_watched_open_files(cx);
                wb.hide_hidden_hovers(cx);
                wb.release_closed_references(cx);
            }),
            cx.observe_in(&editor_groups, window, |wb, _, window, cx| {
                wb.refocus_if_hidden(window, cx)
            }),
            cx.observe_in(&editor_groups, window, |wb, _, window, cx| {
                wb.active_tabs_changed(window, cx)
            }),
            // Before the scroll links: they pair the dynamic preview by
            // the file it follows.
            cx.observe_in(&editor_groups, window, |wb, _, window, cx| {
                markdown_preview::follow_active(wb, window, cx)
            }),
            cx.observe_in(&editor_groups, window, |wb, groups, window, cx| {
                wb.scroll_sync
                    .refresh(&groups, &wb.root, &wb.settings, window, cx)
            }),
            cx.observe_global::<ConfigState>(Self::config_files_changed),
            cx.observe(&settings, |wb, _, cx| {
                wb.workspace_settings_changed(cx);
                wb.excludes_changed(cx);
                wb.languages_changed(cx);
                wb.editor_settings_changed(cx);
            }),
            // Saving `.lucerna/settings.json` here applies it at once.
            cx.subscribe(&documents, |wb, _, ev: &DocumentEvent, cx| {
                if let DocumentEvent::Saved { path, .. } = ev {
                    let saved = wb.absolute_path(path);
                    let settings = wb.settings.clone();
                    if settings.read(cx).settings_path() == Some(&*saved) {
                        settings.update(cx, WindowSettings::reload);
                    }
                }
            }),
            // VS Code: a preview tab whose file gets unsaved changes is
            // pinned, so opening another file never replaces it.
            cx.subscribe(&documents, |wb, _, ev: &DocumentEvent, cx| match ev {
                DocumentEvent::DirtyChanged { dirty: true, .. } => wb.pin_dirty_previews(cx),
                DocumentEvent::ChangedOnDisk { path }
                    if decorates_external_changes(wb.settings.read(cx).view(cx)) =>
                {
                    wb.explorer
                        .update(cx, |explorer, cx| explorer.mark_external_change(path, cx));
                }
                _ => {}
            }),
            crate::status_message::observe(cx, |_, cx| cx.notify()),
            // Before the language servers' shutdown below, which can block
            // for 5 s: Dock Quit and Log Out reach only these hooks, and the
            // OS may not wait. The first window's hook writes every window's
            // backups (hot exit spec §5.3).
            cx.on_app_quit(|_, cx| {
                HotExit::flush_all_on_quit(cx);
                async {}
            }),
            // The first window's hook shuts every window's servers down
            // against one 5 s deadline; the others find nothing left.
            cx.on_app_quit(|_, cx| {
                LanguageServers::shutdown_all_windows_on_quit(cx);
                async {}
            }),
            // The Search tab's title is its query; any input change is a
            // layout change too (the query is saved with the layout).
            cx.observe(&search_view, |wb, view, cx| {
                let title = view.read(cx).title(cx);
                wb.editor_groups
                    .update(cx, |groups, cx| groups.set_search_title(title, cx));
                wb.layout_changed(cx);
            }),
            cx.observe(&references_view, |wb, view, cx| {
                let title = view.read(cx).title();
                wb.editor_groups
                    .update(cx, |groups, cx| groups.set_references_title(title, cx));
            }),
            // The Changes since tab's ref is in its title and the layout.
            cx.observe(&branch_base, |wb, _, cx| {
                wb.update_branch_title(cx);
                wb.layout_changed(cx);
            }),
        ];
        if cx.has_global::<ExtensionService>() {
            subscriptions.push(cx.observe_global::<ExtensionService>(|wb, cx| {
                wb.offer_installed_themes(cx);
                wb.update_extension_ids(installable_ids, cx);
                // The status bar button's badge and spinner.
                cx.notify();
            }));
        }
        let fresh_installs_seen = cx
            .try_global::<ExtensionService>()
            .map_or(0, |s| s.fresh_installs().len());
        let config_error = {
            let workbench = cx.weak_entity();
            let settings = settings.clone();
            cx.new(|cx| ConfigErrorItem::new(workbench, settings, cx))
        };
        let watch_status = {
            let folder = root.file_name().map_or_else(
                || root.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            let git = git.clone();
            cx.new(|cx| WatchStatusItem::new(folder, git, cx))
        };
        let vim = {
            let settings = settings.clone();
            cx.new(|cx| VimLayer::new(settings, cx))
        };
        vim.update(cx, |vim, cx| vim.attach(&editor_groups, cx));
        let mode_indicator = {
            let (vim, groups) = (vim.clone(), editor_groups.clone());
            cx.new(|cx| ModeIndicator::new(vim, groups, cx))
        };
        let diagnostics_status = {
            let (workbench, servers) = (cx.weak_entity(), language_servers.clone());
            let groups = editor_groups.clone();
            cx.new(|cx| DiagnosticsStatusItem::new(workbench, servers, groups, cx))
        };
        let git_head = crate::git_branch::head_path(&root);
        let git_branch = crate::git_branch::read_branch(&root);
        subscriptions.push(cx.subscribe(&documents, Self::git_document_event));
        subscriptions.push(
            cx.subscribe(&git, |wb, git, ev: &GitStoreEvent, cx| match ev {
                GitStoreEvent::SnapshotChanged => {
                    let snapshot = git.read(cx).snapshot();
                    wb.explorer
                        .update(cx, |explorer, cx| explorer.set_git(snapshot, cx));
                    // The default branch may have changed.
                    wb.update_branch_title(cx);
                }
                GitStoreEvent::HunksChanged(path) => wb.git_hunks_changed(path, cx),
                GitStoreEvent::StateChanged => wb.git_state_changed(cx),
            }),
        );
        if let (Some(watcher), Some(head)) = (&open_file_watcher, &git_head) {
            watcher.set_files(vec![head.clone()]);
        }
        let mut workbench = Workbench {
            root,
            worktree,
            scan_started,
            scan_progress_logged: false,
            scan_complete_logged: false,
            own_changes: HashMap::new(),
            deletes_in_flight: Rc::default(),
            file_operations: Rc::default(),
            explorer_clipboard: None,
            scan_options,
            window_id: window.window_handle().window_id(),
            unloaded_notice_shown: false,
            _settings: cx.observe_global::<AppState>(Self::settings_changed),
            keymap_generation: keymap_generation(cx),
            languages_generation: settings.read(cx).languages_generation(),
            _worktree_events: worktree_events,
            scanning,
            single_file: single_file_path,
            open_file_watcher,
            watched_open_files: Vec::new(),
            _open_file_events: open_file_events,
            git_head,
            git_branch,
            git,
            config_batch: cx
                .try_global::<ConfigState>()
                .map_or(0, |state| state.last_changed().0),
            settings_generation: settings.read(cx).generation(),
            settings,
            _subscriptions: subscriptions,
            panel: panel_state,
            terminal_panel: terminal_panel.clone(),
            terminal_spawner: default_spawner(cx),
            restore_terminal: false,
            shown_editor: (0, None),
            commands: Rc::new(commands),
            focus,
            sidebar_focus,
            editor_focus,
            active_view: None,
            sidebar_visible: restored.as_ref().is_none_or(|s| s.sidebar_visible),
            sidebar_width: restored.as_ref().map_or(SIDEBAR_DEFAULT_WIDTH, |s| {
                px(s.sidebar_width).clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH)
            }),
            explorer_view: Some(explorer.clone().into()),
            explorer,
            documents,
            language_servers,
            symbols,
            breadcrumbs: Breadcrumbs::default(),
            lsp_status: lsp_status.clone(),
            lsp_health: lsp_health.clone(),
            onboarding,
            diagnostics_status: diagnostics_status.clone(),
            watch_status: watch_status.clone(),
            marker_task: None,
            marker_hover: None,
            goto_task: None,
            lsp_logs,
            search_view,
            references_view,
            references_used_last: false,
            extensions_page,
            editor_area: Some(editor_groups.clone().into()),
            editor_groups,
            scroll_sync: ScrollSync::default(),
            status_left: Vec::new(),
            status_right: Vec::new(),
            pending_chord: Vec::new(),
            vim,
            title_bar_pressed: false,
            branch_button_bounds: Rc::default(),
            branch_button_press_closed: false,
            switching_branch: false,
            switches_started: 0,
            quick_open: None,
            focus_before_quick_open: None,
            recent_files: RecentFiles::default(),
            nav_history: HashMap::new(),
            theme_picker: None,
            recent_picker: None,
            recent_picker_loading: None,
            locations_picker: None,
            branch_picker: None,
            branch_picker_loading: None,
            branch_base,
            trust,
            auto_save: Default::default(),
            toast: None,
            toast_count: 0,
            fresh_installs_seen,
            fresh_installs_waiting: Vec::new(),
            persistence: None,
            hot_exit,
            confirming_quit: false,
        };
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |wb, cx| wb.should_close_window(window, cx))
                .unwrap_or(true)
        });
        if let Some(store) = store {
            workbench.start_persistence(store, restored.as_ref(), cx);
        }
        workbench.shown_editor = workbench.shown_editor(cx);
        // A panel shown when the window closed shows a new terminal, once
        // the trust store has answered (`open_restored_terminal`).
        workbench.restore_terminal = workbench.panel.visible;
        if workbench.panel.visible && workbench.panel.maximized {
            // The editor area is not drawn: the panel takes the focus.
            let focus = terminal_panel.read(cx).focus_target(cx);
            window.focus(&focus, cx);
        }
        let groups = workbench.editor_groups.clone();
        workbench._subscriptions.extend([
            cx.observe(&groups, |wb, _, cx| wb.editor_groups_changed(cx)),
            cx.observe(&terminal_panel, |_, _, cx| cx.notify()),
            cx.subscribe_in(&terminal_panel, window, Self::on_terminal_panel_event),
        ]);
        let documents = workbench.documents.clone();
        let trust_events = workbench.start_trust(&documents, window, cx);
        workbench._subscriptions.push(trust_events);
        workbench._subscriptions.push(
            cx.observe_global_in::<trust::TrustGeneration>(window, |wb, window, cx| {
                wb.trust_changed(window, cx)
            }),
        );
        if workbench.language_servers().read(cx).trust_known() {
            workbench.open_restored_terminal(window, cx);
        }
        workbench.start_auto_save(&documents, window, cx);
        workbench.refresh_watched_open_files(cx);
        workbench.update_branch_title(cx);
        workbench.add_status_item(StatusBarSide::Left, diagnostics_status.into(), cx);
        workbench.add_status_item(StatusBarSide::Left, config_error.into(), cx);
        workbench.add_status_item(StatusBarSide::Left, watch_status.into(), cx);
        workbench.add_status_item(StatusBarSide::Left, mode_indicator.into(), cx);
        workbench.add_status_item(StatusBarSide::Right, large_file_status.into(), cx);
        workbench.add_status_item(StatusBarSide::Right, lsp_status.into(), cx);
        workbench.add_status_item(StatusBarSide::Right, document_status.into(), cx);
        // A restored layout may already show a preview beside its editor.
        cx.defer_in(window, |wb, window, cx| {
            let groups = wb.editor_groups.clone();
            wb.scroll_sync
                .refresh(&groups, &wb.root, &wb.settings, window, cx);
        });
        workbench
    }

    /// Starts following layout changes. Runs after the restore, so the
    /// restore itself is not seen as a change.
    fn start_persistence(
        &mut self,
        store: WorkspaceStateStore,
        restored: Option<&WorkspaceState>,
        cx: &mut Context<Self>,
    ) {
        let groups = self.editor_groups.clone();
        self.persistence = Some(Persistence {
            store,
            last_saved: None,
            save_pending: false,
            scroll_rows: restored.map(|s| s.scroll.clone()).unwrap_or_default(),
            _subscriptions: vec![
                cx.observe(&groups, |wb, _, cx| wb.layout_changed(cx)),
                cx.on_app_quit(|wb, cx| {
                    // The backups were written by the earlier hot exit hook
                    // ([`HotExit::flush_all_on_quit`]).
                    wb.save_workspace_state(cx);
                    async {}
                }),
            ],
        });
        let current = self.workspace_state(cx);
        if let Some(p) = &mut self.persistence {
            p.last_saved = Some(current);
        }
    }

    /// The layout as it is now. Scroll rows come from the live views, but
    /// for tabs without a view, or whose text has not loaded, from the
    /// restored rows. Only open tabs keep a row.
    fn workspace_state(&mut self, cx: &mut Context<Self>) -> WorkspaceState {
        let groups = self.editor_groups.read(cx);
        let mut scroll = HashMap::new();
        for tab in groups.model().groups().iter().flat_map(|g| &g.tabs) {
            let Some(path) = tab.path() else {
                continue;
            };
            let live = groups
                .body_of(tab.id)
                .and_then(|b| b.downcast::<CodeView>().ok())
                .filter(|_| !self.documents.read(cx).has_pending_scroll(path))
                .map(|v| v.read(cx).scroll_top_row());
            let cached = self
                .persistence
                .as_ref()
                .and_then(|p| p.scroll_rows.get(&**path).copied());
            if let Some(row) = live.or(cached) {
                scroll.insert(path.to_path_buf(), row);
            }
        }
        let state = WorkspaceState {
            version: crate::workspace_state::STATE_VERSION,
            sidebar_visible: self.sidebar_visible,
            sidebar_width: f32::from(self.sidebar_width),
            sidebar_view: SidebarView::Explorer.state_name().into(),
            groups: groups.model().state(),
            scroll,
            group_widths: groups.group_widths().to_vec(),
            search: self.search_view.read(cx).state(cx),
            branch_diff_base: self.branch_base.read(cx).0.clone(),
            panel: self.panel.clone(),
        };
        if let Some(p) = &mut self.persistence {
            p.scroll_rows = state.scroll.clone();
        }
        state
    }

    /// Writes the layout now, unless it is what was last written. Does
    /// nothing without a state store.
    pub fn save_workspace_state(&mut self, cx: &mut Context<Self>) {
        if self.persistence.is_none() {
            return;
        }
        let state = self.workspace_state(cx);
        let Some(p) = &mut self.persistence else {
            return;
        };
        if p.last_saved.as_ref() == Some(&state) {
            return;
        }
        match p.store.save(&state) {
            Ok(()) => p.last_saved = Some(state),
            Err(err) => tracing::warn!(%err, "could not save the workspace state"),
        }
    }

    /// Something that belongs in the saved layout changed: saves it after
    /// [`SAVE_DELAY`], at most once per delay.
    fn layout_changed(&mut self, cx: &mut Context<Self>) {
        let Some(p) = &mut self.persistence else {
            return;
        };
        if std::mem::replace(&mut p.save_pending, true) {
            return;
        }
        cx.spawn(async move |wb, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            wb.update(cx, |wb, cx| {
                if let Some(p) = &mut wb.persistence {
                    p.save_pending = false;
                }
                wb.save_workspace_state(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The opened folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// What this window shows, which Open Recent leaves out: the folder,
    /// or in single-file mode only the file (its parent folder is not
    /// open).
    pub fn opened_paths(&self) -> Vec<PathBuf> {
        vec![
            self.single_file
                .clone()
                .unwrap_or_else(|| self.root.clone()),
        ]
    }

    /// Whether the window shows one file alone (single-file mode).
    pub fn is_single_file(&self) -> bool {
        self.single_file.is_some()
    }

    /// The opened folder's name, as the title bar shows it; in
    /// single-file mode, the file's name.
    pub fn root_name(&self) -> String {
        let path = self.single_file.as_deref().unwrap_or(&self.root);
        path.file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }

    /// Whether a press on the title bar waits to start a window move, for
    /// tests.
    #[doc(hidden)]
    pub fn title_bar_pressed(&self) -> bool {
        self.title_bar_pressed
    }

    /// The branch checked out in the repository holding the root (a
    /// short SHA when detached); `None` outside a repository.
    pub fn git_branch(&self) -> Option<&str> {
        self.git_branch.as_deref()
    }

    /// Whether the root is in a git repository, judged from the `.git`
    /// found when the window opened ([`GitStore::has_repo_hint`]); no
    /// `git` runs for it.
    pub(crate) fn has_repo_hint(&self) -> bool {
        self.git_head.is_some()
    }

    /// The status bar's warning when a file watcher could not start.
    pub fn watch_status(&self) -> &Entity<WatchStatusItem> {
        &self.watch_status
    }

    /// The background scan of the opened folder.
    pub fn worktree(&self) -> &Worktree {
        &self.worktree
    }

    /// The options of the current scan; `files.exclude` and
    /// `files.scanDepth` changes replace them.
    pub fn scan_options(&self) -> &ScanOptions {
        &self.scan_options
    }

    /// After any [`AppState`] change: drops a pending chord when the
    /// keymap was reloaded, and redraws (the theme may have changed).
    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        // A chord typed against the old keymap means nothing now.
        let generation = keymap_generation(cx);
        if generation != self.keymap_generation {
            self.keymap_generation = generation;
            self.pending_chord.clear();
        }
        // `session.trustAllWorktrees` may have changed.
        self.apply_trust(cx);
        cx.notify();
    }

    /// Rescans when the window's `files.exclude` or `files.scanDepth`
    /// changed.
    fn excludes_changed(&mut self, cx: &mut Context<Self>) {
        let options = scan_options(self.settings.read(cx).view(cx));
        if options.exclude != self.scan_options.exclude
            || options.scan_depth != self.scan_options.scan_depth
        {
            self.scan_options = options;
            self.worktree.set_options(self.scan_options.clone());
        }
    }

    /// A new language registry for this window (the app's changed, or the
    /// folder's `files.associations`): open documents look theirs up again.
    fn languages_changed(&mut self, cx: &mut Context<Self>) {
        let generation = self.settings.read(cx).languages_generation();
        if generation != self.languages_generation {
            self.languages_generation = generation;
            self.documents
                .update(cx, |documents, cx| documents.languages_changed(cx));
        }
    }

    /// After any change to the window's settings (its workspace layer, or
    /// the app's user layer under it): the editor area reads them again.
    fn editor_settings_changed(&mut self, cx: &mut Context<Self>) {
        // `editor.detectIndentation`, `editor.tabSize` or
        // `editor.insertSpaces` may have changed.
        self.documents
            .update(cx, |documents, cx| documents.settings_changed(cx));
        // `breadcrumbs.*` may have changed.
        self.breadcrumbs_changed(cx);
        cx.notify();
    }

    /// The Explorer, installed in the Explorer sidebar slot by [`Workbench::new`].
    pub fn explorer(&self) -> &Entity<ExplorerView> {
        &self.explorer
    }

    /// How many Explorer file operations (Paste, Duplicate, Delete) are
    /// running.
    pub fn file_operations_running(&self) -> usize {
        self.file_operations.get()
    }

    /// Counts a file operation as running until the guard is dropped.
    /// Public for tests, which hold one in place of a long copy.
    #[doc(hidden)]
    pub fn begin_file_operation(&self) -> FileOperation {
        self.file_operations.set(self.file_operations.get() + 1);
        FileOperation(self.file_operations.clone())
    }

    /// What the Explorer's Cut or Copy last took, if anything.
    pub fn explorer_clipboard(&self) -> Option<&ExplorerClipboard> {
        self.explorer_clipboard.as_ref()
    }

    /// Replaces the Explorer's clipboard. The Explorer learns of it once
    /// this update ends (to dim a cut row and enable Paste): this may run
    /// while the Explorer itself is being updated.
    pub(crate) fn set_explorer_clipboard(
        &mut self,
        clip: Option<ExplorerClipboard>,
        cx: &mut Context<Self>,
    ) {
        if self.explorer_clipboard == clip {
            return;
        }
        let cut = clip
            .as_ref()
            .filter(|c| c.op == ClipOp::Cut)
            .and_then(|c| c.path.strip_prefix(&self.root).ok())
            .map(Arc::from);
        let filled = clip.is_some();
        self.explorer_clipboard = clip;
        let explorer = self.explorer.clone();
        cx.defer(move |cx| {
            explorer.update(cx, |explorer, cx| explorer.set_clipboard(cut, filled, cx))
        });
        cx.notify();
    }

    /// Remembers root-relative paths Lucerna itself just changed, so their
    /// watcher events are not decorated as another program's. A directory
    /// also covers the paths under it. An entry is dropped once the
    /// worktree reports its path as added or modified, and after
    /// [`OWN_CHANGE_TTL`] at the latest.
    pub fn expect_own_changes(&mut self, rel_paths: impl IntoIterator<Item = Arc<Path>>) {
        let now = Instant::now();
        self.own_changes
            .extend(rel_paths.into_iter().map(|p| (p, now)));
    }

    /// Drops paths from [`Workbench::expect_own_changes`] whose operation
    /// failed, so another program's later change to them is marked.
    pub fn forget_own_changes(&mut self, rel_paths: &[Arc<Path>]) {
        for path in rel_paths {
            self.own_changes.remove(path);
        }
    }

    /// After Lucerna renamed `from` to `to` (absolute) on disk: open
    /// documents, tabs, recent files and Go Back / Go Forward places at
    /// `from` or under it follow to the new path. Called right after the
    /// rename, before the watcher reports it.
    pub(crate) fn paths_renamed(&mut self, from: &Path, to: &Path, cx: &mut Context<Self>) {
        let (from, to) = (self.absolute_path(from), self.absolute_path(to));
        self.documents
            .update(cx, |documents, cx| documents.rename(&from, &to, cx));
        self.update_editor_groups(cx, |m| m.retarget(&from, &to));
        crate::markdown_preview::retarget_previews(self, &from, &to, cx);
        self.recent_files.retarget(&from, &to);
        for history in self.nav_history.values_mut() {
            history.retarget(&from, &to);
        }
        if self
            .marker_hover
            .as_ref()
            .is_some_and(|(path, _)| path.starts_with(&from))
        {
            self.marker_hover = None;
        }
        // The clipboard's entry follows a rename too.
        // (`moved_path`: the entry itself becomes `to`, never `to/`.)
        if let Some(clip) = &self.explorer_clipboard
            && let Some(moved) = crate::workspace_state::moved_path(&clip.path, &from, &to)
        {
            let moved = ExplorerClipboard {
                op: clip.op,
                path: Arc::from(moved),
            };
            self.set_explorer_clipboard(Some(moved), cx);
        }
        self.refresh_watched_open_files(cx);
        cx.notify();
    }

    /// Save As (spec §4.3, §4.4): asks where to save the document at
    /// `path` (a file or an Untitled document), writes its text there, and
    /// moves the document with every tab, preview and Go Back place to the
    /// new file, undo history and cursors kept. The old file stays as it
    /// was on disk. A tab already open on the chosen file closes and its
    /// document is dropped: the dialog asked before replacing the file.
    /// Choosing the document's own file is a plain save.
    ///
    /// `true` once the text is saved. Cancel or a failed write (reported
    /// in the status bar) is `false` and leaves the document where it was,
    /// unsaved.
    pub fn save_as(
        &mut self,
        path: Arc<Path>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let saved = self.save_as_to(path, window, cx);
        cx.spawn(async move |_, _| saved.await.is_some())
    }

    /// [`Workbench::save_as`], resolving to where the document is once
    /// saved (the chosen file, or its own file when that was chosen), so
    /// a caller can check that path for edits made while it wrote.
    pub(crate) fn save_as_to(
        &mut self,
        path: Arc<Path>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Option<Arc<Path>>> {
        let from = self.absolute_path(&path);
        let (folder, name) = save::save_dialog_start(self, &from, cx);
        let answer = cx.prompt_for_new_path(&folder, Some(&name));
        let documents = self.documents.clone();
        let root = self.root.clone();
        cx.spawn_in(window, async move |wb, cx| {
            let to: Arc<Path> = match answer.await {
                Ok(Ok(Some(to))) => Arc::from(save::under_root(&root, &to)),
                Ok(Err(err)) => {
                    tracing::warn!(%err, "the save dialog failed");
                    return None;
                }
                _ => return None,
            };
            if save::same_file(&from, &to) {
                let opts = save::SaveOptions {
                    reason: save::SaveReason::Explicit,
                    participants: true,
                    overwrite: false,
                };
                let save = cx
                    .update(|window, cx| {
                        documents.update(cx, |d, cx| d.save(&from, opts, window, cx))
                    })
                    .ok()?;
                return matches!(save.await, Ok(true)).then_some(from);
            }
            let write = cx
                .update(|window, cx| {
                    documents.update(cx, |d, cx| d.write_as(&from, &to, window, cx))
                })
                .ok()?;
            let saved = write.await.ok()?;
            wb.update(cx, |wb, cx| {
                wb.close_file_tabs(&to, cx);
                wb.paths_renamed(&from, &to, cx);
                wb.documents
                    .update(cx, |d, cx| d.finish_save_as(&to, saved, cx));
            })
            .ok()?;
            Some(to)
        })
    }

    /// Closes every tab of the file `path` (absolute) without asking and
    /// drops its document, unsaved changes and all: Save As is about to
    /// put another document there. A tab names the file under any spelling
    /// that resolves to it ([`save::same_file`]).
    fn close_file_tabs(&mut self, path: &Path, cx: &mut Context<Self>) {
        let (closing, spellings): (Vec<TabId>, Vec<Arc<Path>>) = self
            .editor_groups
            .read(cx)
            .model()
            .groups()
            .iter()
            .flat_map(|g| &g.tabs)
            .filter_map(|t| match &t.item {
                TabItem::File(p) if save::same_file(p, path) => Some((t.id, p.clone())),
                _ => None,
            })
            .unzip();
        if !closing.is_empty() {
            self.update_editor_groups(cx, |m| {
                for id in closing {
                    m.close(id);
                }
            });
        }
        let mut spellings = spellings;
        spellings.push(Arc::from(path));
        spellings.sort();
        spellings.dedup();
        for p in spellings {
            if self.documents.read(cx).state(&p).is_some() {
                self.documents.update(cx, |d, cx| d.discard(&p, cx));
            }
        }
    }

    /// After Lucerna deleted `path` (absolute): the tabs of files at it or
    /// under it close, except those with unsaved changes, which stay to be
    /// marked deleted when the watcher reports the removal.
    pub(crate) fn paths_deleted(&mut self, path: &Path, cx: &mut Context<Self>) {
        let path = self.absolute_path(path);
        if self
            .explorer_clipboard
            .as_ref()
            .is_some_and(|clip| clip.path.starts_with(&path))
        {
            self.set_explorer_clipboard(None, cx);
        }
        let documents = self.documents.read(cx);
        let closing: Vec<TabId> = self
            .editor_groups
            .read(cx)
            .model()
            .groups()
            .iter()
            .flat_map(|g| &g.tabs)
            .filter(|t| {
                t.path()
                    .is_some_and(|p| p.starts_with(&path) && !documents.is_dirty(p))
            })
            .map(|t| t.id)
            .collect();
        if !closing.is_empty() {
            self.update_editor_groups(cx, |m| {
                for id in closing {
                    m.close(id);
                }
            });
        }
        self.refresh_watched_open_files(cx);
        cx.notify();
    }

    /// Says once per window, after a scan that left folders past
    /// `files.scanDepth` unloaded, that they load when expanded.
    fn show_unloaded_notice(&mut self, snapshot: &WorktreeSnapshot, cx: &mut Context<Self>) {
        if self.unloaded_notice_shown || snapshot.unloaded().is_empty() {
            return;
        }
        let Some(depth) = self.scan_options.scan_depth else {
            return;
        };
        self.unloaded_notice_shown = true;
        let text = format!("Some folders deeper than {depth} levels load when expanded");
        crate::status_message::show(self.window_id, text, cx);
    }

    /// Hands a worktree event to every view that follows the tree. Called
    /// for each event the worktree sends; tests call it directly.
    pub fn on_worktree_event(&mut self, ev: &WorktreeEvent, cx: &mut Context<Self>) {
        let scan_ms = self.scan_started.elapsed().as_millis();
        match ev {
            WorktreeEvent::ScanProgress if !self.scan_progress_logged => {
                self.scan_progress_logged = true;
                tracing::info!(scan_ms, "first scan progress");
            }
            WorktreeEvent::ScanComplete if !self.scan_complete_logged => {
                self.scan_complete_logged = true;
                tracing::info!(scan_ms, "scan complete");
            }
            WorktreeEvent::WatchFailed(reason) => {
                let reason = reason.clone();
                self.watch_status
                    .update(cx, |item, cx| item.set_worktree_error(reason, cx));
            }
            _ => {}
        }
        // Another program respelled a file or folder (an agent's `git mv
        // src Src`): its open documents and tabs follow, as after Rename,
        // instead of going Deleted at the old spelling.
        if let WorktreeEvent::Changed(changes) = ev {
            for (old, new) in respelled(&self.root, changes) {
                self.paths_renamed(&old, &new, cx);
            }
        }
        let snapshot = self.worktree.snapshot();
        // Set on the first `ScanComplete`: until then the unloaded folders
        // are not all known, and searches wait.
        if self.scan_complete_logged {
            self.search_view
                .update(cx, |view, cx| view.set_unloaded(snapshot.unloaded(), cx));
        }
        // Only a finished scan lists every unloaded folder.
        if matches!(ev, WorktreeEvent::ScanComplete | WorktreeEvent::Rescanned) {
            self.show_unloaded_notice(&snapshot, cx);
        }
        let decorate = decorates_external_changes(self.settings.read(cx).view(cx));
        // Open documents tell our own saves from other programs' writes
        // once they reload (`DocumentEvent::ChangedOnDisk`).
        let now = Instant::now();
        self.own_changes
            .retain(|_, at| now.duration_since(*at) < OWN_CHANGE_TTL);
        let own = &self.own_changes;
        let followed: HashSet<Arc<Path>> = match ev {
            WorktreeEvent::Changed(changes) => {
                let documents = self.documents.read(cx);
                changes
                    .iter()
                    .filter(|c| {
                        documents.follows_disk(&c.path)
                            || c.path.ancestors().any(|a| own.contains_key(a))
                    })
                    .map(|c| c.path.clone())
                    .collect()
            }
            _ => HashSet::new(),
        };
        // Reported: forget it, so another program's later change to it is
        // marked. A removal (a rename's or delete's old path) does not end
        // the wait; the TTL still bounds every entry.
        if let WorktreeEvent::Changed(changes) = ev {
            for change in changes.iter().filter(|c| c.kind != ChangeKind::Removed) {
                self.own_changes.remove(&change.path);
            }
            // The clipboard's entry is gone (another program removed it,
            // or a folder around it): nothing is left to paste.
            let gone = self.explorer_clipboard.as_ref().is_some_and(|clip| {
                clip.path.strip_prefix(&self.root).is_ok_and(|rel| {
                    !snapshot.contains(rel)
                        && changes
                            .iter()
                            .any(|c| c.kind == ChangeKind::Removed && rel.starts_with(&c.path))
                })
            });
            if gone {
                self.set_explorer_clipboard(None, cx);
            }
        }
        self.explorer.update(cx, |explorer, cx| {
            explorer.on_worktree_event(
                ev,
                snapshot,
                &|path| decorate && !followed.contains(path),
                cx,
            )
        });
        self.documents
            .update(cx, |documents, cx| documents.on_worktree_event(ev, cx));
        // After the documents: removed files are closed in their servers.
        self.language_servers
            .update(cx, |servers, cx| servers.on_worktree_event(ev, cx));
        self.git_worktree_event(ev, cx);
        // The tree changed: some open files may have joined or left it.
        self.refresh_watched_open_files(cx);
    }

    /// Open files the worktree does not follow (outside the root, not in
    /// the tree, e.g. ignored, or reached through a symlink, whose target
    /// the tree reports under its own path; all of them in single-file
    /// mode), which the open-file watcher follows.
    pub fn watched_open_files(&self) -> &[PathBuf] {
        &self.watched_open_files
    }

    /// Re-picks [`Workbench::watched_open_files`] after the tabs or the
    /// tree changed. The watcher is told even when the list is the same:
    /// a link among it may point elsewhere now.
    fn refresh_watched_open_files(&mut self, cx: &App) {
        let snapshot = self.worktree.snapshot();
        let mut files: Vec<PathBuf> = self
            .open_tab_paths(cx)
            .into_iter()
            .filter(|path| match path.strip_prefix(&self.root) {
                // The tree does not descend into linked folders, so only
                // the file itself can be a link.
                Ok(rel) => {
                    !self.scanning
                        || !snapshot.contains(rel)
                        || path.symlink_metadata().is_ok_and(|m| m.is_symlink())
                }
                Err(_) => true,
            })
            .collect();
        files.sort();
        files.dedup();
        if let Some(watcher) = &self.open_file_watcher {
            watcher.set_files(files.iter().chain(&self.git_head).cloned().collect());
        }
        self.watched_open_files = files;
    }

    /// Every open tab's path, once per path, with the file the dynamic
    /// Markdown preview shows (it has no path of its own, but its file must
    /// stay watched).
    pub fn open_tab_paths(&self, cx: &App) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self
            .editor_groups
            .read(cx)
            .model()
            .groups()
            .iter()
            .flat_map(|g| {
                g.tabs
                    .iter()
                    .filter_map(|t| t.path().or(t.previewed_path()))
            })
            // An untitled document has no file to watch.
            .filter(|p| !crate::untitled::is_untitled(p))
            .map(|p| p.to_path_buf())
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }

    /// A repository found after the window opened (`git init`, then Git:
    /// Refresh) gives the title bar its branch and watched `HEAD`.
    pub(crate) fn git_state_changed(&mut self, cx: &mut Context<Self>) {
        if self.git_head.is_some() {
            return;
        }
        let Some(head) = self.git.read(cx).git_dir().map(|d| d.join("HEAD")) else {
            return;
        };
        self.git_head = Some(head);
        self.git_branch = crate::git_branch::read_branch(self.root());
        if let Some(watcher) = &self.open_file_watcher {
            watcher.set_files(
                self.watched_open_files
                    .iter()
                    .chain(&self.git_head)
                    .cloned()
                    .collect(),
            );
        }
        cx.notify();
    }

    /// Changes to files on disk that the worktree does not report: from
    /// the open-file watcher and the config watcher. Open documents among
    /// `changed` (compared with symlinks resolved, as watchers report
    /// real paths) reload, or are marked deleted when gone. Tests call it
    /// directly.
    pub fn on_open_file_changes(&mut self, changed: &[PathBuf], cx: &mut Context<Self>) {
        if let Some(head) = &self.git_head
            && !matching_files(std::slice::from_ref(head), changed).is_empty()
        {
            self.git_branch = crate::git_branch::read_branch(&self.root);
            cx.notify();
        }
        let open = self.open_tab_paths(cx);
        let changes: Vec<Change> = matching_files(&open, changed)
            .into_iter()
            .map(|path| Change {
                kind: if path.exists() {
                    ChangeKind::Modified
                } else {
                    ChangeKind::Removed
                },
                path: Arc::from(path),
            })
            .collect();
        if changes.is_empty() {
            return;
        }
        let ev = WorktreeEvent::Changed(changes);
        self.documents
            .update(cx, |documents, cx| documents.on_worktree_event(&ev, cx));
        self.language_servers
            .update(cx, |servers, cx| servers.on_worktree_event(&ev, cx));
        // Open files the worktree does not report (single-file mode).
        self.git_worktree_event(&ev, cx);
        // A link outside the tree may point elsewhere now.
        self.refresh_watched_open_files(cx);
    }

    /// A configuration batch arrived: an open `settings.json` (or other
    /// configuration file) among it reloads.
    fn config_files_changed(&mut self, cx: &mut Context<Self>) {
        let Some((batch, changed)) = cx
            .try_global::<ConfigState>()
            .map(|state| state.last_changed())
            .map(|(batch, changed)| (batch, changed.to_vec()))
        else {
            return;
        };
        if batch != self.config_batch {
            self.config_batch = batch;
            self.on_open_file_changes(&changed, cx);
        }
    }

    /// The workspace settings reloaded: an open `.lucerna/settings.json`
    /// reloads too.
    fn workspace_settings_changed(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings.read(cx);
        let generation = settings.generation();
        if generation == self.settings_generation {
            return;
        }
        self.settings_generation = generation;
        if let Some(path) = settings.settings_path().map(Path::to_path_buf) {
            self.on_open_file_changes(&[path], cx);
        }
    }

    /// This window's settings: its folder's `.lucerna/settings.json` over
    /// the app's default and user layers.
    pub fn settings(&self) -> &Entity<WindowSettings> {
        &self.settings
    }

    /// The Search tab's body.
    pub fn search_view(&self) -> &Entity<SearchView> {
        &self.search_view
    }

    /// Focuses the Search tab in whichever group holds it, or opens it in
    /// the active group, then puts the cursor in its query.
    pub fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update_editor_groups(cx, EditorGroups::open_search);
        self.search_view
            .update(cx, |view, cx| view.focus_query(None, window, cx));
        cx.notify();
    }

    /// The References tab's body.
    pub fn references_view(&self) -> &Entity<ReferencesView> {
        &self.references_view
    }

    /// Records which results tab was used last: `true` for References,
    /// `false` for Search (see `references_used_last`).
    pub(crate) fn set_references_used_last(&mut self, references: bool) {
        self.references_used_last = references;
    }

    /// Before a key is resolved: a results list with the focus is the one
    /// used last.
    fn note_results_focus(&mut self, window: &Window, cx: &App) {
        let focused = |handle: FocusHandle| handle.contains_focused(window, cx);
        if focused(self.references_view.read(cx).focus_handle(cx)) {
            self.references_used_last = true;
        } else if focused(self.search_view.read(cx).focus_handle(cx)) {
            self.references_used_last = false;
        }
    }

    /// Drops the References tab's hold on its lookup's document once the
    /// tab is closed.
    fn release_closed_references(&mut self, cx: &mut Context<Self>) {
        if self
            .editor_groups
            .read(cx)
            .model()
            .references_tab()
            .is_none()
        {
            self.references_view
                .update(cx, |view, _| view.release_origin());
        }
    }

    /// `referencesViewFocus`: the References tab is open and is the
    /// results tab used last (see `references_used_last`), so `F4` walks
    /// it even after a reference it opened took the focus.
    fn references_view_focus(&self, cx: &App) -> bool {
        self.references_used_last
            && self
                .editor_groups
                .read(cx)
                .model()
                .references_tab()
                .is_some()
    }

    /// Whether keyboard focus is in the open Search tab.
    fn search_focused(&self, window: &Window, cx: &App) -> bool {
        self.editor_groups.read(cx).model().search_tab().is_some()
            && self
                .search_view
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
    }

    /// The Extensions page, shown in the Extensions tab.
    pub fn extensions_page(&self) -> &Entity<ExtensionsPage> {
        &self.extensions_page
    }

    /// Focuses the Extensions tab in whichever group holds it, or opens it
    /// in the active group, then puts the cursor in its search input.
    pub fn open_extensions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update_editor_groups(cx, EditorGroups::open_extensions);
        self.extensions_page.update(cx, |page, cx| {
            page.shown(cx);
            page.focus_search(window, cx);
        });
        cx.notify();
    }

    /// Whether keyboard focus is in the open Extensions tab.
    fn extensions_focused(&self, window: &Window, cx: &App) -> bool {
        self.editor_groups
            .read(cx)
            .model()
            .extensions_tab()
            .is_some()
            && self
                .extensions_page
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
    }

    /// After the tabs changed (and wherever focus is restored): when the
    /// focused tab body is no longer shown, because its tab closed or
    /// stopped being its group's active tab (a jump in the same group, a
    /// search result opened in its place), focus goes to the editor area,
    /// so keys keep working. Covers code views, Markdown previews, the
    /// Extensions, Search, References, Health and Get Started tabs and
    /// server logs.
    pub(crate) fn refocus_if_hidden(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let groups = self.editor_groups.read(cx);
        let model = groups.model();
        let hidden = |tab: Option<TabId>| {
            tab.is_none_or(|id| !model.groups().iter().any(|g| g.active == Some(id)))
        };
        let (extensions_hidden, search_hidden, references_hidden, health_hidden) = (
            hidden(model.extensions_tab()),
            hidden(model.search_tab()),
            hidden(model.references_tab()),
            hidden(model.lsp_health_tab()),
        );
        let had_focus = |handle: FocusHandle| handle.contains_focused(window, cx);
        let lost = (extensions_hidden && had_focus(self.extensions_page.read(cx).focus_handle(cx)))
            || (search_hidden && had_focus(self.search_view.read(cx).focus_handle(cx)))
            || (references_hidden && had_focus(self.references_view.read(cx).focus_handle(cx)))
            || (health_hidden && had_focus(self.lsp_health.read(cx).focus_handle(cx)))
            || (hidden(model.onboarding_tab())
                && self
                    .onboarding_page()
                    .is_some_and(|page| had_focus(page.read(cx).focus_handle(cx))))
            || self.lsp_logs.hidden_with_focus(model, window, cx)
            || groups.hidden_bodies().any(|body| {
                let handle = match body.clone().downcast::<CodeView>() {
                    Ok(view) => Some(view.read(cx).focus_handle(cx)),
                    Err(body) => match body.downcast::<DiffView>() {
                        Ok(view) => Some(view.read(cx).focus_handle(cx)),
                        Err(body) => body
                            .downcast::<MarkdownPreview>()
                            .ok()
                            .map(|preview| preview.read(cx).focus_handle(cx)),
                    },
                };
                handle.is_some_and(had_focus)
            });
        if lost {
            self.focus_editor_area(window, cx);
        }
    }

    /// The number on the Extensions button: the available updates, when
    /// there are any.
    pub fn extension_badge(&self, cx: &App) -> Option<String> {
        let updates = cx.try_global::<ExtensionService>()?.updates().len();
        (updates > 0).then(|| updates.to_string())
    }

    /// Every open file, shown by the code views in the tabs.
    pub fn documents(&self) -> &Entity<Documents> {
        &self.documents
    }

    /// What backs up this window's unsaved documents; `None` without a
    /// data dir, where hot exit is off.
    pub fn hot_exit(&self) -> Option<&Entity<HotExit>> {
        self.hot_exit.as_ref()
    }

    /// The language servers of this window's folder.
    pub fn language_servers(&self) -> &Entity<LanguageServers> {
        &self.language_servers
    }

    /// The IDs of the commands every workbench has
    /// ([`default_registry`]), ordered by ID; no window needed.
    pub fn command_ids() -> Vec<&'static str> {
        default_registry().ids().collect()
    }

    pub fn commands(&self) -> &CommandRegistry {
        &self.commands
    }

    /// For other modules to register their commands at startup.
    pub fn commands_mut(&mut self) -> &mut CommandRegistry {
        Rc::make_mut(&mut self.commands)
    }

    /// Runs a registered command. Returns `false` when `id` is unknown.
    pub fn run_command(
        &mut self,
        id: &str,
        args: Option<&Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // The registry is shared so a handler may borrow `self` mutably.
        let commands = Rc::clone(&self.commands);
        commands.run(id, args, self, window, cx)
    }

    /// The editor groups view, installed in the editor area by [`Workbench::new`].
    pub fn editor_groups(&self) -> &Entity<EditorGroupsView> {
        &self.editor_groups
    }

    /// Changes the editor groups model and redraws it.
    pub fn update_editor_groups<R>(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut EditorGroups) -> R,
    ) -> R {
        self.editor_groups
            .update(cx, |view, cx| view.update_model(cx, f))
    }

    /// `path` as the workbench names it: absolute, a relative path joined
    /// onto the root.
    pub fn absolute_path(&self, path: &Path) -> Arc<Path> {
        Arc::from(absolute_path(&self.root, path))
    }

    /// `path` of a file in the project: absolute, a relative path (the
    /// Explorer's and Quick Open's worktree paths) always joined onto the
    /// root, so a real file named `untitled:…` is never taken for an
    /// Untitled key (spec §4.1).
    pub(crate) fn project_path(&self, path: &Path) -> Arc<Path> {
        Arc::from(self.root.join(path))
    }

    /// Opens `path` without a place to go to (the Explorer, Quick Open
    /// without `:line`, Open Recent, `luc file`, a restored session): a
    /// tab of the group already showing the file (source or preview) is
    /// activated; otherwise it opens in the editor
    /// `workbench.editorAssociations` names, the Markdown preview or the
    /// source (spec §4.1). Otherwise like [`Workbench::open_source`]. A
    /// relative path is a file of the project, never an Untitled key.
    pub fn open_path(&mut self, path: Arc<Path>, mode: OpenMode, cx: &mut Context<Self>) {
        let path = self.project_path(&path);
        let view =
            markdown_preview::associated_editor(&self.root, &path, self.settings.read(cx).view(cx));
        self.open_file(path, view, mode, true, cx);
    }

    /// Opens the source of `path` in the editor area, for opens that go to
    /// a place in it ([`Workbench::open_path_at`], [`Workbench::jump_to`],
    /// Go Back): the group's source tab of the file is reused, a preview of
    /// it is not. `Preview` honours the `workbench.editor.enablePreview`
    /// setting. Records `path` in [`Workbench::recent_files`]. A relative
    /// path is resolved against the root first, so tabs, recent files and
    /// the saved state only ever hold absolute paths.
    pub fn open_source(&mut self, path: Arc<Path>, mode: OpenMode, cx: &mut Context<Self>) {
        let path = self.absolute_path(&path);
        self.open_file(path, FileView::Source, mode, false, cx);
    }

    /// `workbench.action.files.newUntitledFile`: a new empty Untitled
    /// document in a pinned tab of the active group, its view focused.
    pub fn new_untitled_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.documents.update(cx, |d, cx| d.new_untitled(cx));
        self.open_source(key, OpenMode::Pinned, cx);
        self.focus_editor_area(window, cx);
    }

    /// Opens the static Markdown preview of `path` as a pinned tab, as the
    /// Explorer's Open Preview does.
    pub(crate) fn open_markdown_preview(&mut self, path: Arc<Path>, cx: &mut Context<Self>) {
        let path = self.absolute_path(&path);
        self.open_file(path, FileView::Preview, OpenMode::Pinned, false, cx);
    }

    /// Opens absolute `path` as `view` (see [`EditorGroups::open_file`]).
    fn open_file(
        &mut self,
        path: Arc<Path>,
        view: FileView,
        mode: OpenMode,
        reuse_any: bool,
        cx: &mut Context<Self>,
    ) {
        // An F8 hover still waiting for its tab is for an earlier open.
        self.marker_hover = None;
        if !crate::untitled::is_untitled(&path) {
            self.recent_files.record(path.clone());
        }
        let preview_enabled = self
            .settings
            .read(cx)
            .get::<bool>("workbench.editor.enablePreview", None, cx)
            .unwrap_or(true);
        self.update_editor_groups(cx, |m| {
            m.open_file(path, view, mode, preview_enabled, reuse_any)
        });
        // A file with unsaved changes opened as a preview.
        self.pin_dirty_previews(cx);
    }

    /// Pins every preview tab whose document has unsaved changes.
    fn pin_dirty_previews(&mut self, cx: &mut Context<Self>) {
        let documents = self.documents.read(cx);
        let previews: Vec<TabId> = self
            .editor_groups
            .read(cx)
            .model()
            .groups()
            .iter()
            .flat_map(|g| &g.tabs)
            .filter(|t| !t.pinned && t.path().is_some_and(|p| documents.is_dirty(p)))
            .map(|t| t.id)
            .collect();
        if !previews.is_empty() {
            self.update_editor_groups(cx, |m| previews.into_iter().for_each(|id| m.pin(id)));
        }
    }

    /// Asks whether to save the documents with unsaved changes before the
    /// app quits or the window closes (spec §6.3): `true` to go on. Save
    /// All saves each one and goes on only if all were saved and nothing
    /// is dirty afterwards, Don't Save
    /// goes on (deleting their hot exit backups), Cancel (or a dismissed
    /// prompt) stops. Then, when
    /// `terminal.integrated.confirmOnExit` asks about running terminals
    /// (terminal spec §6.1), whether to terminate them. Then, while an
    /// Explorer file operation runs, whether to quit anyway (a copy cut
    /// short leaves only its hidden name). Nothing to ask: `true` at once.
    pub fn confirm_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        let paths = self.documents.read(cx).dirty_paths();
        self.confirm_unsaved(paths, false, window, cx)
    }

    /// [`Workbench::confirm_quit`] for only `paths` (and the terminals and
    /// file operations):
    /// the files whose hot exit backup failed as the app quits. The other
    /// unsaved files are neither saved nor checked.
    pub fn confirm_quit_for(
        &mut self,
        paths: Vec<Arc<Path>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        self.confirm_unsaved(paths, true, window, cx)
    }

    /// Asks about `paths`; once answered, acts on the files dirty then:
    /// every one, or with `only_these`, those among `paths`.
    fn confirm_unsaved(
        &mut self,
        paths: Vec<Arc<Path>>,
        only_these: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let documents = self.documents.clone();
        let hot_exit = self.hot_exit.clone();
        let asked = paths.clone();
        let dirty_now = move |d: &Documents| -> Vec<Arc<Path>> {
            d.dirty_paths()
                .into_iter()
                .filter(|p| !only_these || asked.contains(p))
                .collect()
        };
        let terminals = self.terminals_to_confirm(cx);
        let operations = self.file_operations.clone();
        if paths.is_empty() && terminals == 0 && operations.get() == 0 {
            return Task::ready(true);
        }
        // A sheet on a minimised window, or one on another Space, is never
        // seen and the quit seems to do nothing: bring the window forward
        // first, also when its prompt is already up (as Zed does).
        window.activate_window();
        // The prompt is already up for an earlier quit or close.
        if self.confirming_quit {
            return Task::ready(false);
        }
        self.confirming_quit = true;
        let unsaved = (!paths.is_empty()).then(|| {
            let names: Vec<String> = paths.iter().map(|p| save::display_name(p)).collect();
            save::prompt_unsaved_all(&names, window, cx)
        });
        cx.spawn_in(window, async move |wb, cx| {
            // Don't Save: the backups go only once nothing else stops the
            // close or quit, so a Cancel at the terminal prompt keeps them.
            let mut discard = false;
            let mut go_on = match unsaved {
                None => true,
                Some(answer) => match SaveChoice::of(answer.await) {
                    SaveChoice::Save => {
                        let paths = documents.read_with(cx, |d, _| dirty_now(d));
                        // An edit typed while the saves wrote is not saved:
                        // stop rather than lose it. Also checked where the
                        // documents are now: Save As moved Untitled ones
                        // off the paths asked about.
                        save::save_each(
                            wb.clone(),
                            documents.clone(),
                            paths,
                            save::UntitledCancel::Stop,
                            cx,
                        )
                        .await
                        .is_some_and(|now| {
                            documents.read_with(cx, |d, _| {
                                dirty_now(d).is_empty() && !now.iter().any(|p| d.is_dirty(p))
                            })
                        })
                    }
                    SaveChoice::DontSave => {
                        discard = true;
                        true
                    }
                    SaveChoice::Cancel => false,
                },
            };
            if go_on && terminals > 0 {
                let answer = wb.update_in(cx, |_, window, cx| {
                    prompt_terminate_terminals(terminals, window, cx)
                });
                go_on = match answer {
                    Ok(answer) => answer.await == Ok(0),
                    Err(_) => false,
                };
            }
            // Asked only if one still runs once the questions before are
            // answered.
            if go_on && operations.get() > 0 {
                let answer = wb.update_in(cx, |_, window, cx| {
                    prompt_file_operation_running(window, cx)
                });
                go_on = match answer {
                    Ok(answer) => answer.await == Ok(0),
                    Err(_) => false,
                };
            }
            if go_on
                && discard
                && let Some(hot_exit) = &hot_exit
            {
                let paths = documents.read_with(cx, |d, _| dirty_now(d));
                hot_exit.update(cx, |h, cx| h.discard(&paths, cx));
            }
            wb.update(cx, |wb, _| wb.confirming_quit = false).ok();
            go_on
        })
    }

    /// What Cmd+Q asks of this window (hot exit spec §5.3). With hot exit
    /// on: saves the files `files.autoSave` saves, writes every backup now,
    /// and asks only about the files whose backup failed, running
    /// terminals and running file operations. Otherwise (or without a data
    /// dir) [`Workbench::confirm_quit`].
    fn confirm_app_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        let Some(hot_exit) = self.hot_exit.clone().filter(|_| HotExit::enabled(cx)) else {
            return self.confirm_quit(window, cx);
        };
        let documents = self.documents.clone();
        let auto_saved = self.auto_save_paths(cx);
        cx.spawn_in(window, async move |wb, cx| {
            // A file that could not be saved stays dirty: its backup is
            // written below.
            // No Untitled document among them: `auto_save_paths` leaves
            // those to their backups.
            if !auto_saved.is_empty()
                && save::save_each(
                    wb.clone(),
                    documents,
                    auto_saved,
                    save::UntitledCancel::Stop,
                    cx,
                )
                .await
                .is_none()
            {
                tracing::warn!("could not auto-save every file as the app quits");
            }
            // Documents closed from here on keep their backups.
            let flush = hot_exit.update(cx, |h, cx| {
                h.set_quitting();
                h.flush(cx)
            });
            let failed = flush.await;
            match wb.update_in(cx, |wb, window, cx| wb.confirm_quit_for(failed, window, cx)) {
                Ok(confirm) => confirm.await,
                // The window closed meanwhile: nothing left to ask.
                Err(_) => true,
            }
        })
    }

    /// How many terminals `terminal.integrated.confirmOnExit` asks about
    /// before the app quits or the window closes: every running one with
    /// `always`, those running something other than their shell (as of the
    /// last poll) with `hasChildProcesses`, none with `never`.
    fn terminals_to_confirm(&self, cx: &App) -> usize {
        let mode = confirm_on_exit(self.settings.read(cx).view(cx));
        if mode == ConfirmOnExit::Never {
            return 0;
        }
        self.terminal_panel
            .read(cx)
            .tabs()
            .iter()
            .filter_map(|tab| tab.view.as_ref())
            .map(|view| view.read(cx))
            .filter(|view| match mode {
                ConfirmOnExit::Always => view.exited().is_none(),
                _ => view.has_child_process(),
            })
            .count()
    }

    /// The window's close button: closes at once when nothing is dirty,
    /// no terminal needs confirming and no file operation runs;
    /// otherwise asks ([`Workbench::confirm_quit`]) and closes the window
    /// itself if told to go on.
    fn should_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.close_theme_picker(true, window, cx);
        if self.documents.read(cx).dirty_paths().is_empty()
            && self.terminals_to_confirm(cx) == 0
            && self.file_operations.get() == 0
        {
            self.save_workspace_state(cx);
            return true;
        }
        let confirm = self.confirm_quit(window, cx);
        cx.spawn_in(window, async move |wb, cx| {
            if confirm.await {
                wb.update_in(cx, |wb, window, cx| {
                    wb.save_workspace_state(cx);
                    window.remove_window();
                })
                .ok();
            }
        })
        .detach();
        false
    }

    /// Opens the source of `path` ([`Workbench::open_source`]), then puts
    /// the cursor at `point` and scrolls it to the middle, once the file is
    /// loaded. For Quick Open, search results and go-to-line.
    pub fn open_path_at(
        &mut self,
        path: Arc<Path>,
        mode: OpenMode,
        point: Point,
        cx: &mut Context<Self>,
    ) {
        self.open_path_at_target(path, mode, GoTo::Byte(point), cx);
    }

    /// [`Workbench::open_path_at`] for a [`GoTo`], which may count its
    /// column in characters.
    fn open_path_at_target(
        &mut self,
        path: Arc<Path>,
        mode: OpenMode,
        to: GoTo,
        cx: &mut Context<Self>,
    ) {
        let path = self.absolute_path(&path);
        self.open_source(path.clone(), mode, cx);
        // An already-drawn tab has its view; a new one gets it on draw.
        let view = self
            .editor_groups
            .read(cx)
            .active_body()
            .and_then(|v| v.downcast::<CodeView>().ok());
        self.documents.update(cx, |docs, cx| match view {
            Some(view) => docs.go_to_target(&view, to, cx),
            None => docs.go_to_when_opened(&path, to),
        });
    }

    /// The place the active editor shows: cursor and scroll, if its tab is
    /// a drawn code view.
    pub(crate) fn nav_here(&self, cx: &App) -> Option<NavEntry> {
        let view = self.active_code_view(cx)?;
        let v = view.read(cx);
        Some(NavEntry {
            path: v.path().clone(),
            point: v.cursor(),
            scroll_top_row: v.scroll_top_row(),
        })
    }

    /// Remembers where the active editor is before a jump to `to`, for
    /// the active group's Go Back.
    pub(crate) fn record_nav(&mut self, to: &NavEntry, cx: &App) {
        let Some(from) = self.nav_here(cx) else {
            return;
        };
        let groups = self.editor_groups.read(cx).model();
        let group = groups.active_group_id();
        // Groups that were closed take their history with them.
        self.nav_history
            .retain(|id, _| groups.groups().iter().any(|g| g.id == *id));
        self.nav_history.entry(group).or_default().record(from, to);
    }

    /// Opens `path` like [`Workbench::open_path_at`] after recording the
    /// place the active editor is at, so Go Back returns to it. Jumps in
    /// the same file go through here too; short ones are not recorded.
    pub fn jump_to(
        &mut self,
        path: Arc<Path>,
        point: Point,
        mode: OpenMode,
        cx: &mut Context<Self>,
    ) {
        let path = self.absolute_path(&path);
        self.record_nav(
            &NavEntry {
                path: path.clone(),
                point,
                scroll_top_row: 0,
            },
            cx,
        );
        self.open_path_at(path, mode, point, cx);
    }

    /// Go Back (`back`) or Go Forward in the active group: opens the
    /// remembered place as a pinned tab with its cursor and scroll. Places
    /// whose file no longer exists are dropped, found by one `stat` each
    /// on this thread when the key is pressed.
    pub(crate) fn navigate(&mut self, back: bool, cx: &mut Context<Self>) {
        let Some(current) = self.nav_here(cx) else {
            return;
        };
        let group = self.editor_groups.read(cx).model().active_group_id();
        let Some(history) = self.nav_history.get_mut(&group) else {
            return;
        };
        let exists = |entry: &NavEntry| entry.path.exists();
        let target = if back {
            history.back_where(current, exists)
        } else {
            history.forward_where(current, exists)
        };
        let Some(target) = target else {
            return;
        };
        self.open_source(target.path.clone(), OpenMode::Pinned, cx);
        let view = self.active_code_view(cx);
        self.documents.update(cx, |docs, cx| match view {
            Some(view) => docs.go_to_and_scroll(&view, target.point, target.scroll_top_row, cx),
            None => {
                docs.go_to_and_scroll_when_opened(&target.path, target.point, target.scroll_top_row)
            }
        });
    }

    /// The `when`-clause context for key bindings.
    pub fn context_keys(&self, window: &Window, cx: &App) -> WhenContext {
        let sidebar_focus = self.sidebar_visible && self.sidebar_focus.contains_focused(window, cx);
        let mut ctx = WhenContext::default();
        let mut flag = |key: &str, on: bool| ctx.set(key, ContextValue::Bool(on));
        flag(
            "editorFocus",
            self.editor_focus.contains_focused(window, cx),
        );
        flag("sideBarVisible", self.sidebar_visible);
        flag("isMac", cfg!(target_os = "macos"));
        flag("isLinux", cfg!(target_os = "linux"));
        flag("isWindows", cfg!(target_os = "windows"));
        let terminal_focus = self.terminal_has_focus(window, cx);
        flag("terminalFocus", terminal_focus);
        let terminal = self
            .active_terminal_view(cx)
            .filter(|_| terminal_focus)
            .map(|view| view.read(cx).terminal());
        flag(
            "terminalTextSelected",
            terminal.is_some_and(|t| t.has_selection()),
        );
        flag(
            "terminalAltBufferActive",
            terminal.is_some_and(|t| t.mode().contains(lucerna_terminal::Modes::ALT_SCREEN)),
        );
        flag("sideBarFocus", sidebar_focus);
        flag("explorerViewletFocus", sidebar_focus);
        let explorer = self.explorer.read(cx);
        let menu_open = explorer.context_menu_open();
        let kind = explorer.selected_kind();
        // Not while a name is typed in the tree: `textInputFocus` is a
        // frame late when the input just took the focus.
        flag(
            "filesExplorerFocus",
            sidebar_focus && !menu_open && !explorer.editing(),
        );
        flag(
            "explorerResourceIsFolder",
            matches!(kind, SelectedKind::Dir | SelectedKind::Root),
        );
        flag("explorerResourceIsRoot", kind == SelectedKind::Root);
        flag(
            "explorerResourceCut",
            self.explorer_clipboard
                .as_ref()
                .is_some_and(|c| c.op == ClipOp::Cut),
        );
        flag("contextMenuVisible", menu_open);
        flag("searchViewletFocus", self.search_focused(window, cx));
        // The Search tab's Replace row (VS Code's `replaceActive` and
        // `replaceInputBoxFocus`), only while the tab is open.
        let search_open = self.editor_groups.read(cx).model().search_tab().is_some();
        let search = self.search_view.read(cx);
        flag("replaceActive", search_open && search.replace_active(cx));
        flag(
            "replaceInputBoxFocus",
            search_open && search.replace_input_focused(window, cx),
        );
        // The selected result in the focused results list (VS Code's
        // `matchFocus`, `fileMatchFocus` and `fileMatchOrMatchFocus`).
        let selection = (search_open && search.list_focused(window))
            .then(|| search.list_selection(cx))
            .flatten();
        let match_focus = matches!(selection, Some(crate::ListSelection::Line(r))
            if search.results(cx).line(r).is_some_and(|l| !l.ranges.is_empty()));
        let file_match_focus = matches!(selection, Some(crate::ListSelection::Header(_)));
        flag("matchFocus", match_focus);
        flag("fileMatchFocus", file_match_focus);
        flag("fileMatchOrMatchFocus", match_focus || file_match_focus);
        flag("referencesViewFocus", self.references_view_focus(cx));
        flag(
            "gitDiffViewFocus",
            self.focused_diff_view(window, cx).is_some(),
        );
        flag(
            "extensionsViewletFocus",
            self.extensions_focused(window, cx),
        );
        flag("onboardingFocus", self.onboarding_focused(window, cx));
        // The theme, locations and recent pickers are quick picks too:
        // Escape closes them.
        flag(
            "inQuickOpen",
            self.quick_open.is_some()
                || self.theme_picker.is_some()
                || self.locations_picker.is_some()
                || self.recent_picker.is_some()
                || self.branch_picker.is_some(),
        );
        flag("inRecentPicker", self.recent_picker.is_some());
        // As of the last frame: GPUI knows key contexts only once drawn.
        flag(
            "textInputFocus",
            window
                .context_stack()
                .iter()
                .any(|c| c.contains(TEXT_INPUT_KEY_CONTEXT)),
        );
        // The code view has focus and no text input (find, Quick Open…)
        // has taken the keys: where the LSP editor actions apply.
        let text_input = window
            .context_stack()
            .iter()
            .any(|c| c.contains(TEXT_INPUT_KEY_CONTEXT));
        // Only a code view takes text: not the Search or References tab.
        let code_view = self.active_code_view(cx);
        flag(
            "editorTextFocus",
            self.editor_focus.contains_focused(window, cx) && !text_input && code_view.is_some(),
        );
        if let Some(view) = &code_view {
            let v = view.read(cx);
            flag("editorReadonly", v.is_readonly());
            let selections = v.selections();
            flag(
                "editorHasSelection",
                selections.all().iter().any(|s| !s.is_empty()),
            );
            flag("editorHasMultipleSelections", selections.len() > 1);
            flag(
                "activeEditorIsDirty",
                self.documents.read(cx).is_dirty(v.path()),
            );
        }
        flag("findWidgetVisible", self.find_widget_visible(cx));
        flag(
            "replaceInputFocussed",
            self.replace_input_focused(window, cx),
        );
        flag(
            "lspHoverVisible",
            self.active_code_view(cx)
                .is_some_and(|v| v.read(cx).hover().is_visible()),
        );
        let active = self.editor_groups.read(cx).model().active_tab();
        let body = self.editor_groups.read(cx).active_body();
        // A preview counts as an editor of the file it shows: the dynamic
        // one's view knows which (its tab until the view is made).
        let preview = body
            .clone()
            .and_then(|body| body.downcast::<MarkdownPreview>().ok());
        let path = match &preview {
            Some(preview) => Some(preview.read(cx).path()),
            None => active.and_then(|tab| tab.path().or(tab.previewed_path())),
        };
        let extname = path.map(|p| extname(p)).unwrap_or_default();
        let preview_active = active.is_some_and(|tab| tab.previewed_path().is_some());
        ctx.set(
            "activeEditor",
            ContextValue::Str(
                if preview_active {
                    "markdownPreview"
                } else {
                    ""
                }
                .into(),
            ),
        );
        // The language the document resolved (`files.associations`
        // included); before its view exists, the same lookup by path.
        let view = body.and_then(|body| body.downcast::<CodeView>().ok());
        let vim = self.vim.read(cx);
        let vim_mode = view
            .as_ref()
            .filter(|v| vim::takes_keys(v.read(cx)))
            .and_then(|v| vim.mode_for(v.entity_id()))
            .map_or("", |m| m.context_value());
        ctx.set("vim.active", ContextValue::Bool(vim.enabled()));
        ctx.set("vim.mode", ContextValue::Str(vim_mode.into()));
        let lang_id = match view {
            Some(view) => view.read(cx).language_id().unwrap_or_default().to_string(),
            None => path
                .and_then(|p| {
                    let rel = p.strip_prefix(&self.root).unwrap_or(p);
                    self.settings
                        .read(cx)
                        .languages(cx)
                        .language_for_file(rel, "")
                })
                .map(|lang| lang.id.to_string())
                .unwrap_or_default(),
        };
        ctx.set("resourceExtname", ContextValue::Str(extname));
        ctx.set("editorLangId", ContextValue::Str(lang_id));
        ctx
    }

    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    /// The sidebar's view: always the Explorer.
    pub fn active_sidebar(&self) -> SidebarView {
        SidebarView::Explorer
    }

    pub fn sidebar_width(&self) -> Pixels {
        self.sidebar_width
    }

    /// Sets the sidebar width, clamped to 160–600 px.
    pub fn set_sidebar_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        self.sidebar_width = width.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
        self.layout_changed(cx);
        cx.notify();
    }

    /// Shows or hides the sidebar. Hiding it cancels a name being typed in
    /// the Explorer, closes its menu and moves focus out of it.
    pub fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_visible {
            let focused = self.sidebar_focus.contains_focused(window, cx);
            self.explorer
                .clone()
                .update(cx, |explorer, cx| explorer.dismiss_transient(window, cx));
            if focused {
                self.focus_editor_area(window, cx);
            }
        }
        self.sidebar_visible = !self.sidebar_visible;
        self.layout_changed(cx);
        cx.notify();
    }

    /// Shows the sidebar with `view` active and focuses it.
    pub fn show_sidebar_view(
        &mut self,
        view: SidebarView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SidebarView::Explorer = view;
        self.sidebar_visible = true;
        self.layout_changed(cx);
        self.focus_sidebar(window, cx);
        cx.notify();
    }

    /// Status-bar Explorer toggle click: hides the shown sidebar, or shows
    /// the Explorer. Runs the same commands as the keybindings.
    pub(crate) fn explorer_toggle_clicked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let command = if self.sidebar_visible {
            "workbench.action.toggleSidebarVisibility"
        } else {
            "workbench.view.explorer"
        };
        self.run_command(command, None, window, cx);
    }

    /// Where the panel is, its sizes and whether it shows.
    pub fn panel_state(&self) -> &PanelState {
        &self.panel
    }

    /// The panel's terminals.
    pub fn terminal_panel(&self) -> &Entity<TerminalPanel> {
        &self.terminal_panel
    }

    fn panel_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.terminal_panel
            .read(cx)
            .focus_handle()
            .contains_focused(window, cx)
    }

    /// `workbench.action.togglePanel`: hides the shown panel, or shows it.
    pub fn toggle_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel.visible {
            self.hide_panel(window, cx);
        } else {
            self.show_panel(window, cx);
        }
    }

    /// Shows the panel and focuses it, opening a terminal when it has none.
    pub fn show_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_panel.read(cx).tab_count() == 0 {
            self.open_terminal(None, window, cx);
        }
        self.panel.visible = true;
        let focus = self.terminal_panel.read(cx).focus_target(cx);
        window.focus(&focus, cx);
        self.layout_changed(cx);
        cx.notify();
    }

    /// Hides the panel, keeping its terminals running. Focus in it goes
    /// back to the editor area.
    pub fn hide_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.panel.visible {
            return;
        }
        if self.panel_has_focus(window, cx) {
            self.focus_editor_area(window, cx);
        }
        self.panel.visible = false;
        self.layout_changed(cx);
        cx.notify();
    }

    /// `workbench.action.toggleMaximizedPanel`: the panel takes the editor
    /// area's place, or goes back to its size. A hidden panel shows
    /// maximized.
    pub fn toggle_maximized_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel.visible {
            self.panel.maximized = !self.panel.maximized;
            if self.panel.maximized || self.panel_has_focus(window, cx) {
                let focus = self.terminal_panel.read(cx).focus_target(cx);
                window.focus(&focus, cx);
            }
        } else {
            self.panel.maximized = true;
            self.show_panel(window, cx);
        }
        self.layout_changed(cx);
        cx.notify();
    }

    /// Moves the panel under the editor area or right of it.
    pub fn set_panel_position(&mut self, position: PanelPosition, cx: &mut Context<Self>) {
        if self.panel.position != position {
            self.panel.position = position;
            self.layout_changed(cx);
            cx.notify();
        }
    }

    /// The ＋ button and `workbench.action.terminal.new`: a new terminal,
    /// shown and focused.
    pub(crate) fn new_terminal_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_terminal(None, window, cx);
        self.show_panel(window, cx);
    }

    /// Explorer: Open in Integrated Terminal: a new terminal in `dir`,
    /// shown and focused.
    pub(crate) fn new_terminal_tab_in(
        &mut self,
        dir: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_terminal(Some(dir), window, cx);
        self.show_panel(window, cx);
    }

    /// Starts a terminal in `cwd` (else as `terminal.integrated.cwd` says,
    /// from the workspace root) and adds its tab; a terminal that cannot
    /// start gets a tab showing why.
    fn open_terminal(
        &mut self,
        cwd: Option<&Path>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        // Never asks for trust (spec §6.3): Restricted Mode only drops the
        // Workspace layer of the shell settings.
        let restricted = self.is_restricted(cx);
        let settings = self.terminal_panel.read(cx).settings().clone();
        match spawn_view(
            &self.terminal_spawner,
            &self.root,
            cwd,
            restricted,
            &settings,
            window,
            cx,
        ) {
            Ok(view) => self
                .terminal_panel
                .update(cx, |panel, cx| panel.add_tab(Some(view), cx)),
            Err(error) => {
                tracing::warn!("{error}");
                self.terminal_panel
                    .update(cx, |panel, cx| panel.add_failed_tab(error, cx))
            }
        }
    }

    /// Starts the terminal of a panel restored visible, now that the
    /// folder's trust is known. Nothing when the panel was hidden or got a
    /// terminal meanwhile. With the focus in the panel (restored
    /// maximized), the terminal takes it.
    pub(crate) fn open_restored_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.restore_terminal)
            || !self.panel.visible
            || self.terminal_panel.read(cx).tab_count() > 0
        {
            return;
        }
        self.open_terminal(None, window, cx);
        if self.panel_has_focus(window, cx) {
            let focus = self.terminal_panel.read(cx).focus_target(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// The shown tab's terminal, if it is running.
    pub fn active_terminal_view(&self, cx: &App) -> Option<Entity<TerminalView>> {
        let panel = self.terminal_panel.read(cx);
        panel.tabs().get(panel.active()?)?.view.clone()
    }

    /// The focus is in the shown panel (`terminalFocus`).
    fn terminal_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.panel.visible && self.panel_has_focus(window, cx)
    }

    /// `workbench.action.terminal.toggleTerminal` (⌃`, spec §6.1): a hidden
    /// panel shows with its terminal focused (one is made when there is
    /// none); a shown one takes the focus; with the focus in it, it hides
    /// and the focus goes back to the editor area.
    pub fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_has_focus(window, cx) {
            self.hide_panel(window, cx);
        } else {
            self.show_panel(window, cx);
        }
    }

    /// `workbench.action.terminal.kill`: closes the shown tab, dropping its
    /// terminal (which hangs up its shell). The focus moves to the tab
    /// shown next.
    pub fn kill_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.terminal_panel.read(cx);
        let Some(id) = panel.active().map(|index| panel.tabs()[index].id) else {
            return;
        };
        self.close_terminal_tab(id, window, cx);
    }

    /// Closes tab `id`. With the focus in the panel, the tab shown next
    /// takes it, or the editor area when none is left: the closed view's
    /// focus handle goes with it.
    fn close_terminal_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let had_focus = self.panel_has_focus(window, cx);
        let panel = self.terminal_panel.clone();
        panel.update(cx, |panel, cx| panel.close_tab(id, cx));
        if !had_focus {
            return;
        }
        if self.panel.visible && panel.read(cx).tab_count() > 0 {
            let focus = panel.read(cx).focus_target(cx);
            window.focus(&focus, cx);
        } else if panel.read(cx).tab_count() == 0 {
            self.focus_editor_area(window, cx);
        }
    }

    /// `focusNext` / `focusPrevious`: shows the tab `delta` places away,
    /// wrapping around, and focuses it.
    pub fn focus_terminal_by(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.terminal_panel.clone();
        let count = panel.read(cx).tab_count();
        let Some(active) = panel.read(cx).active() else {
            return;
        };
        let next = (active as isize + delta).rem_euclid(count as isize) as usize;
        panel.update(cx, |panel, cx| panel.activate(next, cx));
        self.show_panel(window, cx);
    }

    /// Replaces what starts new terminals; see [`TerminalSpawner`].
    pub fn set_terminal_spawner(&mut self, spawner: TerminalSpawner) {
        self.terminal_spawner = spawner;
    }

    fn shown_editor(&self, cx: &App) -> (usize, Option<TabId>) {
        let model = self.editor_groups.read(cx).model();
        (model.active_group(), model.active_tab().map(|t| t.id))
    }

    /// A file opened, or another tab or group became active: a maximized
    /// panel goes back to its size, so the editor it shows is seen.
    fn editor_groups_changed(&mut self, cx: &mut Context<Self>) {
        let shown = self.shown_editor(cx);
        if std::mem::replace(&mut self.shown_editor, shown) == shown {
            return;
        }
        if self.panel.visible && self.panel.maximized && shown.1.is_some() {
            self.panel.maximized = false;
            self.layout_changed(cx);
            cx.notify();
        }
    }

    fn on_terminal_panel_event(
        &mut self,
        _: &Entity<TerminalPanel>,
        event: &PanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PanelEvent::Empty => {
                let hide = self
                    .settings
                    .read(cx)
                    .get::<bool>("terminal.integrated.hideOnLastClosed", None, cx)
                    .unwrap_or(true);
                if hide {
                    self.hide_panel(window, cx);
                }
            }
            PanelEvent::OpenLink(target) => self.open_terminal_link(target, window, cx),
            PanelEvent::ExitedCleanly(id) => self.close_terminal_tab(*id, window, cx),
        }
    }

    /// The modifiers changed: the shown terminal looks for a link under
    /// the mouse when ⌘ goes down and hides it when ⌘ goes up.
    fn terminal_modifiers_changed(
        &mut self,
        modifiers: &gpui::Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.active_terminal_view(cx) else {
            return;
        };
        let pos = window.mouse_position();
        let shown = self.panel.visible;
        view.update(cx, |view, cx| {
            view.link_modifiers_changed(modifiers, pos, shown, cx)
        });
    }

    /// A terminal link was ⌘+clicked (spec §6.7): a file opens in the
    /// editor at its line and column, as Quick Open does; a folder in the
    /// workspace is revealed in the Explorer; one outside it does nothing.
    fn open_terminal_link(
        &mut self,
        target: &LinkTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            LinkTarget::File { path, line, col } => {
                // The file may already be the active editor, which leaves
                // the editor groups unchanged: restore the panel here.
                if self.panel.visible && self.panel.maximized {
                    self.panel.maximized = false;
                    self.layout_changed(cx);
                    cx.notify();
                }
                // Terminal positions are 1-based and count characters (as
                // rustc, tsc and gcc print them); `Point` is 0-based bytes,
                // so the column is converted once the text is loaded.
                let row = line.unwrap_or(1).saturating_sub(1);
                let col = col.unwrap_or(1).saturating_sub(1);
                let path = self.absolute_path(&Arc::from(path.as_path()));
                // History records the row; the column is approximate until
                // the text is known.
                self.record_nav(
                    &NavEntry {
                        path: path.clone(),
                        point: Point { row, column: col },
                        scroll_top_row: 0,
                    },
                    cx,
                );
                let to = GoTo::Chars { row, col };
                self.open_path_at_target(path, OpenMode::Preview, to, cx);
            }
            LinkTarget::Dir(dir) => {
                if dir.strip_prefix(&self.root).is_err() {
                    return;
                }
                self.show_sidebar_view(SidebarView::Explorer, window, cx);
                self.explorer
                    .update(cx, |explorer, cx| explorer.reveal_path(dir, cx));
            }
            LinkTarget::Url(_) => {}
        }
    }

    /// The sash dragged to `position` in `row`, the bounds of the sidebar,
    /// editor area and panel together.
    fn drag_panel_sash(
        &mut self,
        position: gpui::Point<Pixels>,
        row: gpui::Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        match self.panel.position {
            PanelPosition::Bottom => {
                let size = row.bottom() - position.y;
                self.panel.bottom_height =
                    panel::clamp_size(PanelPosition::Bottom, size, row.size.height);
            }
            PanelPosition::Right => {
                let sidebar = if self.sidebar_visible {
                    self.sidebar_width
                } else {
                    px(0.)
                };
                let size = row.right() - position.x;
                self.panel.right_width =
                    panel::clamp_size(PanelPosition::Right, size, row.size.width - sidebar);
            }
        }
        self.layout_changed(cx);
        cx.notify();
    }

    pub fn focus_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.sidebar_focus, cx);
        cx.notify();
    }

    /// Closes every modal overlay (Quick Open, the locations picker, the
    /// theme picker, reverting its preview, the Open Recent picker, the
    /// branch picker and the trust modal). Each `show_*` calls it first, so at most one is ever
    /// shown and one Esc never has to close two.
    pub(crate) fn close_modal_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_quick_open(window, cx);
        self.close_locations_picker(window, cx);
        self.close_theme_picker(true, window, cx);
        self.close_recent_picker(window, cx);
        self.close_branch_picker(window, cx);
        self.close_trust_modal(window, cx);
    }

    /// Whether Quick Open, the theme picker, the Open Recent picker, the
    /// branch picker or the trust modal is shown: a late go-to answer then does not open the
    /// locations picker over it.
    pub(crate) fn other_modal_overlay_shown(&self) -> bool {
        self.quick_open.is_some()
            || self.theme_picker.is_some()
            || self.recent_picker.is_some()
            || self.branch_picker.is_some()
            || self.trust_modal().is_some()
    }

    /// Focuses the active tab's code view or Markdown preview, or the
    /// editor area when the active tab shows none (yet: a new tab's view
    /// is made while drawing, and [`Workbench::focus_active_view`] focuses
    /// it then).
    pub fn focus_editor_area(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.active_focus_target(cx) {
            Some((_, handle, false)) => window.focus(&handle, cx),
            _ => window.focus(&self.editor_focus, cx),
        }
        cx.notify();
    }

    /// The active tab's code view or Markdown preview, once made: its
    /// entity, focus handle and whether it is passive (never focused).
    fn active_focus_target(&self, cx: &App) -> Option<(EntityId, FocusHandle, bool)> {
        let body = self.editor_groups.read(cx).active_body()?;
        if let Ok(view) = body.clone().downcast::<CodeView>() {
            let v = view.read(cx);
            return Some((view.entity_id(), v.focus_handle(cx), v.is_passive()));
        }
        let body = match body.downcast::<DiffView>() {
            // The list takes the keys.
            Ok(view) => {
                return Some((
                    view.entity_id(),
                    view.read(cx).list_focus_handle().clone(),
                    false,
                ));
            }
            Err(body) => body,
        };
        let preview = body.downcast::<MarkdownPreview>().ok()?;
        Some((
            preview.entity_id(),
            preview.read(cx).focus_handle(cx),
            false,
        ))
    }

    /// Like Zed, a code view or Markdown preview that becomes active (a
    /// file opened or a tab activated) takes the focus, unless the focus
    /// is somewhere else than the editor area itself or the previously
    /// active view (the sidebar, a picker, the Search tab).
    fn focus_active_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, handle, passive)) = self.active_focus_target(cx) else {
            return;
        };
        if self.active_view.as_ref().is_some_and(|(p, _)| *p == id) {
            return;
        }
        let previous = self
            .active_view
            .replace((id, handle.downgrade()))
            .and_then(|(_, h)| h.upgrade());
        if passive {
            return;
        }
        let focus_free = window.focused(cx).is_none()
            || self.editor_focus.is_focused(window)
            || previous.is_some_and(|p| p.contains_focused(window, cx));
        if focus_free {
            window.focus(&handle, cx);
        }
    }

    /// Puts `view` in the sidebar slot for `kind`; shown while `kind` is active.
    pub fn set_sidebar_view(&mut self, kind: SidebarView, view: AnyView, cx: &mut Context<Self>) {
        match kind {
            SidebarView::Explorer => self.explorer_view = Some(view),
        }
        cx.notify();
    }

    /// Replaces the editor area placeholder.
    pub fn set_editor_area(&mut self, view: AnyView, cx: &mut Context<Self>) {
        self.editor_area = Some(view);
        cx.notify();
    }

    /// The view in the sidebar slot for `kind`, if one was set.
    pub fn sidebar_view(&self, kind: SidebarView) -> Option<&AnyView> {
        match kind {
            SidebarView::Explorer => self.explorer_view.as_ref(),
        }
    }

    /// The view in the editor area, if one replaced the placeholder.
    pub fn editor_area(&self) -> Option<&AnyView> {
        self.editor_area.as_ref()
    }

    /// The status bar items on `side`, in order.
    pub fn status_items(&self, side: StatusBarSide) -> &[AnyView] {
        match side {
            StatusBarSide::Left => &self.status_left,
            StatusBarSide::Right => &self.status_right,
        }
    }

    /// Appends an item to one end of the status bar.
    pub fn add_status_item(&mut self, side: StatusBarSide, view: AnyView, cx: &mut Context<Self>) {
        match side {
            StatusBarSide::Left => self.status_left.push(view),
            StatusBarSide::Right => self.status_right.push(view),
        }
        cx.notify();
    }

    /// Vim navigation: its settings, per-view modes and mode indicator.
    pub fn vim(&self) -> &Entity<VimLayer> {
        &self.vim
    }

    /// The chord typed so far, e.g. `"⌘K"`, while waiting for its next key.
    pub fn pending_chord_label(&self) -> Option<String> {
        if self.pending_chord.is_empty() {
            return None;
        }
        Some(
            self.pending_chord
                .iter()
                .map(Keystroke::label)
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    /// Resolves the key against the user keymap (GPUI's own keymap is not
    /// used). Runs in the capture phase so bindings win over child views;
    /// unbound keys, and keys bound to commands nobody registered,
    /// propagate to them.
    ///
    /// Because this runs before any text input sees the key, a binding for
    /// a plain character (no cmd/ctrl/alt) must carry a `when` clause, or it
    /// swallows that character in every text input.
    fn dispatch_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = keys::to_keystroke(ev);
        self.note_results_focus(window, cx);
        let ctx = self.context_keys(window, cx);
        let chord_pending = !self.pending_chord.is_empty();
        // Typing hides the hover, also when vim or a command takes the
        // key before the editor sees it.
        let hover = self
            .active_code_view(cx)
            .filter(|v| v.read(cx).hover().is_visible());
        let hide_hover = |cx: &mut Context<Self>| {
            if let Some(view) = &hover {
                view.update(cx, |v, cx| v.hover_mut().hide(cx));
            }
        };
        // Vim never takes the terminal's keys (spec §6.4).
        let terminal_focus = self.terminal_has_focus(window, cx);
        if !terminal_focus
            && vim::dispatch(
                &self.vim,
                &self.editor_groups,
                ev,
                &ctx,
                chord_pending,
                window,
                cx,
            )
        {
            hide_hover(cx);
            cx.stop_propagation();
            // `gd`, `]d`, `ctrl-o`, `:`...: after the hover is hidden, so
            // `gh` can show it again.
            self.run_vim_requests(window, cx);
            return;
        }
        let had_pending = !self.pending_chord.is_empty();
        let mut chord = std::mem::take(&mut self.pending_chord);
        chord.push(stroke.clone());
        let keymap = &cx.global::<AppState>().keymap;
        let mut resolution = keymap.resolve(&chord, &ctx);
        if had_pending && resolution == Resolution::NoMatch {
            // VS Code: a key that breaks a chord is tried again on its own.
            chord = vec![stroke];
            resolution = keymap.resolve(&chord, &ctx);
        }
        if terminal_focus {
            let completes_chord = chord.len() > 1;
            self.dispatch_terminal_key(ev, chord, completes_chord, resolution, window, cx);
            if had_pending || !self.pending_chord.is_empty() {
                cx.notify();
            }
            return;
        }
        match resolution {
            // A held Delete key would delete one entry after another.
            Resolution::Command { command, .. }
                if ev.is_held && matches!(command.as_str(), "moveFileToTrash" | "deleteFile") =>
            {
                cx.stop_propagation();
            }
            Resolution::Command { command, args } => {
                if command != SHOW_HOVER {
                    hide_hover(cx);
                }
                if self.run_command(&command, args.as_ref(), window, cx) {
                    cx.stop_propagation();
                } else {
                    tracing::debug!(command, "key bound to an unregistered command");
                }
            }
            Resolution::Pending => {
                cx.stop_propagation();
                self.pending_chord = chord;
            }
            Resolution::NoMatch => {}
        }
        if had_pending || !self.pending_chord.is_empty() {
            cx.notify();
        }
    }
}

impl Workbench {
    /// A key pressed while the terminal has focus (spec §6.4): a command
    /// in the skip-shell list runs, and so does any command a pending chord
    /// completes (as VS Code); on macOS a key with ⌘ runs its command
    /// or is dropped, since a shell cannot receive ⌘; a chord's first key
    /// waits for the next one while `allowChords` is on; anything else is
    /// encoded for the shell.
    fn dispatch_terminal_key(
        &mut self,
        ev: &KeyDownEvent,
        chord: Vec<Keystroke>,
        completes_chord: bool,
        resolution: Resolution,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cmd = cfg!(target_os = "macos") && ev.keystroke.modifiers.platform;
        match resolution {
            Resolution::Command { command, args }
                if completes_chord
                    || cmd
                    || skips_shell(&command, self.settings.read(cx).view(cx)) =>
            {
                cx.stop_propagation();
                if !self.run_command(&command, args.as_ref(), window, cx) {
                    tracing::debug!(command, "key bound to an unregistered command");
                }
            }
            Resolution::Pending if allow_chords(self.settings.read(cx).view(cx)) => {
                cx.stop_propagation();
                self.pending_chord = chord;
            }
            _ if cmd => cx.stop_propagation(),
            _ => self.send_key_to_terminal(ev, window, cx),
        }
    }

    /// Encodes `ev` for the active terminal's mode and sends it. Text the
    /// view's input handler will receive (typed characters, IME input) is
    /// left to it, so it reaches the shell once.
    fn send_key_to_terminal(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.active_terminal_view(cx) else {
            cx.stop_propagation();
            return;
        };
        // A tab kept after its process failed closes on the next key,
        // which goes nowhere (spec §6.1).
        if view.read(cx).exited().is_some() {
            cx.stop_propagation();
            self.kill_terminal(window, cx);
            return;
        }
        self.terminal_panel
            .update(cx, |panel, cx| panel.key_typed(cx));
        // The multi-line paste warning takes enter and escape, and keeps
        // other keys from the shell while it shows.
        let key = ev.keystroke.key.as_str();
        if view.update(cx, |view, cx| view.paste_warning_key(key, cx)) {
            cx.stop_propagation();
            return;
        }
        let meta = option_is_meta(self.settings.read(cx).view(cx));
        if view.read(cx).input_handler_takes(ev, meta) {
            return;
        }
        cx.stop_propagation();
        let mode = view.read(cx).terminal().mode();
        if let Some(bytes) = key_bytes(ev, mode, meta) {
            view.update(cx, |view, cx| view.write_input(&bytes, cx));
        }
    }
}

/// VS Code's `resourceExtname`: `".rs"` for `main.rs`, and the whole name
/// for a dotfile such as `.gitignore`.
fn extname(path: &Path) -> String {
    match path.extension() {
        Some(ext) => format!(".{}", ext.to_string_lossy()),
        None => path
            .file_name()
            .map(|n| n.to_string_lossy())
            .filter(|n| n.starts_with('.'))
            .map(|n| n.into_owned())
            .unwrap_or_default(),
    }
}

fn keymap_generation(cx: &App) -> u64 {
    cx.try_global::<ConfigState>()
        .map_or(0, ConfigState::keymap_generation)
}

/// The default `files.scanDepth`, for a value that is not an integer.
const DEFAULT_SCAN_DEPTH: i64 = 5;

/// The options of the folder scan: `files.exclude`, and `files.scanDepth`
/// (`0` or less scans everything). Lookups above the root stop at `$HOME`,
/// as search and git discovery do.
fn scan_options(settings: SettingsView<'_>) -> ScanOptions {
    let depth = settings
        .get::<i64>("files.scanDepth", None)
        .unwrap_or(DEFAULT_SCAN_DEPTH);
    ScanOptions {
        exclude: enabled_globs(settings, "files.exclude"),
        scan_depth: usize::try_from(depth).ok().filter(|d| *d > 0),
        home: std::env::home_dir(),
    }
}

/// The globs of a glob → bool setting such as `files.exclude` that are
/// switched on.
pub(crate) fn enabled_globs(settings: SettingsView<'_>, key: &str) -> Vec<String> {
    settings
        .get::<serde_json::Map<String, Value>>(key, None)
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, on)| *on == Value::Bool(true))
        .map(|(glob, _)| glob)
        .collect()
}

/// The entries a watcher batch shows respelled: a case-only rename on a
/// case-insensitive file system, as root-relative `(old, new)` pairs, `old`
/// Removed and `new` Added, the names equal but for case and the very same
/// entry on disk. For a folder only its own pair is listed, not those of
/// the paths below it.
fn respelled(root: &Path, changes: &[Change]) -> Vec<(Arc<Path>, Arc<Path>)> {
    let lower = |p: &Path| p.to_string_lossy().to_lowercase();
    let mut added: HashMap<String, Vec<&Arc<Path>>> = HashMap::new();
    for change in changes.iter().filter(|c| c.kind == ChangeKind::Added) {
        added
            .entry(lower(&change.path))
            .or_default()
            .push(&change.path);
    }
    if added.is_empty() {
        return Vec::new();
    }
    let mut removed: Vec<&Arc<Path>> = changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Removed)
        .map(|c| &c.path)
        .collect();
    removed.sort_by_key(|p| p.components().count());
    let mut pairs: Vec<(Arc<Path>, Arc<Path>)> = Vec::new();
    for old in removed {
        if pairs.iter().any(|(o, _)| old.starts_with(o)) {
            continue;
        }
        let Some(candidates) = added.get(&lower(old)) else {
            continue;
        };
        if let Some(new) = candidates.iter().find(|new| {
            **new != old && lucerna_project::fs_ops::same_entry(&root.join(old), &root.join(new))
        }) {
            pairs.push((old.clone(), (*new).clone()));
        }
    }
    pairs
}

/// The single consumer of `worktree.events()`. `recv` blocks, so a
/// dedicated thread receives and the foreground task delivers each event
/// to [`Workbench::on_worktree_event`]. Both stop once the worktree or the
/// workbench is gone.
///
/// Not started on GPUI's deterministic test scheduler, which rejects
/// wake-ups from other threads: tests deliver events by calling
/// `on_worktree_event` themselves.
fn pump_worktree_events(worktree: &Worktree, cx: &mut Context<Workbench>) -> Task<()> {
    if cx.background_executor().is_test() {
        return Task::ready(());
    }
    let events = worktree.events();
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    let spawned = try_spawn_guarded("lucerna-worktree-events", move || {
        while let Ok(ev) = events.recv() {
            if tx.unbounded_send(ev).is_err() {
                break;
            }
        }
    });
    if let Err(err) = spawned {
        tracing::error!(%err, "could not start the worktree event pump");
    }
    cx.spawn(async move |workbench, cx| {
        while let Some(ev) = rx.next().await {
            if workbench
                .update(cx, |wb, cx| wb.on_worktree_event(&ev, cx))
                .is_err()
            {
                break;
            }
        }
    })
}

/// Starts the [`OpenFileWatcher`] and delivers its batches to
/// [`Workbench::on_open_file_changes`]. Not started on GPUI's test
/// scheduler (see [`pump_worktree_events`]): tests deliver batches
/// themselves.
fn pump_open_file_changes(cx: &mut Context<Workbench>) -> (Option<OpenFileWatcher>, Task<()>) {
    if cx.background_executor().is_test() {
        return (None, Task::ready(()));
    }
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    let watcher = OpenFileWatcher::start(move |batch| {
        let _ = tx.unbounded_send(batch);
    });
    match watcher {
        Ok(watcher) => {
            let task = cx.spawn(async move |workbench, cx| {
                while let Some(batch) = rx.next().await {
                    if workbench
                        .update(cx, |wb, cx| wb.on_open_file_changes(&batch, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            });
            (Some(watcher), task)
        }
        Err(err) => {
            tracing::error!(%err, "could not watch open files");
            (None, Task::ready(()))
        }
    }
}

/// An Explorer file operation counted as running
/// ([`Workbench::begin_file_operation`]) until dropped.
#[must_use]
pub struct FileOperation(Rc<Cell<usize>>);

impl Drop for FileOperation {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

/// VS Code's `confirmOnExit` prompt for `count` running terminals; the
/// answer is 0 for Terminate.
fn prompt_terminate_terminals(
    count: usize,
    window: &mut Window,
    cx: &mut App,
) -> futures::channel::oneshot::Receiver<usize> {
    let message = if count == 1 {
        "Do you want to terminate the active terminal session?".to_string()
    } else {
        format!("Do you want to terminate the {count} active terminal sessions?")
    };
    window.prompt(
        gpui::PromptLevel::Warning,
        &message,
        None,
        &["Terminate", "Cancel"],
        cx,
    )
}

/// Asks whether to quit (or close the window) while an Explorer file
/// operation runs: Quit (0) or Cancel.
fn prompt_file_operation_running(
    window: &mut Window,
    cx: &mut App,
) -> futures::channel::oneshot::Receiver<usize> {
    window.prompt(
        gpui::PromptLevel::Warning,
        "A file operation is still running",
        None,
        &["Quit", "Cancel"],
        cx,
    )
}

/// Quits the app once every workbench window agreed to
/// ([`Workbench::confirm_app_quit`], one window after another, the most
/// recently used first; spec M11). With hot exit on, a window agrees
/// without asking once its backups are written. Cancel in any window
/// stops the quit.
/// Before quitting, every window is checked again: one opened meanwhile,
/// or one edited since it agreed (beyond what it agreed to lose), is asked
/// too, so nothing unsaved is lost without a question. Then the window
/// list is written (spec §6.6). Saves still writing are waited for, up to
/// [`QUIT_SAVE_TIMEOUT`]; past it their temp files are deleted and the app
/// quits anyway. Quitting runs the `on_app_quit` hooks, which save the
/// layout.
pub(crate) fn quit(cx: &mut App) {
    cx.spawn(async move |cx| {
        // What each window agreed to lose: its dirty files once it agreed.
        let mut agreed: HashMap<gpui::WindowId, Vec<Arc<Path>>> = HashMap::new();
        // Asking a window brings it forward: the saved window list keeps
        // the order from before.
        let used = cx.update(|cx| crate::router::activity(cx));
        loop {
            let ask = cx.update(|cx| windows_to_ask(&agreed, cx));
            if ask.is_empty() {
                break;
            }
            for handle in ask {
                let Ok(confirm) =
                    handle.update(cx, |wb, window, cx| wb.confirm_app_quit(window, cx))
                else {
                    // The window closed meanwhile.
                    continue;
                };
                if !confirm.await {
                    cx.update(cancel_quitting);
                    return;
                }
                let dirty = handle
                    .read_with(cx, |wb, cx| wb.documents.read(cx).dirty_paths())
                    .unwrap_or_default();
                agreed.insert(handle.window_id(), dirty);
            }
        }
        let saves = cx.update(saves_done);
        let timeout = cx.background_executor().timer(QUIT_SAVE_TIMEOUT);
        if let Either::Right(_) = futures::future::select(saves, timeout).await {
            let removed = cx.update(|cx| save::save_temps(cx).remove_all());
            tracing::warn!(
                removed,
                "saves were still writing after {QUIT_SAVE_TIMEOUT:?}; quitting without them"
            );
        }
        cx.update(|cx| {
            crate::router::restore_activity(&used, cx);
            crate::session::save_now(cx);
            crate::app::quit_now(cx);
        });
    })
    .detach();
}

/// How long [`quit`] waits for the saves still writing.
const QUIT_SAVE_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolves once the saves asked for so far in every workbench window have
/// ended.
fn saves_done(cx: &mut App) -> Pin<Box<dyn Future<Output = ()>>> {
    let waits: Vec<_> = cx
        .windows()
        .into_iter()
        .filter_map(|window| {
            WindowHandle::<Workbench>::new(window.window_id())
                .read_with(cx, |wb, cx| wb.documents.read(cx).saves_done())
                .ok()
        })
        .collect();
    Box::pin(futures::future::join_all(waits).map(|_| ()))
}

/// A cancelled [`quit`]: the windows that flushed their backups for it
/// delete them again when a document closes with Don't Save.
fn cancel_quitting(cx: &mut App) {
    for window in cx.windows() {
        let handle = WindowHandle::<Workbench>::new(window.window_id());
        let Ok(Some(hot_exit)) = handle.read_with(cx, |wb, _| wb.hot_exit.clone()) else {
            continue;
        };
        hot_exit.update(cx, |h, _| h.cancel_quitting());
    }
}

/// The workbench windows [`quit`] must still ask, the most recently used
/// first: those that have not agreed, and those with a dirty file they did
/// not agree to lose.
fn windows_to_ask(
    agreed: &HashMap<gpui::WindowId, Vec<Arc<Path>>>,
    cx: &mut App,
) -> Vec<WindowHandle<Workbench>> {
    let mut windows: Vec<gpui::AnyWindowHandle> = crate::router::windows(cx)
        .into_iter()
        .map(|(handle, _)| handle)
        .collect();
    // Windows the router did not open (tests build some directly) last.
    for window in cx.windows() {
        if !windows.contains(&window) {
            windows.push(window);
        }
    }
    // Not `downcast`: a Welcome window that became a workbench keeps the
    // handle type it opened with. Reading a window of another root fails.
    windows
        .into_iter()
        .map(|w| WindowHandle::<Workbench>::new(w.window_id()))
        .filter(|handle| {
            let Ok(dirty) = handle.read_with(cx, |wb, cx| wb.documents.read(cx).dirty_paths())
            else {
                // Closed, or the Welcome screen.
                return false;
            };
            match agreed.get(&handle.window_id()) {
                None => true,
                Some(lost) => dirty.iter().any(|path| !lost.contains(path)),
            }
        })
        .collect()
}

/// The commands every workbench registers, as [`Workbench::new`] builds
/// them; the menu bar reads their IDs without a window.
pub(crate) fn default_registry() -> CommandRegistry {
    let mut commands = CommandRegistry::default();
    register_shell_commands(&mut commands);
    panel::register_commands(&mut commands);
    crate::terminal::commands::register_commands(&mut commands);
    editor_groups::register_commands(&mut commands);
    code_view::register_commands(&mut commands);
    quick_open::register_commands(&mut commands);
    find_widget::register_commands(&mut commands);
    search_view::register_commands(&mut commands);
    references_view::register_commands(&mut commands);
    config_files::register_commands(&mut commands);
    theme_picker::register_commands(&mut commands);
    open_commands::register_commands(&mut commands);
    trust::register_commands(&mut commands);
    crate::git::register_commands(&mut commands);
    lsp_status::register_commands(&mut commands);
    diagnostics_status::register_commands(&mut commands);
    crate::hover::register_commands(&mut commands);
    crate::goto::register_commands(&mut commands);
    crate::extensions::register_commands(&mut commands);
    crate::symbols::register_commands(&mut commands);
    crate::editing::register_commands(&mut commands);
    crate::save::register_commands(&mut commands);
    crate::about::register_commands(&mut commands);
    markdown_preview::register_commands(&mut commands);
    crate::explorer::register_commands(&mut commands);
    onboarding::register_commands(&mut commands);
    commands
}

fn register_shell_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.files.newUntitledFile",
        "File: New Untitled Text File",
        |wb, _, window, cx| wb.new_untitled_file(window, cx),
    );
    commands.register("workbench.action.quit", "File: Quit", |_, _, _, cx| {
        quit(cx)
    });
    commands.register(
        "workbench.action.toggleSidebarVisibility",
        "View: Toggle Primary Side Bar Visibility",
        |wb, _, window, cx| wb.toggle_sidebar(window, cx),
    );
    commands.register(
        "workbench.view.explorer",
        "View: Show Explorer",
        |wb, _, window, cx| wb.show_sidebar_view(SidebarView::Explorer, window, cx),
    );
    commands.register(
        "filesExplorer.openContextMenu",
        "Explorer: Open Context Menu",
        |wb, _, window, cx| {
            if !crate::explorer::editing(wb, cx) {
                crate::explorer::show_explorer(wb, window, cx);
                wb.explorer.clone().update(cx, |explorer, cx| {
                    explorer.open_context_menu(None, window, cx)
                })
            }
        },
    );
}

impl Focusable for Workbench {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focus_active_view(window, cx);
        // Tabs closed since the last frame have dropped their views: their
        // documents close (and leave their language servers) now.
        let documents = self.documents.downgrade();
        cx.defer(move |cx| {
            documents
                .update(cx, |documents, cx| documents.collect_closed(cx))
                .ok();
        });
        let theme = cx.global::<AppState>().theme.clone();
        let ui = &theme.ui;
        let root_name = self.root_name();
        let model = self.editor_groups.read(cx).model();
        let extensions_open = model.extensions_tab().is_some();
        let search_active = model
            .active_tab()
            .is_some_and(|t| t.item == TabItem::Search);
        let sidebar_slot = self.explorer_view.clone();
        let editor_area = div()
            .id("editor-area")
            .debug_selector(|| "editor-area".into())
            .track_focus(&self.editor_focus)
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(color(ui.editor_background))
            .children(self.editor_area.clone())
            .when(self.editor_area.is_none(), |area| {
                area.justify_center()
                    .items_center()
                    .text_color(color(ui.text_muted))
                    .child("Lucerna")
            });

        // The panel: under the editor area, in its column, or right of it,
        // in the row with the sidebar. Maximized, it takes the editor
        // area's place.
        let shown_panel = self.panel.visible.then_some(&self.panel);
        let panel_at = |position| shown_panel.filter(|p| p.position == position);
        let editor_column = match (panel_at(PanelPosition::Bottom), shown_panel) {
            (Some(state), _) => {
                let panel = panel::render(state, &self.terminal_panel, cx);
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .when(!state.maximized, |column| {
                            column.child(editor_area.min_h(px(EDITOR_MIN_SIZE)))
                        })
                        .child(panel)
                        .into_any_element(),
                )
            }
            (None, Some(state)) if state.maximized => None,
            (None, Some(_)) => Some(editor_area.min_w(px(EDITOR_MIN_SIZE)).into_any_element()),
            (None, None) => Some(editor_area.into_any_element()),
        };
        let right_panel = panel_at(PanelPosition::Right)
            .map(|state| panel::render(state, &self.terminal_panel, cx));

        let status_message = crate::status_message::current(window.window_handle().window_id(), cx);
        div()
            .id("workbench")
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::dispatch_key))
            // At the root, so the terminal hears ⌘ go up and down wherever
            // the focus is (gpui sends modifier changes along the focus path).
            .on_modifiers_changed(cx.listener(|wb, ev: &ModifiersChangedEvent, window, cx| {
                wb.terminal_modifiers_changed(&ev.modifiers, window, cx)
            }))
            .on_drag_move(cx.listener(|wb, ev: &DragMoveEvent<SidebarSash>, _, cx| {
                // The sidebar starts at the window's left edge.
                wb.set_sidebar_width(ev.event.position.x - ev.bounds.left(), cx)
            }))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(color(ui.background))
            .text_color(color(ui.text))
            .font_family(UI_FONT_FAMILY)
            .text_size(px(UI_FONT_SIZE))
            .child(title_bar::render(
                title_bar::TitleBarState {
                    root_name: root_name.clone(),
                    branch: self.git_branch.clone(),
                    switching: self.switching_branch,
                    branch_bounds: self.branch_button_bounds.clone(),
                    restricted: self.shows_restricted_mode(cx),
                },
                ui,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .when(self.sidebar_visible, |row| {
                        row.child(sidebar::render(
                            self.sidebar_width,
                            sidebar_slot,
                            &self.sidebar_focus,
                            ui,
                        ))
                    })
                    .children(editor_column)
                    .children(right_panel)
                    .on_drag_move(cx.listener(|wb, ev: &DragMoveEvent<PanelSash>, _, cx| {
                        wb.drag_panel_sash(ev.event.position, ev.bounds, cx);
                    })),
            )
            .child(status_bar::render(
                panel_toggles::render(
                    self.sidebar_visible,
                    search_active,
                    extensions_open,
                    self.panel.visible,
                    self.extension_badge(cx),
                    cx,
                ),
                &self.status_left,
                status_message,
                self.pending_chord_label(),
                &self.status_right,
                ui,
            ))
            .children(
                self.quick_open
                    .as_ref()
                    .map(|(qo, _)| popover_overlay(qo.clone())),
            )
            .children(
                self.theme_picker()
                    .map(|picker| popover_overlay(picker.clone())),
            )
            .children(
                self.recent_picker()
                    .map(|picker| popover_overlay(picker.clone())),
            )
            .children(
                self.locations_picker()
                    .map(|picker| popover_overlay(picker.clone())),
            )
            .children(self.branch_picker_overlay(window))
            .children(
                self.trust_modal()
                    .map(|modal| popover_overlay(modal.clone())),
            )
            .children(
                self.toast
                    .as_ref()
                    .map(|toast| crate::extensions::toast_element(toast, cx)),
            )
    }
}

/// `explorer.decorations.externalChanges`: whether the Explorer marks files
/// that other programs changed.
fn decorates_external_changes(settings: SettingsView<'_>) -> bool {
    settings
        .get::<bool>("explorer.decorations.externalChanges", None)
        .unwrap_or(true)
}
