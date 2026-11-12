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

fn key(root: &Path, server: &str) -> ServerKey {
    ServerKey {
        root: Arc::from(root),
        server: server.into(),
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
    assert_eq!(sent(&first, "textDocument/didOpen").len(), 1);
    // The old process is shut down on its own thread.
    wait_until("old server shut down", || count(&first, "shutdown") == 1);
    assert_eq!(count(&second, "shutdown"), 0);
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

#[gpui::test]
fn running_for_accepts_a_relative_path(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    open(&wb, "src/main.rs", cx);

    let servers = language_servers(&wb, cx);
    let relative = cx.update(|_, cx| servers.read(cx).running_for(Path::new("src/main.rs")));
    let absolute = cx.update(|_, cx| {
        servers
            .read(cx)
            .running_for(&dir.path().join("src/main.rs"))
    });
    assert_eq!(relative.len(), 1);
    assert_eq!(absolute.len(), 1);
    assert_eq!(relative[0].0, key(dir.path(), "rust-analyzer"));
}

#[gpui::test]
fn stop_while_starting_shuts_the_process_down(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new().hang_on("initialize");
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    open(&wb, "src/main.rs", cx);
    let key = key(dir.path(), "rust-analyzer");
    assert_eq!(state_of(&wb, &key, cx), SlotState::Starting);

    let servers = language_servers(&wb, cx);
    servers.update(cx, |servers, cx| servers.stop(&key, cx));
    settle(cx);

    assert_eq!(state_of(&wb, &key, cx), SlotState::Stopped);
    wait_until("starting server shut down", || {
        count(&fake, "shutdown") == 1
    });
    assert_eq!(count(&fake, "textDocument/didOpen"), 0);
}

#[gpui::test]
fn closing_a_document_while_starting_sends_nothing(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new().hang_on("initialize");
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    open(&wb, "src/main.rs", cx);

    close_active_tab(&wb, cx);

    assert_eq!(count(&fake, "textDocument/didOpen"), 0);
    assert_eq!(count(&fake, "textDocument/didClose"), 0);
    assert_eq!(
        state_of(&wb, &key(dir.path(), "rust-analyzer"), cx),
        SlotState::Starting
    );
}

#[gpui::test]
fn untrusting_stops_every_server(cx: &mut TestAppContext) {
    let dir = rust_project();
    fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
    let rust = FakeServer::new();
    let python = FakeServer::new().hang_on("initialize");
    let launcher = FakeLauncher::new()
        .with_server("rust-analyzer", rust.clone())
        .with_server("basedpyright-langserver", python.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    open(&wb, "src/main.rs", cx);
    open(&wb, "a.py", cx);

    let servers = language_servers(&wb, cx);
    servers.update(cx, |servers, cx| servers.set_trusted(false, cx));
    settle(cx);

    assert!(cx.update(|_, cx| servers.read(cx).statuses()).is_empty());
    wait_until("running server shut down", || count(&rust, "shutdown") == 1);
    wait_until("starting server shut down", || {
        count(&python, "shutdown") == 1
    });
}

#[gpui::test]
fn shutdown_all_waits_for_running_and_starting_servers(cx: &mut TestAppContext) {
    let dir = rust_project();
    fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
    let rust = FakeServer::new();
    let python = FakeServer::new().hang_on("initialize");
    let launcher = FakeLauncher::new()
        .with_server("rust-analyzer", rust.clone())
        .with_server("basedpyright-langserver", python.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    open(&wb, "src/main.rs", cx);
    open(&wb, "a.py", cx);

    let servers = language_servers(&wb, cx);
    servers.update(cx, |servers, _| servers.shutdown_all());

    // Joined before returning: no waiting needed.
    assert_eq!(count(&rust, "shutdown"), 1);
    assert_eq!(count(&rust, "exit"), 1);
    assert_eq!(count(&python, "shutdown"), 1);
    let statuses = cx.update(|_, cx| servers.read(cx).statuses());
    assert_eq!(statuses.len(), 2);
    assert!(statuses.iter().all(|s| s.state == SlotState::Stopped));
}

#[gpui::test]
fn file_growing_past_the_large_file_limit_leaves_its_servers(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let settings = r#"{ "session.trustAllWorktrees": true, "lucerna.largeFile.maxLines": 5 }"#;
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher, settings, cx);
    open(&wb, "src/main.rs", cx);
    assert_eq!(count(&fake, "textDocument/didOpen"), 1);

    fs::write(dir.path().join("src/main.rs"), "//\n".repeat(20)).unwrap();
    changed(&wb, "src/main.rs", ChangeKind::Modified, cx);
    assert_eq!(
        count(&fake, "textDocument/didChange"),
        0,
        "no full text past the limit"
    );
    assert_eq!(count(&fake, "textDocument/didClose"), 1);

    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    changed(&wb, "src/main.rs", ChangeKind::Modified, cx);
    let opens = sent(&fake, "textDocument/didOpen");
    assert_eq!(opens.len(), 2, "back under the limit: opened again");
    assert_eq!(opens[1]["textDocument"]["text"], "fn main() {}\n");
}

/// A crate outside the workspace (like `~/.cargo/registry/.../x.rs`).
fn outside_crate() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"dep\"\n").unwrap();
    fs::write(dir.path().join("lib.rs"), "pub fn dep() {}\n").unwrap();
    dir
}

#[gpui::test]
fn file_outside_the_workspace_joins_the_running_server_without_a_launch(cx: &mut TestAppContext) {
    let dir = rust_project();
    let outside = outside_crate();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);
    let lib = outside.path().join("lib.rs");
    wb.update(cx, |wb, cx| {
        wb.open_path(
            Arc::from(lib.as_path()),
            lucerna_workbench::OpenMode::Pinned,
            cx,
        )
    });
    settle(cx);

    assert_eq!(
        launcher.launches().len(),
        1,
        "no server for the outside root"
    );
    let opened: Vec<_> = sent(&fake, "textDocument/didOpen")
        .into_iter()
        .map(|p| p["textDocument"]["uri"].as_str().unwrap().to_owned())
        .collect();
    assert!(opened.contains(&uri(&lib)), "{opened:?}");
    let servers = language_servers(&wb, cx);
    let statuses = cx.update(|_, cx| servers.read(cx).statuses());
    assert_eq!(statuses.len(), 1, "{statuses:?}");
}

