//! The bridge between open documents and language servers (spec §4, §6).
//!
//! One [`LanguageServers`] entity per workbench follows [`Documents`]:
//! when a document opens it finds the document's servers (root search off
//! the UI thread), starts missing ones in the background, and keeps them
//! in step with `didOpen` / `didChange` / `didClose`. Each server's events
//! are pumped onto the UI thread by a `cx.spawn` loop: diagnostics go into
//! a [`DiagnosticStore`], logs into a 2000-line ring per server.
//!
//! Paths are the keys of [`Documents`] (absolute, joined onto the root,
//! not canonicalised). Servers echo the URIs they were sent, so a
//! diagnostic's `uri_to_path` meets the key it was opened under.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::task::Poll;
use std::thread;
use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedReceiver;
use futures::future::BoxFuture;
use futures::{FutureExt, StreamExt};
use gpui::{App, Context, Entity, EventEmitter, Subscription, Task};
use lsp_types::notification::DidChangeWatchedFiles;
use lsp_types::{DidChangeWatchedFilesParams, FileChangeType};
use lucerna_lsp::diagnostics::DiagnosticStore;
use lucerna_lsp::position::uri_to_path;
use lucerna_lsp::sync::{DocumentSync, WatchedFiles, too_large_for_lsp};
use lucerna_lsp::transport::LaunchSpec;
use lucerna_lsp::{
    Adapter, LanguageServer, Launcher, LogKind, LogLine, LspError, ProcessLauncher, Registry,
    ServerEvent, ServerKey, ServerSettings, ServersSettings, Slot, StartError, builtin_adapters,
    find_root, initialize_params, resolve_launch, select_servers,
};
use lucerna_project::{ChangeKind, WorktreeEvent};
use lucerna_text::BufferSnapshot;
use serde_json::Value;

use crate::AppState;
use crate::documents::{DocState, DocumentEvent, Documents};
use crate::tasks::guarded;
use crate::workspace_state::absolute_path;

/// Lines kept in each server's log.
const LOG_LINES: usize = 2000;
/// Lines of stderr shown with a failed server.
const STDERR_TAIL: usize = 50;
const DIAGNOSTICS_BATCH: Duration = Duration::from_millis(50);
const WATCHED_FILES_BATCH: Duration = Duration::from_millis(100);
/// The most [`LanguageServers::shutdown_all`] waits, for all servers.
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(5);

/// A server slot as the status UI shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotState {
    Starting,
    Running,
    Failed {
        message: String,
        stderr_tail: Vec<String>,
    },
    NotFound {
        message: String,
    },
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerStatus {
    pub key: ServerKey,
    pub state: SlotState,
    /// The newest unfinished `$/progress`: title, percentage and message,
    /// e.g. `Indexing 45% — crates/foo`. Without the server name.
    pub progress: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LanguageServersEvent {
    /// The diagnostics of these files changed (one event per 50 ms batch).
    DiagnosticsChanged(Vec<Arc<Path>>),
    /// A slot changed state, or a server reported progress.
    StatusChanged,
    /// A server finished `initialize`.
    ServerReady(ServerKey),
    /// A document was sent to its servers (opened or changed).
    DocumentChanged(Arc<Path>),
}

/// The settings a language's servers are picked by.
#[derive(Clone, Debug, PartialEq)]
struct Selection {
    enabled: bool,
    servers: Vec<String>,
}

/// A document [`Documents`] has opened.
struct OpenDoc {
    language_id: Option<Arc<str>>,
    snapshot: BufferSnapshot,
    /// Its servers, once found.
    keys: Vec<ServerKey>,
    /// The settings `keys` were found with; `None` before a search.
    selection: Option<Selection>,
    /// Bumped per search, so an older search's result is dropped.
    search: u64,
    _search_task: Option<Task<()>>,
    /// Gone from disk: closed in the servers until it is back.
    deleted: bool,
}

struct ProgressItem {
    token: String,
    title: Option<String>,
    message: Option<String>,
    percentage: Option<u32>,
}

impl ProgressItem {
    fn text(&self) -> String {
        let mut text = self.title.clone().unwrap_or_default();
        if let Some(percentage) = self.percentage {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&format!("{percentage}%"));
        }
        if let Some(message) = &self.message {
            if !text.is_empty() {
                text.push_str(" — ");
            }
            text.push_str(message);
        }
        text
    }
}

/// A started process, before `initialize`.
struct Launched {
    server: Arc<LanguageServer>,
    events: UnboundedReceiver<ServerEvent>,
    spec: LaunchSpec,
    /// `lsp.<server>.settings` the server was started with.
    settings: Value,
}

