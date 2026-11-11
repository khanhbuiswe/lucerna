//! The Vue `tsserver/request` relay shapes (spec §5), as sent by
//! `vuejs/language-tools` and answered by its VS Code client.

use std::path::Path;

use futures::executor::block_on;
use lucerna_lsp::relay::{
    TsRequest, execute_command_params, parse_tsserver_request, response_body, tsserver_response,
};
use lucerna_lsp::test_support::FakeServer;
use lucerna_lsp::{LanguageServer, ServerEvent, initialize_params};
use serde_json::{Value, json};

#[test]
fn parses_a_batch_of_two_requests() {
    // `sendNotification('tsserver/request', [id, command, args])` puts the
    // tuple in a one-element params array on the wire.
    let params = json!([
        [1, "_vue:projectInfo", { "file": "/p/App.vue", "needFileNameList": false }],
        [2, "_vue:quickinfo", { "file": "/p/App.vue", "line": 3, "offset": 7 }],
    ]);
    let requests = parse_tsserver_request(&params);
    assert_eq!(
        requests,
        vec![
            TsRequest {
                id: json!(1),
                command: "_vue:projectInfo".into(),
                args: json!({ "file": "/p/App.vue", "needFileNameList": false }),
            },
            TsRequest {
                id: json!(2),
                command: "_vue:quickinfo".into(),
                args: json!({ "file": "/p/App.vue", "line": 3, "offset": 7 }),
            },
        ]
    );
}

#[test]
fn parses_a_bare_tuple_and_skips_malformed_entries() {
    let bare = parse_tsserver_request(&json!([7, "_vue:x", { "a": 1 }]));
    assert_eq!(bare.len(), 1);
    assert_eq!(bare[0].id, json!(7));
    assert_eq!(bare[0].command, "_vue:x");

    // No args: `null` is forwarded, as `undefined` would be.
    let no_args = parse_tsserver_request(&json!([[3, "_vue:y"]]));
    assert_eq!(no_args[0].args, Value::Null);

    let malformed = parse_tsserver_request(&json!([[4], [5, 6, {}], "x", { "id": 1 }]));
    assert!(malformed.is_empty(), "{malformed:?}");
    assert!(parse_tsserver_request(&Value::Null).is_empty());
    assert!(parse_tsserver_request(&json!({})).is_empty());
}

#[test]
fn builds_the_execute_command_for_vtsls() {
    let req = TsRequest {
        id: json!(2),
        command: "_vue:quickinfo".into(),
        args: json!({ "file": "/p/App.vue", "line": 3, "offset": 7 }),
    };
    let params = execute_command_params(&req);
    assert_eq!(params.command, "typescript.tsserverRequest");
    assert_eq!(
        params.arguments,
        vec![
            json!("_vue:quickinfo"),
            json!({ "file": "/p/App.vue", "line": 3, "offset": 7 }),
            json!({ "isAsync": true, "lowPriority": true }),
        ]
    );
}

#[test]
fn builds_the_response_and_reads_the_body() {
    assert_eq!(
        tsserver_response(&json!(2), Some(json!({ "displayString": "x: number" }))),
        json!([[2, { "displayString": "x: number" }]])
    );
    assert_eq!(tsserver_response(&json!(3), None), json!([[3, null]]));

    let result = json!({ "seq": 0, "type": "response", "body": { "kind": "let" } });
    assert_eq!(response_body(&result), Some(json!({ "kind": "let" })));
    assert_eq!(response_body(&json!({ "success": false })), None);
    assert_eq!(response_body(&json!({ "body": null })), None);
    assert_eq!(response_body(&Value::Null), None);
}

#[test]
fn unhandled_server_notifications_become_events() {
    let fake = FakeServer::new();
    let (server, mut events) =
        LanguageServer::start("vue", |inbox| fake.launch(inbox), Value::Null).unwrap();
    block_on(server.initialize(initialize_params(Path::new("/p"), None))).unwrap();
    fake.notify("tsserver/request", json!([[1, "_vue:projectInfo", {}]]));

    let mut found = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let ServerEvent::Notification { method, params } = event {
            found.push((method, params));
        }
    }
    assert_eq!(
        found,
        vec![(
            "tsserver/request".to_owned(),
            json!([[1, "_vue:projectInfo", {}]])
        )]
    );
}
