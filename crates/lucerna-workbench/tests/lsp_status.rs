//! The language-server status button, its popover, the progress text and
//! the per-server log tab (spec §7.7), with in-process fake servers.

mod support;

use std::fs;
use std::sync::Arc;

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_workbench::{LspLogView, LspStatusItem, SlotState, StatusDot, TabItem, Workbench};
use serde_json::{Value, json};
use support::lsp::*;
use tempfile::TempDir;

/// A folder with `Cargo.toml`, `src/main.rs` and `notes.txt`.
fn rust_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(dir.path().join("notes.txt"), "plain text\n").unwrap();
    dir
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

fn run(wb: &Entity<Workbench>, command: &str, args: Option<Value>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(
                wb.run_command(command, args.as_ref(), window, cx),
                "{command}"
            );
        })
    });
    settle(cx);
}

fn status_item(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<LspStatusItem> {
    cx.update(|_, cx| wb.read(cx).lsp_status().clone())
}

fn dot(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<StatusDot> {
    let item = status_item(wb, cx);
    cx.update(|_, cx| item.read(cx).dot(cx))
}

fn progress(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<String> {
    let servers = language_servers(wb, cx);
    cx.update(|_, cx| servers.read(cx).progress_text())
}

fn progress_params(token: &str, value: Value) -> Value {
    json!({ "token": token, "value": value })
}

#[gpui::test]
fn button_hidden_without_servers(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(&fake), TRUSTED, cx);
    settle(cx);

    assert!(!shown("lsp-button", cx), "no file open");
    open(&wb, "notes.txt", cx);
    assert!(!shown("lsp-button", cx), "plain text has no server");

    open(&wb, "src/main.rs", cx);
    assert!(shown("lsp-button", cx));
    assert_eq!(dot(&wb, cx), None, "a running server has no dot");
}

#[gpui::test]
fn not_found_shows_warning_dot_and_message(cx: &mut TestAppContext) {
    let dir = rust_project();
    let (wb, cx) = lsp_workbench_with(dir.path(), Arc::new(FakeLauncher::new()), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);

    assert!(shown("lsp-button", cx));
    assert_eq!(dot(&wb, cx), Some(StatusDot::Warning));
    assert!(!shown("lsp-popover", cx));
    click("lsp-button", cx);
    assert!(shown("lsp-popover", cx));

    let item = status_item(&wb, cx);
    let rows = cx.update(|_, cx| item.read(cx).rows(cx));
    assert_eq!(rows.len(), 1);
    assert_eq!(&*rows[0].key.server, "rust-analyzer");
    assert!(matches!(rows[0].state, SlotState::NotFound { .. }));
    assert_eq!(
        rows[0].lines,
        vec![
            "rust-analyzer not found on PATH. Install it or set \"lsp\": { \"rust-analyzer\": { \"binary\": { \"path\": … } } }"
                .to_owned()
        ]
    );
    assert!(shown("lsp-row-message-0-0", cx));

    // Escape closes it.
    cx.simulate_keystrokes("escape");
    settle(cx);
    assert!(!shown("lsp-popover", cx));
}

#[gpui::test]
fn failed_shows_stderr_tail(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(&fake), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);

    fake.stderr("thread 'main' panicked");
    fake.stderr("note: run with RUST_BACKTRACE=1");
    settle(cx);
    fake.crash(Some(101));
    settle(cx);

    assert_eq!(dot(&wb, cx), Some(StatusDot::Error));
    let item = status_item(&wb, cx);
    let rows = cx.update(|_, cx| item.read(cx).rows(cx));
    assert_eq!(
        rows[0].lines,
        vec![
            "exited with code 101".to_owned(),
            "thread 'main' panicked".to_owned(),
            "note: run with RUST_BACKTRACE=1".to_owned(),
        ]
    );
    click("lsp-button", cx);
    assert!(shown("lsp-row-message-0-2", cx));
}

#[gpui::test]
fn restart_relaunches(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = rust_launcher(&fake);
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher.clone(), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);
    assert_eq!(launcher.launches().len(), 1);

    click("lsp-button", cx);
    click("lsp-restart-0", cx);
    assert_eq!(launcher.launches().len(), 2);
    assert_eq!(
        state_of_first(&wb, cx),
        SlotState::Running,
        "running again after the restart"
    );

    // From the popover's footer and the commands.
    click("lsp-button", cx);
    click("lsp-stop-all", cx);
    assert_eq!(state_of_first(&wb, cx), SlotState::Stopped);
    run(&wb, "lucerna.lsp.restart", None, cx);
    assert_eq!(launcher.launches().len(), 3);
    run(&wb, "lucerna.lsp.stopAll", None, cx);
    assert_eq!(state_of_first(&wb, cx), SlotState::Stopped);
    run(&wb, "lucerna.lsp.restartAll", None, cx);
    assert_eq!(launcher.launches().len(), 4);
    assert_eq!(state_of_first(&wb, cx), SlotState::Running);
}

