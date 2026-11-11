//! Starting, stopping and restarting servers, shutting them all down, and
//! applying settings changes to the slots.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::task::Poll;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::channel::mpsc::UnboundedReceiver;
use futures::future::BoxFuture;
use gpui::{App, Context, Global, Task, WeakEntity};
use lucerna_lsp::sync::{DocumentSync, WatchedFiles};
use lucerna_lsp::transport::LaunchSpec;
use lucerna_lsp::vars::{self, Vars, data_dir_for, expand_value, uses_npm_root, uses_variable};
use lucerna_lsp::{
    LanguageServer, Launcher, LogKind, LspError, ServerDef, ServerEvent, ServerKey, Slot,
    StartError, initialize_params, resolve_launch, select_servers,
};
use serde_json::Value;

use super::{LanguageServers, LanguageServersEvent, selection};
use crate::tasks::{guarded, try_spawn_guarded};

/// The most [`LanguageServers::shutdown_all`] waits, for all servers (and,
/// on quit, for every window's servers together).
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(5);

/// A server whose shutdown has begun, and the thread running it (`None`
/// when the thread could not start and the process was killed).
type ShuttingDown = (Arc<LanguageServer>, Option<JoinHandle<()>>);

/// Every window's [`LanguageServers`], so the first quit hook shuts them all
/// down against one deadline instead of 5 s per window.
#[derive(Default)]
pub(super) struct QuitShutdown {
    servers: Vec<WeakEntity<LanguageServers>>,
    done: bool,
}

impl Global for QuitShutdown {}

impl QuitShutdown {
    pub(super) fn register(cx: &mut Context<LanguageServers>) {
        let entity = cx.weak_entity();
        let global = cx.default_global::<QuitShutdown>();
        global.servers.retain(|s| s.upgrade().is_some());
        global.servers.push(entity);
    }
}

/// Waits for the shutdown threads until `deadline`, then kills the servers
/// whose shutdown has not finished, so none outlives the app.
fn finish_or_kill(pending: Vec<ShuttingDown>, deadline: Instant) {
    for (server, handle) in pending {
        let Some(handle) = handle else { continue };
        while !handle.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        if !handle.is_finished() {
            tracing::warn!(
                server = server.name(),
                "language server did not shut down in time; killing it"
            );
            server.kill();
        }
        // The kill closes the transport, which ends the shutdown thread.
        let _ = handle.join();
    }
}

type Initialize = BoxFuture<'static, Result<(), LspError>>;

/// A started process, before `initialize`.
struct Launched {
    server: Arc<LanguageServer>,
    events: UnboundedReceiver<ServerEvent>,
    spec: LaunchSpec,
    /// `lsp.<server>.settings` the server was started with, expanded.
    settings: Value,
    /// `lsp.<server>.initialization_options`, expanded.
    initialization_options: Option<Value>,
}

/// A launched process waiting for `initialize`, kept so stopping the slot
/// can shut it down. Dropping it drops the pending `initialize` request
/// first, so the shutdown thread's reply cannot wake the UI executor.
pub(super) struct Starting {
    server: Arc<LanguageServer>,
    initialize: Rc<RefCell<Option<Initialize>>>,
    _task: Task<()>,
}

impl Drop for Starting {
    fn drop(&mut self) {
        self.initialize.borrow_mut().take();
    }
}

impl LanguageServers {
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

    /// Shuts every server (running or starting) down in parallel and waits
    /// for them, at most 5 s in all; servers still running then are killed.
    pub fn shutdown_all(&mut self) {
        let deadline = Instant::now() + SHUTDOWN_BUDGET;
        finish_or_kill(self.begin_shutdown_all(), deadline);
    }

    /// For app quit: shuts down the servers of every window at once and
    /// waits at most 5 s in all, then kills the ones still running. Later
    /// calls (one quit hook per window) do nothing.
    pub fn shutdown_all_windows_on_quit(cx: &mut App) {
        Self::shutdown_all_windows_within(SHUTDOWN_BUDGET, cx);
    }

    /// [`LanguageServers::shutdown_all_windows_on_quit`] with a custom
    /// budget (tests).
    pub fn shutdown_all_windows_within(budget: Duration, cx: &mut App) {
        let deadline = Instant::now() + budget;
        let global = cx.default_global::<QuitShutdown>();
        if std::mem::replace(&mut global.done, true) {
            return;
        }
        let entities = std::mem::take(&mut global.servers);
        let mut pending = Vec::new();
        for entity in entities.iter().filter_map(WeakEntity::upgrade) {
            pending.extend(entity.update(cx, |servers, _| servers.begin_shutdown_all()));
        }
        finish_or_kill(pending, deadline);
    }

