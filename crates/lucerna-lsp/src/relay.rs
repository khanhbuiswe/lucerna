//! The Vue `tsserver/request` relay (spec §5).
//!
//! vue-language-server (hybrid mode) does not run TypeScript itself: it
//! sends the notification `tsserver/request` and waits for the client to
//! ask a TypeScript server (vtsls, else typescript-language-server, both
//! with `@vue/typescript-plugin`) through `workspace/executeCommand`
//! `typescript.tsserverRequest`, then to send the body back as the
//! notification `tsserver/response`.
//!
//! Wire shapes, as `vuejs/language-tools` sends them (`vscode-jsonrpc`
//! wraps a single array argument in the params array):
//! - `tsserver/request`: `[[id, command, args]]`
//! - `executeCommand` arguments: `[command, args, { isAsync: true, lowPriority: true }]`
//! - `tsserver/response`: `[[id, body]]`, `body` `null` on failure.

use lsp_types::ExecuteCommandParams;
use lsp_types::notification::Notification;
use serde_json::{Value, json};

/// The notification vue-language-server sends.
pub const TSSERVER_REQUEST: &str = "tsserver/request";
/// The command a TypeScript server runs a tsserver request with.
pub const TSSERVER_REQUEST_COMMAND: &str = "typescript.tsserverRequest";

/// The `tsserver/response` notification sent back.
pub enum TsserverResponse {}

impl Notification for TsserverResponse {
    type Params = Value;
    const METHOD: &'static str = "tsserver/response";
}

/// One tsserver request relayed for vue-language-server.
#[derive(Clone, Debug, PartialEq)]
pub struct TsRequest {
    /// Echoed in the response; vue-language-server matches on it.
    pub id: Value,
    pub command: String,
    pub args: Value,
}

/// The requests in `tsserver/request` params: an array of
/// `[id, command, args]` (a bare tuple is accepted too). Entries without
/// an id or a string command are skipped; missing `args` are `null`.
pub fn parse_tsserver_request(params: &Value) -> Vec<TsRequest> {
    let Some(items) = params.as_array() else {
        return Vec::new();
    };
    if items.first().is_some_and(|first| !first.is_array()) {
        return parse_tuple(items).into_iter().collect();
    }
    items
        .iter()
        .filter_map(|item| parse_tuple(item.as_array()?))
        .collect()
}

fn parse_tuple(tuple: &[Value]) -> Option<TsRequest> {
    let id = tuple
        .first()
        .filter(|id| id.is_number() || id.is_string())?;
    let command = tuple.get(1)?.as_str()?;
    Some(TsRequest {
        id: id.clone(),
        command: command.to_owned(),
        args: tuple.get(2).cloned().unwrap_or(Value::Null),
    })
}

/// `workspace/executeCommand` params asking a TypeScript server to run
/// `req`, as the Vue VS Code extension does.
pub fn execute_command_params(req: &TsRequest) -> ExecuteCommandParams {
    ExecuteCommandParams {
        command: TSSERVER_REQUEST_COMMAND.to_owned(),
        arguments: vec![
            Value::String(req.command.clone()),
            req.args.clone(),
            json!({ "isAsync": true, "lowPriority": true }),
        ],
        work_done_progress_params: Default::default(),
    }
}

/// The params of `tsserver/response` for request `id`: `[[id, body]]`.
pub fn tsserver_response(id: &Value, body: Option<Value>) -> Value {
    json!([[id, body]])
}

/// `result.body` of a `typescript.tsserverRequest` answer; `None` when
/// absent or `null`.
pub fn response_body(result: &Value) -> Option<Value> {
    result.get("body").filter(|body| !body.is_null()).cloned()
}
