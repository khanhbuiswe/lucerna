//! The Vue `tsserver/request` relay (spec §5): vue-language-server asks a
//! TypeScript server through Lucerna, with in-process fake servers.

mod support;

use std::fs;
use std::sync::Arc;

use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use serde_json::{Value, json};
use support::lsp::*;
use tempfile::TempDir;

/// A folder with `package.json`, `App.vue` and `main.ts`.
fn vue_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("package.json"), "{}\n").unwrap();
    fs::write(dir.path().join("App.vue"), "<script setup>\n</script>\n").unwrap();
    fs::write(dir.path().join("main.ts"), "export {}\n").unwrap();
    dir
}

/// A vue-language-server that sends `tsserver/request` with `request`
/// once a document is opened.
fn vue_server(request: Value) -> FakeServer {
    FakeServer::new().on_notification("textDocument/didOpen", move |_, peer| {
        peer.notify("tsserver/request", request.clone());
    })
}

/// A TypeScript server answering `typescript.tsserverRequest` with
/// `{ body: { displayString: <command> } }`.
fn ts_server() -> FakeServer {
    FakeServer::new().on_request("workspace/executeCommand", |params| {
        let command = params["arguments"][0].clone();
        Ok(json!({ "seq": 0, "type": "response", "body": { "displayString": command } }))
    })
}

fn launcher() -> FakeLauncher {
    FakeLauncher::new().with_output("npm", &["root", "-g"], "/fake/npm\n")
}

fn quickinfo() -> Value {
    json!([[7, "_vue:quickinfo", { "file": "/p/App.vue", "line": 1, "offset": 1 }]])
}

#[gpui::test]
fn relay_forwards_to_vtsls_and_answers(cx: &mut gpui::TestAppContext) {
    let dir = vue_project();
    let (vue, vtsls) = (vue_server(quickinfo()), ts_server());
    let launcher = launcher()
        .with_server("vue-language-server", vue.clone())
        .with_server("vtsls", vtsls.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);

    open(&wb, "App.vue", cx);

    let commands = sent(&vtsls, "workspace/executeCommand");
    assert_eq!(commands.len(), 1, "{:?}", methods(&vtsls));
    assert_eq!(commands[0]["command"], "typescript.tsserverRequest");
    assert_eq!(
        commands[0]["arguments"],
        json!([
            "_vue:quickinfo",
            { "file": "/p/App.vue", "line": 1, "offset": 1 },
            { "isAsync": true, "lowPriority": true },
        ])
    );
    assert_eq!(
        sent(&vue, "tsserver/response"),
        vec![json!([[7, { "displayString": "_vue:quickinfo" }]])]
    );
    assert_eq!(count(&vue, "workspace/executeCommand"), 0);
}

#[gpui::test]
fn relay_without_typescript_server_answers_null(cx: &mut gpui::TestAppContext) {
    let dir = vue_project();
    let vue = vue_server(quickinfo());
    // vtsls and typescript-language-server are not on PATH.
    let launcher = launcher().with_server("vue-language-server", vue.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);

    open(&wb, "App.vue", cx);

    assert_eq!(sent(&vue, "tsserver/response"), vec![json!([[7, null]])]);
}

#[gpui::test]
fn relay_answers_null_when_the_typescript_server_errors(cx: &mut gpui::TestAppContext) {
    let dir = vue_project();
    let vue = vue_server(quickinfo());
    let vtsls = FakeServer::new().on_request("workspace/executeCommand", |_| {
        Err((-32603, "tsserver crashed".into()))
    });
    let launcher = launcher()
        .with_server("vue-language-server", vue.clone())
        .with_server("vtsls", vtsls.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);

    open(&wb, "App.vue", cx);

    assert_eq!(count(&vtsls, "workspace/executeCommand"), 1);
    assert_eq!(sent(&vue, "tsserver/response"), vec![json!([[7, null]])]);
}

#[gpui::test]
fn relay_falls_back_to_typescript_language_server(cx: &mut gpui::TestAppContext) {
    let dir = vue_project();
    let (vue, tls) = (vue_server(quickinfo()), ts_server());
    let launcher = Arc::new(
        launcher()
            .with_server("vue-language-server", vue.clone())
            .with_server("typescript-language-server", tls.clone()),
    );
    let settings = r#"{
        "session.trustAllWorktrees": true,
        "lsp": { "vtsls": { "enabled": false } }
    }"#;
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher, settings, cx);

    // typescript-language-server runs for the same root.
    open(&wb, "main.ts", cx);
    open(&wb, "App.vue", cx);

    let commands = sent(&tls, "workspace/executeCommand");
    assert_eq!(commands.len(), 1, "{:?}", methods(&tls));
    assert_eq!(commands[0]["command"], "typescript.tsserverRequest");
    assert_eq!(
        sent(&vue, "tsserver/response"),
        vec![json!([[7, { "displayString": "_vue:quickinfo" }]])]
    );
}

