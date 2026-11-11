//! Transport abstraction between the client and a language server.

use std::collections::HashMap;
use std::path::PathBuf;

/// Sending half: bodies are framed and written by the transport.
pub trait Transport: Send + Sync + 'static {
    /// Queues one JSON-RPC body; never blocks.
    fn send(&self, body: String);
    /// Terminates the server. `Inbox::closed` follows.
    fn kill(&self);
}

/// Receiving half. Called from transport threads, never the UI thread.
pub trait Inbox: Send + Sync + 'static {
    /// One framed message, on the reader thread.
    fn message(&self, body: String);
    /// One line of the server's stderr.
    fn stderr(&self, line: String);
    /// The server is gone; called exactly once.
    fn closed(&self, exit_code: Option<i32>);
}

/// How to start a server. `env` replaces the child environment entirely.
#[derive(Clone, Debug)]
pub struct LaunchSpec {
    pub server: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: PathBuf,
}