fn state_of_first(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> SlotState {
    let servers = language_servers(wb, cx);
    cx.update(|_, cx| servers.read(cx).statuses()[0].state.clone())
}

#[gpui::test]
fn starting_progress_text(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new().hang_on("initialize");
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(&fake), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);
    assert_eq!(
        progress(&wb, cx).as_deref(),
        Some("Starting rust-analyzer…")
    );
    assert!(shown("lsp-progress", cx));
}

#[gpui::test]
fn progress_text_format(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(&fake), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);
    assert_eq!(progress(&wb, cx), None);
    assert!(!shown("lsp-progress", cx));

    fake.notify(
        "$/progress",
        progress_params(
            "a",
            json!({ "kind": "begin", "title": "Indexing", "percentage": 45, "message": "crates/foo" }),
        ),
    );
    settle(cx);
    assert_eq!(
        progress(&wb, cx).as_deref(),
        Some("rust-analyzer: Indexing 45% — crates/foo")
    );
    assert!(shown("lsp-progress", cx));

    // The newest wins; the others are counted.
    fake.notify(
        "$/progress",
        progress_params("b", json!({ "kind": "begin", "title": "Fetching" })),
    );
    settle(cx);
    assert_eq!(
        progress(&wb, cx).as_deref(),
        Some("rust-analyzer: Fetching +1")
    );

    // Cut at 60 characters, before the count.
    let long = "x".repeat(80);
    fake.notify(
        "$/progress",
        progress_params("b", json!({ "kind": "report", "message": long })),
    );
    settle(cx);
    let text = progress(&wb, cx).unwrap();
    let (main, count) = text.rsplit_once(' ').unwrap();
    assert_eq!(count, "+1");
    assert_eq!(main.chars().count(), 60, "{text}");
    assert!(main.starts_with("rust-analyzer: Fetching — xxx"), "{text}");
    assert!(main.ends_with('…'), "{text}");

    for token in ["a", "b"] {
        fake.notify(
            "$/progress",
            progress_params(token, json!({ "kind": "end" })),
        );
    }
    settle(cx);
    assert_eq!(progress(&wb, cx), None);
    assert!(!shown("lsp-progress", cx));
}

#[gpui::test]
fn show_log_opens_tab_with_stderr_lines(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(&fake), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);
    fake.stderr("first stderr line");
    fake.stderr("second stderr line");
    settle(cx);

    run(&wb, "lucerna.lsp.showLog", None, cx);
    let active = cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .active_tab()
            .map(|t| t.item.clone())
    });
    assert_eq!(active, Some(TabItem::LspLog(Arc::from("rust-analyzer"))));
    assert!(shown("lsp-log", cx));
    let log = log_view(&wb, cx);
    let lines = cx.update(|_, cx| log.read(cx).lines(cx));
    let first = lines.iter().position(|l| l == "first stderr line");
    let second = lines.iter().position(|l| l == "second stderr line");
    assert!(first.is_some() && first < second, "{lines:?}");

    // New lines show up; ⌘A ⌘C copies every line.
    fake.stderr("third stderr line");
    settle(cx);
    let lines = cx.update(|_, cx| log.read(cx).lines(cx));
    assert_eq!(lines.last().map(String::as_str), Some("third stderr line"));
    cx.simulate_keystrokes("cmd-a cmd-c");
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied, Some(lines.join("\n")));

    // One tab per server, and never saved.
    run(&wb, "lucerna.lsp.showLog", None, cx);
    let (tabs, state) = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx).model();
        (groups.groups()[0].tabs.len(), groups.state())
    });
    assert_eq!(tabs, 2);
    assert_eq!(state.groups[0].tabs.len(), 1, "{state:?}");
    assert_eq!(state.groups[0].active, None);
}

