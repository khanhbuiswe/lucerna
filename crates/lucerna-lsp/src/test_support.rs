//! An in-process language server for tests. It is fully synchronous: the
//! transport's `send` handles each message inline and delivers replies by
//! calling the inbox inline, so it runs under a deterministic executor
//! without threads.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use lsp_types::{
    HoverProviderCapability, OneOf, PositionEncodingKind, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind,
};
use serde_json::{Value, json};

use crate::launcher::Launcher;
use crate::transport::{Inbox, LaunchSpec, Transport};

/// A JSON-RPC result, or an error `(code, message)`.
pub type FakeReply = Result<Value, (i64, String)>;
type RequestHandler = Arc<dyn Fn(Value) -> FakeReply + Send + Sync>;
type NotificationHandler = Arc<dyn Fn(Value, &FakePeer) + Send + Sync>;

const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Clone)]
pub struct FakeServer {
    config: Config,
    shared: Arc<Shared>,
}

#[derive(Clone)]
struct Config {
    capabilities: ServerCapabilities,
    requests: HashMap<String, RequestHandler>,
    notifications: HashMap<String, NotificationHandler>,
    hang: HashSet<String>,
}

#[derive(Default)]
struct Shared {
    received: Mutex<Vec<(String, Value)>>,
    inbox: Mutex<Option<Arc<dyn Inbox>>>,
    closed: AtomicBool,
    /// Client replies to `FakePeer::request`, by id.
    responses: Mutex<HashMap<i64, FakeReply>>,
    next_id: AtomicI64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Default for FakeServer {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeServer {
    /// Answers `initialize` with full sync, definition, references, hover,
    /// documentSymbol, workspaceSymbol and UTF-16 positions.
    pub fn new() -> FakeServer {
        FakeServer {
            config: Config {
                capabilities: ServerCapabilities {
                    position_encoding: Some(PositionEncodingKind::UTF16),
                    text_document_sync: Some(TextDocumentSyncCapability::Kind(
                        TextDocumentSyncKind::FULL,
                    )),
                    definition_provider: Some(OneOf::Left(true)),
                    references_provider: Some(OneOf::Left(true)),
                    hover_provider: Some(HoverProviderCapability::Simple(true)),
                    document_symbol_provider: Some(OneOf::Left(true)),
                    workspace_symbol_provider: Some(OneOf::Left(true)),
                    ..Default::default()
                },
                requests: HashMap::new(),
                notifications: HashMap::new(),
                hang: HashSet::new(),
            },
            shared: Arc::default(),
        }
    }

    /// Replaces the capabilities answered to `initialize`.
    pub fn capabilities(mut self, caps: ServerCapabilities) -> Self {
        self.config.capabilities = caps;
        self
    }

    /// Answers `method` with `f(params)`: `Ok` is the result, `Err` an
    /// error response `(code, message)`.
    pub fn on_request(
        mut self,
        method: &str,
        f: impl Fn(Value) -> FakeReply + Send + Sync + 'static,
    ) -> Self {
        self.config.requests.insert(method.to_owned(), Arc::new(f));
        self
    }

    /// Runs `f` when the client sends notification `method`; the peer can
    /// send notifications and requests back.
    pub fn on_notification(
        mut self,
        method: &str,
        f: impl Fn(Value, &FakePeer) + Send + Sync + 'static,
    ) -> Self {
        self.config
            .notifications
            .insert(method.to_owned(), Arc::new(f));
        self
    }

    /// Requests for `method` are never answered.
    pub fn hang_on(mut self, method: &str) -> Self {
        self.config.hang.insert(method.to_owned());
        self
    }

    /// Method and params (`null` when absent) of every request and
    /// notification the client sent, in order.
    pub fn received(&self) -> Vec<(String, Value)> {
        lock(&self.shared.received).clone()
    }

    /// Sends notification `method` to the client, as the server would.
    pub fn notify(&self, method: &str, params: Value) {
        self.shared
            .deliver(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Writes `line` to the server's stderr.
    pub fn stderr(&self, line: &str) {
        if self.shared.closed.load(Ordering::Acquire) {
            return;
        }
        let inbox = lock(&self.shared.inbox).clone();
        if let Some(inbox) = inbox {
            inbox.stderr(line.to_owned());
        }
    }

    /// Ends the server as if the process exited with `code`.
    pub fn crash(&self, code: Option<i32>) {
        self.shared.close(code);
    }

    /// Connects `inbox`. Synchronous: no threads are started.
    ///
    /// # Errors
    /// Never; the signature matches a real launch.
    pub fn launch(&self, inbox: Arc<dyn Inbox>) -> io::Result<Arc<dyn Transport>> {
        *lock(&self.shared.inbox) = Some(inbox);
        self.shared.closed.store(false, Ordering::Release);
        Ok(Arc::new(FakeTransport {
            config: self.config.clone(),
            shared: Arc::clone(&self.shared),
        }))
    }
}

impl Shared {
    fn deliver(&self, msg: &Value) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let inbox = lock(&self.inbox).clone();
        if let Some(inbox) = inbox {
            inbox.message(msg.to_string());
        }
    }

    fn close(&self, code: Option<i32>) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let inbox = lock(&self.inbox).clone();
        if let Some(inbox) = inbox {
            inbox.closed(code);
        }
    }
}

/// The server side of the connection, handed to notification handlers.
pub struct FakePeer {
    shared: Arc<Shared>,
}

impl FakePeer {
    pub fn notify(&self, method: &str, params: Value) {
        self.shared
            .deliver(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Sends a server→client request and returns the client's reply, which
    /// arrives inline. `None` when the client did not answer.
    pub fn request(&self, method: &str, params: Value) -> Option<FakeReply> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let mut msg = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if !params.is_null() {
            msg["params"] = params;
        }
        self.shared.deliver(&msg);
        lock(&self.shared.responses).remove(&id)
    }
}

struct FakeTransport {
    config: Config,
    shared: Arc<Shared>,
}

impl FakeTransport {
    fn answer(&self, method: &str, params: Value) -> FakeReply {
        if let Some(handler) = self.config.requests.get(method) {
            return handler(params);
        }
        match method {
            "initialize" => Ok(json!({
                "capabilities": self.config.capabilities,
                "serverInfo": { "name": "fake", "version": "1.0.0" },
            })),
            "shutdown" => Ok(Value::Null),
            _ => Err((METHOD_NOT_FOUND, format!("Unhandled method {method}"))),
        }
    }

    fn on_client_response(&self, msg: &Value) {
        let Some(id) = msg.get("id").and_then(Value::as_i64) else {
            return;
        };
        let reply = match msg.get("error") {
            Some(error) => Err((
                error.get("code").and_then(Value::as_i64).unwrap_or(0),
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            )),
            None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
        };
        lock(&self.shared.responses).insert(id, reply);
    }
}

impl Transport for FakeTransport {
    fn send(&self, body: String) {
        if self.shared.closed.load(Ordering::Acquire) {
            return;
        }
        let msg: Value = serde_json::from_str(&body).expect("client sent valid JSON");
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            self.on_client_response(&msg);
            return;
        };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        lock(&self.shared.received).push((method.to_owned(), params.clone()));

        if let Some(id) = msg.get("id") {
            if self.config.hang.contains(method) {
                return;
            }
            let mut reply = json!({ "jsonrpc": "2.0", "id": id });
            match self.answer(method, params) {
                Ok(result) => reply["result"] = result,
                Err((code, message)) => {
                    reply["error"] = json!({ "code": code, "message": message });
                }
            }
            self.shared.deliver(&reply);
            return;
        }

        if let Some(handler) = self.config.notifications.get(method) {
            handler(
                params,
                &FakePeer {
                    shared: Arc::clone(&self.shared),
                },
            );
        }
        if method == "exit" {
            self.shared.close(Some(0));
        }
    }

    fn kill(&self) {
        self.shared.close(None);
    }
}

/// A launcher whose "binaries" are in-process [`FakeServer`]s, registered
/// under the binary name an adapter looks up.
#[derive(Default)]
pub struct FakeLauncher {
    servers: HashMap<String, FakeServer>,
    launches: Mutex<Vec<LaunchSpec>>,
}

impl FakeLauncher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes `binary_name` resolvable and launchable as `server`.
    pub fn with_server(mut self, binary_name: &str, server: FakeServer) -> Self {
        self.servers.insert(binary_name.to_owned(), server);
        self
    }

    /// Every spec passed to `launch`, in order.
    pub fn launches(&self) -> Vec<LaunchSpec> {
        lock(&self.launches).clone()
    }

    fn lookup(&self, program: &Path) -> Option<&FakeServer> {
        self.servers.get(program.file_name()?.to_str()?)
    }
}

impl Launcher for FakeLauncher {
    fn env(&self) -> HashMap<String, String> {
        HashMap::from([("PATH".to_owned(), "/fake/bin".to_owned())])
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        self.servers
            .contains_key(name)
            .then(|| Path::new("/fake/bin").join(name))
    }

    fn probe(&self, program: &Path, _args: &[&str]) -> bool {
        self.lookup(program).is_some()
    }

    fn launch(&self, spec: &LaunchSpec, inbox: Arc<dyn Inbox>) -> io::Result<Arc<dyn Transport>> {
        lock(&self.launches).push(spec.clone());
        match self.lookup(&spec.program) {
            Some(server) => server.launch(inbox),
            None => Err(io::ErrorKind::NotFound.into()),
        }
    }
}
