//! The bridge between open documents and language servers (spec §4, §6).
//!
//! One [`LanguageServers`] entity per workbench follows [`Documents`]:
//! when a document opens it finds the document's servers (root search off
//! the UI thread), starts missing ones in the background, and keeps them
//! in step with `didOpen` / `didChange` / `didClose`. Each server's events
//! are pumped onto the UI thread by a `cx.spawn` loop: diagnostics go into
//! a [`DiagnosticStore`], logs into a 2000-line ring per server.
//!
//! - `lifecycle`: starting, stopping and restarting servers; settings.
//! - `pump`: server events, diagnostics batches, logs and progress.
//! - `sync`: document events and file changes sent to the servers.
//! - `requests`: requests the editor sends (hover, locations).
//! - `relay`: the Vue `tsserver/request` relay.
//!
//! Paths are the keys of [`Documents`] (absolute, joined onto the root,
//! not canonicalised). Servers echo the URIs they were sent, so a
//! diagnostic's `uri_to_path` meets the key it was opened under.

mod lifecycle;
mod pump;
mod relay;
mod requests;
mod sync;

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use gpui::{App, Context, Entity, EventEmitter, Subscription, Task};
use lsp_types::FileChangeType;
use lucerna_lsp::diagnostics::DiagnosticStore;
use lucerna_lsp::sync::{DocumentSync, WatchedFiles};
use lucerna_lsp::{
    LanguageServer, Launcher, LogLine, ProcessLauncher, Registry, ServerKey, ServerTable, Slot,
    TableError, select_servers,
};
use lucerna_text::BufferSnapshot;
use serde_json::Value;

use crate::AppState;
use crate::documents::Documents;
use crate::tasks::guarded;
use crate::workspace_state::absolute_path;

use lifecycle::Starting;
use pump::{ProgressItem, Pump};

pub use requests::HoverInfo;

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
    /// `window/showMessage` errors since the server last started.
    pub errors: Vec<String>,
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
    /// A line was added to the log of this server.
    LogChanged(Arc<str>),
    /// [`LanguageServers::table`] was built again from changed settings
    /// or a trust change.
    TableChanged,
    /// The folder was trusted or untrusted ([`LanguageServers::is_trusted`]),
    /// whether or not the table changed with it.
    TrustChanged,
}

/// The settings a file's servers are picked by: its language's, or for a
/// file without a language the global `lucerna.lsp.enabled` and `...`.
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
    /// Over `lucerna.largeFile.*` (or with a very long line) when its
    /// servers were last looked for: it has none until it shrinks.
    too_large: bool,
}

