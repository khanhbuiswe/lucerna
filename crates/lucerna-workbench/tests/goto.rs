//! Go to definition, type definition, implementation, declaration and
//! references (spec §7.3, §7.5): the decision table, the "No … found"
//! message and opening aside.

mod support;

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_lsp::position::path_to_uri;
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_text::Point;
use lucerna_workbench::Workbench;
use lucerna_workbench::code_view::CodeView;
use serde_json::{Value, json};
use support::lsp::{TRUSTED, active_view, count, lsp_workbench_with, open, sent, settle};
use tempfile::TempDir;

const MAIN: &str = "fn main() {\n    b::helper();\n}\n";
const B: &str = "// b\npub fn helper() {}\npub fn other() {}\n";
const DEFINITION: &str = "textDocument/definition";
const REFERENCES: &str = "textDocument/references";

fn rust_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), MAIN).unwrap();
    fs::write(dir.path().join("src/b.rs"), B).unwrap();
    dir
}

fn uri(dir: &Path, name: &str) -> String {
    path_to_uri(&dir.join("src").join(name)).as_str().to_owned()
}

fn range(line: u32, start: u32, end: u32) -> Value {
    json!({ "start": { "line": line, "character": start },
            "end": { "line": line, "character": end } })
}

fn location(dir: &Path, name: &str, line: u32, start: u32, end: u32) -> Value {
    json!({ "uri": uri(dir, name), "range": range(line, start, end) })
}

fn pt(row: u32, column: u32) -> Point {
    Point { row, column }
}

/// The project, a workbench on it with main.rs open and the cursor at
/// `cursor`. `answer` builds the fake server from the project's folder.
fn setup<'a>(
    settings: &str,
    cursor: Point,
    answer: impl FnOnce(&Path) -> FakeServer,
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    FakeServer,
    Entity<Workbench>,
    Entity<CodeView>,
    &'a mut VisualTestContext,
) {
    let dir = rust_project();
    let fake = answer(dir.path());
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher, settings, cx);
    let view = open(&wb, "src/main.rs", cx);
    view.update(cx, |v, cx| v.set_cursor(cursor, cx));
    settle(cx);
    (dir, fake, wb, view, cx)
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(wb.run_command(command, None, window, cx), "{command}");
        })
    });
    settle(cx);
}

/// The file name of the active tab.
fn active_file(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        let tab = groups.model().active_tab().expect("an active tab");
        let path = tab.path().expect("a file tab");
        path.file_name().unwrap().to_string_lossy().into_owned()
    })
}

fn cursor(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Point {
    let view = active_view(wb, cx);
    view.read_with(cx, |v, _| v.cursor())
}

fn message(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| v.goto_message().map(str::to_owned))
}

#[gpui::test]
fn f12_single_result_jumps(cx: &mut TestAppContext) {
    let (dir, fake, wb, _view, cx) = setup(
        TRUSTED,
        pt(1, 8),
        |dir| {
            let target = location(dir, "b.rs", 1, 7, 13);
            FakeServer::new().on_request(DEFINITION, move |_| Ok(target.clone()))
        },
        cx,
    );
    run(&wb, "editor.action.revealDefinition", cx);
    let requests = sent(&fake, DEFINITION);
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]["textDocument"]["uri"],
        uri(dir.path(), "main.rs")
    );
    assert_eq!(
        requests[0]["position"],
        json!({ "line": 1, "character": 8 })
    );
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));

    // The jump is in the navigation history.
    run(&wb, "workbench.action.navigateBack", cx);
    assert_eq!(active_file(&wb, cx), "main.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 8));
}

#[gpui::test]
fn location_link_target_selection_used(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(
        TRUSTED,
        pt(1, 8),
        |dir| {
            let link = json!([{
                "originSelectionRange": range(1, 7, 13),
                "targetUri": uri(dir, "b.rs"),
                "targetRange": { "start": { "line": 1, "character": 0 },
                                 "end": { "line": 1, "character": 18 } },
                "targetSelectionRange": range(1, 7, 13),
            }]);
            FakeServer::new().on_request(DEFINITION, move |_| Ok(link.clone()))
        },
        cx,
    );
    run(&wb, "editor.action.revealDefinition", cx);
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));
}

#[gpui::test]
fn same_location_falls_back_to_references(cx: &mut TestAppContext) {
    let (_dir, fake, wb, _view, cx) = setup(
        TRUSTED,
        pt(0, 4),
        |dir| {
            // The definition is the word under the cursor itself.
            let here = location(dir, "main.rs", 0, 3, 7);
            let refs = json!([location(dir, "b.rs", 2, 7, 12)]);
            FakeServer::new()
                .on_request(DEFINITION, move |_| Ok(here.clone()))
                .on_request(REFERENCES, move |_| Ok(refs.clone()))
        },
        cx,
    );
    run(&wb, "editor.action.revealDefinition", cx);
    let requests = sent(&fake, REFERENCES);
    assert_eq!(requests.len(), 1, "references asked instead");
    assert_eq!(requests[0]["context"]["includeDeclaration"], true);
    assert_eq!(
        requests[0]["position"],
        json!({ "line": 0, "character": 4 })
    );
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(2, 7));

    // A peek shows the definition itself: no references.
    run(&wb, "workbench.action.navigateBack", cx);
    assert_eq!(active_file(&wb, cx), "main.rs");
    run(&wb, "editor.action.peekDefinition", cx);
    assert_eq!(count(&fake, REFERENCES), 1);
    assert_eq!(active_file(&wb, cx), "main.rs");
    assert_eq!(cursor(&wb, cx), pt(0, 3));
}