#[gpui::test]
fn relay_waits_for_a_starting_typescript_server(cx: &mut gpui::TestAppContext) {
    let dir = vue_project();
    let vue = vue_server(quickinfo());
    let vtsls = ts_server().hang_on("initialize");
    let launcher = launcher()
        .with_server("vue-language-server", vue.clone())
        .with_server("vtsls", vtsls.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);

    open(&wb, "App.vue", cx);
    // vtsls is still starting: the request waits instead of failing.
    assert!(sent(&vue, "tsserver/response").is_empty());

    let id = vtsls.hung_ids("initialize")[0].clone();
    assert!(vtsls.answer_hung(&id, json!({ "capabilities": {} })));
    settle(cx);

    assert_eq!(count(&vtsls, "workspace/executeCommand"), 1);
    // vtsls has the file before it is asked about it.
    let order = methods(&vtsls);
    let opened = order.iter().position(|m| m == "textDocument/didOpen");
    let asked = order.iter().position(|m| m == "workspace/executeCommand");
    assert!(opened.is_some() && opened < asked, "{order:?}");
    assert_eq!(
        sent(&vue, "tsserver/response"),
        vec![json!([[7, { "displayString": "_vue:quickinfo" }]])]
    );
}

#[gpui::test]
fn relay_answers_null_when_starting_typescript_server_fails(cx: &mut gpui::TestAppContext) {
    let dir = vue_project();
    let vue = vue_server(quickinfo());
    let vtsls = ts_server().hang_on("initialize");
    let launcher = launcher()
        .with_server("vue-language-server", vue.clone())
        .with_server("vtsls", vtsls.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);

    open(&wb, "App.vue", cx);
    assert!(sent(&vue, "tsserver/response").is_empty());

    // vtsls dies before answering `initialize`.
    vtsls.crash(Some(1));
    settle(cx);

    assert_eq!(count(&vtsls, "workspace/executeCommand"), 0);
    assert_eq!(sent(&vue, "tsserver/response"), vec![json!([[7, null]])]);
}

#[gpui::test]
fn relay_answers_null_when_waiting_typescript_server_is_stopped(cx: &mut gpui::TestAppContext) {
    let dir = vue_project();
    let vue = vue_server(quickinfo());
    let vtsls = ts_server().hang_on("initialize");
    let launcher = launcher()
        .with_server("vue-language-server", vue.clone())
        .with_server("vtsls", vtsls.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);

    open(&wb, "App.vue", cx);
    assert!(sent(&vue, "tsserver/response").is_empty());

    let servers = language_servers(&wb, cx);
    let key = lucerna_lsp::ServerKey {
        root: Arc::from(dir.path()),
        server: "vtsls".into(),
    };
    servers.update(cx, |servers, cx| servers.stop(&key, cx));
    settle(cx);

    assert_eq!(count(&vtsls, "workspace/executeCommand"), 0);
    assert_eq!(sent(&vue, "tsserver/response"), vec![json!([[7, null]])]);
}

#[gpui::test]
fn relay_does_not_block_other_events(cx: &mut gpui::TestAppContext) {
    let dir = vue_project();
    let vue = vue_server(quickinfo());
    let vtsls = ts_server().hang_on("workspace/executeCommand");
    let launcher = launcher()
        .with_server("vue-language-server", vue.clone())
        .with_server("vtsls", vtsls.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);

    open(&wb, "App.vue", cx);
    assert_eq!(count(&vtsls, "workspace/executeCommand"), 1);
    assert!(sent(&vue, "tsserver/response").is_empty());

    // The vue server keeps talking while its request is unanswered.
    vue.notify(
        "window/logMessage",
        json!({ "type": 3, "message": "still here" }),
    );
    settle(cx);
    let servers = language_servers(&wb, cx);
    let logged = cx.update(|_, cx| {
        servers
            .read(cx)
            .log("vue-language-server")
            .iter()
            .any(|line| line.text == "still here")
    });
    assert!(logged);

    let id = vtsls.hung_ids("workspace/executeCommand")[0].clone();
    assert!(vtsls.answer_hung(&id, json!({ "body": { "kind": "let" } })));
    settle(cx);
    assert_eq!(
        sent(&vue, "tsserver/response"),
        vec![json!([[7, { "kind": "let" }]])]
    );
}

#[gpui::test]
fn relay_ignores_requests_from_servers_without_relay(cx: &mut gpui::TestAppContext) {
    let dir = vue_project();
    // vtsls itself sends `tsserver/request`: it has no `relay`.
    let vtsls = vue_server(quickinfo()).on_request("workspace/executeCommand", |_| Ok(json!({})));
    let launcher = launcher().with_server("vtsls", vtsls.clone());
    let settings = r#"{
        "session.trustAllWorktrees": true,
        "lsp": { "vue-language-server": { "enabled": false } }
    }"#;
    let (wb, cx) = lsp_workbench_with(dir.path(), Arc::new(launcher), settings, cx);

    open(&wb, "App.vue", cx);

    assert_eq!(count(&vtsls, "textDocument/didOpen"), 1);
    assert_eq!(count(&vtsls, "workspace/executeCommand"), 0);
    assert!(sent(&vtsls, "tsserver/response").is_empty());
}
