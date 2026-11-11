//! JSON-RPC client for one language server.
//!
//! Messages are parsed on the transport's reader thread. Responses resolve
//! their pending request there; server requests are answered there too
//! (§3.1 auto-reply table), and notifications become [`ServerEvent`]s.
//!
//! A transport may deliver messages re-entrantly from inside `send` (the
//! in-process fake does), so no lock is ever held across `Transport::send`
//! and a request is registered before it is sent.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::io;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::task::{Context, Poll, ready};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::channel::oneshot;
use lsp_types::notification::{
    Cancel, DidChangeConfiguration, DidChangeWatchedFiles, Exit, Initialized, LogMessage,
    Notification, Progress, PublishDiagnostics, ShowMessage,
};
use lsp_types::request::{
    ApplyWorkspaceEdit, Initialize, RegisterCapability, Request, ShowDocument, ShowMessageRequest,
    Shutdown, UnregisterCapability, WorkDoneProgressCreate, WorkspaceConfiguration,
    WorkspaceFoldersRequest,
};
use lsp_types::{
    DidChangeConfigurationParams, DidChangeWatchedFilesRegistrationOptions, FileSystemWatcher,
    InitializeParams, InitializedParams, LogMessageParams, MessageType, NumberOrString,
    ProgressParams, ProgressParamsValue, PublishDiagnosticsParams, RegistrationParams,
    ServerCapabilities, ShowMessageParams, UnregistrationParams, WorkDoneProgress, WorkspaceFolder,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::guard::catch_panic;
use crate::position::PositionEncoding;
use crate::transport::{Inbox, Transport};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(60);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const EXIT_GRACE: Duration = Duration::from_secs(1);
const WATCHDOG_TICK: Duration = Duration::from_millis(100);
const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspError {
    Timeout,
    Closed,
    Response { code: i64, message: String },
    Decode(String),
}

impl fmt::Display for LspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("request timed out"),
            Self::Closed => f.write_str("language server closed"),
            Self::Response { code, message } => write!(f, "{message} ({code})"),
            Self::Decode(message) => write!(f, "invalid response: {message}"),
        }
    }
}

impl std::error::Error for LspError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    /// A line of the server's stderr.
    Stderr,
    /// `window/logMessage` and `window/showMessageRequest`.
    Server,
    /// Written by Lucerna: server version, exit code.
    Lifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub kind: LogKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    Diagnostics(PublishDiagnosticsParams),
    Progress {
        token: String,
        title: Option<String>,
        message: Option<String>,
        percentage: Option<u32>,
        done: bool,
    },
    Log(LogLine),
    ShowMessage {
        kind: MessageType,
        message: String,
    },
    WatchedFilesRegistered {
        id: String,
        watchers: Vec<FileSystemWatcher>,
    },
    WatchedFilesUnregistered {
        id: String,
    },
    /// A server notification the client does not handle itself, such as
    /// vue-language-server's `tsserver/request`.
    Notification {
        method: String,
        params: Value,
    },
    Exited(Option<i32>),
}

type Reply = Result<Value, LspError>;

struct Pending {
    tx: oneshot::Sender<Reply>,
    deadline: Instant,
}

struct State {
    closed: bool,
    pending: HashMap<i64, Pending>,
    capabilities: Option<Arc<ServerCapabilities>>,
    server_version: Option<String>,
    settings: Value,
    workspace_folders: Option<Vec<WorkspaceFolder>>,
    /// Disconnects once the first shutdown has finished.
    shutdown_done: Option<crossbeam_channel::Receiver<()>>,
}

struct Inner {
    name: String,
    transport: OnceLock<Arc<dyn Transport>>,
    state: Mutex<State>,
    closed_changed: Condvar,
    events: UnboundedSender<ServerEvent>,
    next_id: AtomicI64,
}

/// A running language server. Dropping the last handle shuts it down on a
/// background thread.
pub struct LanguageServer {
    inner: Arc<Inner>,
}