#[gpui::test]
fn file_outside_the_workspace_alone_starts_nothing(cx: &mut TestAppContext) {
    let dir = rust_project();
    let outside = outside_crate();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), TRUSTED, cx);
    let lib = outside.path().join("lib.rs");
    wb.update(cx, |wb, cx| {
        wb.open_path(
            Arc::from(lib.as_path()),
            lucerna_workbench::OpenMode::Pinned,
            cx,
        )
    });
    settle(cx);
    assert!(launcher.launches().is_empty());
    assert!(sent(&fake, "textDocument/didOpen").is_empty());
}

#[gpui::test]
fn quit_shares_one_deadline_across_windows_and_kills_hung_servers(cx: &mut TestAppContext) {
    let (a, b) = (rust_project(), rust_project());
    let (fa, fb) = (
        FakeServer::new().hang_on("shutdown"),
        FakeServer::new().hang_on("shutdown"),
    );
    let mut app = cx.clone();
    let (wa, vcx) = lsp_workbench(
        a.path(),
        FakeLauncher::new().with_server("rust-analyzer", fa.clone()),
        cx,
    );
    open(&wa, "src/main.rs", vcx);
    let root_b = b.path().to_path_buf();
    let (wb2, vcx2) = app.add_window_view(|window, cx| Workbench::new(root_b, window, cx));
    let servers_b = language_servers(&wb2, vcx2);
    servers_b.update(vcx2, |servers, _| {
        servers.set_launcher(Arc::new(
            FakeLauncher::new().with_server("rust-analyzer", fb.clone()),
        ));
    });
    open(&wb2, "src/main.rs", vcx2);
    assert!(fa.received().iter().any(|(m, _)| m == "initialize"));
    assert!(fb.received().iter().any(|(m, _)| m == "initialize"));

    let begun = std::time::Instant::now();
    app.quit();
    let took = begun.elapsed();
    assert!(
        took < std::time::Duration::from_secs(7),
        "one shared 5 s deadline, not 5 s per window: {took:?}"
    );
    assert!(fa.is_closed() && fb.is_closed(), "hung servers killed");
}