pub struct LanguageServers {
    root: PathBuf,
    documents: Entity<Documents>,
    launcher: Arc<dyn Launcher>,
    /// The base of each server's `{data_dir}`.
    data_dir: PathBuf,
    /// `npm root -g`, found the first time a server uses `{npm_root}`
    /// (off the UI thread); `None` inside when npm failed.
    npm_root: Arc<OnceLock<Option<PathBuf>>>,
    /// The `lsp` value the registry's table was built from (the trusted
    /// layers only while untrusted).
    lsp_setting: Option<Value>,
    trusted: bool,
    /// Whether the folder's trust is known (the store is read, or
    /// `session.trustAllWorktrees` is set). Until then Restricted Mode is
    /// not shown, so a trusted folder does not flash it at startup.
    trust_known: bool,
    registry: Registry,
    /// Bumped per start or stop of a key: results and events of an older
    /// process are dropped (its log lines are kept).
    generations: HashMap<ServerKey, u64>,
    /// Launched processes still waiting for `initialize`.
    starting: HashMap<ServerKey, Starting>,
    /// The event pump of each key's current process.
    pumps: HashMap<ServerKey, Pump>,
    sync: DocumentSync,
    watched: WatchedFiles,
    diagnostics: DiagnosticStore,
    /// `(errors, warnings)` over every file, as of the last diagnostics
    /// batch: the status bar reads it every frame.
    diagnostic_counts: (usize, usize),
    docs: HashMap<Arc<Path>, OpenDoc>,
    logs: HashMap<Arc<str>, VecDeque<LogLine>>,
    progress: HashMap<ServerKey, Vec<ProgressItem>>,
    /// Ticks per progress update; orders progress across servers.
    progress_clock: u64,
    /// `window/showMessage` errors per server, until it starts again.
    errors: HashMap<ServerKey, Vec<String>>,
    changed_diagnostics: BTreeSet<Arc<Path>>,
    diagnostics_flush_pending: bool,
    watched_changes: Vec<(PathBuf, FileChangeType)>,
    watched_flush_pending: bool,
    /// Whether each server command is on `PATH`, as found by
    /// [`LanguageServers::server_binary_found`].
    on_path: HashMap<String, bool>,
    /// For files outside the workspace found in a server's answer
    /// (definition, references, workspace symbols): that server, which the
    /// file joins when it is opened.
    origins: HashMap<PathBuf, ServerKey>,
    /// `tsserver/request`s waiting for a starting TypeScript server.
    relay_waiting: Vec<relay::WaitingRelay>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LanguageServersEvent> for LanguageServers {}

impl LanguageServers {
    /// The servers of the folder at `root`, following `documents`. Starts
    /// processes with a [`ProcessLauncher`]. Trusted when the User setting
    /// `session.trustAllWorktrees` is `true`; untrusted, nothing starts.
    /// `data_dir` is the app's data directory: servers' `{data_dir}` lie
    /// under its `lsp/`.
    pub fn new(
        root: PathBuf,
        documents: &Entity<Documents>,
        data_dir: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        let trusted = cx
            .global::<AppState>()
            .settings
            .get_trusted::<bool>("session.trustAllWorktrees", None)
            == Some(true);
        lifecycle::QuitShutdown::register(cx);
        let lsp_setting = lsp_setting(trusted, cx);
        let registry = Registry::new(ServerTable::build(lsp_setting.as_ref()));
        LanguageServers {
            root,
            documents: documents.clone(),
            launcher: Arc::new(ProcessLauncher::new()),
            data_dir,
            npm_root: Arc::default(),
            lsp_setting,
            trusted,
            trust_known: trusted,
            registry,
            generations: HashMap::new(),
            starting: HashMap::new(),
            pumps: HashMap::new(),
            sync: DocumentSync::default(),
            watched: WatchedFiles::default(),
            diagnostics: DiagnosticStore::default(),
            diagnostic_counts: (0, 0),
            docs: HashMap::new(),
            logs: HashMap::new(),
            progress: HashMap::new(),
            progress_clock: 0,
            errors: HashMap::new(),
            changed_diagnostics: BTreeSet::new(),
            diagnostics_flush_pending: false,
            watched_changes: Vec::new(),
            watched_flush_pending: false,
            on_path: HashMap::new(),
            origins: HashMap::new(),
            relay_waiting: Vec::new(),
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

    /// How servers are found and started.
    pub fn launcher(&self) -> Arc<dyn Launcher> {
        self.launcher.clone()
    }

    /// Untrusted stops every server; trusted sends every open document
    /// to its servers, starting them.
    pub fn set_trusted(&mut self, trusted: bool, cx: &mut Context<Self>) {
        if trusted == self.trusted {
            return;
        }
        self.trusted = trusted;
        let table_changed;
        if trusted {
            // No slot exists while untrusted: nothing to restart.
            table_changed = self.rebuild_table(cx).is_some();
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
            table_changed = self.rebuild_table(cx).is_some();
        }
        if table_changed {
            cx.emit(LanguageServersEvent::TableChanged);
        }
        cx.emit(LanguageServersEvent::TrustChanged);
        cx.emit(LanguageServersEvent::StatusChanged);
        cx.notify();
    }

    pub fn is_trusted(&self) -> bool {
        self.trusted
    }

    /// The server table: the built-in servers merged with the `lsp`
    /// setting (without the Workspace layer while untrusted).
    pub fn table(&self) -> &ServerTable {
        self.registry.table()
    }

    /// The `lsp` entries left out of [`LanguageServers::table`], and why.
    pub fn table_errors(&self) -> &[TableError] {
        &self.registry.table().errors
    }

    /// [`LanguageServers::table_errors`] as shown in the ⚡ popover and the
    /// Health tab: "Invalid `lsp.<name>`: <reason>", one line each.
    pub fn table_error_lines(&self) -> Vec<String> {
        self.table_errors()
            .iter()
            .map(|e| format!("Invalid `lsp.{}`: {}", e.server, e.message))
            .collect()
    }

    /// Builds the table from the settings again when the `lsp` value
    /// changed. Returns the old and new table's difference for the slots,
    /// or `None` when nothing changed.
    pub(super) fn rebuild_table(&mut self, cx: &App) -> Option<lucerna_lsp::SettingsDiff> {
        let lsp = lsp_setting(self.trusted, cx);
        if lsp == self.lsp_setting {
            return None;
        }
        let table = ServerTable::build(lsp.as_ref());
        self.lsp_setting = lsp;
        Some(self.registry.set_table(table))
    }

    /// Whether the folder's trust is known yet (see the field).
    pub fn trust_known(&self) -> bool {
        self.trust_known
    }

    /// Whether the window shows Restricted Mode: untrusted, and known.
    pub fn shows_restricted_mode(&self) -> bool {
        self.trust_known && !self.trusted
    }

    pub fn set_trust_known(&mut self, known: bool, cx: &mut Context<Self>) {
        if self.trust_known != known {
            self.trust_known = known;
            cx.emit(LanguageServersEvent::StatusChanged);
            cx.notify();
        }
    }

    /// Whether any server was selected for some file (any slot exists).
    /// Cheap, for per-frame checks; [`LanguageServers::statuses`] builds
    /// the full list.
    pub fn has_servers(&self) -> bool {
        self.registry.keys().next().is_some()
    }

    /// Whether some server failed to start or crashed.
    pub fn any_failed(&self) -> bool {
        self.registry
            .keys()
            .any(|key| matches!(self.registry.slot(key), Some(Slot::Failed { .. })))
    }

    /// Whether some server's binary was not found.
    pub fn any_not_found(&self) -> bool {
        self.registry
            .keys()
            .any(|key| matches!(self.registry.slot(key), Some(Slot::NotFound { .. })))
    }

    /// `(errors, warnings)` over every file, as of the last 50 ms
    /// diagnostics batch (cached; no walk over the store).
    pub fn diagnostic_counts(&self) -> (usize, usize) {
        self.diagnostic_counts
    }

    /// Every slot, ordered by root then server name.
    pub fn statuses(&self) -> Vec<ServerStatus> {
        let mut keys: Vec<&ServerKey> = self.registry.keys().collect();
        keys.sort_by(|a, b| (&a.root, &a.server).cmp(&(&b.root, &b.server)));
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
                    errors: self.errors.get(key).cloned().unwrap_or_default(),
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

    /// How many lines the log of `server` holds.
    pub fn log_len(&self, server: &str) -> usize {
        self.logs.get(server).map_or(0, VecDeque::len)
    }

    /// Lines `range` of the log of `server` (clamped to it).
    pub fn log_range(&self, server: &str, range: Range<usize>) -> Vec<LogLine> {
        let Some(log) = self.logs.get(server) else {
            return Vec::new();
        };
        let end = range.end.min(log.len());
        log.range(range.start.min(end)..end).cloned().collect()
    }

    /// The servers found for the open document `path` (any state). A
    /// relative `path` is joined onto the root.
    pub fn servers_for(&self, path: &Path) -> Vec<ServerKey> {
        let path = absolute_path(&self.root, path);
        self.docs
            .get(path.as_path())
            .map(|doc| doc.keys.clone())
            .unwrap_or_default()
    }

    pub fn diagnostics(&self) -> &DiagnosticStore {
        &self.diagnostics
    }

    /// The running servers that have `path` open, or will once it is
    /// sent to them. A relative `path` is joined onto the root, as
    /// [`Documents`] does.
    /// Remembers which server answered with files outside the workspace,
    /// so opening one attaches it to that server (see `find_servers`).
    fn note_origins(&mut self, found: Vec<(PathBuf, ServerKey)>) {
        /// Bounds the map over a long session; it only steers a choice.
        const MAX_ORIGINS: usize = 4096;
        for (path, key) in found {
            if path.starts_with(&self.root) {
                continue;
            }
            if self.origins.len() >= MAX_ORIGINS {
                self.origins.clear();
            }
            self.origins.insert(path, key);
        }
    }

    pub fn running_for(&self, path: &Path) -> Vec<(ServerKey, Arc<LanguageServer>)> {
        let path = absolute_path(&self.root, path);
        let Some(doc) = self.docs.get(path.as_path()) else {
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

    /// Whether the file `path` (of `language`, if any) would get a server
    /// that can start: language servers are enabled for it, and a server
    /// selected from the table of the trusted layers (never the folder's
    /// own settings) has `binary.path` set or its command on `PATH`.
    /// `PATH` is searched off the UI thread, once per command per session.
    pub fn server_binary_found(
        &mut self,
        language: Option<&str>,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let selection = selection(language, cx);
        if !selection.enabled {
            return Task::ready(false);
        }
        let table = ServerTable::build(lsp_setting(false, cx).as_ref());
        let defs = select_servers(&selection.servers, &table, language, path);
        let has_path = defs.iter().any(|def| {
            def.binary
                .as_ref()
                .and_then(|b| b.path.as_ref())
                .is_some_and(|p| !p.as_os_str().is_empty())
        });
        let commands: Vec<String> = defs.iter().filter_map(|d| d.command.clone()).collect();
        if has_path || commands.iter().any(|c| self.on_path.get(c) == Some(&true)) {
            return Task::ready(true);
        }
        let mut unknown: Vec<String> = commands
            .into_iter()
            .filter(|c| !self.on_path.contains_key(c))
            .collect();
        unknown.sort_unstable();
        unknown.dedup();
        if unknown.is_empty() {
            return Task::ready(false);
        }
        let launcher = self.launcher.clone();
        let work = cx.background_executor().spawn(async move {
            guarded("lsp-which", move || {
                unknown
                    .into_iter()
                    .map(|command| {
                        let found = launcher.which(&command).is_some();
                        (command, found)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
        });
        cx.spawn(async move |this, cx| {
            let results = work.await;
            this.update(cx, |servers, _| {
                let found = results.iter().any(|(_, found)| *found);
                servers.on_path.extend(results);
                found
            })
            .unwrap_or(false)
        })
    }
}

/// The `lsp` setting the table is built from: every layer once the folder
/// is trusted, without the Workspace layer before.
fn lsp_setting(trusted: bool, cx: &App) -> Option<Value> {
    let settings = &cx.global::<AppState>().settings;
    if trusted {
        settings.get::<Value>("lsp", None)
    } else {
        settings.get_trusted::<Value>("lsp", None)
    }
}

/// `lucerna.lsp.enabled` and `lucerna.lsp.languageServers` for
/// `language`; for a file without a language, the global
/// `lucerna.lsp.enabled` and `["..."]`.
fn selection(language: Option<&str>, cx: &App) -> Selection {
    let settings = &cx.global::<AppState>().settings;
    let servers = language.and_then(|language| {
        settings.get::<Vec<String>>("lucerna.lsp.languageServers", Some(language))
    });
    Selection {
        enabled: settings
            .get::<bool>("lucerna.lsp.enabled", language)
            .unwrap_or(true),
        servers: servers.unwrap_or_else(|| vec!["...".to_owned()]),
    }
}