impl LanguageServer {
    /// Launches the server through `launch`, which receives the inbox the
    /// transport delivers to. The server is not initialized yet.
    ///
    /// # Errors
    /// When `launch` fails or the timeout thread cannot be started.
    pub fn start(
        name: &str,
        launch: impl FnOnce(Arc<dyn Inbox>) -> io::Result<Arc<dyn Transport>>,
        settings: Value,
    ) -> io::Result<(Arc<LanguageServer>, UnboundedReceiver<ServerEvent>)> {
        let (events, receiver) = unbounded();
        let inner = Arc::new(Inner {
            name: name.to_owned(),
            transport: OnceLock::new(),
            state: Mutex::new(State {
                closed: false,
                pending: HashMap::new(),
                capabilities: None,
                server_version: None,
                settings,
                workspace_folders: None,
                shutdown_done: None,
            }),
            closed_changed: Condvar::new(),
            events,
            next_id: AtomicI64::new(1),
        });
        spawn_watchdog(&inner)?;
        let inbox = Arc::new(ClientInbox {
            inner: Arc::downgrade(&inner),
        });
        let transport = launch(inbox)?;
        let _ = inner.transport.set(transport);
        Ok((Arc::new(LanguageServer { inner }), receiver))
    }

    /// Sends `initialize` (60 s), records the server's capabilities, then
    /// sends `initialized` and `workspace/didChangeConfiguration`.
    pub fn initialize(
        self: &Arc<Self>,
        params: InitializeParams,
    ) -> impl Future<Output = Result<(), LspError>> + Send + 'static {
        self.inner.lock().workspace_folders = params.workspace_folders.clone();
        let request = self.request_with_timeout::<Initialize>(params, INITIALIZE_TIMEOUT);
        let this = Arc::clone(self);
        async move {
            let result = request.await?;
            let version = result.server_info.as_ref().and_then(|i| i.version.clone());
            if let Some(info) = &result.server_info {
                let text = match &version {
                    Some(v) => format!("Server: {} {v}", info.name),
                    None => format!("Server: {}", info.name),
                };
                this.inner.log(LogKind::Lifecycle, text);
            }
            {
                let mut state = this.inner.lock();
                state.capabilities = Some(Arc::new(result.capabilities));
                state.server_version = version;
            }
            this.notify::<Initialized>(InitializedParams {});
            let settings = this.inner.lock().settings.clone();
            this.notify::<DidChangeConfiguration>(DidChangeConfigurationParams { settings });
            Ok(())
        }
    }

    /// Sends a request with the default 120 s timeout.
    pub fn request<R: Request>(&self, params: R::Params) -> RequestFuture<R::Result>
    where
        R::Result: DeserializeOwned,
    {
        self.request_with_timeout::<R>(params, REQUEST_TIMEOUT)
    }

    /// Sends a request now; the future resolves with its result. Dropping
    /// the future first sends `$/cancelRequest`.
    pub fn request_with_timeout<R: Request>(
        &self,
        params: R::Params,
        timeout: Duration,
    ) -> RequestFuture<R::Result>
    where
        R::Result: DeserializeOwned,
    {
        let (id, rx) = match serde_json::to_value(params) {
            Ok(params) => self.inner.request(R::METHOD, params, timeout),
            Err(err) => (0, failed(LspError::Decode(err.to_string()))),
        };
        RequestFuture {
            id,
            rx,
            inner: Arc::downgrade(&self.inner),
            finished: false,
            _result: PhantomData,
        }
    }

    pub fn notify<N: Notification>(&self, params: N::Params) {
        match serde_json::to_value(params) {
            Ok(params) => self.inner.notify(N::METHOD, params),
            Err(err) => tracing::warn!(method = N::METHOD, error = %err, "unserializable params"),
        }
    }

    /// `None` until `initialize` has completed.
    pub fn capabilities(&self) -> Option<Arc<ServerCapabilities>> {
        self.inner.lock().capabilities.clone()
    }

    /// The negotiated position encoding (UTF-16 until initialized).
    pub fn encoding(&self) -> PositionEncoding {
        let state = self.inner.lock();
        let kind = state
            .capabilities
            .as_ref()
            .and_then(|c| c.position_encoding.as_ref());
        PositionEncoding::from_lsp(kind)
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn server_version(&self) -> Option<String> {
        self.inner.lock().server_version.clone()
    }

    /// Stores the settings answered to `workspace/configuration` and sends
    /// `workspace/didChangeConfiguration`.
    pub fn set_settings(&self, settings: Value) {
        self.inner.lock().settings = settings.clone();
        self.notify::<DidChangeConfiguration>(DidChangeConfigurationParams { settings });
    }

    pub fn is_closed(&self) -> bool {
        self.inner.lock().closed
    }

    /// `shutdown` (5 s) → `exit` → wait 1 s → kill, on a background thread.
    /// Idempotent: later calls return a handle that waits for the first.
    /// `None` when the OS cannot start a thread: the error is logged and
    /// the process killed instead.
    pub fn shutdown(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        match begin_shutdown(&self.inner) {
            Ok(handle) => Some(handle),
            Err(err) => {
                tracing::error!(server = %self.inner.name, error = %err, "cannot start shutdown thread");
                self.kill();
                None
            }
        }
    }

    /// Kills the process (and what it spawned) now, without `shutdown`.
    /// A pending shutdown then ends as soon as the transport reports it
    /// closed.
    pub fn kill(&self) {
        if let Some(transport) = self.inner.transport.get() {
            transport.kill();
        }
    }
}