/// A server's event stream and the task pumping it. Closing the stream
/// before a shutdown keeps the shutdown thread's last events (exit, log)
/// from waking the UI executor from another thread.
struct Pump {
    events: Rc<RefCell<Option<UnboundedReceiver<ServerEvent>>>>,
    _task: Task<()>,
}

/// What [`LanguageServers::launched`] hands back to the start task.
struct Initializing {
    server: Arc<LanguageServer>,
    settings: Value,
    initialize: BoxFuture<'static, Result<(), LspError>>,
}

pub struct LanguageServers {
    root: PathBuf,
    documents: Entity<Documents>,
    launcher: Arc<dyn Launcher>,
    adapters: Vec<Arc<dyn Adapter>>,
    trusted: bool,
    registry: Registry,
    /// Bumped per start or stop of a key: results and events of an older
    /// process are dropped (its log lines are kept).
    generations: HashMap<ServerKey, u64>,
    /// The event pump of each key's current process.
    pumps: HashMap<ServerKey, Pump>,
    sync: DocumentSync,
    watched: WatchedFiles,
    diagnostics: DiagnosticStore,
    docs: HashMap<Arc<Path>, OpenDoc>,
    logs: HashMap<&'static str, VecDeque<LogLine>>,
    progress: HashMap<ServerKey, Vec<ProgressItem>>,
    changed_diagnostics: BTreeSet<Arc<Path>>,
    diagnostics_flush_pending: bool,
    watched_changes: Vec<(PathBuf, FileChangeType)>,
    watched_flush_pending: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LanguageServersEvent> for LanguageServers {}

impl Drop for Pump {
    fn drop(&mut self) {
        // Senders now fail instead of waking the cancelled task.
        self.events.borrow_mut().take();
    }
}

impl LanguageServers {
    /// The servers of the folder at `root`, following `documents`. Starts
    /// processes with a [`ProcessLauncher`]. Trusted when the User setting
    /// `session.trustAllWorktrees` is `true`; untrusted, nothing starts.
    pub fn new(root: PathBuf, documents: &Entity<Documents>, cx: &mut Context<Self>) -> Self {
        let adapters = builtin_adapters();
        let trusted = cx
            .global::<AppState>()
            .settings
            .get_trusted::<bool>("session.trustAllWorktrees", None)
            == Some(true);
        let mut registry = Registry::new(adapters.clone());
        if trusted {
            registry.set_settings(servers_settings(cx));
        }
        LanguageServers {
            root,
            documents: documents.clone(),
            launcher: Arc::new(ProcessLauncher::new()),
            adapters,
            trusted,
            registry,
            generations: HashMap::new(),
            pumps: HashMap::new(),
            sync: DocumentSync::default(),
            watched: WatchedFiles::default(),
            diagnostics: DiagnosticStore::default(),
            docs: HashMap::new(),
            logs: HashMap::new(),
            progress: HashMap::new(),
            changed_diagnostics: BTreeSet::new(),
            diagnostics_flush_pending: false,
            watched_changes: Vec::new(),
            watched_flush_pending: false,
            _subscriptions: vec![
                cx.subscribe(documents, Self::on_document_event),
                cx.observe_global::<AppState>(Self::settings_changed),
            ],
        }
    }

    /// Replaces how servers are found and started (tests use fakes).
    pub fn set_launcher(&mut self, launcher: Arc<dyn Launcher>) {
        self.launcher = launcher;
    }

    /// Untrusted stops every server; trusted sends every open document
    /// to its servers, starting them.
    pub fn set_trusted(&mut self, trusted: bool, cx: &mut Context<Self>) {
        if trusted == self.trusted {
            return;
        }
        self.trusted = trusted;
        if trusted {
            self.registry.set_settings(servers_settings(cx));
            let paths: Vec<Arc<Path>> = self.docs.keys().cloned().collect();
            for path in paths {
                self.find_servers(&path, cx);
            }
        } else {
            let keys: Vec<ServerKey> = self.registry.keys().cloned().collect();
            for key in keys {
                self.stop_server(&key, cx);
                self.registry.remove(&key);
            }
            for doc in self.docs.values_mut() {
                doc.keys.clear();
                doc.selection = None;
                doc.search += 1;
                doc._search_task = None;
            }
        }
        cx.emit(LanguageServersEvent::StatusChanged);
        cx.notify();
    }

    pub fn is_trusted(&self) -> bool {
        self.trusted
    }

