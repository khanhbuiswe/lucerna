//! Folder trust (spec §5): the trust store, Restricted Mode, the trust
//! modal and `session.trustAllWorktrees`, with in-process fake servers.

mod support;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use gpui::{BorrowAppContext, Entity, Modifiers, TestAppContext, VisualTestContext};
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_settings::Layer;
use lucerna_workbench::{AppState, TrustStore, Workbench};
use serde_json::Value;
use support::lsp::*;
use tempfile::TempDir;

/// A folder with `Cargo.toml`, `src/main.rs`, `src/lib.rs` and `notes.txt`.
fn rust_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(dir.path().join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    fs::write(dir.path().join("notes.txt"), "plain text\n").unwrap();
    dir
}

/// A workbench on `root` keeping its state (and `trusted.json`) in
/// `data_dir`, with `user` / `workspace` settings layers and servers from
/// `launcher`.
fn trust_workbench<'a>(
    root: &Path,
    data_dir: &Path,
    launcher: Arc<FakeLauncher>,
    user: &str,
    workspace: Option<&str>,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    init_with(user, cx);
    if let Some(workspace) = workspace {
        cx.update_global::<AppState, _>(|state, _| {
            state
                .settings
                .set_layer(Layer::Workspace, workspace)
                .expect("valid workspace settings");
        });
    }
    let (root, data_dir) = (root.to_path_buf(), data_dir.to_path_buf());
    let (wb, cx) =
        cx.add_window_view(|window, cx| Workbench::with_data_dir(root, data_dir, window, cx));
    let servers = language_servers(&wb, cx);
    servers.update(cx, |servers, _| servers.set_launcher(launcher));
    settle(cx);
    (wb, cx)
}

fn rust_launcher(fake: &FakeServer) -> Arc<FakeLauncher> {
    Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()))
}

fn shown(selector: &'static str, cx: &mut VisualTestContext) -> bool {
    draw(cx);
    cx.debug_bounds(selector).is_some()
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    draw(cx);
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    settle(cx);
}

fn is_trusted(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    let servers = language_servers(wb, cx);
    cx.update(|_, cx| servers.read(cx).is_trusted())
}

fn trusted_json(data_dir: &Path) -> Value {
    let text = fs::read_to_string(data_dir.join("trusted.json")).expect("trusted.json written");
    serde_json::from_str(&text).expect("trusted.json is JSON")
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(wb.run_command(command, None, window, cx), "{command}");
        })
    });
    settle(cx);
}

#[test]
fn trust_store_parent_covers_children() {
    let data = TempDir::new().unwrap();
    let mut store = TrustStore::load(data.path());
    let (projects, app, other) = (
        Path::new("/work/projects"),
        Path::new("/work/projects/app"),
        Path::new("/work/other"),
    );
    assert!(!store.is_trusted(app));

    store.trust(app, true).unwrap();
    assert!(store.is_trusted(app));
    assert!(store.is_trusted(Path::new("/work/projects/sibling")));
    assert!(store.is_trusted(Path::new("/work/projects/sibling/deep")));
    assert!(store.is_trusted(projects));
    assert!(!store.is_trusted(other));
    assert!(!store.is_trusted(Path::new("/work/projectsX")));

    store.trust(other, false).unwrap();
    assert!(store.is_trusted(other));
    assert!(!store.is_trusted(Path::new("/work/other/sub")));

    // Saved as `{ "folders": [..], "parents": [..] }` and read back.
    let json = trusted_json(data.path());
    assert_eq!(json["folders"], serde_json::json!(["/work/other"]));
    assert_eq!(json["parents"], serde_json::json!(["/work/projects"]));
    let reloaded = TrustStore::load(data.path());
    assert!(reloaded.is_trusted(Path::new("/work/projects/sibling")));
    assert!(reloaded.is_trusted(other));

    let mut reloaded = reloaded;
    reloaded.clear().unwrap();
    assert!(!reloaded.is_trusted(other));
    assert!(!TrustStore::load(data.path()).is_trusted(app));
}

#[test]
fn corrupt_trust_file_is_empty() {
    let data = TempDir::new().unwrap();
    fs::write(data.path().join("trusted.json"), "{ not json").unwrap();
    let mut store = TrustStore::load(data.path());
    assert!(!store.is_trusted(Path::new("/a")));

    // The next change writes a valid file over it.
    store.trust(Path::new("/a"), false).unwrap();
    assert!(TrustStore::load(data.path()).is_trusted(Path::new("/a")));
}

#[gpui::test]
fn untrusted_folder_spawns_nothing(cx: &mut TestAppContext) {
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    let fake = FakeServer::new();
    let launcher = rust_launcher(&fake);
    let (wb, cx) = trust_workbench(dir.path(), data.path(), launcher.clone(), "{}", None, cx);

    assert!(shown("restricted-mode", cx), "the title bar shows it");
    open(&wb, "src/main.rs", cx);

    assert!(launcher.launches().is_empty());
    assert!(fake.received().is_empty());
    assert!(!is_trusted(&wb, cx));
    assert!(shown("restricted-mode", cx));
}

