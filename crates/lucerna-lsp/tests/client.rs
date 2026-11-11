use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedReceiver;
use futures::executor::block_on;
use lsp_types::request::{GotoDefinition, HoverRequest};
use lsp_types::{
    FileSystemWatcher, GlobPattern, HoverContents, HoverParams, MarkedString, MessageType,
    Position, TextDocumentIdentifier, TextDocumentPositionParams,
};
use lucerna_lsp::position::PositionEncoding;
use lucerna_lsp::test_support::FakeServer;
use lucerna_lsp::transport::{Inbox, Transport};
use lucerna_lsp::{LanguageServer, LogKind, LogLine, LspError, ServerEvent, initialize_params};
use serde_json::{Value, json};

type Events = UnboundedReceiver<ServerEvent>;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn start(fake: &FakeServer, settings: Value) -> (Arc<LanguageServer>, Events) {
    LanguageServer::start("fake", |inbox| fake.launch(inbox), settings).unwrap()
}

fn init(server: &Arc<LanguageServer>) {
    block_on(server.initialize(initialize_params(root(), None))).unwrap();
}

fn methods(fake: &FakeServer) -> Vec<String> {
    fake.received().into_iter().map(|(m, _)| m).collect()
}

/// Every event already queued (the fake delivers inline).
fn drain(events: &mut Events) -> Vec<ServerEvent> {
    let mut out = Vec::new();
    while let Ok(event) = events.try_recv() {
        out.push(event);
    }
    out
}

fn hover_params() -> HoverParams {
    HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: "file:///a.rs".parse().unwrap(),
            },
            position: Position::new(0, 0),
        },
        work_done_progress_params: Default::default(),
    }
}

/// A transport the test drives by hand: it records what the client sends
/// and hands the inbox back so responses can arrive in any order.
#[derive(Default)]
struct Manual {
    sent: Mutex<Vec<Value>>,
    inbox: Mutex<Option<Arc<dyn Inbox>>>,
}

impl Manual {
    fn start() -> (Arc<Manual>, Arc<LanguageServer>, Events) {
        let manual = Arc::new(Manual::default());
        let (server, events) = LanguageServer::start(
            "manual",
            |inbox| {
                *manual.inbox.lock().unwrap() = Some(inbox);
                Ok(Arc::clone(&manual) as Arc<dyn Transport>)
            },
            json!({}),
        )
        .unwrap();
        (manual, server, events)
    }

    fn inbox(&self) -> Arc<dyn Inbox> {
        self.inbox.lock().unwrap().clone().unwrap()
    }

    fn reply(&self, body: Value) {
        self.inbox().message(body.to_string());
    }

    fn sent(&self) -> Vec<Value> {
        self.sent.lock().unwrap().clone()
    }
}

impl Transport for Manual {
    fn send(&self, body: String) {
        self.sent
            .lock()
            .unwrap()
            .push(serde_json::from_str(&body).unwrap());
    }
    fn kill(&self) {}
}

#[test]
fn initialize_sends_capabilities_then_initialized_and_configuration() {
    let fake = FakeServer::new();
    let settings = json!({"check": {"command": "clippy"}});
    let (server, _events) = start(&fake, settings.clone());
    init(&server);

    assert_eq!(
        methods(&fake),
        [
            "initialize",
            "initialized",
            "workspace/didChangeConfiguration"
        ]
    );
    let received = fake.received();
    let caps = &received[0].1["capabilities"];
    assert_eq!(
        caps.pointer("/general/positionEncodings"),
        Some(&json!(["utf-8", "utf-16"]))
    );
    assert_eq!(
        caps.pointer("/textDocument/hover/contentFormat"),
        Some(&json!(["markdown", "plaintext"]))
    );
    assert_eq!(caps.pointer("/textDocument/completion"), None);
    assert_eq!(received[0].1["clientInfo"]["name"], "Lucerna");
    assert_eq!(received[2].1, json!({ "settings": settings }));

    let caps = server
        .capabilities()
        .expect("capabilities after initialize");
    assert!(caps.hover_provider.is_some());
    assert_eq!(server.encoding(), PositionEncoding::Utf16);
    assert_eq!(server.name(), "fake");
    assert_eq!(server.server_version().as_deref(), Some("1.0.0"));
}