    /// Every slot, ordered by root then server name.
    pub fn statuses(&self) -> Vec<ServerStatus> {
        let mut keys: Vec<&ServerKey> = self.registry.keys().collect();
        keys.sort_by(|a, b| (&a.root, a.server).cmp(&(&b.root, b.server)));
        keys.into_iter()
            .filter_map(|key| {
                let state = match self.registry.slot(key)? {
                    Slot::Starting => SlotState::Starting,
                    Slot::Running(_) => SlotState::Running,
                    Slot::Failed {
                        message,
                        stderr_tail,
                    } => SlotState::Failed {
                        message: message.clone(),
                        stderr_tail: stderr_tail.clone(),
                    },
                    Slot::NotFound { message } => SlotState::NotFound {
                        message: message.clone(),
                    },
                    Slot::Stopped => SlotState::Stopped,
                };
                let progress = self
                    .progress
                    .get(key)
                    .and_then(|items| items.last())
                    .map(ProgressItem::text);
                Some(ServerStatus {
                    key: key.clone(),
                    state,
                    progress,
                })
            })
            .collect()
    }

    /// The log of the server named `server` (all its roots): stderr,
    /// `window/logMessage`, `window/showMessage` and lifecycle lines.
    pub fn log(&self, server: &str) -> Vec<LogLine> {
        self.logs
            .get(server)
            .map(|log| log.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn diagnostics(&self) -> &DiagnosticStore {
        &self.diagnostics
    }

    /// The running servers that have `path` open, or will once it is
    /// sent to them.
    pub fn running_for(&self, path: &Path) -> Vec<(ServerKey, Arc<LanguageServer>)> {
        let Some(doc) = self.docs.get(path) else {
            return Vec::new();
        };
        doc.keys
            .iter()
            .filter_map(|key| match self.registry.slot(key) {
                Some(Slot::Running(server)) => Some((key.clone(), server.clone())),
                _ => None,
            })
            .collect()
    }

    /// Whether a file of `language_id` would get a server: language
    /// servers are enabled for it and one is selected.
    pub fn wants_server(&self, _path: &Path, language_id: &str, cx: &App) -> bool {
        let selection = selection(language_id, cx);
        selection.enabled
            && !select_servers(&selection.servers, &self.adapters, language_id).is_empty()
    }

    /// Stops `key` and starts it again.
    pub fn restart(&mut self, key: &ServerKey, cx: &mut Context<Self>) {
        if !self.trusted || self.registry.slot(key).is_none() {
            return;
        }
        self.stop_server(key, cx);
        self.start(key.clone(), cx);
    }

    pub fn stop(&mut self, key: &ServerKey, cx: &mut Context<Self>) {
        if self.registry.slot(key).is_none() {
            return;
        }
        self.stop_server(key, cx);
        self.registry.set_slot(key.clone(), Slot::Stopped);
        cx.emit(LanguageServersEvent::StatusChanged);
        cx.notify();
    }

    pub fn restart_all(&mut self, cx: &mut Context<Self>) {
        let keys: Vec<ServerKey> = self.registry.keys().cloned().collect();
        for key in keys {
            self.restart(&key, cx);
        }
    }

    pub fn stop_all(&mut self, cx: &mut Context<Self>) {
        let keys: Vec<ServerKey> = self.registry.keys().cloned().collect();
        for key in keys {
            self.stop(&key, cx);
        }
    }

    /// Follows the files on disk: documents removed from disk are closed in
    /// their servers, and changes matching a server's watch registrations
    /// are sent as `workspace/didChangeWatchedFiles`, in 100 ms batches.
    /// Call it after [`Documents::on_worktree_event`] has seen the event.
    pub fn on_worktree_event(&mut self, ev: &WorktreeEvent, cx: &mut Context<Self>) {
        let documents = self.documents.read(cx);
        let gone: Vec<Arc<Path>> = self
            .docs
            .iter()
            .filter(|(path, doc)| !doc.deleted && documents.state(path) == Some(&DocState::Deleted))
            .map(|(path, _)| path.clone())
            .collect();
        for path in gone {
            let servers = self.running_for(&path);
            self.sync.close(&path, &servers);
            if let Some(doc) = self.docs.get_mut(&path) {
                doc.deleted = true;
            }
        }
        let WorktreeEvent::Changed(changes) = ev else {
            return;
        };
        if !self.trusted {
            return;
        }
        self.watched_changes.extend(changes.iter().map(|change| {
            let typ = match change.kind {
                ChangeKind::Added => FileChangeType::CREATED,
                ChangeKind::Modified => FileChangeType::CHANGED,
                ChangeKind::Removed => FileChangeType::DELETED,
            };
            (absolute_path(&self.root, &change.path), typ)
        }));
        if !self.watched_flush_pending {
            self.watched_flush_pending = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(WATCHED_FILES_BATCH).await;
                this.update(cx, |servers, _| servers.flush_watched_files())
                    .ok();
            })
            .detach();
        }
    }