#[gpui::test]
fn no_result_popover_text(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, view, cx) = setup(
        TRUSTED,
        pt(0, 4),
        |_| {
            FakeServer::new()
                .on_request(DEFINITION, |_| Ok(Value::Null))
                .on_request(REFERENCES, |_| Ok(json!([])))
        },
        cx,
    );
    run(&wb, "editor.action.revealDefinition", cx);
    assert_eq!(
        message(&view, cx).as_deref(),
        Some("No definition found for 'main'")
    );
    assert!(cx.debug_bounds("goto-message").is_some(), "message drawn");
    assert_eq!(active_file(&wb, cx), "main.rs");

    // Gone after 3 s (each settle lets 200 ms pass: 2.9 s here).
    cx.executor().advance_clock(Duration::from_millis(2500));
    settle(cx);
    assert!(message(&view, cx).is_some());
    cx.executor().advance_clock(Duration::from_millis(200));
    settle(cx);
    assert_eq!(message(&view, cx), None);
    assert!(cx.debug_bounds("goto-message").is_none());

    // References say so in their own words; moving the cursor hides it.
    run(&wb, "editor.action.goToReferences", cx);
    assert_eq!(
        message(&view, cx).as_deref(),
        Some("No references found for 'main'")
    );
    view.update(cx, |v, cx| v.set_cursor(pt(1, 4), cx));
    settle(cx);
    assert_eq!(message(&view, cx), None);
}

#[gpui::test]
fn no_result_wording_per_kind(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, view, cx) = setup(
        TRUSTED,
        pt(1, 8),
        |_| {
            let caps = lsp_types::ServerCapabilities {
                type_definition_provider: Some(
                    lsp_types::TypeDefinitionProviderCapability::Simple(true),
                ),
                implementation_provider: Some(lsp_types::ImplementationProviderCapability::Simple(
                    true,
                )),
                declaration_provider: Some(lsp_types::DeclarationCapability::Simple(true)),
                text_document_sync: Some(lsp_types::TextDocumentSyncCapability::Kind(
                    lsp_types::TextDocumentSyncKind::FULL,
                )),
                ..Default::default()
            };
            FakeServer::new()
                .capabilities(caps)
                .on_request("textDocument/typeDefinition", |_| Ok(Value::Null))
                .on_request("textDocument/implementation", |_| Ok(json!([])))
                .on_request("textDocument/declaration", |_| Ok(Value::Null))
        },
        cx,
    );
    for (command, text) in [
        (
            "editor.action.goToTypeDefinition",
            "No type definition found for 'helper'",
        ),
        (
            "editor.action.goToImplementation",
            "No implementation found for 'helper'",
        ),
        (
            "editor.action.revealDeclaration",
            "No declaration found for 'helper'",
        ),
    ] {
        run(&wb, command, cx);
        assert_eq!(message(&view, cx).as_deref(), Some(text), "{command}");
    }
}

#[gpui::test]
fn aside_opens_in_second_group(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(
        TRUSTED,
        pt(1, 8),
        |dir| {
            let target = location(dir, "b.rs", 1, 7, 13);
            FakeServer::new().on_request(DEFINITION, move |_| Ok(target.clone()))
        },
        cx,
    );
    run(&wb, "editor.action.revealDefinitionAside", cx);
    let (groups, active, first) = cx.update(|_, cx| {
        let model = wb.read(cx).editor_groups().read(cx).model();
        let first = model.groups()[0]
            .tabs
            .iter()
            .find(|t| Some(t.id) == model.groups()[0].active)
            .and_then(|t| t.path().cloned());
        (model.groups().len(), model.active_group(), first)
    });
    assert_eq!(groups, 2, "split first");
    assert_eq!(active, 1);
    assert!(first.unwrap().ends_with("main.rs"), "the first group stays");
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));

    // With a group to the right already, it is reused.
    run(&wb, "workbench.action.focusFirstEditorGroup", cx);
    run(&wb, "editor.action.revealDefinitionAside", cx);
    let (groups, active) = cx.update(|_, cx| {
        let model = wb.read(cx).editor_groups().read(cx).model();
        (model.groups().len(), model.active_group())
    });
    assert_eq!((groups, active), (2, 1));
    assert_eq!(active_file(&wb, cx), "b.rs");
}

#[gpui::test]
fn goto_setting_goes_to_first(cx: &mut TestAppContext) {
    let settings = r#"{ "session.trustAllWorktrees": true,
                        "editor.gotoLocation.multipleDefinitions": "goto" }"#;
    let (_dir, _fake, wb, _view, cx) = setup(
        settings,
        pt(1, 8),
        |dir| {
            let targets = json!([
                location(dir, "b.rs", 2, 7, 12),
                location(dir, "b.rs", 1, 7, 13),
            ]);
            FakeServer::new().on_request(DEFINITION, move |_| Ok(targets.clone()))
        },
        cx,
    );
    run(&wb, "editor.action.revealDefinition", cx);
    // Sorted by place: the first is helper on row 1.
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));
}

#[gpui::test]
fn duplicate_results_collapse_into_one(cx: &mut TestAppContext) {
    let (_dir, fake, wb, _view, cx) = setup(
        TRUSTED,
        pt(1, 8),
        |dir| {
            // Duplicates collapse into one result, so this is a jump.
            let target = location(dir, "b.rs", 1, 7, 13);
            let twice = json!([target.clone(), target]);
            FakeServer::new().on_request(DEFINITION, move |_| Ok(twice.clone()))
        },
        cx,
    );
    run(&wb, "editor.action.revealDefinition", cx);
    assert_eq!(count(&fake, DEFINITION), 1);
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));
}