impl Drop for LanguageServer {
    fn drop(&mut self) {
        let idle = {
            let state = self.inner.lock();
            state.closed || state.shutdown_done.is_some()
        };
        if !idle && let Err(err) = begin_shutdown(&self.inner) {
            tracing::error!(server = %self.inner.name, error = %err, "cannot start shutdown thread");
            self.kill();
        }
    }
}

fn begin_shutdown(inner: &Arc<Inner>) -> io::Result<JoinHandle<()>> {
    let name = format!("lsp-shutdown-{}", inner.name);
    let mut state = inner.lock();
    if let Some(done) = state.shutdown_done.clone() {
        drop(state);
        return thread::Builder::new().name(name).spawn(move || {
            let _ = done.recv();
        });
    }
    let (done_tx, done_rx) = crossbeam_channel::bounded::<()>(0);
    state.shutdown_done = Some(done_rx);
    drop(state);
    let inner = Arc::clone(inner);
    let task = name.clone();
    thread::Builder::new().name(name).spawn(move || {
        let _done = done_tx;
        if catch_panic(&task, || inner.shutdown_blocking()).is_err()
            && let Some(transport) = inner.transport.get()
        {
            transport.kill();
        }
    })
}

/// One thread per server fails requests past their deadline. It holds only
/// a `Weak`, so it ends once the server is dropped or closed.
fn spawn_watchdog(inner: &Arc<Inner>) -> io::Result<()> {
    let weak = Arc::downgrade(inner);
    let name = format!("lsp-timeout-{}", inner.name);
    let task = name.clone();
    thread::Builder::new().name(name).spawn(move || {
        let _ = catch_panic(&task, || {
            loop {
                thread::sleep(WATCHDOG_TICK);
                let Some(inner) = weak.upgrade() else { break };
                if !inner.expire(Instant::now()) {
                    break;
                }
            }
        });
    })?;
    Ok(())
}

fn failed(err: LspError) -> oneshot::Receiver<Reply> {
    let (tx, rx) = oneshot::channel();
    let _ = tx.send(Err(err));
    rx
}

