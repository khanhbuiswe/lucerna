//! Document sync between the workbench and language servers (spec §4,
//! §6), with in-process fake servers on GPUI's test scheduler.

mod support;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{BorrowAppContext, Entity, TestAppContext, VisualTestContext};
use lucerna_lsp::ServerKey;
use lucerna_lsp::position::path_to_uri;
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::Layer;
use lucerna_syntax::Associations;
use lucerna_workbench::{AppState, LanguageServersEvent, SlotState, Workbench};
use serde_json::json;
use support::lsp::*;
use tempfile::TempDir;

/// A folder with `Cargo.toml` and `src/main.rs`.
fn rust_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    dir
}

fn uri(path: &Path) -> String {
    path_to_uri(path).as_str().to_owned()
}

fn changed(wb: &Entity<Workbench>, path: &str, kind: ChangeKind, cx: &mut VisualTestContext) {
    let ev = WorktreeEvent::Changed(vec![Change {
        path: p(path),
        kind,
    }]);
    wb.update(cx, |wb, cx| wb.on_worktree_event(&ev, cx));
    settle(cx);
}

fn key(root: &Path, server: &'static str) -> ServerKey {
    ServerKey {
        root: Arc::from(root),
        server,
    }
}

fn state_of(wb: &Entity<Workbench>, key: &ServerKey, cx: &mut VisualTestContext) -> SlotState {
    let servers = language_servers(wb, cx);
    cx.update(|_, cx| {
        servers
            .read(cx)
            .statuses()
            .into_iter()
            .find(|s| &s.key == key)
            .unwrap_or_else(|| panic!("no status for {key:?}"))
            .state
    })
}

#[gpui::test]
fn opening_rust_file_starts_server_and_sends_did_open(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);

    open(&wb, "src/main.rs", cx);

    let opens = sent(&fake, "textDocument/didOpen");
    assert_eq!(opens.len(), 1, "{:?}", methods(&fake));
    let doc = &opens[0]["textDocument"];
    assert_eq!(doc["languageId"], "rust");
    assert_eq!(doc["version"], 0);
    assert_eq!(doc["text"], "fn main() {}\n");
    assert_eq!(doc["uri"], uri(&dir.path().join("src/main.rs")));
    let order = methods(&fake);
    assert_eq!(
        &order[..3],
        [
            "initialize",
            "initialized",
            "workspace/didChangeConfiguration"
        ]
    );
    assert_eq!(
        state_of(&wb, &key(dir.path(), "rust-analyzer"), cx),
        SlotState::Running
    );
}

#[gpui::test]
fn two_tabs_one_did_open_close_after_last(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);

    open(&wb, "src/main.rs", cx);
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| {
            m.split_active();
        })
    });
    settle(cx);
    assert_eq!(sent(&fake, "textDocument/didOpen").len(), 1);

    close_active_tab(&wb, cx);
    assert!(
        sent(&fake, "textDocument/didClose").is_empty(),
        "one view is still open"
    );
    close_active_tab(&wb, cx);
    let closes = sent(&fake, "textDocument/didClose");
    assert_eq!(closes.len(), 1);
    assert_eq!(
        closes[0]["textDocument"]["uri"],
        uri(&dir.path().join("src/main.rs"))
    );
}

#[gpui::test]
fn reload_sends_did_change_full_text(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    open(&wb, "src/main.rs", cx);

    fs::write(dir.path().join("src/main.rs"), "fn main() { let x = 1; }\n").unwrap();
    changed(&wb, "src/main.rs", ChangeKind::Modified, cx);

    let changes = sent(&fake, "textDocument/didChange");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["textDocument"]["version"], 1);
    assert_eq!(
        changes[0]["contentChanges"],
        json!([{ "text": "fn main() { let x = 1; }\n" }])
    );
}

#[gpui::test]
fn untouched_reload_sends_nothing(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    open(&wb, "src/main.rs", cx);

    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    changed(&wb, "src/main.rs", ChangeKind::Modified, cx);

    assert!(sent(&fake, "textDocument/didChange").is_empty());
}