#[gpui::test]
fn quit_kills_servers_still_running_at_the_deadline(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new().hang_on("shutdown");
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    open(&wb, "src/main.rs", cx);

    let begun = std::time::Instant::now();
    cx.update(|_, cx| {
        lucerna_workbench::LanguageServers::shutdown_all_windows_within(
            std::time::Duration::from_millis(200),
            cx,
        );
    });
    assert!(begun.elapsed() < std::time::Duration::from_secs(2));
    assert!(fake.is_closed(), "killed at the deadline");
    assert_eq!(count(&fake, "exit"), 0, "never reached exit");
}

#[gpui::test]
fn closing_the_window_shuts_servers_down(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    open(&wb, "src/main.rs", cx);

    drop(wb);
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();

    wait_until("server shut down", || count(&fake, "exit") == 1);
}

// ---- The server table (language server table spec §3–§4) ----

/// Replaces the User settings layer and settles.
fn set_user_settings(text: &str, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        cx.update_global::<AppState, _>(|state, _| {
            state.settings.set_layer(Layer::User, text).unwrap();
        });
    });
    settle(cx);
}

/// User settings: trusted, with `lsp` set to `lsp`.
fn trusted_with_lsp(lsp: &serde_json::Value) -> String {
    json!({ "session.trustAllWorktrees": true, "lsp": lsp }).to_string()
}

#[gpui::test]
fn language_less_file_starts_file_type_server(cx: &mut TestAppContext) {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("x.rb"), "puts 1\n").unwrap();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("fake-rb", fake.clone()));
    let settings = trusted_with_lsp(&json!({
        "fake-rb": { "command": "fake-rb", "file_types": ["rb"], "language_id": "ruby" }
    }));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), &settings, cx);

    open(&wb, "x.rb", cx);

    assert_eq!(launcher.launches().len(), 1);
    let opens = sent(&fake, "textDocument/didOpen");
    assert_eq!(opens.len(), 1, "{:?}", methods(&fake));
    assert_eq!(opens[0]["textDocument"]["languageId"], "ruby");
    assert_eq!(opens[0]["textDocument"]["text"], "puts 1\n");
    assert_eq!(
        state_of(&wb, &key(dir.path(), "fake-rb"), cx),
        SlotState::Running
    );
}

#[gpui::test]
fn file_type_and_language_share_one_server(cx: &mut TestAppContext) {
    let dir = rust_project();
    fs::write(dir.path().join("src/gen.rsx"), "x\n").unwrap();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let settings = trusted_with_lsp(&json!({ "rust-analyzer": { "file_types": ["rsx"] } }));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), &settings, cx);

    open(&wb, "src/main.rs", cx);
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| {
            m.split_active();
        })
    });
    settle(cx);
    open(&wb, "src/gen.rsx", cx);

    assert_eq!(launcher.launches().len(), 1, "one process for both files");
    let opens = sent(&fake, "textDocument/didOpen");
    assert_eq!(opens.len(), 2);
    let language_of = |file: &str| {
        let uri = uri(&dir.path().join(file));
        opens
            .iter()
            .find(|o| o["textDocument"]["uri"] == uri)
            .map(|o| o["textDocument"]["languageId"].clone())
    };
    assert_eq!(language_of("src/main.rs"), Some(json!("rust")));
    assert_eq!(language_of("src/gen.rsx"), Some(json!("rust")));
    let servers = language_servers(&wb, cx);
    let keys = cx.update(|_, cx| {
        let s = servers.read(cx);
        (
            s.servers_for(Path::new("src/main.rs")),
            s.servers_for(Path::new("src/gen.rsx")),
        )
    });
    assert_eq!(keys.0, [key(dir.path(), "rust-analyzer")]);
    assert_eq!(keys.0, keys.1);
}

