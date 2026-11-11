//! The language server Health tab (`lucerna.lsp.health`, spec §7): every
//! (language, server) pair of the table and whether it is installed, with
//! in-process fake servers.

mod support;

use std::collections::BTreeSet;
use std::fs;
use std::sync::Arc;

use gpui::{BorrowAppContext, Entity, Modifiers, TestAppContext, VisualTestContext};
use lucerna_lsp::health::{HealthRow, HealthStatus};
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_settings::Layer;
use lucerna_workbench::{AppState, LspHealthView, TabItem, Workbench};
use support::lsp::*;
use tempfile::TempDir;

fn rust_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    dir
}

fn shown(selector: &'static str, cx: &mut VisualTestContext) -> bool {
    draw(cx);
    cx.debug_bounds(selector).is_some()
}

fn click(selector: &str, cx: &mut VisualTestContext) {
    draw(cx);
    let selector: &'static str = selector.to_owned().leak();
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    settle(cx);
}

fn show_health(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(wb.run_command("lucerna.lsp.health", None, window, cx));
        })
    });
    settle(cx);
}

fn health_view(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<LspHealthView> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("the Health tab has a body")
            .downcast::<LspHealthView>()
            .expect("the Health tab's body is the health view")
    })
}

fn rows(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<HealthRow> {
    let view = health_view(wb, cx);
    cx.update(|_, cx| view.read(cx).rows().expect("checked").to_vec())
}

fn found(rows: &[HealthRow], server: &str) -> bool {
    rows.iter()
        .any(|r| &*r.server == server && matches!(r.status, HealthStatus::Found(_)))
}

fn rust_launcher() -> Arc<FakeLauncher> {
    Arc::new(FakeLauncher::new().with_server("rust-analyzer", FakeServer::new()))
}

#[gpui::test]
fn health_tab_lists_every_language(cx: &mut TestAppContext) {
    let dir = rust_project();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(), TRUSTED, cx);
    settle(cx);
    show_health(&wb, cx);

    let active = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups.model().active_tab().map(|t| t.item.clone())
    });
    assert_eq!(active, Some(TabItem::LspHealth));
    assert!(shown("lsp-health", cx));

    let rows = rows(&wb, cx);
    let servers = language_servers(&wb, cx);
    let languages: BTreeSet<String> = cx.update(|_, cx| {
        let table = servers.read(cx).table();
        table
            .servers
            .values()
            .flat_map(|def| {
                if def.languages.is_empty() {
                    def.file_types.iter().take(1).cloned().collect::<Vec<_>>()
                } else {
                    def.languages.clone()
                }
            })
            .collect()
    });
    let listed: BTreeSet<String> = rows.iter().map(|r| r.language.clone()).collect();
    assert_eq!(listed, languages);
    assert!(found(&rows, "rust-analyzer"));
    for i in 0..rows.len() {
        let selector: &'static str = format!("lsp-health-row-{i}").leak();
        assert!(shown(selector, cx), "{selector}");
    }

    // One tab: running the command again focuses it.
    show_health(&wb, cx);
    let tabs = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups.model().groups()[0].tabs.len()
    });
    assert_eq!(tabs, 1);
}

#[gpui::test]
fn summary_counts(cx: &mut TestAppContext) {
    let dir = rust_project();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(), TRUSTED, cx);
    settle(cx);
    show_health(&wb, cx);

    let rows = rows(&wb, cx);
    let found = rows
        .iter()
        .filter(|r| matches!(r.status, HealthStatus::Found(_)))
        .count();
    assert_eq!(found, 1, "only rust-analyzer is installed");
    let view = health_view(&wb, cx);
    let summary = cx.update(|_, cx| view.read(cx).summary());
    assert_eq!(
        summary.as_deref(),
        Some(format!("1 of {} servers found", rows.len()).as_str())
    );
    assert!(shown("lsp-health-summary", cx));
}

#[gpui::test]
fn copy_puts_install_command_on_clipboard(cx: &mut TestAppContext) {
    let dir = rust_project();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(), TRUSTED, cx);
    settle(cx);
    show_health(&wb, cx);

    let rows = rows(&wb, cx);
    let i = rows
        .iter()
        .position(|r| r.status == HealthStatus::NotFound && r.install.is_some())
        .expect("a server not found with an install command");
    click(&format!("lsp-health-copy-{i}"), cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied, rows[i].install.clone());
}

#[gpui::test]
fn refresh_recomputes(cx: &mut TestAppContext) {
    let dir = rust_project();
    let (wb, cx) = lsp_workbench_with(dir.path(), Arc::new(FakeLauncher::new()), TRUSTED, cx);
    settle(cx);
    show_health(&wb, cx);
    assert!(!found(&rows(&wb, cx), "rust-analyzer"));

    // Installed since: the cached rows stay until refresh.
    let servers = language_servers(&wb, cx);
    servers.update(cx, |servers, _| servers.set_launcher(rust_launcher()));
    show_health(&wb, cx);
    assert!(!found(&rows(&wb, cx), "rust-analyzer"), "cached");

    click("lsp-health-refresh", cx);
    assert!(found(&rows(&wb, cx), "rust-analyzer"));

    // A settings change checks again too.
    cx.update(|_, cx| {
        cx.update_global::<AppState, _>(|state, _| {
            state
                .settings
                .set_layer(
                    Layer::User,
                    r#"{ "session.trustAllWorktrees": true,
                         "lsp": { "rust-analyzer": { "enabled": false } } }"#,
                )
                .unwrap();
        })
    });
    settle(cx);
    let rows = rows(&wb, cx);
    let rust = rows.iter().find(|r| &*r.server == "rust-analyzer").unwrap();
    assert_eq!(rust.status, HealthStatus::Disabled);
}

#[gpui::test]
fn not_persisted_in_workspace_state(cx: &mut TestAppContext) {
    let dir = rust_project();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);
    show_health(&wb, cx);

    let (tabs, state) = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx).model();
        (groups.groups()[0].tabs.len(), groups.state())
    });
    assert_eq!(tabs, 2);
    assert_eq!(state.groups[0].tabs.len(), 1, "{state:?}");
    assert_eq!(state.groups[0].active, None);
}

#[gpui::test]
fn untrusted_workspace_server_not_listed_as_found(cx: &mut TestAppContext) {
    let dir = rust_project();
    let launcher = Arc::new(FakeLauncher::new().with_server("evil", FakeServer::new()));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher, "{}", cx);
    cx.update(|_, cx| {
        cx.update_global::<AppState, _>(|state, _| {
            state
                .settings
                .set_layer(
                    Layer::Workspace,
                    r#"{ "lsp": { "evil": { "command": "evil", "file_types": ["rb"] } } }"#,
                )
                .unwrap();
        })
    });
    settle(cx);
    let servers = language_servers(&wb, cx);
    assert!(!cx.update(|_, cx| servers.read(cx).is_trusted()));

    show_health(&wb, cx);
    let rows = rows(&wb, cx);
    assert!(!rows.is_empty());
    assert!(!found(&rows, "evil"), "{rows:#?}");
}