#[gpui::test]
fn missing_binary_is_not_found_status(cx: &mut TestAppContext) {
    let dir = rust_project();
    fs::write(dir.path().join("src/lib.rs"), "\n").unwrap();
    let launcher = Arc::new(FakeLauncher::new());
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), TRUSTED, cx);

    open(&wb, "src/main.rs", cx);
    open(&wb, "src/lib.rs", cx);

    let servers = language_servers(&wb, cx);
    let statuses = cx.update(|_, cx| servers.read(cx).statuses());
    assert_eq!(statuses.len(), 1, "reported once per server");
    assert_eq!(statuses[0].key, key(dir.path(), "rust-analyzer"));
    assert_eq!(
        statuses[0].state,
        SlotState::NotFound {
            message: "rust-analyzer not found on PATH. Install it or set \"lsp\": { \"rust-analyzer\": { \"binary\": { \"path\": … } } }".into()
        }
    );
    assert!(launcher.launches().is_empty());
}

#[gpui::test]
fn crash_marks_failed_and_clears_diagnostics(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new().on_notification("textDocument/didOpen", |params, peer| {
        peer.notify(
            "textDocument/publishDiagnostics",
            json!({
                "uri": params["textDocument"]["uri"],
                "version": 0,
                "diagnostics": [{
                    "range": { "start": { "line": 0, "character": 3 },
                               "end": { "line": 0, "character": 7 } },
                    "severity": 1,
                    "message": "boom",
                }],
            }),
        );
    });
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    let servers = language_servers(&wb, cx);
    let changed_paths: Rc<RefCell<Vec<Vec<PathBuf>>>> = Rc::default();
    let _sub = {
        let changed_paths = changed_paths.clone();
        cx.update(|_, cx| {
            cx.subscribe(&servers, move |_, ev: &LanguageServersEvent, _| {
                if let LanguageServersEvent::DiagnosticsChanged(paths) = ev {
                    changed_paths
                        .borrow_mut()
                        .push(paths.iter().map(|p| p.to_path_buf()).collect());
                }
            })
        })
    };
    let main = dir.path().join("src/main.rs");

    open(&wb, "src/main.rs", cx);
    let diagnostics = cx.update(|_, cx| servers.read(cx).diagnostics().for_file(&main));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "boom");
    assert_eq!(&*changed_paths.borrow(), &[vec![main.clone()]]);

    fake.crash(Some(101));
    settle(cx);

    assert_eq!(
        state_of(&wb, &key(dir.path(), "rust-analyzer"), cx),
        SlotState::Failed {
            message: "exited with code 101".into(),
            stderr_tail: Vec::new(),
        }
    );
    let diagnostics = cx.update(|_, cx| servers.read(cx).diagnostics().for_file(&main));
    assert!(diagnostics.is_empty());
    assert_eq!(changed_paths.borrow().last(), Some(&vec![main.clone()]));
    let log = cx.update(|_, cx| servers.read(cx).log("rust-analyzer"));
    assert!(
        log.iter().any(|l| l.text == "Exited with code 101"),
        "{log:?}"
    );
}

#[gpui::test]
fn deleted_file_sends_did_close(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    open(&wb, "src/main.rs", cx);

    fs::remove_file(dir.path().join("src/main.rs")).unwrap();
    changed(&wb, "src/main.rs", ChangeKind::Removed, cx);

    assert_eq!(sent(&fake, "textDocument/didClose").len(), 1);
}