    /// Like [`LanguageServers::shutdown_all`] without waiting: the
    /// shutdowns are joined on a background thread. For window close.
    pub fn shutdown_all_in_background(&mut self) {
        let pending = self.begin_shutdown_all();
        if pending.is_empty() {
            return;
        }
        let joined = try_spawn_guarded("lsp-shutdown-all", move || {
            for (_, handle) in pending {
                if let Some(handle) = handle {
                    let _ = handle.join();
                }
            }
        });
        if let Err(err) = joined {
            tracing::error!(%err, "cannot start the language server shutdown thread");
        }
    }

    /// Starts every server's shutdown; every slot ends `Stopped`.
    fn begin_shutdown_all(&mut self) -> Vec<ShuttingDown> {
        let keys: Vec<ServerKey> = self.registry.keys().cloned().collect();
        let mut handles = Vec::new();
        for key in keys {
            self.bump_generation(&key);
            self.pumps.remove(&key);
            if let Some(server) = self.take_starting(&key) {
                let handle = server.shutdown();
                handles.push((server, handle));
            }
            if let Some(Slot::Running(server)) = self.registry.remove(&key) {
                let handle = server.shutdown();
                handles.push((server, handle));
            }
            self.registry.set_slot(key, Slot::Stopped);
        }
        self.sync = DocumentSync::default();
        self.watched = WatchedFiles::default();
        handles
    }

    pub(super) fn bump_generation(&mut self, key: &ServerKey) -> u64 {
        let generation = self.generations.entry(key.clone()).or_default();
        *generation += 1;
        *generation
    }

    pub(super) fn is_current(&self, key: &ServerKey, generation: u64) -> bool {
        self.generations.get(key) == Some(&generation)
    }

    /// Removes the process of `key` that waits for `initialize`, dropping
    /// its pending request.
    fn take_starting(&mut self, key: &ServerKey) -> Option<Arc<LanguageServer>> {
        let starting = self.starting.remove(key)?;
        let server = starting.server.clone();
        drop(starting);
        Some(server)
    }

    /// Finds the binary and spawns the server in the background, then
    /// awaits `initialize`. The slot is `Starting` meanwhile.
    pub(super) fn start(&mut self, key: ServerKey, cx: &mut Context<Self>) {
        let Some(def) = self.registry.table().get(&key.server).cloned() else {
            return;
        };
        let generation = self.bump_generation(&key);
        self.registry.set_slot(key.clone(), Slot::Starting);
        self.progress.remove(&key);
        self.errors.remove(&key);
        let (launcher, root) = (self.launcher.clone(), key.root.clone());
        let (data_dir, npm_root) = (self.data_dir.clone(), self.npm_root.clone());
        let work = cx.background_executor().spawn(async move {
            guarded("lsp-launch", move || {
                launch(&def, &root, &*launcher, &data_dir, &npm_root)
            })
        });
        cx.spawn(async move |this, cx| {
            let launched = work.await;
            this.update(cx, |servers, cx| {
                servers.launched(&key, generation, launched, cx)
            })
            .ok();
        })
        .detach();
        cx.emit(LanguageServersEvent::StatusChanged);
        cx.notify();
    }

    /// The process is up (or could not start): pumps its events and sends
    /// `initialize`, awaited by a task kept in [`Starting`].
    fn launched(
        &mut self,
        key: &ServerKey,
        generation: u64,
        launched: Option<Result<Launched, StartError>>,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current(key, generation) {
            if let Some(Ok(Launched { server, events, .. })) = launched {
                drop(events);
                let _ = server.shutdown();
            }
            return;
        }
        let launched = match launched {
            Some(Ok(launched)) => launched,
            Some(Err(StartError::NotFound { message })) => {
                self.push_log(&key.server, LogKind::Lifecycle, message.clone(), cx);
                self.registry
                    .set_slot(key.clone(), Slot::NotFound { message });
                self.flush_relay(None, cx);
                cx.emit(LanguageServersEvent::StatusChanged);
                cx.notify();
                return;
            }
            Some(Err(StartError::Failed { message, .. })) => {
                self.push_log(&key.server, LogKind::Lifecycle, message.clone(), cx);
                self.set_failed(key, message, cx);
                return;
            }
            // The panic is logged by `guarded`.
            None => {
                self.set_failed(key, "internal error while starting".into(), cx);
                return;
            }
        };
        let Launched {
            server,
            events,
            spec,
            settings,
            initialization_options,
        } = launched;
        let mut command = spec.program.display().to_string();
        for arg in &spec.args {
            command.push(' ');
            command.push_str(arg);
        }
        self.push_log(
            &key.server,
            LogKind::Lifecycle,
            format!("Binary: {command}"),
            cx,
        );
        self.push_log(
            &key.server,
            LogKind::Lifecycle,
            format!("Root: {}", key.root.display()),
            cx,
        );
        self.pump(key.clone(), generation, Arc::downgrade(&server), events, cx);
        let initialize = Rc::new(RefCell::new(Some(
            server
                .initialize(initialize_params(&key.root, initialization_options))
                .boxed(),
        )));
        let (pending, task_key) = (initialize.clone(), key.clone());
        let task = cx.spawn(async move |this, cx| {
            let result = futures::future::poll_fn(|task_cx| match pending.borrow_mut().as_mut() {
                Some(initialize) => initialize.poll_unpin(task_cx).map(Some),
                None => Poll::Ready(None),
            })
            .await;
            let Some(result) = result else {
                return;
            };
            this.update(cx, |servers, cx| {
                servers.initialized(&task_key, generation, settings, result, cx)
            })
            .ok();
        });
        self.starting.insert(
            key.clone(),
            Starting {
                server,
                initialize,
                _task: task,
            },
        );
    }