#[test]
fn initialize_params_lists_read_only_capabilities() {
    let params = serde_json::to_value(initialize_params(root(), Some(json!({"x": 1})))).unwrap();
    let td = &params["capabilities"]["textDocument"];
    for goto in [
        "definition",
        "typeDefinition",
        "implementation",
        "declaration",
    ] {
        assert_eq!(td[goto]["linkSupport"], true, "{goto}");
    }
    assert_eq!(
        td["documentSymbol"]["hierarchicalDocumentSymbolSupport"],
        true
    );
    assert_eq!(
        td["publishDiagnostics"]["tagSupport"]["valueSet"],
        json!([1, 2])
    );
    assert_eq!(td["synchronization"].get("willSave"), None);
    assert_eq!(td["synchronization"].get("didSave"), None);
    let ws = &params["capabilities"]["workspace"];
    assert_eq!(ws["configuration"], true);
    assert_eq!(ws["workspaceFolders"], true);
    assert_eq!(
        ws["didChangeWatchedFiles"],
        json!({"dynamicRegistration": true, "relativePatternSupport": true})
    );
    assert_eq!(params["capabilities"]["window"]["workDoneProgress"], true);
    assert_eq!(params["initializationOptions"], json!({"x": 1}));
    assert_eq!(params["workspaceFolders"].as_array().unwrap().len(), 1);
    assert_eq!(params["processId"], std::process::id());
}

#[test]
fn request_resolves_by_id() {
    let (manual, server, _events) = Manual::start();
    let first = server.request::<HoverRequest>(hover_params());
    let second = server.request::<HoverRequest>(hover_params());
    let sent = manual.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0]["method"], "textDocument/hover");
    let (id1, id2) = (sent[0]["id"].clone(), sent[1]["id"].clone());
    assert_ne!(id1, id2);

    manual.reply(json!({"jsonrpc": "2.0", "id": id2, "result": {"contents": "second"}}));
    manual.reply(json!({"jsonrpc": "2.0", "id": id1, "result": {"contents": "first"}}));

    let text = |hover: Option<lsp_types::Hover>| match hover.unwrap().contents {
        HoverContents::Scalar(MarkedString::String(s)) => s,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(text(block_on(first).unwrap()), "first");
    assert_eq!(text(block_on(second).unwrap()), "second");
}

#[test]
fn error_and_decode_failures() {
    let (manual, server, _events) = Manual::start();
    let failed = server.request::<HoverRequest>(hover_params());
    let garbled = server.request::<HoverRequest>(hover_params());
    let sent = manual.sent();
    manual.reply(json!({
        "jsonrpc": "2.0", "id": sent[0]["id"],
        "error": {"code": -32801, "message": "content modified"}
    }));
    manual.reply(json!({"jsonrpc": "2.0", "id": sent[1]["id"], "result": 42}));
    assert_eq!(
        block_on(failed),
        Err(LspError::Response {
            code: -32801,
            message: "content modified".into()
        })
    );
    assert!(matches!(block_on(garbled), Err(LspError::Decode(_))));
}

#[test]
fn dropping_request_sends_cancel() {
    let fake = FakeServer::new().hang_on("textDocument/hover");
    let (server, _events) = start(&fake, json!({}));
    init(&server);
    let pending = server.request::<HoverRequest>(hover_params());
    let id = pending.id();
    drop(pending);
    let received = fake.received();
    assert_eq!(
        received.last().unwrap(),
        &("$/cancelRequest".to_owned(), json!({ "id": id }))
    );

    // A request that already completed sends no cancel.
    let done = server.request::<GotoDefinition>(lsp_types::GotoDefinitionParams {
        text_document_position_params: hover_params().text_document_position_params,
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    });
    assert!(block_on(done).is_err()); // the fake has no handler: MethodNotFound
    assert_eq!(methods(&fake).last().unwrap(), "textDocument/definition");
}

#[test]
fn timeout_fails_request() {
    let fake = FakeServer::new().hang_on("textDocument/hover");
    let (server, _events) = start(&fake, json!({}));
    init(&server);
    let started = Instant::now();
    let result = block_on(
        server.request_with_timeout::<HoverRequest>(hover_params(), Duration::from_millis(200)),
    );
    assert_eq!(result, Err(LspError::Timeout));
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(200), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(1), "{elapsed:?}");
    assert_eq!(methods(&fake).last().unwrap(), "$/cancelRequest");
}