#[gpui::test]
fn user_defined_server_runs(cx: &mut TestAppContext) {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("mine-ls", fake.clone()));
    let settings = trusted_with_lsp(&json!({
        "mine": { "command": "mine-ls", "args": ["--stdio"], "languages": ["python"] }
    }));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), &settings, cx);

    open(&wb, "a.py", cx);

    let launches = launcher.launches();
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].server, "mine");
    assert_eq!(launches[0].program, Path::new("/fake/bin/mine-ls"));
    assert_eq!(launches[0].args, ["--stdio"]);
    let opens = sent(&fake, "textDocument/didOpen");
    assert_eq!(opens.len(), 1);
    assert_eq!(opens[0]["textDocument"]["languageId"], "python");
    let servers = language_servers(&wb, cx);
    let table_has_mine = cx.update(|_, cx| servers.read(cx).table().get("mine").is_some());
    assert!(table_has_mine);
}

#[gpui::test]
fn invalid_table_entry_is_reported_and_others_run(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let settings = trusted_with_lsp(&json!({ "bad": { "command": "bad", "args": "--stdio" } }));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), &settings, cx);

    open(&wb, "src/main.rs", cx);

    assert_eq!(sent(&fake, "textDocument/didOpen").len(), 1);
    let servers = language_servers(&wb, cx);
    let errors = cx.update(|_, cx| servers.read(cx).table_errors().to_vec());
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].server, "bad");
}

#[gpui::test]
fn settings_only_override_reconfigures(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);
    assert_eq!(count(&fake, "workspace/didChangeConfiguration"), 1);

    set_user_settings(
        &trusted_with_lsp(&json!({ "rust-analyzer": { "settings": { "a": "{root}" } } })),
        cx,
    );

    assert_eq!(launcher.launches().len(), 1, "no second launch");
    assert_eq!(count(&fake, "shutdown"), 0);
    let configs = sent(&fake, "workspace/didChangeConfiguration");
    assert_eq!(configs.len(), 2);
    assert_eq!(
        configs[1]["settings"],
        json!({ "a": dir.path().to_string_lossy() }),
        "settings are expanded"
    );
    assert_eq!(
        state_of(&wb, &key(dir.path(), "rust-analyzer"), cx),
        SlotState::Running
    );
}

#[gpui::test]
fn disabling_server_stops_it(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);

    set_user_settings(
        &trusted_with_lsp(&json!({ "rust-analyzer": { "enabled": false } })),
        cx,
    );

    wait_until("server shut down", || count(&fake, "shutdown") == 1);
    let servers = language_servers(&wb, cx);
    assert!(cx.update(|_, cx| servers.read(cx).statuses()).is_empty());
    assert!(
        cx.update(|_, cx| servers.read(cx).servers_for(Path::new("src/main.rs")))
            .is_empty()
    );
    assert_eq!(launcher.launches().len(), 1);

    // Enabled again: it starts for the open file.
    set_user_settings(TRUSTED, cx);
    assert_eq!(launcher.launches().len(), 2);
    assert_eq!(sent(&fake, "textDocument/didOpen").len(), 2);
}