    fn initialized(
        &mut self,
        key: &ServerKey,
        generation: u64,
        started_with: Value,
        result: Result<(), LspError>,
        cx: &mut Context<Self>,
    ) {
        // A stop or a failure took the process out of `starting` (and shut
        // it down) already.
        if !self.is_current(key, generation) {
            return;
        }
        let Some(server) = self.take_starting(key) else {
            return;
        };
        if !matches!(self.registry.slot(key), Some(Slot::Starting)) {
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
        if let Some(settings) = self.current_settings(key)
            && settings != started_with
        {
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
        // Files outside the workspace opened before any server ran can
        // join this one now.
        let waiting: Vec<Arc<Path>> = self
            .docs
            .iter()
            .filter(|(path, doc)| doc.keys.is_empty() && !path.starts_with(&self.root))
            .map(|(path, _)| path.clone())
            .collect();
        for path in waiting {
            self.find_servers(&path, cx);
        }
        // After `didOpen`: tsserver answers relayed requests about open
        // files only.
        self.flush_relay(None, cx);
        cx.notify();
    }

    /// Ends what the server of `key` had: open documents, watch
    /// registrations, progress and diagnostics. Shuts a running or
    /// starting process down; events from it are ignored from now on.
    pub(super) fn stop_server(&mut self, key: &ServerKey, cx: &mut Context<Self>) {
        self.bump_generation(key);
        self.pumps.remove(key);
        if let Some(server) = self.take_starting(key) {
            let _ = server.shutdown();
        }
        if let Some(Slot::Running(server)) = self.registry.slot(key) {
            let _ = server.shutdown();
        }
        self.forget(key, cx);
        self.flush_relay(Some(key), cx);
    }

    fn forget(&mut self, key: &ServerKey, cx: &mut Context<Self>) {
        self.sync.forget_server(key);
        self.watched.forget_server(key);
        self.progress.remove(key);
        let cleared = self.diagnostics.clear_server(key);
        self.diagnostics_changed(cleared.into_iter().map(Arc::from), cx);
    }

    /// The process is gone or never came up.
    pub(super) fn set_failed(&mut self, key: &ServerKey, message: String, cx: &mut Context<Self>) {
        self.starting.remove(key);
        let stderr_tail = self.stderr_tail(&key.server);
        self.registry.set_slot(
            key.clone(),
            Slot::Failed {
                message,
                stderr_tail,
            },
        );
        self.forget(key, cx);
        self.flush_relay(None, cx);
        cx.emit(LanguageServersEvent::StatusChanged);
        cx.notify();
    }

    /// The variables of the server of `key`. `{npm_root}` has a value
    /// only once some server's start has looked it up.
    pub(super) fn vars_for(&self, key: &ServerKey) -> Vars {
        Vars {
            root: key.root.to_path_buf(),
            data_dir: data_dir_for(&self.data_dir, &key.server, &key.root),
            npm_root: self.npm_root.get().cloned().flatten(),
        }
    }

    /// `lsp.<server>.settings` of `key` from the table, expanded (`Null`
    /// when unset); `None` when a variable cannot be expanded.
    fn current_settings(&self, key: &ServerKey) -> Option<Value> {
        let def = self.registry.table().get(&key.server)?;
        let raw = def.settings.clone().unwrap_or(Value::Null);
        expand_value(&raw, &self.vars_for(key))
            .inspect_err(|err| tracing::warn!(server = &*key.server, %err, "lsp settings"))
            .ok()
    }

    /// After any [`crate::AppState`] change: rebuilds the server table
    /// when the `lsp` setting changed and applies the difference (stop,
    /// restart, retry or reconfigure), stops servers no longer selected,
    /// and finds the servers again for documents whose selection or table
    /// changed.
    pub(super) fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let old_errors = self.registry.table().errors.clone();
        let diff = self.rebuild_table(cx);
        let table_changed = diff.is_some();
        if table_changed {
            cx.emit(LanguageServersEvent::TableChanged);
        }
        let mut status_changed = old_errors != self.registry.table().errors;
        if !self.trusted {
            // No slot exists while untrusted.
            if status_changed {
                cx.emit(LanguageServersEvent::StatusChanged);
                cx.notify();
            }
            return;
        }
        let diff = diff.unwrap_or_default();
        for key in &diff.stop {
            status_changed = true;
            self.stop_server(key, cx);
            self.registry.remove(key);
        }
        let deselected: Vec<ServerKey> = self
            .registry
            .keys()
            .filter(|key| !self.selected_anywhere(&key.server, cx))
            .cloned()
            .collect();
        status_changed |= !deselected.is_empty();
        for key in deselected {
            self.stop_server(&key, cx);
            self.registry.remove(&key);
        }
        for key in diff.restart {
            match self.registry.slot(&key) {
                Some(Slot::Running(_) | Slot::Starting) => self.restart(&key, cx),
                Some(Slot::NotFound { .. } | Slot::Failed { .. }) => self.start(key, cx),
                Some(Slot::Stopped) | None => {}
            }
        }
        for (key, _) in diff.reconfigure {
            let Some(Slot::Running(server)) = self.registry.slot(&key) else {
                continue;
            };
            match self.current_settings(&key) {
                Some(settings) => server.set_settings(settings),
                // A variable (`{npm_root}`) not known yet: a start finds it,
                // or fails with its message.
                None => self.restart(&key, cx),
            }
        }
        let stale: Vec<Arc<Path>> = self
            .docs
            .iter()
            .filter(|(_, doc)| {
                table_changed || Some(selection(doc.language_id.as_deref(), cx)) != doc.selection
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

    /// Whether `server` is in the table and selected for any of its
    /// languages, or (for its file types) enabled with language servers
    /// on.
    fn selected_anywhere(&self, server: &str, cx: &App) -> bool {
        let table = self.registry.table();
        let Some(def) = table.get(server) else {
            return false;
        };
        let by_file_type =
            || !def.file_types.is_empty() && def.enabled && selection(None, cx).enabled;
        by_file_type()
            || def.languages.iter().any(|lang| {
                let selection = selection(Some(lang), cx);
                selection.enabled
                    && select_servers(&selection.servers, table, Some(lang), Path::new(""))
                        .iter()
                        .any(|d| &*d.name == server)
            })
    }
}

fn var_failed(err: &vars::VarError) -> StartError {
    StartError::Failed {
        message: err.to_string(),
        stderr_tail: Vec::new(),
    }
}

/// Resolves the binary, expands the variables (looking `npm root -g` up
/// once, when `def` needs it; creating `{data_dir}` when used) and starts
/// the process. Blocks (`which`, probe, `npm`, login-shell environment,
/// spawn): runs on the background executor.
fn launch(
    def: &ServerDef,
    root: &Path,
    launcher: &dyn Launcher,
    data_dir: &Path,
    npm_root: &OnceLock<Option<PathBuf>>,
) -> Result<Launched, StartError> {
    let npm_root = if uses_npm_root(def) {
        npm_root.get_or_init(|| vars::npm_root(launcher)).clone()
    } else {
        npm_root.get().cloned().flatten()
    };
    let vars = Vars {
        root: root.to_path_buf(),
        data_dir: data_dir_for(data_dir, &def.name, root),
        npm_root,
    };
    let spec = resolve_launch(def, root, launcher, &vars)?;
    let expand = |value: Option<&Value>| {
        value
            .map(|v| expand_value(v, &vars))
            .transpose()
            .map_err(|err| var_failed(&err))
    };
    let initialization_options = expand(def.initialization_options.as_ref())?;
    let server_settings = expand(def.settings.as_ref())?.unwrap_or(Value::Null);
    if uses_variable(def, "data_dir") {
        std::fs::create_dir_all(&vars.data_dir).map_err(|err| StartError::Failed {
            message: format!("could not create {}: {err}", vars.data_dir.display()),
            stderr_tail: Vec::new(),
        })?;
    }
    let (server, events) = LanguageServer::start(
        &def.name,
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
        initialization_options,
    })
}