#[test]
fn closed_fails_pending_and_emits_exited() {
    let fake = FakeServer::new().hang_on("textDocument/hover");
    let (server, mut events) = start(&fake, json!({}));
    init(&server);
    let pending = server.request::<HoverRequest>(hover_params());
    fake.crash(Some(1));
    assert_eq!(block_on(pending), Err(LspError::Closed));
    assert!(server.is_closed());
    assert!(drain(&mut events).contains(&ServerEvent::Exited(Some(1))));
    // Requests after the close fail at once.
    assert_eq!(
        block_on(server.request::<HoverRequest>(hover_params())),
        Err(LspError::Closed)
    );
}

#[test]
fn auto_replies() {
    let replies = Arc::new(Mutex::new(Vec::new()));
    let fake = FakeServer::new().on_notification("initialized", {
        let replies = Arc::clone(&replies);
        move |_, peer| {
            let mut replies = replies.lock().unwrap();
            replies.push(peer.request(
                "workspace/configuration",
                json!({"items": [{"section": "a.b"}, {}, {"section": "a.missing"}]}),
            ));
            replies.push(peer.request("window/workDoneProgress/create", json!({"token": "t"})));
            replies.push(peer.request("workspace/workspaceFolders", Value::Null));
            replies.push(peer.request("workspace/applyEdit", json!({"edit": {}})));
            replies.push(peer.request("window/showDocument", json!({"uri": "file:///a"})));
            replies.push(peer.request(
                "window/showMessageRequest",
                json!({"type": 3, "message": "pick one"}),
            ));
            replies.push(peer.request("foo/bar", Value::Null));
        }
    });
    let (server, mut events) = start(&fake, json!({"a": {"b": 1}}));
    init(&server);

    let replies = replies.lock().unwrap().clone();
    assert_eq!(replies[0], Some(Ok(json!([1, {"a": {"b": 1}}, null]))));
    assert_eq!(replies[1], Some(Ok(Value::Null)));
    let folders = replies[2].clone().unwrap().unwrap();
    assert_eq!(
        folders[0]["uri"],
        json!(lucerna_lsp::position::path_to_uri(root()).as_str())
    );
    assert_eq!(replies[3], Some(Ok(json!({"applied": false}))));
    assert_eq!(replies[4], Some(Ok(json!({"success": false}))));
    assert_eq!(replies[5], Some(Ok(Value::Null)));
    let Some(Err((code, _))) = &replies[6] else {
        panic!("expected an error, got {:?}", replies[6]);
    };
    assert_eq!(*code, -32601);
    assert!(drain(&mut events).contains(&ServerEvent::Log(LogLine {
        kind: LogKind::Server,
        text: "pick one".into()
    })));
}

#[test]
fn register_watched_files_emits_event() {
    let replies = Arc::new(Mutex::new(Vec::new()));
    let fake = FakeServer::new().on_notification("initialized", {
        let replies = Arc::clone(&replies);
        move |_, peer| {
            let mut replies = replies.lock().unwrap();
            replies.push(peer.request(
                "client/registerCapability",
                json!({"registrations": [
                    {"id": "w1", "method": "workspace/didChangeWatchedFiles",
                     "registerOptions": {"watchers": [{"globPattern": "**/*.rs"}]}},
                    {"id": "f1", "method": "textDocument/formatting"}
                ]}),
            ));
            replies.push(peer.request(
                "client/unregisterCapability",
                json!({"unregisterations": [
                    {"id": "w1", "method": "workspace/didChangeWatchedFiles"},
                    {"id": "f1", "method": "textDocument/formatting"}
                ]}),
            ));
        }
    });
    let (server, mut events) = start(&fake, json!({}));
    init(&server);

    assert_eq!(
        *replies.lock().unwrap(),
        [Some(Ok(Value::Null)), Some(Ok(Value::Null))]
    );
    let watched: Vec<_> = drain(&mut events)
        .into_iter()
        .filter(|e| {
            matches!(
                e,
                ServerEvent::WatchedFilesRegistered { .. }
                    | ServerEvent::WatchedFilesUnregistered { .. }
            )
        })
        .collect();
    assert_eq!(
        watched,
        [
            ServerEvent::WatchedFilesRegistered {
                id: "w1".into(),
                watchers: vec![FileSystemWatcher {
                    glob_pattern: GlobPattern::String("**/*.rs".into()),
                    kind: None,
                }],
            },
            ServerEvent::WatchedFilesUnregistered { id: "w1".into() },
        ]
    );
}