#[gpui::test]
fn data_dir_variable_passed_to_server(cx: &mut TestAppContext) {
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    init_with(
        &trusted_with_lsp(&json!({ "rust-analyzer": { "args": ["-data", "{data_dir}"] } })),
        cx,
    );
    let (root, data_dir) = (dir.path().to_path_buf(), data.path().to_path_buf());
    let (wb, cx) =
        cx.add_window_view(|window, cx| Workbench::with_data_dir(root, data_dir, window, cx));
    let servers = language_servers(&wb, cx);
    servers.update(cx, |servers, _| servers.set_launcher(launcher.clone()));
    settle(cx);

    open(&wb, "src/main.rs", cx);

    let expected = lucerna_lsp::vars::data_dir_for(data.path(), "rust-analyzer", dir.path());
    assert!(expected.starts_with(data.path().join("lsp").join("rust-analyzer")));
    let launches = launcher.launches();
    assert_eq!(launches.len(), 1);
    assert_eq!(
        launches[0].args,
        ["-data".to_owned(), expected.to_string_lossy().into_owned()]
    );
    assert!(expected.is_dir(), "the data dir is created");
    assert_eq!(sent(&fake, "textDocument/didOpen").len(), 1);
}

#[gpui::test]
fn npm_root_failure_marks_failed(cx: &mut TestAppContext) {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.vue"), "<template/>\n").unwrap();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("vue-ls", fake.clone()));
    let settings = trusted_with_lsp(&json!({
        "vue": { "command": "vue-ls", "file_types": ["vue"],
                 "settings": { "location": "{npm_root}/@vue/language-server" } }
    }));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), &settings, cx);

    open(&wb, "a.vue", cx);

    assert!(launcher.launches().is_empty());
    let SlotState::Failed { message, .. } = state_of(&wb, &key(dir.path(), "vue"), cx) else {
        panic!("expected Failed");
    };
    assert_eq!(message, "npm not found; {npm_root} unavailable");
}

#[gpui::test]
fn npm_root_expands_in_settings(cx: &mut TestAppContext) {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.vue"), "<template/>\n").unwrap();
    let fake = FakeServer::new();
    let launcher = Arc::new(
        FakeLauncher::new()
            .with_server("vue-ls", fake.clone())
            .with_output("npm", &["root", "-g"], "/opt/node/lib\n"),
    );
    let settings = trusted_with_lsp(&json!({
        "vue": { "command": "vue-ls", "file_types": ["vue"],
                 "initialization_options": { "ts": "{npm_root}/typescript" },
                 "settings": { "location": "{npm_root}/@vue/language-server" } }
    }));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), &settings, cx);

    open(&wb, "a.vue", cx);

    assert_eq!(launcher.launches().len(), 1);
    let init = sent(&fake, "initialize");
    assert_eq!(
        init[0]["initializationOptions"],
        json!({ "ts": "/opt/node/lib/typescript" })
    );
    let configs = sent(&fake, "workspace/didChangeConfiguration");
    assert_eq!(
        configs[0]["settings"],
        json!({ "location": "/opt/node/lib/@vue/language-server" })
    );
    assert_eq!(
        state_of(&wb, &key(dir.path(), "vue"), cx),
        SlotState::Running
    );
}

#[gpui::test]
fn bad_override_keeps_running_default_server(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);

    set_user_settings(
        &trusted_with_lsp(&json!({ "rust-analyzer": { "args": "x" } })),
        cx,
    );

    assert_eq!(launcher.launches().len(), 1, "not relaunched");
    assert_eq!(count(&fake, "shutdown"), 0, "not stopped");
    assert_eq!(
        state_of(&wb, &key(dir.path(), "rust-analyzer"), cx),
        SlotState::Running
    );
    let servers = language_servers(&wb, cx);
    let errors = cx.update(|_, cx| servers.read(cx).table_errors().to_vec());
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].server, "rust-analyzer");
}