fn message(method: &str, id: Option<i64>, params: Value) -> Value {
    let mut msg = json!({ "jsonrpc": "2.0", "method": method });
    if let Some(id) = id {
        msg["id"] = id.into();
    }
    if !params.is_null() {
        msg["params"] = params;
    }
    msg
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn send(&self, msg: &Value) {
        if let Some(transport) = self.transport.get() {
            transport.send(msg.to_string());
        }
    }

    fn emit(&self, event: ServerEvent) {
        let _ = self.events.unbounded_send(event);
    }

    fn log(&self, kind: LogKind, text: String) {
        self.emit(ServerEvent::Log(LogLine { kind, text }));
    }

    fn notify(&self, method: &str, params: Value) {
        if self.lock().closed {
            return;
        }
        self.send(&message(method, None, params));
    }

    /// Registers the request before sending it: the reply may arrive from
    /// inside `send`.
    fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> (i64, oneshot::Receiver<Reply>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut state = self.lock();
            if state.closed {
                drop(state);
                let _ = tx.send(Err(LspError::Closed));
                return (id, rx);
            }
            let deadline = Instant::now() + timeout;
            state.pending.insert(id, Pending { tx, deadline });
        }
        self.send(&message(method, Some(id), params));
        (id, rx)
    }

    fn cancel(&self, id: i64) {
        self.notify(Cancel::METHOD, json!({ "id": id }));
    }

    /// Fails requests past their deadline. `false` once the server closed.
    fn expire(&self, now: Instant) -> bool {
        let expired: Vec<(i64, Pending)> = {
            let mut state = self.lock();
            if state.closed {
                return false;
            }
            let ids: Vec<i64> = state
                .pending
                .iter()
                .filter(|(_, p)| p.deadline <= now)
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| state.pending.remove(&id).map(|p| (id, p)))
                .collect()
        };
        for (id, pending) in expired {
            // Cancel first, so the caller wakes to a server already told.
            self.cancel(id);
            let _ = pending.tx.send(Err(LspError::Timeout));
        }
        true
    }

    fn shutdown_blocking(&self) {
        if self.lock().closed {
            return;
        }
        let (_, reply) = self.request(Shutdown::METHOD, Value::Null, SHUTDOWN_TIMEOUT);
        let _ = futures::executor::block_on(reply);
        self.notify(Exit::METHOD, Value::Null);
        let state = self.lock();
        let (state, _) = self
            .closed_changed
            .wait_timeout_while(state, EXIT_GRACE, |s| !s.closed)
            .unwrap_or_else(PoisonError::into_inner);
        let closed = state.closed;
        drop(state);
        if !closed && let Some(transport) = self.transport.get() {
            transport.kill();
        }
    }

    fn on_message(&self, body: &str) {
        let msg: Value = match serde_json::from_str(body) {
            Ok(msg) => msg,
            Err(err) => {
                tracing::warn!(server = %self.name, error = %err, "unparsable message");
                return;
            }
        };
        let method = msg.get("method").and_then(Value::as_str);
        let id = msg.get("id").filter(|id| !id.is_null());
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (method, id) {
            (Some(method), Some(id)) => {
                let mut reply = json!({ "jsonrpc": "2.0", "id": id });
                match self.answer(method, params) {
                    Ok(result) => reply["result"] = result,
                    Err((code, message)) => {
                        reply["error"] = json!({ "code": code, "message": message });
                    }
                }
                self.send(&reply);
            }
            (Some(method), None) => self.on_notification(method, params),
            (None, Some(id)) => self.on_response(id, &msg),
            (None, None) => tracing::warn!(server = %self.name, "message without method or id"),
        }
    }

    fn on_response(&self, id: &Value, msg: &Value) {
        let Some(id) = id.as_i64() else {
            tracing::warn!(server = %self.name, %id, "response with a foreign id");
            return;
        };
        let Some(pending) = self.lock().pending.remove(&id) else {
            return; // cancelled, timed out or unknown
        };
        let reply = match msg.get("error") {
            Some(error) => Err(LspError::Response {
                code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }),
            None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = pending.tx.send(reply);
    }

    /// The auto-reply table of spec §3.1.
    fn answer(&self, method: &str, params: Value) -> Result<Value, (i64, String)> {
        match method {
            WorkspaceConfiguration::METHOD => Ok(self.configuration(&params)),
            WorkDoneProgressCreate::METHOD => Ok(Value::Null),
            RegisterCapability::METHOD => {
                self.register(params);
                Ok(Value::Null)
            }
            UnregisterCapability::METHOD => {
                self.unregister(params);
                Ok(Value::Null)
            }
            WorkspaceFoldersRequest::METHOD => {
                let folders = self.lock().workspace_folders.clone();
                Ok(serde_json::to_value(folders).unwrap_or(Value::Null))
            }
            ShowMessageRequest::METHOD => {
                if let Some(message) = params.get("message").and_then(Value::as_str) {
                    self.log(LogKind::Server, message.to_owned());
                }
                Ok(Value::Null)
            }
            ApplyWorkspaceEdit::METHOD => Ok(json!({ "applied": false })),
            ShowDocument::METHOD => Ok(json!({ "success": false })),
            _ => Err((METHOD_NOT_FOUND, format!("Unhandled method {method}"))),
        }
    }

    /// One value per item: the whole settings object without a `section`,
    /// else the value at that section (literal dotted keys such as
    /// `"basedpyright.analysis"` first, then nested objects), else `null`.
    fn configuration(&self, params: &Value) -> Value {
        let items = params
            .get("items")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let state = self.lock();
        let answers = items
            .iter()
            .map(|item| match item.get("section").and_then(Value::as_str) {
                None | Some("") => state.settings.clone(),
                Some(section) => settings_section(&state.settings, section)
                    .cloned()
                    .unwrap_or(Value::Null),
            })
            .collect();
        Value::Array(answers)
    }

    fn register(&self, params: Value) {
        let Some(params) = self.parse::<RegistrationParams>(RegisterCapability::METHOD, params)
        else {
            return;
        };
        for registration in params.registrations {
            if registration.method != DidChangeWatchedFiles::METHOD {
                continue;
            }
            let options = registration.register_options.unwrap_or(Value::Null);
            if let Some(options) = self
                .parse::<DidChangeWatchedFilesRegistrationOptions>(&registration.method, options)
            {
                self.emit(ServerEvent::WatchedFilesRegistered {
                    id: registration.id,
                    watchers: options.watchers,
                });
            }
        }
    }

    fn unregister(&self, params: Value) {
        let Some(params) = self.parse::<UnregistrationParams>(UnregisterCapability::METHOD, params)
        else {
            return;
        };
        for unregistration in params.unregisterations {
            if unregistration.method == DidChangeWatchedFiles::METHOD {
                self.emit(ServerEvent::WatchedFilesUnregistered {
                    id: unregistration.id,
                });
            }
        }
    }

    fn on_notification(&self, method: &str, params: Value) {
        match method {
            PublishDiagnostics::METHOD => {
                if let Some(params) = self.parse_diagnostics(params) {
                    self.emit(ServerEvent::Diagnostics(params));
                }
            }
            Progress::METHOD => {
                if let Some(params) = self.parse::<ProgressParams>(method, params) {
                    self.emit(progress_event(params));
                }
            }
            LogMessage::METHOD => {
                if let Some(params) = self.parse::<LogMessageParams>(method, params) {
                    self.log(LogKind::Server, params.message);
                }
            }
            ShowMessage::METHOD => {
                if let Some(params) = self.parse::<ShowMessageParams>(method, params) {
                    self.emit(ServerEvent::ShowMessage {
                        kind: params.typ,
                        message: params.message,
                    });
                }
            }
            _ => self.emit(ServerEvent::Notification {
                method: method.to_owned(),
                params,
            }),
        }
    }

    /// Decodes each diagnostic on its own, so one malformed entry is
    /// dropped (and logged) instead of the whole publish.
    fn parse_diagnostics(&self, mut params: Value) -> Option<PublishDiagnosticsParams> {
        let method = PublishDiagnostics::METHOD;
        let items = params
            .get_mut("diagnostics")
            .map(Value::take)
            .unwrap_or_default();
        if let Some(object) = params.as_object_mut() {
            object.insert("diagnostics".into(), Value::Array(Vec::new()));
        }
        let mut parsed = self.parse::<PublishDiagnosticsParams>(method, params)?;
        let Value::Array(items) = items else {
            tracing::warn!(server = %self.name, method, "diagnostics is not an array");
            return Some(parsed);
        };
        parsed.diagnostics = items
            .into_iter()
            .filter_map(|item| self.parse(method, item))
            .collect();
        Some(parsed)
    }

    fn parse<T: DeserializeOwned>(&self, method: &str, params: Value) -> Option<T> {
        serde_json::from_value(params)
            .inspect_err(|err| {
                tracing::warn!(server = %self.name, method, error = %err, "malformed params");
            })
            .ok()
    }

    fn on_closed(&self, code: Option<i32>) {
        let pending = {
            let mut state = self.lock();
            state.closed = true;
            std::mem::take(&mut state.pending)
        };
        self.closed_changed.notify_all();
        for (_, p) in pending {
            let _ = p.tx.send(Err(LspError::Closed));
        }
        let text = match code {
            Some(code) => format!("Exited with code {code}"),
            None => "Exited".to_owned(),
        };
        self.log(LogKind::Lifecycle, text);
        self.emit(ServerEvent::Exited(code));
    }
}