#[test]
fn progress_and_log_events() {
    let fake = FakeServer::new().on_notification("initialized", |_, peer| {
        peer.notify(
            "$/progress",
            json!({"token": "idx", "value": {"kind": "begin", "title": "Indexing", "percentage": 0}}),
        );
        peer.notify(
            "$/progress",
            json!({"token": 7, "value": {"kind": "report", "message": "1/2", "percentage": 50}}),
        );
        peer.notify(
            "$/progress",
            json!({"token": "idx", "value": {"kind": "end", "message": "done"}}),
        );
        peer.notify("window/logMessage", json!({"type": 3, "message": "hello"}));
        peer.notify("window/showMessage", json!({"type": 1, "message": "boom"}));
        peer.notify(
            "textDocument/publishDiagnostics",
            json!({"uri": "file:///a.rs", "diagnostics": []}),
        );
        peer.notify("custom/ignored", json!({}));
    });
    let (server, mut events) = start(&fake, json!({}));
    init(&server);

    let events: Vec<_> = drain(&mut events)
        .into_iter()
        .filter(|e| {
            !matches!(
                e,
                ServerEvent::Log(LogLine {
                    kind: LogKind::Lifecycle,
                    ..
                })
            )
        })
        .collect();
    assert_eq!(
        events,
        [
            ServerEvent::Progress {
                token: "idx".into(),
                title: Some("Indexing".into()),
                message: None,
                percentage: Some(0),
                done: false,
            },
            ServerEvent::Progress {
                token: "7".into(),
                title: None,
                message: Some("1/2".into()),
                percentage: Some(50),
                done: false,
            },
            ServerEvent::Progress {
                token: "idx".into(),
                title: None,
                message: Some("done".into()),
                percentage: None,
                done: true,
            },
            ServerEvent::Log(LogLine {
                kind: LogKind::Server,
                text: "hello".into()
            }),
            ServerEvent::ShowMessage {
                kind: MessageType::ERROR,
                message: "boom".into()
            },
            ServerEvent::Diagnostics(lsp_types::PublishDiagnosticsParams {
                uri: "file:///a.rs".parse().unwrap(),
                diagnostics: vec![],
                version: None,
            }),
        ]
    );
}

#[test]
fn stderr_lines_become_log_events() {
    let (manual, _server, mut events) = Manual::start();
    manual.inbox().stderr("warming up".into());
    assert_eq!(
        drain(&mut events),
        [ServerEvent::Log(LogLine {
            kind: LogKind::Stderr,
            text: "warming up".into()
        })]
    );
}

#[test]
fn shutdown_sends_shutdown_then_exit() {
    let fake = FakeServer::new();
    let (server, _events) = start(&fake, json!({}));
    init(&server);
    server.shutdown().join().unwrap();
    let methods = methods(&fake);
    assert_eq!(methods[methods.len() - 2..], ["shutdown", "exit"]);
    assert!(server.is_closed());
    // Idempotent: a second call sends nothing more.
    server.shutdown().join().unwrap();
    assert_eq!(fake.received().len(), methods.len());
}

#[test]
fn dropping_last_handle_shuts_down_in_background() {
    let fake = FakeServer::new();
    let (server, _events) = start(&fake, json!({}));
    init(&server);
    drop(server);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !methods(&fake).iter().any(|m| m == "exit") {
        assert!(
            Instant::now() < deadline,
            "no exit sent: {:?}",
            methods(&fake)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn set_settings_sends_did_change_configuration() {
    let fake = FakeServer::new();
    let (server, _events) = start(&fake, json!({}));
    init(&server);
    server.set_settings(json!({"x": true}));
    assert_eq!(
        fake.received().last().unwrap(),
        &(
            "workspace/didChangeConfiguration".to_owned(),
            json!({"settings": {"x": true}})
        )
    );
}
