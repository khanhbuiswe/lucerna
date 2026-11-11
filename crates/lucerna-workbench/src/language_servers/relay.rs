//! The Vue `tsserver/request` relay (spec §5): requests from a server
//! whose def has `relay: "vue-tsserver"` go to the TypeScript server of
//! the same root (vtsls, else typescript-language-server) as
//! `workspace/executeCommand`; the answer goes back as
//! `tsserver/response`. Each request is awaited off the UI thread, so a
//! slow tsserver blocks nothing else. Without a TypeScript server the
//! answer is `null` at once; while one is starting, requests wait for it.

use std::sync::{Arc, Weak};

use gpui::Context;
use lsp_types::request::ExecuteCommand;
use lucerna_lsp::relay::{
    TsRequest, TsserverResponse, execute_command_params, parse_tsserver_request, response_body,
    tsserver_response,
};
use lucerna_lsp::table::Relay;
use lucerna_lsp::{LanguageServer, ServerKey, Slot};
use serde_json::Value;

use super::LanguageServers;
use crate::tasks::guarded;

/// The servers asked, in order of preference.
const TYPESCRIPT_SERVERS: [&str; 2] = ["vtsls", "typescript-language-server"];

/// Requests waiting for a starting TypeScript server.
pub(super) struct WaitingRelay {
    /// The server that sent them, and its generation then.
    from: ServerKey,
    generation: u64,
    server: Weak<LanguageServer>,
    requests: Vec<TsRequest>,
}

enum Target {
    Running(Arc<LanguageServer>),
    Starting,
    None,
}

impl LanguageServers {
    /// Whether the server of `key` relays `tsserver/request`.
    pub(super) fn relays(&self, key: &ServerKey) -> bool {
        self.registry
            .table()
            .get(&key.server)
            .is_some_and(|def| def.relay == Some(Relay::VueTsserver))
    }

    /// Handles `tsserver/request` from the (current) server of `key`.
    pub(super) fn relay(
        &mut self,
        key: &ServerKey,
        generation: u64,
        server: &Weak<LanguageServer>,
        params: &Value,
        cx: &mut Context<Self>,
    ) {
        let requests = parse_tsserver_request(params);
        if requests.is_empty() {
            tracing::warn!(server = &*key.server, %params, "malformed tsserver/request");
            return;
        }
        self.route(
            WaitingRelay {
                from: key.clone(),
                generation,
                server: server.clone(),
                requests,
            },
            None,
            cx,
        );
    }

    /// Routes the waiting requests again after a TypeScript server's slot
    /// changed. `leaving` is a server about to stop: it no longer counts.
    pub(super) fn flush_relay(&mut self, leaving: Option<&ServerKey>, cx: &mut Context<Self>) {
        if self.relay_waiting.is_empty() {
            return;
        }
        for waiting in std::mem::take(&mut self.relay_waiting) {
            if self.is_current(&waiting.from, waiting.generation) {
                self.route(waiting, leaving, cx);
            }
        }
    }

    fn route(
        &mut self,
        waiting: WaitingRelay,
        leaving: Option<&ServerKey>,
        cx: &mut Context<Self>,
    ) {
        match self.typescript_target(&waiting.from, leaving) {
            Target::Running(ts) => {
                for request in waiting.requests {
                    forward(&ts, waiting.server.clone(), request, cx);
                }
            }
            Target::Starting => self.relay_waiting.push(waiting),
            Target::None => {
                let Some(server) = waiting.server.upgrade() else {
                    return;
                };
                for request in &waiting.requests {
                    server.notify::<TsserverResponse>(tsserver_response(&request.id, None));
                }
            }
        }
    }

    /// The TypeScript server for the root of `from`: a running vtsls, else
    /// a running typescript-language-server, else whether one starts.
    fn typescript_target(&self, from: &ServerKey, leaving: Option<&ServerKey>) -> Target {
        let keys = TYPESCRIPT_SERVERS.map(|name| ServerKey {
            root: from.root.clone(),
            server: name.into(),
        });
        let slots = || {
            keys.iter()
                .filter(|key| Some(*key) != leaving)
                .filter_map(|key| self.registry.slot(key))
        };
        if let Some(server) = slots().find_map(|slot| match slot {
            Slot::Running(server) if !server.is_closed() => Some(server.clone()),
            _ => None,
        }) {
            return Target::Running(server);
        }
        if slots().any(|slot| matches!(slot, Slot::Starting)) {
            Target::Starting
        } else {
            Target::None
        }
    }
}

/// Sends `request` to `ts` now and answers `vue` from a background task.
fn forward(
    ts: &Arc<LanguageServer>,
    vue: Weak<LanguageServer>,
    request: TsRequest,
    cx: &mut Context<LanguageServers>,
) {
    let answer = ts.request::<ExecuteCommand>(execute_command_params(&request));
    cx.background_executor()
        .spawn(async move {
            let body = match answer.await {
                Ok(Some(result)) => guarded("lsp-relay", || response_body(&result)).flatten(),
                Ok(None) => None,
                Err(err) => {
                    tracing::debug!(%err, command = request.command, "tsserver request failed");
                    None
                }
            };
            if let Some(vue) = vue.upgrade() {
                vue.notify::<TsserverResponse>(tsserver_response(&request.id, body));
            }
        })
        .detach();
}
