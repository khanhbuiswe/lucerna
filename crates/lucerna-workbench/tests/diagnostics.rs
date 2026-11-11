//! Diagnostics in the code view (spec §7.1): pushed by the language
//! servers into every view of a file, and painted per the settings.

mod support;

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use gpui::{BorrowAppContext, Entity, TestAppContext, VisualTestContext};
use lucerna_lsp::diagnostics::Severity;
use lucerna_lsp::position::path_to_uri;
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::Layer;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, Workbench};
use serde_json::{Value, json};
use support::lsp::*;
use tempfile::TempDir;

const MAIN: &str = "fn main() {\n    let x = 1;\n}\n";

fn rust_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), MAIN).unwrap();
    dir
}

fn main_uri(dir: &TempDir) -> String {
    path_to_uri(&dir.path().join("src/main.rs"))
        .as_str()
        .to_owned()
}

fn diagnostic(line: u32, start: u32, end: u32, severity: u32, message: &str) -> Value {
    json!({
        "range": { "start": { "line": line, "character": start },
                   "end": { "line": line, "character": end } },
        "severity": severity,
        "message": message,
    })
}

/// Two diagnostics on `main.rs`: an error on `main`, a warning on `x`.
fn two(dir: &TempDir, version: i32) -> Value {
    json!({
        "uri": main_uri(dir),
        "version": version,
        "diagnostics": [
            diagnostic(0, 3, 7, 1, "boom\nsecond line"),
            diagnostic(1, 8, 9, 2, "unused variable"),
        ],
    })
}

/// Lets a publish through the 50 ms batch and draws.
fn after_batch(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    draw(cx);
}

fn diagnostics_len(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| view.read(cx).diagnostics().len())
}

fn set_user_settings(json: &str, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        cx.update_global::<AppState, _>(|state, _| {
            state.settings.set_layer(Layer::User, json).unwrap();
        });
    });
    settle(cx);
}

fn setup<'a>(
    settings: &str,
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    FakeServer,
    Entity<Workbench>,
    &'a mut VisualTestContext,
) {
    let dir = rust_project();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher, settings, cx);
    (dir, fake, wb, cx)
}

#[gpui::test]
fn diagnostics_reach_view_after_publish(cx: &mut TestAppContext) {
    let (dir, fake, wb, cx) = setup(TRUSTED, cx);
    let view = open(&wb, "src/main.rs", cx);
    assert_eq!(diagnostics_len(&view, cx), 0);

    fake.notify("textDocument/publishDiagnostics", two(&dir, 0));
    after_batch(cx);

    assert_eq!(diagnostics_len(&view, cx), 2);
    let rows = cx.update(|_, cx| {
        let rows = view.read(cx).diagnostic_rows();
        (rows.gutter(0), rows.gutter(1), rows.scrollbar().to_vec())
    });
    assert_eq!(
        rows,
        (
            Some(Severity::Error),
            Some(Severity::Warning),
            vec![(0, Severity::Error), (1, Severity::Warning)]
        )
    );
}

#[gpui::test]
fn diagnostics_reach_every_view_of_the_file(cx: &mut TestAppContext) {
    let (dir, fake, wb, cx) = setup(TRUSTED, cx);
    let first = open(&wb, "src/main.rs", cx);
    fake.notify("textDocument/publishDiagnostics", two(&dir, 0));
    after_batch(cx);

    // A second view opened after the publish starts with them.
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| {
            m.split_active();
        })
    });
    settle(cx);
    let second = active_view(&wb, cx);
    assert_ne!(first, second);
    assert_eq!(diagnostics_len(&second, cx), 2);

    // A later publish reaches both.
    let one = json!({
        "uri": main_uri(&dir),
        "version": 0,
        "diagnostics": [diagnostic(0, 3, 7, 1, "boom")],
    });
    fake.notify("textDocument/publishDiagnostics", one);
    after_batch(cx);
    assert_eq!(diagnostics_len(&first, cx), 1);
    assert_eq!(diagnostics_len(&second, cx), 1);
}

#[gpui::test]
fn gutter_column_only_when_enabled(cx: &mut TestAppContext) {
    let (dir, _fake, wb, cx) = setup(TRUSTED, cx);
    let view = open(&wb, "src/main.rs", cx);
    // A server runs for the file: the column is on, even with nothing
    // published yet.
    let with_icons = cx.update(|_, cx| view.read(cx).gutter_width());

    set_user_settings(
        r#"{ "session.trustAllWorktrees": true, "lucerna.diagnostics.gutterIcons": false }"#,
        cx,
    );
    let without = cx.update(|_, cx| view.read(cx).gutter_width());
    assert_eq!(with_icons - without, 16.);

    // A plain-text file has no server: no column.
    fs::write(dir.path().join("notes.txt"), "hello\n").unwrap();
    set_user_settings(TRUSTED, cx);
    let plain = open(&wb, "notes.txt", cx);
    assert_eq!(cx.update(|_, cx| plain.read(cx).gutter_width()), without);
}

#[gpui::test]
fn inline_off_by_default(cx: &mut TestAppContext) {
    let (dir, fake, wb, cx) = setup(TRUSTED, cx);
    let view = open(&wb, "src/main.rs", cx);
    fake.notify("textDocument/publishDiagnostics", two(&dir, 0));
    after_batch(cx);
    assert_eq!(diagnostics_len(&view, cx), 2);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).painted_inline_rows()),
        Vec::<u32>::new()
    );

    set_user_settings(
        r#"{ "session.trustAllWorktrees": true, "lucerna.diagnostics.inline.enabled": true }"#,
        cx,
    );
    assert_eq!(
        cx.update(|_, cx| view.read(cx).painted_inline_rows()),
        vec![0, 1]
    );
}

#[gpui::test]
fn stale_version_ignored(cx: &mut TestAppContext) {
    let (dir, fake, wb, cx) = setup(TRUSTED, cx);
    let view = open(&wb, "src/main.rs", cx);
    let one = json!({
        "uri": main_uri(&dir),
        "version": 0,
        "diagnostics": [diagnostic(1, 8, 9, 2, "unused variable")],
    });
    fake.notify("textDocument/publishDiagnostics", one);
    after_batch(cx);
    assert_eq!(diagnostics_len(&view, cx), 1);

    // The file shrinks on disk: version 1, and the old diagnostic's row
    // is past the new text until the server publishes again.
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    let ev = WorktreeEvent::Changed(vec![Change {
        path: p("src/main.rs"),
        kind: ChangeKind::Modified,
    }]);
    wb.update(cx, |wb, cx| wb.on_worktree_event(&ev, cx));
    settle(cx);
    assert_eq!(sent(&fake, "textDocument/didChange").len(), 1);
    let marks = cx.update(|_, cx| {
        let view = view.read(cx);
        let line = view.snapshot().line(0).len() as u32;
        view.diagnostic_rows()
            .marks_in(0..u32::MAX)
            .all(|m| m.range.end.row == 0 && m.range.end.column <= line)
    });
    assert!(marks, "marks are clamped to the new text");

    // A publish for version 0 is stale now.
    fake.notify("textDocument/publishDiagnostics", two(&dir, 0));
    after_batch(cx);
    assert_eq!(diagnostics_len(&view, cx), 1);

    fake.notify("textDocument/publishDiagnostics", two(&dir, 1));
    after_batch(cx);
    assert_eq!(diagnostics_len(&view, cx), 2);
}