    /// Shuts every server down in parallel and waits for them, at most
    /// 5 s in all. For window close and app quit.
    pub fn shutdown_all(&mut self) {
        let deadline = Instant::now() + SHUTDOWN_BUDGET;
        let keys: Vec<ServerKey> = self.registry.keys().cloned().collect();
        let mut handles = Vec::new();
        for key in keys {
            self.bump_generation(&key);
            self.pumps.remove(&key);
            if let Some(Slot::Running(server)) = self.registry.remove(&key) {
                handles.push(server.shutdown());
            }
            self.registry.set_slot(key, Slot::Stopped);
        }
        self.sync = DocumentSync::default();
        self.watched = WatchedFiles::default();
        for handle in handles {
            while !handle.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
    }

    fn on_document_event(
        &mut self,
        _: Entity<Documents>,
        ev: &DocumentEvent,
        cx: &mut Context<Self>,
    ) {
        match ev {
            DocumentEvent::Opened {
                path,
                language_id,
                snapshot,
            } => {
                self.diagnostics.resolve_open(path, snapshot);
                self.docs.insert(
                    path.clone(),
                    OpenDoc {
                        language_id: language_id.clone(),
                        snapshot: snapshot.clone(),
                        keys: Vec::new(),
                        selection: None,
                        search: 0,
                        _search_task: None,
                        deleted: false,
                    },
                );
                self.find_servers(path, cx);
            }
            DocumentEvent::Reloaded { path, snapshot } => {
                self.diagnostics.resolve_open(path, snapshot);
                let Some(doc) = self.docs.get_mut(path) else {
                    return;
                };
                doc.snapshot = snapshot.clone();
                if std::mem::take(&mut doc.deleted) {
                    self.open_in_servers(path, cx);
                } else {
                    let servers = self.running_for(path);
                    if !servers.is_empty() {
                        self.sync.change(path, snapshot, &servers);
                        cx.emit(LanguageServersEvent::DocumentChanged(path.clone()));
                    }
                }
            }
            DocumentEvent::Closed { path } => {
                let servers = self.running_for(path);
                self.sync.close(path, &servers);
                self.docs.remove(path);
            }
            DocumentEvent::LanguageChanged {
                path,
                language_id,
                snapshot,
            } => {
                // Closed with the old servers (`didClose`), then opened
                // with the new language's.
                self.set_keys(path, Vec::new(), cx);
                let Some(doc) = self.docs.get_mut(path) else {
                    return;
                };
                doc.language_id = language_id.clone();
                doc.snapshot = snapshot.clone();
                self.find_servers(path, cx);
            }
        }
    }

    /// Picks the servers of `path` by its language's settings and looks
    /// for their roots (and the file's size limits) off the UI thread.
    fn find_servers(&mut self, path: &Arc<Path>, cx: &mut Context<Self>) {
        if !self.trusted {
            return;
        }
        let Some(doc) = self.docs.get_mut(path) else {
            return;
        };
        doc.search += 1;
        let search = doc.search;
        let language = doc.language_id.clone();
        doc.selection = language.as_deref().map(|lang| selection(lang, cx));
        let adapters = match (&language, &doc.selection) {
            (Some(lang), Some(selection)) if selection.enabled => {
                select_servers(&selection.servers, &self.adapters, lang)
            }
            _ => Vec::new(),
        };
        if adapters.is_empty() {
            doc._search_task = None;
            self.set_keys(path, Vec::new(), cx);
            return;
        }
        let settings = &cx.global::<AppState>().settings;
        let max_bytes = settings
            .get::<u64>("lucerna.largeFile.maxBytes", language.as_deref())
            .unwrap_or(20 * 1024 * 1024);
        let max_lines = settings
            .get::<u32>("lucerna.largeFile.maxLines", language.as_deref())
            .unwrap_or(300_000);
        let (snapshot, file, root) = (doc.snapshot.clone(), path.clone(), self.root.clone());
        let work = cx.background_executor().spawn(async move {
            guarded("lsp-find-servers", move || {
                if too_large_for_lsp(&snapshot, max_bytes, max_lines) {
                    return Vec::new();
                }
                adapters
                    .iter()
                    .map(|adapter| ServerKey {
                        root: find_root(&**adapter, &file, &root).into(),
                        server: adapter.name(),
                    })
                    .collect()
            })
            .unwrap_or_default()
        });
        let path = path.clone();
        doc._search_task = Some(cx.spawn(async move |this, cx| {
            let keys = work.await;
            this.update(cx, |servers, cx| {
                let current = servers
                    .docs
                    .get(&path)
                    .is_some_and(|doc| doc.search == search);
                if current && servers.trusted {
                    servers.set_keys(&path, keys, cx);
                }
            })
            .ok();
        }));
    }

    /// Gives `path` the servers `keys`: closes it in servers it no longer
    /// has, starts missing servers and opens it in running ones.
    fn set_keys(&mut self, path: &Arc<Path>, keys: Vec<ServerKey>, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(path) else {
            return;
        };
        let old = std::mem::replace(&mut doc.keys, keys.clone());
        let dropped: Vec<(ServerKey, Arc<LanguageServer>)> = old
            .into_iter()
            .filter(|key| !keys.contains(key))
            .filter_map(|key| match self.registry.slot(&key) {
                Some(Slot::Running(server)) => Some((key, server.clone())),
                _ => None,
            })
            .collect();
        self.sync.close(path, &dropped);
        for key in keys {
            if self.registry.slot(&key).is_none() {
                self.start(key, cx);
            }
        }
        self.open_in_servers(path, cx);
    }

    /// `didOpen` for `path` in each of its running servers that does not
    /// have it yet.
    fn open_in_servers(&mut self, path: &Arc<Path>, cx: &mut Context<Self>) {
        let servers = self.running_for(path);
        let Some(doc) = self.docs.get(path) else {
            return;
        };
        let Some(language) = doc.language_id.as_deref() else {
            return;
        };
        if !self.trusted || doc.deleted {
            return;
        }
        let mut opened = false;
        for (key, server) in servers {
            if self.sync.is_open(path, &key) {
                continue;
            }
            let Some(adapter) = self.registry.adapter(key.server) else {
                continue;
            };
            let language_id = adapter.lsp_language_id(language);
            self.sync
                .open(path, &language_id, &doc.snapshot, &[(key, server)]);
            opened = true;
        }
        if opened {
            cx.emit(LanguageServersEvent::DocumentChanged(path.clone()));
        }
    }

    fn bump_generation(&mut self, key: &ServerKey) -> u64 {
        let generation = self.generations.entry(key.clone()).or_default();
        *generation += 1;
        *generation
    }

    fn is_current(&self, key: &ServerKey, generation: u64) -> bool {
        self.generations.get(key) == Some(&generation)
    }

    /// Finds the binary and spawns the server in the background, then
    /// awaits `initialize`. The slot is `Starting` meanwhile.
    fn start(&mut self, key: ServerKey, cx: &mut Context<Self>) {
        let Some(adapter) = self.registry.adapter(key.server) else {
            return;
        };
        let generation = self.bump_generation(&key);
        self.registry.set_slot(key.clone(), Slot::Starting);
        self.progress.remove(&key);
        let settings = self.registry.settings(key.server).cloned();
        let (launcher, root) = (self.launcher.clone(), key.root.clone());
        let work = cx.background_executor().spawn(async move {
            guarded("lsp-launch", move || {
                launch(&*adapter, settings.as_ref(), &root, &*launcher)
            })
        });
        cx.spawn(async move |this, cx| {
            let launched = work.await;
            let Ok(Some(init)) = this.update(cx, |servers, cx| {
                servers.launched(&key, generation, launched, cx)
            }) else {
                return;
            };
            let result = init.initialize.await;
            this.update(cx, |servers, cx| {
                servers.initialized(&key, generation, init.server, init.settings, result, cx)
            })
            .ok();
        })
        .detach();
        cx.emit(LanguageServersEvent::StatusChanged);
        cx.notify();
    }

    /// The process is up (or could not start): pumps its events and sends
    /// `initialize`.
    fn launched(
        &mut self,
        key: &ServerKey,
        generation: u64,
        launched: Option<Result<Launched, StartError>>,
        cx: &mut Context<Self>,
    ) -> Option<Initializing> {
        if !self.is_current(key, generation) {
            if let Some(Ok(Launched { server, events, .. })) = launched {
                drop(events);
                let _ = server.shutdown();
            }
            return None;
        }
        let launched = match launched {
            Some(Ok(launched)) => launched,
            Some(Err(StartError::NotFound { message })) => {
                self.push_log(key.server, LogKind::Lifecycle, message.clone());
                self.registry
                    .set_slot(key.clone(), Slot::NotFound { message });
                cx.emit(LanguageServersEvent::StatusChanged);
                cx.notify();
                return None;
            }
            Some(Err(StartError::Failed { message, .. })) => {
                self.push_log(key.server, LogKind::Lifecycle, message.clone());
                self.set_failed(key, message, cx);
                return None;
            }
            // The panic is logged by `guarded`.
            None => {
                self.set_failed(key, "internal error while starting".into(), cx);
                return None;
            }
        };
        let Launched {
            server,
            events,
            spec,
            settings,
        } = launched;
        let mut command = spec.program.display().to_string();
        for arg in &spec.args {
            command.push(' ');
            command.push_str(arg);
        }
        self.push_log(key.server, LogKind::Lifecycle, format!("Binary: {command}"));
        self.push_log(
            key.server,
            LogKind::Lifecycle,
            format!("Root: {}", key.root.display()),
        );
        self.pump(key.clone(), generation, Arc::downgrade(&server), events, cx);
        let defaults = self
            .registry
            .adapter(key.server)
            .and_then(|adapter| adapter.initialization_options());
        let overrides = self
            .registry
            .settings(key.server)
            .and_then(|s| s.initialization_options.clone());
        let options = merge_options(defaults, overrides);
        let initialize = server
            .initialize(initialize_params(&key.root, options))
            .boxed();
        Some(Initializing {
            server,
            settings,
            initialize,
        })
    }

    fn initialized(
        &mut self,
        key: &ServerKey,
        generation: u64,
        server: Arc<LanguageServer>,
        started_with: Value,
        result: Result<(), LspError>,
        cx: &mut Context<Self>,
    ) {
        let starting = matches!(self.registry.slot(key), Some(Slot::Starting));
        if !self.is_current(key, generation) || !starting {
            // The stream was closed when it was stopped, or it exited.
            let _ = server.shutdown();
            return;
        }
        if let Err(err) = result {
            self.pumps.remove(key);
            let _ = server.shutdown();
            self.set_failed(key, err.to_string(), cx);
            return;
        }
        // `settings` changed while it started.
        let settings = self
            .registry
            .settings(key.server)
            .and_then(|s| s.settings.clone())
            .unwrap_or(Value::Null);
        if settings != started_with {
            server.set_settings(settings);
        }
        self.registry.set_slot(key.clone(), Slot::Running(server));
        cx.emit(LanguageServersEvent::ServerReady(key.clone()));
        cx.emit(LanguageServersEvent::StatusChanged);
        let paths: Vec<Arc<Path>> = self
            .docs
            .iter()
            .filter(|(_, doc)| doc.keys.contains(key))
            .map(|(path, _)| path.clone())
            .collect();
        for path in paths {
            self.open_in_servers(&path, cx);
        }
        cx.notify();
    }

    /// Delivers a server's events on the UI thread until it is gone. Holds
    /// the server weakly, so dropping it ends the stream.
    fn pump(
        &mut self,
        key: ServerKey,
        generation: u64,
        server: Weak<LanguageServer>,
        events: UnboundedReceiver<ServerEvent>,
        cx: &mut Context<Self>,
    ) {
        let events = Rc::new(RefCell::new(Some(events)));
        let (stream, pump_key) = (events.clone(), key.clone());
        let task = cx.spawn(async move |this, cx| {
            let mut stream =
                futures::stream::poll_fn(move |task_cx| match stream.borrow_mut().as_mut() {
                    Some(events) => events.poll_next_unpin(task_cx),
                    None => Poll::Ready(None),
                });
            while let Some(event) = stream.next().await {
                let delivered = this.update(cx, |servers, cx| {
                    servers.server_event(&key, generation, &server, event, cx)
                });
                if delivered.is_err() {
                    break;
                }
            }
        });
        // Replacing an older pump drops it, and its stream with it.
        self.pumps.insert(
            pump_key,
            Pump {
                events,
                _task: task,
            },
        );
    }

    fn server_event(
        &mut self,
        key: &ServerKey,
        generation: u64,
        server: &Weak<LanguageServer>,
        event: ServerEvent,
        cx: &mut Context<Self>,
    ) {
        let current = self.is_current(key, generation);
        match event {
            ServerEvent::Log(line) => self.push_log(key.server, line.kind, line.text),
            ServerEvent::ShowMessage { message, .. } => {
                self.push_log(key.server, LogKind::Server, message);
            }
            _ if !current => {}
            ServerEvent::Exited(code) => {
                if matches!(
                    self.registry.slot(key),
                    Some(Slot::Running(_) | Slot::Starting)
                ) {
                    let message = match code {
                        Some(code) => format!("exited with code {code}"),
                        None => "exited".to_owned(),
                    };
                    self.set_failed(key, message, cx);
                }
            }
            ServerEvent::Diagnostics(params) => {
                let Some(server) = server.upgrade() else {
                    return;
                };
                let Some(file) = uri_to_path(&params.uri) else {
                    return;
                };
                let file = absolute_path(&self.root, &file);
                let open = self
                    .docs
                    .get(file.as_path())
                    .and_then(|doc| Some((&doc.snapshot, self.sync.version(&file, key)?)));
                if let Some(path) = self
                    .diagnostics
                    .publish(key, params, server.encoding(), open)
                {
                    self.diagnostics_changed([Arc::from(path)], cx);
                }
            }
            ServerEvent::Progress {
                token,
                title,
                message,
                percentage,
                done,
            } => {
                let items = self.progress.entry(key.clone()).or_default();
                let existing = items.iter().position(|item| item.token == token);
                let mut item = match existing {
                    Some(i) => items.remove(i),
                    None => ProgressItem {
                        token,
                        title: None,
                        message: None,
                        percentage: None,
                    },
                };
                if !done {
                    item.title = title.or(item.title);
                    item.message = message.or(item.message);
                    item.percentage = percentage.or(item.percentage);
                    items.push(item);
                }
                cx.emit(LanguageServersEvent::StatusChanged);
                cx.notify();
            }
            ServerEvent::WatchedFilesRegistered { id, watchers } => {
                self.watched.register(key.clone(), id, watchers);
            }
            ServerEvent::WatchedFilesUnregistered { id } => {
                self.watched.unregister(key, &id);
            }
        }
    }

    /// Ends what the server of `key` had: open documents, watch
    /// registrations, progress and diagnostics. Shuts a running server
    /// down; events from it are ignored from now on.
    fn stop_server(&mut self, key: &ServerKey, cx: &mut Context<Self>) {
        self.bump_generation(key);
        self.pumps.remove(key);
        if let Some(Slot::Running(server)) = self.registry.slot(key) {
            let _ = server.shutdown();
        }
        self.forget(key, cx);
    }

    fn forget(&mut self, key: &ServerKey, cx: &mut Context<Self>) {
        self.sync.forget_server(key);
        self.watched.forget_server(key);
        self.progress.remove(key);
        let cleared = self.diagnostics.clear_server(key);
        self.diagnostics_changed(cleared.into_iter().map(Arc::from), cx);
    }

    fn set_failed(&mut self, key: &ServerKey, message: String, cx: &mut Context<Self>) {
        let stderr_tail = self.stderr_tail(key.server);
        self.registry.set_slot(
            key.clone(),
            Slot::Failed {
                message,
                stderr_tail,
            },
        );
        self.forget(key, cx);
        cx.emit(LanguageServersEvent::StatusChanged);
        cx.notify();
    }

    fn stderr_tail(&self, server: &str) -> Vec<String> {
        let Some(log) = self.logs.get(server) else {
            return Vec::new();
        };
        let mut tail: Vec<String> = log
            .iter()
            .rev()
            .filter(|line| line.kind == LogKind::Stderr)
            .take(STDERR_TAIL)
            .map(|line| line.text.clone())
            .collect();
        tail.reverse();
        tail
    }

    fn push_log(&mut self, server: &'static str, kind: LogKind, text: String) {
        let log = self.logs.entry(server).or_default();
        if log.len() == LOG_LINES {
            log.pop_front();
        }
        log.push_back(LogLine { kind, text });
    }

    /// Queues `paths` for the next [`LanguageServersEvent::DiagnosticsChanged`],
    /// sent 50 ms after the first of a batch.
    fn diagnostics_changed(
        &mut self,
        paths: impl IntoIterator<Item = Arc<Path>>,
        cx: &mut Context<Self>,
    ) {
        let before = self.changed_diagnostics.len();
        self.changed_diagnostics.extend(paths);
        if self.changed_diagnostics.len() == before || self.diagnostics_flush_pending {
            return;
        }
        self.diagnostics_flush_pending = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DIAGNOSTICS_BATCH).await;
            this.update(cx, |servers, cx| {
                servers.diagnostics_flush_pending = false;
                let paths = std::mem::take(&mut servers.changed_diagnostics);
                if !paths.is_empty() {
                    cx.emit(LanguageServersEvent::DiagnosticsChanged(
                        paths.into_iter().collect(),
                    ));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn flush_watched_files(&mut self) {
        self.watched_flush_pending = false;
        let changes = std::mem::take(&mut self.watched_changes);
        if !self.trusted {
            return;
        }
        for (key, events) in self.watched.events_for(&changes) {
            if let Some(Slot::Running(server)) = self.registry.slot(&key) {
                server.notify::<DidChangeWatchedFiles>(DidChangeWatchedFilesParams {
                    changes: events,
                });
            }
        }
    }

    /// After any [`AppState`] change: stops servers no longer selected,
    /// applies `lsp` changes (restart, retry or reconfigure), and finds the
    /// servers again for documents whose language's selection changed.
    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        if !self.trusted {
            return;
        }
        let deselected: Vec<ServerKey> = self
            .registry
            .keys()
            .filter(|key| !self.selected_anywhere(key.server, cx))
            .cloned()
            .collect();
        let mut status_changed = !deselected.is_empty();
        for key in deselected {
            self.stop_server(&key, cx);
            self.registry.remove(&key);
        }
        let diff = self.registry.set_settings(servers_settings(cx));
        for key in diff.restart {
            match self.registry.slot(&key) {
                Some(Slot::Running(_) | Slot::Starting) => self.restart(&key, cx),
                Some(Slot::NotFound { .. } | Slot::Failed { .. }) => self.start(key, cx),
                Some(Slot::Stopped) | None => {}
            }
        }
        for (key, settings) in diff.reconfigure {
            if let Some(Slot::Running(server)) = self.registry.slot(&key) {
                server.set_settings(settings);
            }
        }
        let stale: Vec<Arc<Path>> = self
            .docs
            .iter()
            .filter(|(_, doc)| {
                doc.language_id.as_deref().map(|lang| selection(lang, cx)) != doc.selection
            })
            .map(|(path, _)| path.clone())
            .collect();
        for path in stale {
            status_changed = true;
            self.find_servers(&path, cx);
        }
        if status_changed {
            cx.emit(LanguageServersEvent::StatusChanged);
            cx.notify();
        }
    }

    /// Whether `server` is enabled and selected for any of its languages.
    fn selected_anywhere(&self, server: &str, cx: &App) -> bool {
        let Some(adapter) = self.registry.adapter(server) else {
            return false;
        };
        adapter.language_ids().iter().any(|lang| {
            let selection = selection(lang, cx);
            selection.enabled
                && select_servers(&selection.servers, &self.adapters, lang)
                    .iter()
                    .any(|a| a.name() == server)
        })
    }
}

/// `lucerna.lsp.enabled` and `lucerna.lsp.languageServers` for `language`.
fn selection(language: &str, cx: &App) -> Selection {
    let settings = &cx.global::<AppState>().settings;
    Selection {
        enabled: settings
            .get::<bool>("lucerna.lsp.enabled", Some(language))
            .unwrap_or(true),
        servers: settings
            .get::<Vec<String>>("lucerna.lsp.languageServers", Some(language))
            .unwrap_or_else(|| vec!["...".to_owned()]),
    }
}

/// The `lsp` setting from every layer. A server whose entry does not
/// parse is left out (it runs with defaults) and logged.
fn servers_settings(cx: &App) -> ServersSettings {
    let Some(Value::Object(map)) = cx.global::<AppState>().settings.get::<Value>("lsp", None)
    else {
        return ServersSettings::new();
    };
    map.into_iter()
        .filter_map(
            |(name, value)| match serde_json::from_value::<ServerSettings>(value) {
                Ok(settings) => Some((name, settings)),
                Err(err) => {
                    tracing::warn!(server = name, %err, "invalid lsp setting ignored");
                    None
                }
            },
        )
        .collect()
}

/// Resolves the binary and starts the process. Blocks (`which`, probe,
/// login-shell environment, spawn): runs on the background executor.
fn launch(
    adapter: &dyn Adapter,
    settings: Option<&ServerSettings>,
    root: &Path,
    launcher: &dyn Launcher,
) -> Result<Launched, StartError> {
    let spec = resolve_launch(adapter, settings, root, launcher)?;
    let server_settings = settings
        .and_then(|s| s.settings.clone())
        .unwrap_or(Value::Null);
    let (server, events) = LanguageServer::start(
        adapter.name(),
        |inbox| launcher.launch(&spec, inbox),
        server_settings.clone(),
    )
    .map_err(|err| StartError::Failed {
        message: format!("could not start {}: {err}", spec.program.display()),
        stderr_tail: Vec::new(),
    })?;
    Ok(Launched {
        server,
        events,
        spec,
        settings: server_settings,
    })
}

/// The adapter's `initializationOptions` with the user's on top: objects
/// merge key by key (recursively), anything else replaces.
fn merge_options(defaults: Option<Value>, overrides: Option<Value>) -> Option<Value> {
    match (defaults, overrides) {
        (Some(Value::Object(mut base)), Some(Value::Object(top))) => {
            for (key, value) in top {
                let merged = merge_options(base.remove(&key), Some(value));
                if let Some(merged) = merged {
                    base.insert(key, merged);
                }
            }
            Some(Value::Object(base))
        }
        (defaults, None) => defaults,
        (_, overrides) => overrides,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_options_overrides_by_key() {
        let merged = merge_options(
            Some(json!({ "a": 1, "nested": { "x": 1, "y": 2 } })),
            Some(json!({ "b": 2, "nested": { "y": 3 } })),
        );
        assert_eq!(
            merged,
            Some(json!({ "a": 1, "b": 2, "nested": { "x": 1, "y": 3 } }))
        );
        assert_eq!(merge_options(None, Some(json!(1))), Some(json!(1)));
        assert_eq!(merge_options(Some(json!({})), None), Some(json!({})));
    }

    #[test]
    fn progress_text() {
        let item = ProgressItem {
            token: "t".into(),
            title: Some("Indexing".into()),
            message: Some("crates/foo".into()),
            percentage: Some(45),
        };
        assert_eq!(item.text(), "Indexing 45% — crates/foo");
    }
}