fn progress_event(params: ProgressParams) -> ServerEvent {
    let token = match params.token {
        NumberOrString::Number(n) => n.to_string(),
        NumberOrString::String(s) => s,
    };
    let ProgressParamsValue::WorkDone(progress) = params.value;
    let (title, message, percentage, done) = match progress {
        WorkDoneProgress::Begin(b) => (Some(b.title), b.message, b.percentage, false),
        WorkDoneProgress::Report(r) => (None, r.message, r.percentage, false),
        WorkDoneProgress::End(e) => (None, e.message, None, true),
    };
    ServerEvent::Progress {
        token,
        title,
        message,
        percentage,
        done,
    }
}

/// What the transport delivers to. Holds a `Weak` so the transport (owned
/// by the server) does not keep the server alive.
struct ClientInbox {
    inner: Weak<Inner>,
}

impl Inbox for ClientInbox {
    fn message(&self, body: String) {
        if let Some(inner) = self.inner.upgrade() {
            inner.on_message(&body);
        }
    }

    fn stderr(&self, line: String) {
        if let Some(inner) = self.inner.upgrade() {
            inner.log(LogKind::Stderr, line);
        }
    }

    fn closed(&self, exit_code: Option<i32>) {
        if let Some(inner) = self.inner.upgrade() {
            inner.on_closed(exit_code);
        }
    }
}

