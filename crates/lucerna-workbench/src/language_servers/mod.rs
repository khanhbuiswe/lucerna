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
//!
//! Paths are the keys of [`Documents`] (absolute, joined onto the root,
//! not canonicalised). Servers echo the URIs they were sent, so a
//! diagnostic's `uri_to_path` meets the key it was opened under.

mod lifecycle;
mod pump;
mod sync;

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, Context, Entity, EventEmitter, Subscription, Task};
use lsp_types::FileChangeType;
use lucerna_lsp::diagnostics::DiagnosticStore;
use lucerna_lsp::sync::{DocumentSync, WatchedFiles};
use lucerna_lsp::{
    Adapter, LanguageServer, Launcher, LogLine, ProcessLauncher, Registry, ServerKey,
    ServerSettings, ServersSettings, Slot, builtin_adapters, select_servers,
};
use lucerna_text::BufferSnapshot;
use serde_json::Value;

use crate::AppState;
use crate::documents::Documents;
use crate::workspace_state::absolute_path;

use lifecycle::Starting;
use pump::{ProgressItem, Pump};

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
    /// Launched processes still waiting for `initialize`.
    starting: HashMap<ServerKey, Starting>,
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
            starting: HashMap::new(),
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
    /// sent to them. A relative `path` is joined onto the root, as
    /// [`Documents`] does.
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

    /// Whether a file of `language_id` would get a server: language
    /// servers are enabled for it and one is selected.
    pub fn wants_server(&self, _path: &Path, language_id: &str, cx: &App) -> bool {
        let selection = selection(language_id, cx);
        selection.enabled
            && !select_servers(&selection.servers, &self.adapters, language_id).is_empty()
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
