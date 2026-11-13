//! The root view: shell layout, view slots and VS Code–style key dispatch.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui::{
    AnyView, App, Context, DragMoveEvent, Entity, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, ParentElement, Pixels, Render, Styled, Subscription, Task, WeakEntity, Window,
    WindowHandle, div, prelude::*, px,
};
use lucerna_project::{Change, ChangeKind, ScanOptions, Worktree, WorktreeEvent};
use lucerna_settings::{Context as WhenContext, ContextValue, Keystroke, Resolution};
use lucerna_text::Point;
use serde_json::Value;

use crate::code_view::{self, CodeView, LargeFileStatus};
use crate::config_files::{self, ConfigErrorItem, ConfigState};
use crate::diagnostics_status::{self, DiagnosticsStatusItem};
use crate::documents::{DocumentEvent, DocumentStatusItem, Documents};
use crate::editor_groups::{
    self, EditorGroups, EditorGroupsView, GroupId, OpenMode, TabId, TabItem,
};
use crate::explorer::{ExplorerView, SelectedKind};
use crate::extensions::{ExtensionService, ExtensionsPage, Toast, installable_ids};
use crate::hover::SHOW_HOVER;
use crate::language_servers::LanguageServers;
use crate::layout::sidebar::SidebarSash;
use crate::layout::{
    SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, StatusBarSide, color,
    panel_toggles, sidebar, status_bar, title_bar,
};
use crate::locations_picker::ShownLocationsPicker;
use crate::lsp_health::LspHealthView;
use crate::lsp_log::LspLogs;
use crate::lsp_status::{self, LspStatusItem};
use crate::nav_history::{NavEntry, NavHistory};
use crate::open_commands::{self, ShownRecentPicker};
use crate::open_files::{OpenFileWatcher, matching_files};
use crate::quick_open::{self, QuickOpen, RecentFiles};
use crate::references_view::{self, ReferencesView};
use crate::save::{self, SaveChoice};
use crate::search_view::{self, SearchView};
use crate::symbols::SymbolSource;
use crate::symbols::breadcrumbs::Breadcrumbs;
use crate::tasks::try_spawn_guarded;
use crate::text_input::TEXT_INPUT_KEY_CONTEXT;
use crate::theme_picker::{self, ShownThemePicker};
use crate::trust::{self, Trust};
use crate::ui::{UI_FONT_FAMILY, UI_FONT_SIZE, popover_overlay};
use crate::vim::{self, VimLayer};
use crate::vim_indicator::ModeIndicator;
use crate::workspace_state::{WorkspaceState, WorkspaceStateStore, absolute_path};
use crate::{AppState, CommandRegistry, find_widget, keys};

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
    /// Absolute paths a Delete is asking about or deleting; another Delete
    /// of them (or of a path inside or around them) is ignored meanwhile.
    pub(crate) deletes_in_flight: Rc<std::cell::RefCell<HashSet<PathBuf>>>,
    /// The options of the scan, from `files.exclude`.
    scan_options: ScanOptions,
    /// Re-themes and rescans when [`AppState`] changes.
    _settings: Subscription,
    /// [`ConfigState::keymap_generation`] as last seen.
    keymap_generation: u64,
    /// [`AppState::languages_generation`] as last seen.
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
    /// The diagnostics counts and message in the status bar.
    pub(crate) diagnostics_status: Entity<DiagnosticsStatusItem>,
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
    git_branch: Option<String>,
    /// The [`ConfigState`] batch last forwarded to the documents.
    config_batch: u64,
    _subscriptions: Vec<Subscription>,
    commands: Rc<CommandRegistry>,
    focus: FocusHandle,
    sidebar_focus: FocusHandle,
    editor_focus: FocusHandle,
    /// The code view last made active, to focus a newly active one when
    /// focus was on the editor area or on that view.
    active_view: Option<WeakEntity<CodeView>>,
    sidebar_visible: bool,
    sidebar_width: Pixels,
    explorer_view: Option<AnyView>,
    editor_groups: Entity<EditorGroupsView>,
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
        let focus = cx.focus_handle();
        let editor_focus = cx.focus_handle();
        window.focus(&editor_focus, cx);
        let commands = default_registry();
        let documents = cx.new(|_| Documents::new(root.clone()));
        let language_servers = {
            let root = root.clone();
            // Servers' `{data_dir}`: the workspace's data dir, else the
            // app's (a temporary folder when there is none).
            let data_dir = store
                .as_ref()
                .map(|store| store.data_dir().to_path_buf())
                .or_else(crate::default_data_dir)
                .unwrap_or_else(|| std::env::temp_dir().join("Lucerna"));
            cx.new(|cx| LanguageServers::new(root, &documents, data_dir, cx))
        };
        let symbols = SymbolSource::new(root.clone(), &documents, &language_servers, cx);
        // Servers end with the window (without waiting for them) or the
        // app (waiting at most 5 s across all windows, then killed; see the
        // `on_app_quit` below).
        cx.on_release(|wb, cx| {
            wb.language_servers
                .update(cx, |servers, _| servers.shutdown_all_in_background())
        })
        .detach();
        let extensions_page = {
            let workbench = cx.weak_entity();
            cx.new(|cx| ExtensionsPage::new(workbench, cx))
        };
        let search_view = {
            let (root, workbench) = (root.clone(), cx.weak_entity());
            let searchable = single_file.is_none();
            cx.new(|cx| SearchView::new(root, workbench, searchable, window, cx))
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
        let workbench = cx.weak_entity();
        let breadcrumbs_target = cx.weak_entity();
        let editor_groups = cx.new(|cx| {
            let mut groups = EditorGroupsView::default();
            let docs = documents.clone();
            let page = extensions_page.clone();
            let search = search_view.clone();
            let references = references_view.clone();
            let health = lsp_health.clone();
            let (logs, servers) = (lsp_logs.clone(), language_servers.clone());
            groups.set_tab_body_factory(
                Rc::new(move |tab, _, cx| {
                    let path = match &tab.item {
                        TabItem::File(path) => path.clone(),
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
                    };
                    let view = docs.update(cx, |docs, cx| docs.open(path, cx));
                    view.update(cx, |v, _| {
                        v.set_hover_sources(workbench.clone(), servers.downgrade())
                    });
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
        let scan_options = ScanOptions {
            exclude: enabled_globs(cx, "files.exclude"),
        };
        let scan_started = std::time::Instant::now();
        let scanning = single_file.is_none();
        let single_file_path = single_file.as_ref().map(|rel| root.join(rel));
        let worktree = match single_file {
            Some(file) => Worktree::without_scan(root.clone(), vec![file]),
            None => Worktree::start(root.clone(), scan_options.clone()),
        };
        let trust = Trust::new(store.as_ref().map(|s| s.data_dir().to_path_buf()));
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
        if let Some(state) = &restored {
            search_view.update(cx, |view, cx| {
                view.restore(state.search.clone(), window, cx)
            });
            let has_tab = editor_groups.read(cx).model().search_tab().is_some();
            if has_tab {
                search_view.update(cx, |view, cx| view.run_search(cx));
            }
        }
        let sidebar_focus = cx.focus_handle();
        let explorer = {
            let (root, snapshot) = (root.clone(), worktree.snapshot());
            let lone = single_file_path.is_some();
            let (workbench, groups) = (cx.weak_entity(), editor_groups.clone());
            let focus = sidebar_focus.clone();
            let docs = documents.downgrade();
            cx.new(|cx| {
                let mut explorer =
                    ExplorerView::new(root, lone, snapshot, workbench, &groups, focus, cx);
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
            cx.observe_global::<ConfigState>(Self::config_files_changed),
            // VS Code: a preview tab whose file gets unsaved changes is
            // pinned, so opening another file never replaces it.
            cx.subscribe(&documents, |wb, _, ev: &DocumentEvent, cx| match ev {
                DocumentEvent::DirtyChanged { dirty: true, .. } => wb.pin_dirty_previews(cx),
                DocumentEvent::ChangedOnDisk { path } if decorates_external_changes(cx) => {
                    wb.explorer
                        .update(cx, |explorer, cx| explorer.mark_external_change(path, cx));
                }
                _ => {}
            }),
            crate::status_message::observe(cx, |_, cx| cx.notify()),
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
            cx.new(|cx| ConfigErrorItem::new(workbench, cx))
        };
        let vim = cx.new(VimLayer::new);
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
            scan_options,
            _settings: cx.observe_global::<AppState>(Self::settings_changed),
            keymap_generation: keymap_generation(cx),
            languages_generation: cx.global::<AppState>().languages_generation,
            _worktree_events: worktree_events,
            scanning,
            single_file: single_file_path,
            open_file_watcher,
            watched_open_files: Vec::new(),
            _open_file_events: open_file_events,
            git_head,
            git_branch,
            config_batch: cx
                .try_global::<ConfigState>()
                .map_or(0, |state| state.last_changed().0),
            _subscriptions: subscriptions,
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
            diagnostics_status: diagnostics_status.clone(),
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
            status_left: Vec::new(),
            status_right: Vec::new(),
            pending_chord: Vec::new(),
            vim,
            title_bar_pressed: false,
            quick_open: None,
            focus_before_quick_open: None,
            recent_files: RecentFiles::default(),
            nav_history: HashMap::new(),
            theme_picker: None,
            recent_picker: None,
            recent_picker_loading: None,
            locations_picker: None,
            trust,
            auto_save: Default::default(),
            toast: None,
            toast_count: 0,
            fresh_installs_seen,
            fresh_installs_waiting: Vec::new(),
            persistence: None,
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
        let documents = workbench.documents.clone();
        let trust_events = workbench.start_trust(&documents, window, cx);
        workbench._subscriptions.push(trust_events);
        workbench.start_auto_save(&documents, window, cx);
        workbench.refresh_watched_open_files(cx);
        workbench.add_status_item(StatusBarSide::Left, diagnostics_status.into(), cx);
        workbench.add_status_item(StatusBarSide::Left, config_error.into(), cx);
        workbench.add_status_item(StatusBarSide::Left, mode_indicator.into(), cx);
        workbench.add_status_item(StatusBarSide::Right, large_file_status.into(), cx);
        workbench.add_status_item(StatusBarSide::Right, lsp_status.into(), cx);
        workbench.add_status_item(StatusBarSide::Right, document_status.into(), cx);
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
    /// and in single-file mode the file too.
    pub fn opened_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.root.clone()];
        paths.extend(self.single_file.clone());
        paths
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

    /// The branch checked out in the repository holding the root (a
    /// short SHA when detached); `None` outside a repository.
    pub fn git_branch(&self) -> Option<&str> {
        self.git_branch.as_deref()
    }

    /// The background scan of the opened folder.
    pub fn worktree(&self) -> &Worktree {
        &self.worktree
    }

    /// The options of the current scan; `files.exclude` changes replace them.
    pub fn scan_options(&self) -> &ScanOptions {
        &self.scan_options
    }

    /// After any [`AppState`] change: rescans when `files.exclude`
    /// changed, drops a pending chord when the keymap was reloaded, and
    /// redraws (the theme may have changed).
    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let exclude = enabled_globs(cx, "files.exclude");
        if exclude != self.scan_options.exclude {
            self.scan_options = ScanOptions { exclude };
            self.worktree.set_options(self.scan_options.clone());
        }
        // A chord typed against the old keymap means nothing now.
        let generation = keymap_generation(cx);
        if generation != self.keymap_generation {
            self.keymap_generation = generation;
            self.pending_chord.clear();
        }
        // A new language registry: open documents look theirs up again.
        let generation = cx.global::<AppState>().languages_generation;
        if generation != self.languages_generation {
            self.languages_generation = generation;
            self.documents
                .update(cx, |documents, cx| documents.languages_changed(cx));
        }
        // `editor.detectIndentation`, `editor.tabSize` or
        // `editor.insertSpaces` may have changed.
        self.documents
            .update(cx, |documents, cx| documents.settings_changed(cx));
        // `session.trustAllWorktrees` may have changed.
        self.apply_trust(cx);
        // `breadcrumbs.*` may have changed.
        self.breadcrumbs_changed(cx);
        cx.notify();
    }

    /// The Explorer, installed in the Explorer sidebar slot by [`Workbench::new`].
    pub fn explorer(&self) -> &Entity<ExplorerView> {
        &self.explorer
    }

    /// Remembers root-relative paths Lucerna itself just changed, so their
    /// watcher events are not decorated as another program's. A directory
    /// also covers the paths under it. Entries expire after
    /// [`OWN_CHANGE_TTL`].
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
        self.refresh_watched_open_files(cx);
        cx.notify();
    }

    /// After Lucerna deleted `path` (absolute): the tabs of files at it or
    /// under it close, except those with unsaved changes, which stay to be
    /// marked deleted when the watcher reports the removal.
    pub(crate) fn paths_deleted(&mut self, path: &Path, cx: &mut Context<Self>) {
        let path = self.absolute_path(path);
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
            _ => {}
        }
        let snapshot = self.worktree.snapshot();
        let decorate = decorates_external_changes(cx);
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
        // The tree changed: some open files may have joined or left it.
        self.refresh_watched_open_files(cx);
    }

    /// Open files the worktree does not follow (outside the root, or not
    /// in the tree, e.g. ignored; all of them in single-file mode), which
    /// the open-file watcher follows.
    pub fn watched_open_files(&self) -> &[PathBuf] {
        &self.watched_open_files
    }

    /// Re-picks [`Workbench::watched_open_files`] after the tabs or the
    /// tree changed.
    fn refresh_watched_open_files(&mut self, cx: &App) {
        let snapshot = self.worktree.snapshot();
        let mut files: Vec<PathBuf> = self
            .open_tab_paths(cx)
            .into_iter()
            .filter(|path| match path.strip_prefix(&self.root) {
                Ok(rel) => !self.scanning || !snapshot.contains(rel),
                Err(_) => true,
            })
            .collect();
        files.sort();
        files.dedup();
        if files != self.watched_open_files {
            if let Some(watcher) = &self.open_file_watcher {
                watcher.set_files(files.iter().chain(&self.git_head).cloned().collect());
            }
            self.watched_open_files = files;
        }
    }

    /// Every open tab's path, once per path.
    fn open_tab_paths(&self, cx: &App) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self
            .editor_groups
            .read(cx)
            .model()
            .groups()
            .iter()
            .flat_map(|g| g.tabs.iter().filter_map(|t| t.path()))
            .map(|p| p.to_path_buf())
            .collect();
        paths.sort();
        paths.dedup();
        paths
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
    /// so keys keep working. Covers code views, the Extensions, Search,
    /// References and Health tabs and server logs.
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
            || self.lsp_logs.hidden_with_focus(model, window, cx)
            || groups.hidden_bodies().any(|body| {
                body.clone()
                    .downcast::<CodeView>()
                    .is_ok_and(|view| had_focus(view.read(cx).focus_handle(cx)))
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

    /// Opens `path` in the editor area. `Preview` honours the
    /// `workbench.editor.enablePreview` setting. Records `path` in
    /// [`Workbench::recent_files`]. A relative path is resolved against the
    /// root first, so tabs, recent files and the saved state only ever
    /// hold absolute paths.
    pub fn open_path(&mut self, path: Arc<Path>, mode: OpenMode, cx: &mut Context<Self>) {
        let path = self.absolute_path(&path);
        // An F8 hover still waiting for its tab is for an earlier open.
        self.marker_hover = None;
        self.recent_files.record(path.clone());
        let preview_enabled = cx
            .global::<AppState>()
            .settings
            .get::<bool>("workbench.editor.enablePreview", None)
            .unwrap_or(true);
        self.update_editor_groups(cx, |m| m.open(path, mode, preview_enabled));
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
    /// goes on, Cancel (or a dismissed prompt) stops. Nothing dirty: `true`
    /// at once.
    pub fn confirm_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        let documents = self.documents.clone();
        let paths = documents.read(cx).dirty_paths();
        if paths.is_empty() {
            return Task::ready(true);
        }
        // The prompt is already up for an earlier quit or close.
        if self.confirming_quit {
            return Task::ready(false);
        }
        self.confirming_quit = true;
        let names: Vec<String> = paths.iter().map(|p| save::display_name(p)).collect();
        let answer = save::prompt_unsaved_all(&names, window, cx);
        cx.spawn_in(window, async move |wb, cx| {
            let go_on = match SaveChoice::of(answer.await) {
                SaveChoice::Save => {
                    let paths = documents.read_with(cx, |d, _| d.dirty_paths());
                    // An edit typed while the saves wrote is not saved:
                    // stop rather than lose it.
                    save::save_each(documents.clone(), paths, cx).await
                        && documents.read_with(cx, |d, _| d.dirty_paths().is_empty())
                }
                SaveChoice::DontSave => true,
                SaveChoice::Cancel => false,
            };
            wb.update(cx, |wb, _| wb.confirming_quit = false).ok();
            go_on
        })
    }

    /// The window's close button: closes at once when nothing is dirty;
    /// otherwise asks ([`Workbench::confirm_quit`]) and closes the window
    /// itself if told to go on.
    fn should_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.documents.read(cx).dirty_paths().is_empty() {
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

    /// Opens `path` like [`Workbench::open_path`], then puts the cursor at
    /// `point` and scrolls it to the middle, once the file is loaded. For
    /// Quick Open, search results and go-to-line.
    pub fn open_path_at(
        &mut self,
        path: Arc<Path>,
        mode: OpenMode,
        point: Point,
        cx: &mut Context<Self>,
    ) {
        let path = self.absolute_path(&path);
        self.open_path(path.clone(), mode, cx);
        // An already-drawn tab has its view; a new one gets it on draw.
        let view = self
            .editor_groups
            .read(cx)
            .active_body()
            .and_then(|v| v.downcast::<CodeView>().ok());
        self.documents.update(cx, |docs, cx| match view {
            Some(view) => docs.go_to(&view, point, cx),
            None => docs.go_to_when_opened(&path, point),
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
        self.open_path(target.path.clone(), OpenMode::Pinned, cx);
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
        flag("contextMenuVisible", menu_open);
        flag("searchViewletFocus", self.search_focused(window, cx));
        flag("referencesViewFocus", self.references_view_focus(cx));
        flag(
            "extensionsViewletFocus",
            self.extensions_focused(window, cx),
        );
        // The theme, locations and recent pickers are quick picks too:
        // Escape closes them.
        flag(
            "inQuickOpen",
            self.quick_open.is_some()
                || self.theme_picker.is_some()
                || self.locations_picker.is_some()
                || self.recent_picker.is_some(),
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
            "lspHoverVisible",
            self.active_code_view(cx)
                .is_some_and(|v| v.read(cx).hover().is_visible()),
        );
        let active = self.editor_groups.read(cx).model().active_tab();
        let path = active.and_then(|tab| tab.path());
        let extname = path.map(|p| extname(p)).unwrap_or_default();
        // The language the document resolved (`files.associations`
        // included); before its view exists, the same lookup by path.
        let view = self
            .editor_groups
            .read(cx)
            .active_body()
            .and_then(|body| body.downcast::<CodeView>().ok());
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
                    cx.global::<AppState>().languages.language_for_file(rel, "")
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

    /// Shows or hides the sidebar. Hiding it moves focus out of it.
    pub fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_visible && self.sidebar_focus.contains_focused(window, cx) {
            self.focus_editor_area(window, cx);
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

    pub fn focus_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.sidebar_focus, cx);
        cx.notify();
    }

    /// Closes every modal overlay (Quick Open, the locations picker, the
    /// theme picker, reverting its preview, the Open Recent picker and the
    /// trust modal). Each `show_*` calls it first, so at most one is ever
    /// shown and one Esc never has to close two.
    pub(crate) fn close_modal_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_quick_open(window, cx);
        self.close_locations_picker(window, cx);
        self.close_theme_picker(true, window, cx);
        self.close_recent_picker(window, cx);
        self.close_trust_modal(window, cx);
    }

    /// Whether Quick Open, the theme picker, the Open Recent picker or the
    /// trust modal is shown: a late go-to answer then does not open the
    /// locations picker over it.
    pub(crate) fn other_modal_overlay_shown(&self) -> bool {
        self.quick_open.is_some()
            || self.theme_picker.is_some()
            || self.recent_picker.is_some()
            || self.trust_modal().is_some()
    }

    /// Focuses the active tab's code view, or the editor area when the
    /// active tab shows none (yet: a new tab's view is made while drawing,
    /// and [`Workbench::focus_active_view`] focuses it then).
    pub fn focus_editor_area(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self
            .active_code_view(cx)
            .filter(|v| !v.read(cx).is_passive())
        {
            Some(view) => window.focus(&view.read(cx).focus_handle(cx), cx),
            None => window.focus(&self.editor_focus, cx),
        }
        cx.notify();
    }

    /// Like Zed, a code view that becomes active (a file opened or a tab
    /// activated) takes the focus, unless the focus is somewhere else
    /// than the editor area itself or the previously active view (the
    /// sidebar, a picker, the Search tab).
    fn focus_active_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.active_code_view(cx) else {
            return;
        };
        let previous = self.active_view.as_ref().and_then(WeakEntity::upgrade);
        if previous.as_ref() == Some(&view) {
            return;
        }
        self.active_view = Some(view.downgrade());
        if view.read(cx).is_passive() {
            return;
        }
        let focus_free = window.focused(cx).is_none()
            || self.editor_focus.is_focused(window)
            || previous.is_some_and(|p| p.read(cx).focus_handle(cx).contains_focused(window, cx));
        if focus_free {
            window.focus(&view.read(cx).focus_handle(cx), cx);
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
        if vim::dispatch(
            &self.vim,
            &self.editor_groups,
            ev,
            &ctx,
            chord_pending,
            window,
            cx,
        ) {
            hide_hover(cx);
            cx.stop_propagation();
            // `gd`, `]d`, `ctrl-o`...: after the hover is hidden, so
            // `gh` can show it again.
            if let Some(command) = self.vim.update(cx, |vim, _| vim.take_command()) {
                self.run_command(command, None, window, cx);
            }
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

/// The globs of a glob → bool setting such as `files.exclude` that are
/// switched on.
pub(crate) fn enabled_globs(cx: &App, key: &str) -> Vec<String> {
    cx.global::<AppState>()
        .settings
        .get::<serde_json::Map<String, Value>>(key, None)
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, on)| *on == Value::Bool(true))
        .map(|(glob, _)| glob)
        .collect()
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

/// Quits the app once every workbench window agreed to
/// ([`Workbench::confirm_quit`], one window after another). Quitting runs
/// the `on_app_quit` hooks, which save the layout.
pub(crate) fn quit(cx: &mut App) {
    // Not `downcast`: a Welcome window that became a workbench keeps the
    // handle type it opened with. Updating a window of another root fails.
    let windows: Vec<WindowHandle<Workbench>> = cx
        .windows()
        .into_iter()
        .map(|w| WindowHandle::new(w.window_id()))
        .collect();
    cx.spawn(async move |cx| {
        for handle in windows {
            let Ok(confirm) = handle.update(cx, |wb, window, cx| wb.confirm_quit(window, cx))
            else {
                // The window closed meanwhile, or shows Welcome.
                continue;
            };
            if !confirm.await {
                return;
            }
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// The commands every workbench registers, as [`Workbench::new`] builds
/// them; the menu bar reads their IDs without a window.
pub(crate) fn default_registry() -> CommandRegistry {
    let mut commands = CommandRegistry::default();
    register_shell_commands(&mut commands);
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
    lsp_status::register_commands(&mut commands);
    diagnostics_status::register_commands(&mut commands);
    crate::hover::register_commands(&mut commands);
    crate::goto::register_commands(&mut commands);
    crate::extensions::register_commands(&mut commands);
    crate::symbols::register_commands(&mut commands);
    crate::editing::register_commands(&mut commands);
    crate::save::register_commands(&mut commands);
    crate::about::register_commands(&mut commands);
    crate::explorer::register_commands(&mut commands);
    commands
}

fn register_shell_commands(commands: &mut CommandRegistry) {
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

        let status_message = crate::status_message::current(cx);
        div()
            .id("workbench")
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::dispatch_key))
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
                root_name.clone(),
                self.git_branch.clone(),
                self.language_servers.read(cx).shows_restricted_mode(),
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
                    .child(editor_area),
            )
            .child(status_bar::render(
                panel_toggles::render(
                    self.sidebar_visible,
                    search_active,
                    extensions_open,
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
fn decorates_external_changes(cx: &App) -> bool {
    cx.global::<AppState>()
        .settings
        .get::<bool>("explorer.decorations.externalChanges", None)
        .unwrap_or(true)
}