#[gpui::test]
fn modal_opens_on_first_server_file_and_trust_starts_server(cx: &mut TestAppContext) {
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    let fake = FakeServer::new();
    let (wb, cx) = trust_workbench(
        dir.path(),
        data.path(),
        rust_launcher(&fake),
        "{}",
        None,
        cx,
    );

    // Not when the folder opens, nor for a file without a server.
    assert!(!shown("trust-modal", cx));
    open(&wb, "notes.txt", cx);
    assert!(!shown("trust-modal", cx));

    open(&wb, "src/main.rs", cx);
    assert!(shown("trust-modal", cx));
    click("trust-continue", cx);

    assert!(!shown("trust-modal", cx));
    assert!(!shown("restricted-mode", cx));
    assert!(is_trusted(&wb, cx));
    assert_eq!(sent(&fake, "textDocument/didOpen").len(), 1);
    let json = trusted_json(data.path());
    assert_eq!(
        json["folders"],
        serde_json::json!([dir.path().to_string_lossy()])
    );
    assert_eq!(json["parents"], serde_json::json!([]));
}

#[gpui::test]
fn stay_does_not_reopen(cx: &mut TestAppContext) {
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    let fake = FakeServer::new();
    let launcher = rust_launcher(&fake);
    let (wb, cx) = trust_workbench(dir.path(), data.path(), launcher.clone(), "{}", None, cx);

    open(&wb, "src/main.rs", cx);
    assert!(shown("trust-modal", cx));
    click("trust-stay", cx);
    assert!(!shown("trust-modal", cx));

    open(&wb, "src/lib.rs", cx);
    assert!(!shown("trust-modal", cx), "Stay holds for the session");
    assert!(shown("restricted-mode", cx));
    assert!(launcher.launches().is_empty());
    assert!(!data.path().join("trusted.json").exists());

    // The title bar button still opens it.
    click("restricted-mode", cx);
    assert!(shown("trust-modal", cx));
}

#[gpui::test]
fn modal_keys_escape_stays_enter_trusts(cx: &mut TestAppContext) {
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    let fake = FakeServer::new();
    let (wb, cx) = trust_workbench(
        dir.path(),
        data.path(),
        rust_launcher(&fake),
        "{}",
        None,
        cx,
    );

    open(&wb, "src/main.rs", cx);
    assert!(shown("trust-modal", cx));
    cx.simulate_keystrokes("escape");
    settle(cx);
    assert!(!shown("trust-modal", cx));
    assert!(!is_trusted(&wb, cx));

    run(&wb, "workbench.trust.manage", cx);
    assert!(shown("trust-modal", cx));
    cx.simulate_keystrokes("enter");
    settle(cx);
    assert!(!shown("trust-modal", cx));
    assert!(is_trusted(&wb, cx));
    assert_eq!(sent(&fake, "textDocument/didOpen").len(), 1);
}

#[gpui::test]
fn parent_checkbox_trusts_the_parent_folder(cx: &mut TestAppContext) {
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    let fake = FakeServer::new();
    let (wb, cx) = trust_workbench(
        dir.path(),
        data.path(),
        rust_launcher(&fake),
        "{}",
        None,
        cx,
    );

    run(&wb, "workbench.trust.manage", cx);
    click("trust-parent-checkbox", cx);
    click("trust-continue", cx);

    assert!(is_trusted(&wb, cx));
    let parent = dir.path().parent().unwrap();
    let json = trusted_json(data.path());
    assert_eq!(json["folders"], serde_json::json!([]));
    assert_eq!(
        json["parents"],
        serde_json::json!([parent.to_string_lossy()])
    );
}

#[gpui::test]
fn trusted_folder_starts_servers_and_clear_restricts(cx: &mut TestAppContext) {
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    TrustStore::load(data.path())
        .trust(dir.path(), false)
        .unwrap();
    let fake = FakeServer::new();
    let launcher = rust_launcher(&fake);
    let (wb, cx) = trust_workbench(dir.path(), data.path(), launcher.clone(), "{}", None, cx);

    assert!(!shown("restricted-mode", cx));
    open(&wb, "src/main.rs", cx);
    assert!(!shown("trust-modal", cx));
    assert_eq!(sent(&fake, "textDocument/didOpen").len(), 1);

    run(&wb, "workbench.trust.clear", cx);
    assert!(!is_trusted(&wb, cx));
    assert!(shown("restricted-mode", cx));
    wait_until("server shut down", || count(&fake, "shutdown") == 1);
    assert!(!TrustStore::load(data.path()).is_trusted(dir.path()));
}