#[gpui::test]
fn enabled_flip_keeps_server_selected_by_name(cx: &mut TestAppContext) {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("pyright-langserver", fake.clone()));
    let base = json!({
        "session.trustAllWorktrees": true,
        "[python]": { "lucerna.lsp.languageServers": ["pyright", "!basedpyright"] }
    });
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), &base.to_string(), cx);
    open(&wb, "a.py", cx);
    assert_eq!(launcher.launches().len(), 1);
    let pyright = key(dir.path(), "pyright");
    assert_eq!(state_of(&wb, &pyright, cx), SlotState::Running);

    for enabled in [true, false] {
        let mut settings = base.clone();
        settings["lsp"] = json!({ "pyright": { "enabled": enabled } });
        set_user_settings(&settings.to_string(), cx);
        assert_eq!(launcher.launches().len(), 1, "enabled: {enabled}");
        assert_eq!(count(&fake, "shutdown"), 0, "enabled: {enabled}");
        assert_eq!(state_of(&wb, &pyright, cx), SlotState::Running);
    }
    set_user_settings(&base.to_string(), cx);
    assert_eq!(launcher.launches().len(), 1);
    assert_eq!(count(&fake, "shutdown"), 0);
}

#[gpui::test]
fn typing_sends_incremental_change(cx: &mut TestAppContext) {
    use lsp_types::{
        PositionEncodingKind, ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind,
    };
    let dir = rust_project();
    let fake = FakeServer::new().capabilities(ServerCapabilities {
        position_encoding: Some(PositionEncodingKind::UTF16),
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        ..Default::default()
    });
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    let view = open(&wb, "src/main.rs", cx);
    let documents = cx.update(|_, cx| wb.read(cx).documents().clone());
    let origin = view.entity_id();

    // "fn main() {}\n": insert after `{`.
    documents.update(cx, |d, cx| {
        d.apply_edits(
            Path::new("src/main.rs"),
            vec![(11..11, Arc::from("x"))],
            None,
            None,
            origin,
            cx,
        )
    });
    settle(cx);

    let changes = sent(&fake, "textDocument/didChange");
    assert_eq!(changes.len(), 1, "{:?}", methods(&fake));
    assert_eq!(changes[0]["textDocument"]["version"], 1);
    assert_eq!(
        changes[0]["contentChanges"],
        json!([{
            "range": { "start": { "line": 0, "character": 11 },
                       "end": { "line": 0, "character": 11 } },
            "text": "x",
        }])
    );
}

/// Final review, finding 7: `didSave` with `includeText` sends the text
/// that was written, not an edit made while writing.
#[gpui::test]
fn did_save_sends_the_written_text(cx: &mut TestAppContext) {
    use lsp_types::{
        SaveOptions, ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind,
        TextDocumentSyncOptions, TextDocumentSyncSaveOptions,
    };
    let dir = rust_project();
    let fake = FakeServer::new().capabilities(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::FULL),
                save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                    include_text: Some(true),
                })),
                ..Default::default()
            },
        )),
        ..Default::default()
    });
    let launcher = FakeLauncher::new().with_server("rust-analyzer", fake.clone());
    let (wb, cx) = lsp_workbench(dir.path(), launcher, cx);
    let view = open(&wb, "src/main.rs", cx);
    let docs = cx.update(|_, cx| wb.read(cx).documents().clone());
    let path = Path::new("src/main.rs");
    let opts = lucerna_workbench::SaveOptions {
        reason: lucerna_workbench::SaveReason::Explicit,
        participants: false,
    };
    let save = cx.update(|window, cx| docs.update(cx, |d, cx| d.save(path, opts, window, cx)));
    // The write runs on a background thread, which has not run yet.
    let id = view.entity_id();
    docs.update(cx, |d, cx| {
        d.apply_edits(path, vec![(0..0, Arc::from("// x\n"))], None, None, id, cx)
    })
    .expect("loaded");
    settle(cx);
    assert_eq!(futures::FutureExt::now_or_never(save), Some(Ok(true)));
    assert_eq!(
        fs::read_to_string(dir.path().join("src/main.rs")).unwrap(),
        "fn main() {}\n"
    );
    let saves = sent(&fake, "textDocument/didSave");
    assert_eq!(saves.len(), 1, "{:?}", methods(&fake));
    assert_eq!(saves[0]["text"], "fn main() {}\n");
}