#[gpui::test]
fn language_change_reopens_with_new_server(cx: &mut TestAppContext) {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
    let python = FakeServer::new();
    let go = FakeServer::new();
    let launcher = FakeLauncher::new()
        .with_server("basedpyright-langserver", python.clone())
        .with_server("gopls", go.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    open(&wb, "a.py", cx);
    let opens = sent(&python, "textDocument/didOpen");
    assert_eq!(opens.len(), 1);
    assert_eq!(opens[0]["textDocument"]["languageId"], "python");

    cx.update(|_, cx| {
        cx.update_global::<AppState, _>(|state, _| {
            let map = BTreeMap::from([("*.py".to_owned(), "go".to_owned())]);
            state.languages = Arc::new(state.languages.with_associations(Associations::new(&map)));
            state.languages_generation += 1;
        });
    });
    settle(cx);

    assert_eq!(sent(&python, "textDocument/didClose").len(), 1);
    let opens = sent(&go, "textDocument/didOpen");
    assert_eq!(opens.len(), 1);
    assert_eq!(opens[0]["textDocument"]["languageId"], "go");
    assert_eq!(opens[0]["textDocument"]["text"], "x = 1\n");
}

#[gpui::test]
fn disabled_by_setting_starts_nothing(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let settings = r#"{ "session.trustAllWorktrees": true, "lucerna.lsp.enabled": false }"#;
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), settings, cx);

    open(&wb, "src/main.rs", cx);

    assert!(launcher.launches().is_empty());
    assert!(fake.received().is_empty());
    let servers = language_servers(&wb, cx);
    assert!(cx.update(|_, cx| servers.read(cx).statuses()).is_empty());
}

#[gpui::test]
fn untrusted_folder_starts_nothing(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), "{}", cx);

    open(&wb, "src/main.rs", cx);

    assert!(launcher.launches().is_empty());
    let servers = language_servers(&wb, cx);
    assert!(!cx.update(|_, cx| servers.read(cx).is_trusted()));

    // Trusting opens what is already open.
    servers.update(cx, |servers, cx| servers.set_trusted(true, cx));
    settle(cx);
    assert_eq!(sent(&fake, "textDocument/didOpen").len(), 1);
}

#[gpui::test]
fn settings_change_restarts_on_binary_change(cx: &mut TestAppContext) {
    let dir = rust_project();
    let first = FakeServer::new();
    let second = FakeServer::new();
    let launcher = Arc::new(
        FakeLauncher::new()
            .with_server("rust-analyzer", first.clone())
            .with_server("ra-custom", second.clone()),
    );
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);
    assert_eq!(launcher.launches().len(), 1);

    cx.update(|_, cx| {
        cx.update_global::<AppState, _>(|state, _| {
            state
                .settings
                .set_layer(
                    Layer::User,
                    r#"{
                        "session.trustAllWorktrees": true,
                        "lsp": { "rust-analyzer": { "binary": { "path": "/opt/ra/ra-custom" } } }
                    }"#,
                )
                .unwrap();
        });
    });
    settle(cx);

    let launches = launcher.launches();
    assert_eq!(launches.len(), 2);
    assert_eq!(launches[1].program, Path::new("/opt/ra/ra-custom"));
    // The old process is shut down on its own thread; not awaited here.
    assert_eq!(sent(&first, "textDocument/didOpen").len(), 1);
    let opens = sent(&second, "textDocument/didOpen");
    assert_eq!(opens.len(), 1, "the open file is sent to the new server");
    assert_eq!(
        state_of(&wb, &key(dir.path(), "rust-analyzer"), cx),
        SlotState::Running
    );
}

#[gpui::test]
fn watched_file_event_forwarded(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new().on_notification("initialized", |_, peer| {
        peer.request(
            "client/registerCapability",
            json!({ "registrations": [{
                "id": "watch-toml",
                "method": "workspace/didChangeWatchedFiles",
                "registerOptions": { "watchers": [{ "globPattern": "**/*.toml" }] },
            }] }),
        );
    });
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    open(&wb, "src/main.rs", cx);

    let ev = WorktreeEvent::Changed(vec![
        Change {
            path: p("Cargo.toml"),
            kind: ChangeKind::Modified,
        },
        Change {
            path: p("src/other.rs"),
            kind: ChangeKind::Added,
        },
    ]);
    wb.update(cx, |wb, cx| wb.on_worktree_event(&ev, cx));
    assert!(
        sent(&fake, "workspace/didChangeWatchedFiles").is_empty(),
        "batched for 100 ms"
    );
    settle(cx);

    let batches = sent(&fake, "workspace/didChangeWatchedFiles");
    assert_eq!(batches.len(), 1);
    assert_eq!(
        batches[0]["changes"],
        json!([{ "uri": uri(&dir.path().join("Cargo.toml")), "type": 2 }])
    );
}
