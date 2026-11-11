//! Starting, stopping and restarting servers, shutting them all down, and
//! applying settings changes to the slots.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::task::Poll;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::channel::mpsc::UnboundedReceiver;
use futures::future::BoxFuture;
use gpui::{App, Context, Task};
use lucerna_lsp::sync::{DocumentSync, WatchedFiles};
use lucerna_lsp::transport::LaunchSpec;
use lucerna_lsp::{
    Adapter, LanguageServer, Launcher, LogKind, LspError, ServerEvent, ServerKey, ServerSettings,
    Slot, StartError, initialize_params, resolve_launch, select_servers,
};
use serde_json::Value;

use super::{LanguageServers, LanguageServersEvent, selection, servers_settings};
use crate::tasks::{guarded, try_spawn_guarded};

/// The most [`LanguageServers::shutdown_all`] waits, for all servers.
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(5);

type Initialize = BoxFuture<'static, Result<(), LspError>>;

/// A started process, before `initialize`.
struct Launched {
    server: Arc<LanguageServer>,
    events: UnboundedReceiver<ServerEvent>,
    spec: LaunchSpec,
    /// `lsp.<server>.settings` the server was started with.
    settings: Value,
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
    /// for them, at most 5 s in all. For app quit.
    pub fn shutdown_all(&mut self) {
        let deadline = Instant::now() + SHUTDOWN_BUDGET;
        for handle in self.begin_shutdown_all() {
            while !handle.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
    }

    /// Like [`LanguageServers::shutdown_all`] without waiting: the
    /// shutdowns are joined on a background thread. For window close.
    pub fn shutdown_all_in_background(&mut self) {
        let handles = self.begin_shutdown_all();
        if handles.is_empty() {
            return;
        }
        let joined = try_spawn_guarded("lsp-shutdown-all", move || {
            for handle in handles {
                let _ = handle.join();
            }
        });
        if let Err(err) = joined {
            tracing::error!(%err, "cannot start the language server shutdown thread");
        }
    }

    /// Starts every server's shutdown; every slot ends `Stopped`.
    fn begin_shutdown_all(&mut self) -> Vec<JoinHandle<()>> {
        let keys: Vec<ServerKey> = self.registry.keys().cloned().collect();
        let mut handles = Vec::new();
        for key in keys {
            self.bump_generation(&key);
            self.pumps.remove(&key);
            if let Some(server) = self.take_starting(&key) {
                handles.push(server.shutdown());
            }
            if let Some(Slot::Running(server)) = self.registry.remove(&key) {
                handles.push(server.shutdown());
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
        let Some(adapter) = self.registry.adapter(key.server) else {
            return;
        };
        let generation = self.bump_generation(&key);
        self.registry.set_slot(key.clone(), Slot::Starting);
        self.progress.remove(&key);
        self.errors.remove(&key);
        let settings = self.registry.settings(key.server).cloned();
        let (launcher, root) = (self.launcher.clone(), key.root.clone());
        let work = cx.background_executor().spawn(async move {
            guarded("lsp-launch", move || {
                launch(&*adapter, settings.as_ref(), &root, &*launcher)
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
                self.push_log(key.server, LogKind::Lifecycle, message.clone(), cx);
                self.registry
                    .set_slot(key.clone(), Slot::NotFound { message });
                cx.emit(LanguageServersEvent::StatusChanged);
                cx.notify();
                return;
            }
            Some(Err(StartError::Failed { message, .. })) => {
                self.push_log(key.server, LogKind::Lifecycle, message.clone(), cx);
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
        } = launched;
        let mut command = spec.program.display().to_string();
        for arg in &spec.args {
            command.push(' ');
            command.push_str(arg);
        }
        self.push_log(
            key.server,
            LogKind::Lifecycle,
            format!("Binary: {command}"),
            cx,
        );
        self.push_log(
            key.server,
            LogKind::Lifecycle,
            format!("Root: {}", key.root.display()),
            cx,
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
        let initialize = Rc::new(RefCell::new(Some(
            server
                .initialize(initialize_params(&key.root, options))
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

    /// After any [`crate::AppState`] change: stops servers no longer
    /// selected, applies `lsp` changes (restart, retry or reconfigure), and
    /// finds the servers again for documents whose language's selection
    /// changed.
    pub(super) fn settings_changed(&mut self, cx: &mut Context<Self>) {
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
}