/// The result of one request. Dropping it before it resolves sends
/// `$/cancelRequest`.
#[must_use = "dropping a request cancels it"]
pub struct RequestFuture<T> {
    id: i64,
    rx: oneshot::Receiver<Reply>,
    inner: Weak<Inner>,
    finished: bool,
    _result: PhantomData<fn() -> T>,
}

impl<T> RequestFuture<T> {
    /// The JSON-RPC id the request was sent with.
    pub fn id(&self) -> i64 {
        self.id
    }
}

impl<T: DeserializeOwned> Future for RequestFuture<T> {
    type Output = Result<T, LspError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let reply = ready!(Pin::new(&mut self.rx).poll(cx));
        self.finished = true;
        Poll::Ready(match reply {
            Ok(Ok(value)) => {
                serde_json::from_value(value).map_err(|err| LspError::Decode(err.to_string()))
            }
            Ok(Err(err)) => Err(err),
            Err(oneshot::Canceled) => Err(LspError::Closed),
        })
    }
}

impl<T> Drop for RequestFuture<T> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        // Still pending means no reply yet: tell the server to stop.
        let removed = inner.lock().pending.remove(&self.id).is_some();
        if removed {
            inner.cancel(self.id);
        }
    }
}

/// The value for a `workspace/configuration` section. Settings may spell a
/// section as a literal dotted key (`"basedpyright.analysis": {..}`), as
/// nested objects, or as a mix, so every split of the dotted path is tried,
/// longest literal key first.
fn settings_section<'a>(settings: &'a Value, section: &str) -> Option<&'a Value> {
    if let Some(value) = settings.get(section) {
        return Some(value);
    }
    section
        .rmatch_indices('.')
        .find_map(|(dot, _)| settings_section(settings.get(&section[..dot])?, &section[dot + 1..]))
}