fn log_view(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<LspLogView> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("the log tab has a body")
            .downcast::<LspLogView>()
            .expect("the log tab's body is the log view")
    })
}

#[gpui::test]
fn log_button_in_popover_opens_the_log(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(&fake), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);

    click("lsp-button", cx);
    click("lsp-log-0", cx);
    assert!(!shown("lsp-popover", cx));
    assert!(shown("lsp-log", cx));
}

#[gpui::test]
fn show_message_error_shown_until_restart(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(&fake), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);

    fake.notify(
        "window/showMessage",
        json!({ "type": 3, "message": "just info" }),
    );
    fake.notify(
        "window/showMessage",
        json!({ "type": 1, "message": "failed to load workspace" }),
    );
    settle(cx);
    let item = status_item(&wb, cx);
    let rows = cx.update(|_, cx| item.read(cx).rows(cx));
    assert_eq!(rows[0].lines, vec!["failed to load workspace".to_owned()]);

    run(&wb, "lucerna.lsp.restart", None, cx);
    let rows = cx.update(|_, cx| item.read(cx).rows(cx));
    assert!(rows[0].lines.is_empty(), "{:?}", rows[0].lines);
}

#[gpui::test]
fn restricted_shows_warning_and_trust_row(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(&fake), "{}", cx);
    settle(cx);

    assert!(shown("lsp-button", cx), "Restricted Mode shows it");
    assert_eq!(dot(&wb, cx), Some(StatusDot::Warning));
    click("lsp-button", cx);
    click("lsp-restricted", cx);
    assert!(!shown("lsp-popover", cx));
    assert!(shown("trust-modal", cx));
}

#[gpui::test]
fn button_toggles_popover_and_outside_click_closes(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(&fake), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);

    click("lsp-button", cx);
    assert!(shown("lsp-popover", cx));
    click("lsp-button", cx);
    assert!(!shown("lsp-popover", cx), "a second press closes it");
    click("lsp-button", cx);
    assert!(shown("lsp-popover", cx), "and the next opens it again");

    click("editor-groups", cx);
    assert!(!shown("lsp-popover", cx), "a press outside closes it");
    click("lsp-button", cx);
    assert!(shown("lsp-popover", cx));
}

#[gpui::test]
fn long_stderr_keeps_footer_in_window(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(&fake), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);
    for i in 0..50 {
        fake.stderr(&format!("stderr line {i}: {}", "word ".repeat(40)));
    }
    settle(cx);
    fake.crash(Some(101));
    settle(cx);

    click("lsp-button", cx);
    let (window, footer, _) = popover_bounds(cx);
    assert!(
        footer.bottom() <= window.height && footer.top() >= gpui::px(0.),
        "footer {footer:?} in a {window:?} window"
    );
    // In a short window the rows scroll and the footer stays visible.
    cx.simulate_resize(gpui::size(gpui::px(800.), gpui::px(220.)));
    let (window, footer, popover) = popover_bounds(cx);
    assert!(
        footer.bottom() <= window.height && footer.top() >= gpui::px(0.),
        "footer {footer:?} in a {window:?} window"
    );
    assert!(popover.top() >= gpui::px(0.), "popover {popover:?}");
    cx.simulate_resize(gpui::size(gpui::px(1920.), gpui::px(1080.)));

    // The message and the last five stderr lines; the rest is in the log.
    assert!(shown("lsp-row-message-0-5", cx));
    assert!(!shown("lsp-row-message-0-6", cx));
    click("lsp-row-more-0", cx);
    assert!(shown("lsp-log", cx));
}