#[gpui::test]
fn workspace_lsp_binary_path_ignored_until_trusted(cx: &mut TestAppContext) {
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    let fake = FakeServer::new();
    let launcher = Arc::new(
        FakeLauncher::new()
            .with_server("rust-analyzer", FakeServer::new())
            .with_server("false-path", fake.clone()),
    );
    let workspace =
        r#"{ "lsp": { "rust-analyzer": { "binary": { "path": "/bin/false-path" } } } }"#;
    let (wb, cx) = trust_workbench(
        dir.path(),
        data.path(),
        launcher.clone(),
        "{}",
        Some(workspace),
        cx,
    );

    open(&wb, "src/main.rs", cx);
    assert!(launcher.launches().is_empty());

    click("trust-continue", cx);
    let launches = launcher.launches();
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].program, Path::new("/bin/false-path"));
    assert_eq!(sent(&fake, "textDocument/didOpen").len(), 1);
}

#[gpui::test]
fn trust_all_worktrees_setting(cx: &mut TestAppContext) {
    // In the workspace layer it is ignored.
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    let fake = FakeServer::new();
    let launcher = rust_launcher(&fake);
    let (wb, cx) = trust_workbench(
        dir.path(),
        data.path(),
        launcher.clone(),
        "{}",
        Some(TRUSTED),
        cx,
    );
    assert!(shown("restricted-mode", cx));
    open(&wb, "src/main.rs", cx);
    assert!(launcher.launches().is_empty());

    // In the user layer it trusts every folder, without writing the store.
    cx.update(|_, cx| {
        cx.update_global::<AppState, _>(|state, _| {
            state.settings.set_layer(Layer::User, TRUSTED).unwrap();
        })
    });
    settle(cx);
    assert!(is_trusted(&wb, cx));
    assert!(!shown("restricted-mode", cx));
    assert_eq!(launcher.launches().len(), 1);
    assert_eq!(sent(&fake, "textDocument/didOpen").len(), 1);
    assert!(!data.path().join("trusted.json").exists());
}

#[gpui::test]
fn trust_all_worktrees_from_start_shows_nothing(cx: &mut TestAppContext) {
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    let fake = FakeServer::new();
    let (wb, cx) = trust_workbench(
        dir.path(),
        data.path(),
        rust_launcher(&fake),
        TRUSTED,
        None,
        cx,
    );
    assert!(!shown("restricted-mode", cx));
    open(&wb, "src/main.rs", cx);
    assert!(!shown("trust-modal", cx));
    assert_eq!(sent(&fake, "textDocument/didOpen").len(), 1);
    assert!(!data.path().join("trusted.json").exists());
}

#[gpui::test]
fn single_file_mode_trusts_the_file_folder(cx: &mut TestAppContext) {
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    let fake = FakeServer::new();
    init_with("{}", cx);
    let (root, file, data_dir) = (
        dir.path().join("src"),
        dir.path().join("src/main.rs"),
        data.path().to_path_buf(),
    );
    let (wb, cx) = cx.add_window_view(|window, cx| {
        Workbench::single_file(root, &file, Some(data_dir), window, cx)
    });
    let servers = language_servers(&wb, cx);
    servers.update(cx, |servers, _| servers.set_launcher(rust_launcher(&fake)));
    settle(cx);

    open(&wb, "main.rs", cx);
    assert!(shown("trust-modal", cx));
    click("trust-continue", cx);

    assert_eq!(sent(&fake, "textDocument/didOpen").len(), 1);
    let json = trusted_json(data.path());
    assert_eq!(
        json["folders"],
        serde_json::json!([dir.path().join("src").to_string_lossy()])
    );
}

#[gpui::test]
fn no_prompt_when_server_binary_missing(cx: &mut TestAppContext) {
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    let launcher = Arc::new(FakeLauncher::new());
    let (wb, cx) = trust_workbench(dir.path(), data.path(), launcher, "{}", None, cx);

    open(&wb, "src/main.rs", cx);
    assert!(!shown("trust-modal", cx), "rust-analyzer is not on PATH");
    assert!(shown("restricted-mode", cx), "the title bar still shows it");

    // The button still opens the modal.
    click("restricted-mode", cx);
    assert!(shown("trust-modal", cx));
}

#[gpui::test]
fn prompt_when_user_binary_path_is_set(cx: &mut TestAppContext) {
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    let launcher = Arc::new(FakeLauncher::new());
    let user =
        r#"{ "lsp": { "rust-analyzer": { "binary": { "path": "/opt/ra/rust-analyzer" } } } }"#;
    let (wb, cx) = trust_workbench(dir.path(), data.path(), launcher, user, None, cx);

    open(&wb, "src/main.rs", cx);
    assert!(shown("trust-modal", cx));
}

#[gpui::test]
fn workspace_binary_path_does_not_prompt(cx: &mut TestAppContext) {
    let (dir, data) = (rust_project(), TempDir::new().unwrap());
    let launcher = Arc::new(FakeLauncher::new());
    let workspace =
        r#"{ "lsp": { "rust-analyzer": { "binary": { "path": "/opt/ra/rust-analyzer" } } } }"#;
    let (wb, cx) = trust_workbench(dir.path(), data.path(), launcher, "{}", Some(workspace), cx);

    open(&wb, "src/main.rs", cx);
    assert!(!shown("trust-modal", cx), "an untrusted folder's setting");
}