/// The window size and the bounds of the popover's footer and of the
/// popover.
fn popover_bounds(
    cx: &mut VisualTestContext,
) -> (
    gpui::Size<gpui::Pixels>,
    gpui::Bounds<gpui::Pixels>,
    gpui::Bounds<gpui::Pixels>,
) {
    draw(cx);
    let window = cx.update(|window, _| window.viewport_size());
    let footer = cx
        .debug_bounds("lsp-restart-all")
        .expect("the footer is drawn");
    let popover = cx
        .debug_bounds("lsp-popover")
        .expect("the popover is drawn");
    (window, footer, popover)
}

#[gpui::test]
fn not_found_row_shows_install_and_copy(cx: &mut TestAppContext) {
    let dir = rust_project();
    let (wb, cx) = lsp_workbench_with(dir.path(), Arc::new(FakeLauncher::new()), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);

    let item = status_item(&wb, cx);
    let rows = cx.update(|_, cx| item.read(cx).rows(cx));
    assert_eq!(
        rows[0].install.as_deref(),
        Some("rustup component add rust-analyzer")
    );
    click("lsp-button", cx);
    assert!(shown("lsp-row-install-0", cx));
    click("lsp-copy-0", cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(
        copied.as_deref(),
        Some("rustup component add rust-analyzer")
    );
}

#[gpui::test]
fn running_row_has_no_install_line(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(&fake), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);

    let item = status_item(&wb, cx);
    let rows = cx.update(|_, cx| item.read(cx).rows(cx));
    assert_eq!(rows[0].install, None);
    click("lsp-button", cx);
    assert!(!shown("lsp-row-install-0", cx));
    assert!(!shown("lsp-copy-0", cx));
}

/// Trusted, with a malformed `lsp.evil` entry (Review Focus 2).
const BAD_ENTRY: &str = r#"{
    "session.trustAllWorktrees": true,
    "lsp": { "evil": { "command": "evil", "args": "--stdio", "languages": ["rust"] } }
}"#;

#[gpui::test]
fn table_error_row_shown(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(&fake), BAD_ENTRY, cx);
    settle(cx);

    // Shown before any server runs, with the warning dot.
    assert!(shown("lsp-button", cx));
    assert_eq!(dot(&wb, cx), Some(StatusDot::Warning));

    open(&wb, "src/main.rs", cx);
    let servers = language_servers(&wb, cx);
    let reason = cx.update(|_, cx| servers.read(cx).table_errors()[0].message.clone());
    let item = status_item(&wb, cx);
    let errors = cx.update(|_, cx| item.read(cx).table_error_lines(cx));
    assert_eq!(errors, vec![format!("Invalid `lsp.evil`: {reason}")]);

    // The other servers still run.
    assert_eq!(state_of_first(&wb, cx), SlotState::Running);
    click("lsp-button", cx);
    assert!(shown("lsp-table-error-0", cx));
    assert!(shown("lsp-row-0", cx));
}

#[gpui::test]
fn footer_opens_health(cx: &mut TestAppContext) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let (wb, cx) = lsp_workbench_with(dir.path(), rust_launcher(&fake), TRUSTED, cx);
    open(&wb, "src/main.rs", cx);

    let registered = cx.update(|_, cx| {
        wb.read(cx)
            .commands()
            .all()
            .any(|(id, _)| id == "lucerna.lsp.health")
    });
    assert!(registered);

    click("lsp-button", cx);
    click("lsp-show-all-languages", cx);
    assert!(!shown("lsp-popover", cx));
    let active = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups.model().active_tab().map(|t| t.item.clone())
    });
    assert_eq!(active, Some(TabItem::LspHealth));
    assert!(shown("lsp-health", cx));
}
