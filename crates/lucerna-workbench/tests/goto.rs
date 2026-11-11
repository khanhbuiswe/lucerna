//! Go to definition, type definition, implementation, declaration and
//! references (spec §7.3, §7.5): the decision table, the "No … found"
//! message, opening aside, and the ⌘-hover link and ⌘-click.

mod support;

use std::fs;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, Focusable, Pixels, TestAppContext, VisualTestContext};
use lucerna_lsp::position::path_to_uri;
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_text::Point;
use lucerna_workbench::Workbench;
use lucerna_workbench::code_view::CodeView;
use serde_json::{Value, json};
use support::lsp::{
    TRUSTED, active_view, count, editor_text_focus, focus_view, lsp_workbench_with, open, sent,
    settle,
};
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
    assert_eq!(rows(&wb, cx), ["header:main.rs", "1:fn main() {"]);
    keys("enter", cx);
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

    // Gone after 3 s. Each settle lets 200 ms pass: shown during `run`,
    // so 200 ms have passed; 200 + 2500 + 200 (this settle) = 2900 ms.
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

    // With a group to the right already, it is reused, and the jump is
    // in that group's history: from main.rs, which it showed.
    let right = open(&wb, "src/main.rs", cx);
    right.update(cx, |v, cx| v.set_cursor(pt(2, 0), cx));
    settle(cx);
    run(&wb, "workbench.action.focusFirstEditorGroup", cx);
    run(&wb, "editor.action.revealDefinitionAside", cx);
    let (groups, active) = cx.update(|_, cx| {
        let model = wb.read(cx).editor_groups().read(cx).model();
        (model.groups().len(), model.active_group())
    });
    assert_eq!((groups, active), (2, 1));
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));
    run(&wb, "workbench.action.navigateBack", cx);
    assert_eq!(active_file(&wb, cx), "main.rs");
    assert_eq!(cursor(&wb, cx), pt(2, 0));
    let active = cx.update(|_, cx| {
        let model = wb.read(cx).editor_groups().read(cx);
        model.model().active_group()
    });
    assert_eq!(active, 1);
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

/// A server that answers `textDocument/definition` only when the test says.
fn slow(_: &Path) -> FakeServer {
    FakeServer::new().hang_on(DEFINITION)
}

/// Answers the oldest waiting definition request with b.rs `helper`.
fn answer_definition(fake: &FakeServer, dir: &Path, cx: &mut VisualTestContext) {
    let id = fake.hung_ids(DEFINITION).remove(0);
    assert!(fake.answer_hung(&id, location(dir, "b.rs", 1, 7, 13)));
    settle(cx);
}

#[gpui::test]
fn late_answer_after_cursor_moved_is_dropped(cx: &mut TestAppContext) {
    let (dir, fake, wb, view, cx) = setup(TRUSTED, pt(1, 8), slow, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    view.update(cx, |v, cx| v.set_cursor(pt(0, 4), cx));
    settle(cx);
    answer_definition(&fake, dir.path(), cx);
    assert_eq!(active_file(&wb, cx), "main.rs");
    assert_eq!(cursor(&wb, cx), pt(0, 4));
    assert_eq!(message(&view, cx), None);
}

#[gpui::test]
fn late_answer_after_file_switched_is_dropped(cx: &mut TestAppContext) {
    let (dir, fake, wb, _view, cx) = setup(TRUSTED, pt(1, 8), slow, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    let b = open(&wb, "src/b.rs", cx);
    b.update(cx, |v, cx| v.set_cursor(pt(2, 3), cx));
    settle(cx);
    answer_definition(&fake, dir.path(), cx);
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(2, 3), "not moved to helper");
}

#[gpui::test]
fn second_press_cancels_the_first(cx: &mut TestAppContext) {
    let (dir, fake, wb, view, cx) = setup(TRUSTED, pt(1, 8), slow, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    let first = fake.hung_ids(DEFINITION).remove(0);
    assert_eq!(count(&fake, "$/cancelRequest"), 0);
    view.update(cx, |v, cx| v.set_cursor(pt(0, 4), cx));
    settle(cx);
    run(&wb, "editor.action.revealDefinition", cx);
    let cancels = sent(&fake, "$/cancelRequest");
    assert_eq!(cancels.len(), 1);
    assert_eq!(cancels[0]["id"], first);

    // The first answer, if it still came, goes nowhere; the second applies.
    fake.answer_hung(&first, location(dir.path(), "b.rs", 2, 7, 12));
    settle(cx);
    assert_eq!(active_file(&wb, cx), "main.rs");
    let ids = fake.hung_ids(DEFINITION);
    assert_eq!(ids.len(), 1);
    assert!(fake.answer_hung(&ids[0], location(dir.path(), "b.rs", 1, 7, 13)));
    settle(cx);
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));
}

// ⌘-hover and ⌘-click (spec §7.3, `code_view/link.rs`).

const HOVER: &str = "textDocument/hover";

/// The platform's secondary modifier (⌘ on macOS, Ctrl elsewhere), plus
/// ⌥ when `alt`.
fn secondary(alt: bool, cx: &mut VisualTestContext) -> gpui::Modifiers {
    let mut modifiers = cx.update(|_, cx| gpui::Modifiers::secondary_key(cx));
    modifiers.alt = alt;
    modifiers
}

fn char_at(view: &Entity<CodeView>, p: Point, cx: &mut VisualTestContext) -> gpui::Point<Pixels> {
    support::lsp::draw(cx);
    view.read_with(cx, |v, _| v.char_position(p))
        .expect("the char is on screen")
}

/// Moves the mouse over the char at `p` with `modifiers` held and settles.
fn move_over(
    view: &Entity<CodeView>,
    p: Point,
    modifiers: gpui::Modifiers,
    cx: &mut VisualTestContext,
) {
    let at = char_at(view, p, cx);
    cx.simulate_mouse_move(at, None, modifiers);
    settle(cx);
}

/// Clicks the char at `p` with `modifiers` held and settles.
fn click(
    view: &Entity<CodeView>,
    p: Point,
    modifiers: gpui::Modifiers,
    cx: &mut VisualTestContext,
) {
    let at = char_at(view, p, cx);
    cx.simulate_click(at, modifiers);
    settle(cx);
}

fn link_range(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Option<Range<Point>> {
    view.read_with(cx, |v, _| v.link_range())
}

/// `b::helper` on row 1 links to b.rs `helper`; the server says the link
/// is the whole path `b::helper`. Elsewhere there is no definition.
fn linking(dir: &Path) -> FakeServer {
    let link = json!([{
        "originSelectionRange": range(1, 4, 13),
        "targetUri": uri(dir, "b.rs"),
        "targetRange": range(1, 0, 18),
        "targetSelectionRange": range(1, 7, 13),
    }]);
    FakeServer::new()
        .on_request(DEFINITION, move |params| {
            let row = params["position"]["line"].as_u64();
            Ok(if row == Some(1) {
                link.clone()
            } else {
                Value::Null
            })
        })
        .on_request(HOVER, |_| Ok(json!({ "contents": "hover text" })))
}

#[gpui::test]
fn cmd_hover_underlines_origin_range(cx: &mut TestAppContext) {
    let (_dir, fake, _wb, view, cx) = setup(TRUSTED, pt(0, 0), linking, cx);
    let cmd = secondary(false, cx);
    move_over(&view, pt(1, 8), cmd, cx);
    assert_eq!(count(&fake, DEFINITION), 1);
    assert_eq!(
        sent(&fake, DEFINITION)[0]["position"],
        json!({ "line": 1, "character": 8 })
    );
    assert_eq!(link_range(&view, cx), Some(pt(1, 4)..pt(1, 13)));
    // While ⌘ is held the hover does not show (as in Zed).
    cx.executor().advance_clock(Duration::from_millis(400));
    settle(cx);
    assert_eq!(count(&fake, HOVER), 0);
    assert!(!view.read_with(cx, |v, _| v.hover().is_visible()));
    // The cursor does not move.
    assert_eq!(view.read_with(cx, |v, _| v.cursor()), pt(0, 0));

    // Moving off the range, or on without ⌘, clears it.
    move_over(&view, pt(0, 1), cmd, cx);
    assert_eq!(link_range(&view, cx), None);
    move_over(&view, pt(1, 8), cmd, cx);
    assert_eq!(link_range(&view, cx), Some(pt(1, 4)..pt(1, 13)));
    move_over(&view, pt(1, 8), gpui::Modifiers::none(), cx);
    assert_eq!(link_range(&view, cx), None);
}

#[gpui::test]
fn plain_location_links_the_word(cx: &mut TestAppContext) {
    let (_dir, _fake, _wb, view, cx) = setup(
        TRUSTED,
        pt(0, 0),
        |dir| {
            let target = location(dir, "b.rs", 1, 7, 13);
            FakeServer::new().on_request(DEFINITION, move |_| Ok(target.clone()))
        },
        cx,
    );
    let cmd = secondary(false, cx);
    move_over(&view, pt(1, 8), cmd, cx);
    assert_eq!(link_range(&view, cx), Some(pt(1, 7)..pt(1, 13)));
}

#[gpui::test]
fn release_cmd_clears_link(cx: &mut TestAppContext) {
    let (_dir, fake, _wb, view, cx) = setup(TRUSTED, pt(0, 0), linking, cx);
    // Modifier changes reach the focused editor.
    click(&view, pt(0, 0), gpui::Modifiers::none(), cx);
    let cmd = secondary(false, cx);
    move_over(&view, pt(1, 8), cmd, cx);
    assert!(link_range(&view, cx).is_some());
    cx.simulate_modifiers_change(gpui::Modifiers::none());
    settle(cx);
    assert_eq!(link_range(&view, cx), None);
    // Pressing it again over the same place shows it again, from the
    // cache.
    cx.simulate_modifiers_change(cmd);
    settle(cx);
    assert_eq!(link_range(&view, cx), Some(pt(1, 4)..pt(1, 13)));
    assert_eq!(count(&fake, DEFINITION), 1);
}

#[gpui::test]
fn cached_by_range_no_second_request(cx: &mut TestAppContext) {
    let (_dir, fake, _wb, view, cx) = setup(TRUSTED, pt(0, 0), linking, cx);
    let cmd = secondary(false, cx);
    move_over(&view, pt(1, 8), cmd, cx);
    move_over(&view, pt(1, 5), cmd, cx);
    move_over(&view, pt(1, 12), cmd, cx);
    assert_eq!(count(&fake, DEFINITION), 1);
    assert_eq!(link_range(&view, cx), Some(pt(1, 4)..pt(1, 13)));
}

#[gpui::test]
fn moving_off_cancels_the_link_request(cx: &mut TestAppContext) {
    let (dir, fake, _wb, view, cx) = setup(TRUSTED, pt(0, 0), slow, cx);
    let cmd = secondary(false, cx);
    move_over(&view, pt(1, 8), cmd, cx);
    let first = fake.hung_ids(DEFINITION).remove(0);
    // Within the word: still waiting, nothing new sent.
    move_over(&view, pt(1, 10), cmd, cx);
    assert_eq!(count(&fake, DEFINITION), 1);
    assert_eq!(count(&fake, "$/cancelRequest"), 0);
    move_over(&view, pt(0, 4), cmd, cx);
    let cancels = sent(&fake, "$/cancelRequest");
    assert_eq!(cancels.len(), 1);
    assert_eq!(cancels[0]["id"], first);
    // Its answer, if it still came, links nothing.
    fake.answer_hung(&first, location(dir.path(), "b.rs", 1, 7, 13));
    settle(cx);
    assert_eq!(link_range(&view, cx), None);
}

#[gpui::test]
fn cmd_click_jumps(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, view, cx) = setup(TRUSTED, pt(0, 0), linking, cx);
    let cmd = secondary(false, cx);
    move_over(&view, pt(1, 8), cmd, cx);
    click(&view, pt(1, 8), cmd, cx);
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));
    // The cursor was put at the click first: back returns there.
    run(&wb, "workbench.action.navigateBack", cx);
    assert_eq!(active_file(&wb, cx), "main.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 8));
}

#[gpui::test]
fn cmd_click_without_link_shown_still_jumps(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, view, cx) = setup(TRUSTED, pt(0, 0), linking, cx);
    let cmd = secondary(false, cx);
    click(&view, pt(1, 10), cmd, cx);
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));
}

#[gpui::test]
fn cmd_click_without_result_is_silent(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, view, cx) = setup(
        TRUSTED,
        pt(1, 8),
        |_| {
            FakeServer::new()
                .on_request(DEFINITION, |_| Ok(Value::Null))
                .on_request(REFERENCES, |_| Ok(json!([])))
        },
        cx,
    );
    let cmd = secondary(false, cx);
    move_over(&view, pt(0, 4), cmd, cx);
    assert_eq!(link_range(&view, cx), None);
    click(&view, pt(0, 4), cmd, cx);
    assert_eq!(active_file(&wb, cx), "main.rs");
    assert_eq!(cursor(&wb, cx), pt(0, 4), "a plain click");
    assert_eq!(message(&view, cx), None);
}

#[gpui::test]
fn alt_cmd_click_opens_aside(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, view, cx) = setup(TRUSTED, pt(0, 0), linking, cx);
    let alt_cmd = secondary(true, cx);
    move_over(&view, pt(1, 8), alt_cmd, cx);
    assert!(link_range(&view, cx).is_some());
    click(&view, pt(1, 8), alt_cmd, cx);
    let (groups, active) = cx.update(|_, cx| {
        let model = wb.read(cx).editor_groups().read(cx).model();
        (model.groups().len(), model.active_group())
    });
    assert_eq!((groups, active), (2, 1));
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));
}

#[gpui::test]
fn cmd_drag_selects_without_jumping(cx: &mut TestAppContext) {
    let (_dir, fake, wb, view, cx) = setup(TRUSTED, pt(0, 0), linking, cx);
    let cmd = secondary(false, cx);
    let (from, to) = (char_at(&view, pt(1, 8), cx), char_at(&view, pt(1, 12), cx));
    let left = gpui::MouseButton::Left;
    cx.simulate_mouse_down(from, left, cmd);
    cx.simulate_mouse_move(to, left, cmd);
    cx.simulate_mouse_up(to, left, cmd);
    settle(cx);
    let selection = view.read_with(cx, |v, _| v.selection());
    assert!(
        selection.as_ref().is_some_and(|s| s.start == pt(1, 8)),
        "{selection:?}"
    );
    assert_eq!(count(&fake, DEFINITION), 0, "no lookup for a drag");
    assert_eq!(active_file(&wb, cx), "main.rs");
}

#[gpui::test]
fn cmd_click_with_nothing_cached_and_no_result_is_silent(cx: &mut TestAppContext) {
    let (_dir, fake, wb, view, cx) = setup(
        TRUSTED,
        pt(0, 0),
        |_| FakeServer::new().on_request(DEFINITION, |_| Ok(Value::Null)),
        cx,
    );
    // No ⌘-hover first: the click itself asks.
    click(&view, pt(1, 8), secondary(false, cx), cx);
    assert_eq!(count(&fake, DEFINITION), 1);
    assert_eq!(active_file(&wb, cx), "main.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 8));
    assert_eq!(message(&view, cx), None, "no 'No definition found'");
    assert!(cx.debug_bounds("goto-message").is_none());
}

#[gpui::test]
fn new_text_clears_the_link(cx: &mut TestAppContext) {
    let (_dir, fake, _wb, view, cx) = setup(TRUSTED, pt(0, 0), linking, cx);
    let cmd = secondary(false, cx);
    move_over(&view, pt(1, 8), cmd, cx);
    assert!(link_range(&view, cx).is_some());
    let text = format!("{MAIN}// more\n");
    let snapshot = lucerna_text::Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot();
    view.update(cx, |v, cx| v.set_buffer(snapshot, cx));
    settle(cx);
    assert_eq!(link_range(&view, cx), None);
    // The cached answer was about the old text: asked again.
    move_over(&view, pt(1, 9), cmd, cx);
    assert_eq!(count(&fake, DEFINITION), 2);
    assert!(link_range(&view, cx).is_some());
}

// The locations picker (spec §7.4, `locations_picker.rs`).

use lucerna_workbench::LocationsPicker;

/// helper and other in b.rs, main in main.rs: three definitions.
fn three(dir: &Path) -> FakeServer {
    let targets = json!([
        location(dir, "b.rs", 2, 7, 12),
        location(dir, "main.rs", 0, 3, 7),
        location(dir, "b.rs", 1, 7, 13),
    ]);
    FakeServer::new().on_request(DEFINITION, move |_| Ok(targets.clone()))
}

fn picker(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<Entity<LocationsPicker>> {
    cx.update(|_, cx| wb.read(cx).locations_picker().cloned())
}

fn rows(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    let picker = picker(wb, cx).expect("the picker is shown");
    picker.read_with(cx, |p, _| p.row_texts())
}

/// The selected location: file name and start.
fn selected(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<(String, Point)> {
    let picker = picker(wb, cx)?;
    picker.read_with(cx, |p, _| {
        p.selected().map(|l| {
            let name = l.path.file_name().unwrap().to_string_lossy().into_owned();
            (name, l.range.start)
        })
    })
}

/// The preview's file name, cursor and highlighted rows.
fn preview(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> (String, Point, Vec<(u32, u32)>) {
    let picker = picker(wb, cx).expect("the picker is shown");
    let view = picker
        .read_with(cx, |p, _| p.preview().cloned())
        .expect("a preview");
    view.read_with(cx, |v, _| {
        (
            v.path().file_name().unwrap().to_string_lossy().into_owned(),
            v.cursor(),
            v.line_highlights()
                .iter()
                .map(|r| (r.start, r.end))
                .collect(),
        )
    })
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    settle(cx);
}

fn groups(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> (usize, usize) {
    cx.update(|_, cx| {
        let model = wb.read(cx).editor_groups().read(cx).model();
        (model.groups().len(), model.active_group())
    })
}

#[gpui::test]
fn multiple_definitions_open_picker_grouped(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(TRUSTED, pt(1, 8), three, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    // The current file first, then by path; by place within a file.
    assert_eq!(
        rows(&wb, cx),
        [
            "header:main.rs",
            "1:fn main() {",
            "header:b.rs",
            "2:pub fn helper() {}",
            "3:pub fn other() {}",
        ]
    );
    assert_eq!(selected(&wb, cx), Some(("main.rs".into(), pt(0, 3))));
    assert!(cx.debug_bounds("locations-picker").is_some(), "drawn");
    assert!(
        cx.debug_bounds("locations-preview").is_some(),
        "preview drawn"
    );
    assert_eq!(preview(&wb, cx), ("main.rs".into(), pt(0, 3), vec![(0, 1)]));
    // Nothing jumped, and the input has the focus.
    assert_eq!(active_file(&wb, cx), "main.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 8));
    let p = picker(&wb, cx).unwrap();
    let placeholder = cx.update(|_, cx| p.read(cx).input().read(cx).placeholder().to_string());
    assert_eq!(placeholder, "Filter definitions…");
}

#[gpui::test]
fn filter_narrows_rows(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(TRUSTED, pt(1, 8), three, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    cx.simulate_input("other");
    settle(cx);
    assert_eq!(rows(&wb, cx), ["header:b.rs", "3:pub fn other() {}"]);
    assert_eq!(selected(&wb, cx), Some(("b.rs".into(), pt(2, 7))));
    assert_eq!(preview(&wb, cx).0, "b.rs");
    // The file name is matched too.
    let picker = picker(&wb, cx).unwrap();
    let input = picker.read_with(cx, |p, _| p.input().clone());
    cx.update(|window, cx| input.update(cx, |i, cx| i.set_text("main", window, cx)));
    settle(cx);
    assert_eq!(rows(&wb, cx), ["header:main.rs", "1:fn main() {"]);
    cx.update(|window, cx| input.update(cx, |i, cx| i.set_text("zzz", window, cx)));
    settle(cx);
    assert!(rows(&wb, cx).is_empty());
    assert_eq!(selected(&wb, cx), None);
}

#[gpui::test]
fn arrow_moves_preview(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(TRUSTED, pt(1, 8), three, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    // Down skips the b.rs header.
    keys("down", cx);
    assert_eq!(selected(&wb, cx), Some(("b.rs".into(), pt(1, 7))));
    assert_eq!(preview(&wb, cx), ("b.rs".into(), pt(1, 7), vec![(1, 2)]));
    keys("down", cx);
    assert_eq!(selected(&wb, cx), Some(("b.rs".into(), pt(2, 7))));
    assert_eq!(preview(&wb, cx), ("b.rs".into(), pt(2, 7), vec![(2, 3)]));
    // Wraps at both ends.
    keys("down", cx);
    assert_eq!(selected(&wb, cx), Some(("main.rs".into(), pt(0, 3))));
    keys("up", cx);
    assert_eq!(selected(&wb, cx), Some(("b.rs".into(), pt(2, 7))));
    // Previewing moved nothing in the editor.
    assert_eq!(active_file(&wb, cx), "main.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 8));
}

#[gpui::test]
fn enter_opens_selected(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(TRUSTED, pt(1, 8), three, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    keys("down", cx);
    keys("enter", cx);
    assert!(picker(&wb, cx).is_none());
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));
    assert_eq!(groups(&wb, cx), (1, 0));
    run(&wb, "workbench.action.navigateBack", cx);
    assert_eq!(active_file(&wb, cx), "main.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 8));
}

#[gpui::test]
fn click_selects_double_click_opens(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(TRUSTED, pt(1, 8), three, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    let row = cx
        .debug_bounds("locations-picker-row-4")
        .expect("row 4 drawn")
        .center();
    cx.simulate_click(row, gpui::Modifiers::none());
    settle(cx);
    assert_eq!(selected(&wb, cx), Some(("b.rs".into(), pt(2, 7))));
    assert!(picker(&wb, cx).is_some(), "a click only selects");
    cx.simulate_event(gpui::MouseDownEvent {
        position: row,
        button: gpui::MouseButton::Left,
        modifiers: gpui::Modifiers::none(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: row,
        button: gpui::MouseButton::Left,
        modifiers: gpui::Modifiers::none(),
        click_count: 2,
    });
    settle(cx);
    assert!(picker(&wb, cx).is_none());
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(2, 7));
}

#[gpui::test]
fn cmd_enter_opens_aside(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(TRUSTED, pt(1, 8), three, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    keys("down", cx);
    keys("cmd-enter", cx);
    assert!(picker(&wb, cx).is_none());
    assert_eq!(groups(&wb, cx), (2, 1));
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));
}

#[gpui::test]
fn reveal_aside_picker_enter_opens_aside(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(TRUSTED, pt(1, 8), three, cx);
    run(&wb, "editor.action.revealDefinitionAside", cx);
    assert!(picker(&wb, cx).is_some());
    assert_eq!(groups(&wb, cx), (1, 0), "nothing opened yet");
    keys("down", cx);
    keys("enter", cx);
    assert_eq!(groups(&wb, cx), (2, 1));
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));
}

#[gpui::test]
fn esc_closes_without_jump(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, view, cx) = setup(TRUSTED, pt(1, 8), three, cx);
    let editor_focus = cx.update(|_, cx| view.read(cx).focus_handle(cx));
    cx.update(|window, cx| window.focus(&editor_focus, cx));
    run(&wb, "editor.action.revealDefinition", cx);
    assert!(
        !cx.update(|window, _| editor_focus.is_focused(window)),
        "the input has it"
    );
    keys("down", cx);
    let preview = picker(&wb, cx)
        .unwrap()
        .read_with(cx, |p, _| p.preview().unwrap().downgrade());
    keys("escape", cx);
    assert!(picker(&wb, cx).is_none());
    assert!(cx.debug_bounds("locations-picker").is_none());
    assert_eq!(active_file(&wb, cx), "main.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 8));
    assert!(preview.upgrade().is_none(), "the preview is dropped");
    // The editor has the focus back.
    assert!(cx.update(|window, _| editor_focus.is_focused(window)));
}

#[gpui::test]
fn peek_with_single_result_still_opens_picker(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(
        TRUSTED,
        pt(1, 8),
        |dir| {
            let target = location(dir, "b.rs", 1, 7, 13);
            FakeServer::new().on_request(DEFINITION, move |_| Ok(target.clone()))
        },
        cx,
    );
    run(&wb, "editor.action.peekDefinition", cx);
    assert_eq!(rows(&wb, cx), ["header:b.rs", "2:pub fn helper() {}"]);
    assert_eq!(active_file(&wb, cx), "main.rs");
    keys("enter", cx);
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));
}

#[gpui::test]
fn goto_and_peek_jumps_and_lists(cx: &mut TestAppContext) {
    let settings = r#"{ "session.trustAllWorktrees": true,
                        "editor.gotoLocation.multipleDefinitions": "gotoAndPeek" }"#;
    let (_dir, _fake, wb, _view, cx) = setup(settings, pt(1, 8), three, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    // Jumps to the first result (by place: b.rs helper), then lists them
    // all from there: the file jumped to comes first, so the selected row
    // is the place shown.
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(1, 7));
    assert_eq!(rows(&wb, cx)[0], "header:b.rs");
    assert_eq!(selected(&wb, cx), Some(("b.rs".into(), pt(1, 7))));
    // Esc keeps the jump.
    keys("escape", cx);
    assert!(picker(&wb, cx).is_none());
    assert_eq!(active_file(&wb, cx), "b.rs");
}

#[gpui::test]
fn preview_click_keeps_focus_in_input(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(TRUSTED, pt(1, 8), three, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    let input = picker(&wb, cx)
        .unwrap()
        .read_with(cx, |p, cx| p.input().read(cx).focus_handle(cx));
    let at = cx
        .debug_bounds("locations-preview")
        .expect("preview drawn")
        .center();
    cx.simulate_click(at, gpui::Modifiers::none());
    settle(cx);
    assert!(picker(&wb, cx).is_some(), "a click inside keeps it open");
    assert!(cx.update(|window, _| input.is_focused(window)));
    // Arrows still move the selection.
    keys("down", cx);
    assert_eq!(selected(&wb, cx), Some(("b.rs".into(), pt(1, 7))));
}

#[gpui::test]
fn many_locations_and_a_huge_line_stay_bounded(cx: &mut TestAppContext) {
    const ROWS: u32 = 5_000;
    let (_dir, _fake, wb, _view, cx) = setup(
        TRUSTED,
        pt(1, 8),
        |dir| {
            // Row 0 is a 200 KB minified line with `needle` at its end.
            let mut text = format!("{} needle\n", "x".repeat(200_000));
            for row in 1..ROWS {
                text.push_str(&format!("fn f{row}() {{}}\n"));
            }
            fs::write(dir.join("src/big.rs"), text).unwrap();
            let mut targets = vec![location(dir, "big.rs", 0, 200_001, 200_007)];
            targets.extend((1..ROWS).map(|row| location(dir, "big.rs", row, 3, 4)));
            let targets = Value::Array(targets);
            FakeServer::new().on_request(DEFINITION, move |_| Ok(targets.clone()))
        },
        cx,
    );
    run(&wb, "editor.action.revealDefinition", cx);
    let texts = rows(&wb, cx);
    assert_eq!(texts.len(), ROWS as usize + 1);
    assert_eq!(texts[0], "header:big.rs");
    for text in &texts[1..] {
        let (_, line) = text.split_once(':').unwrap();
        assert!(line.chars().count() <= lucerna_workbench::LOCATION_EXCERPT_CHARS);
    }
    assert!(texts[1].ends_with("x needle"), "{}", &texts[1]);
    // Only the rows on screen are built.
    assert!(cx.debug_bounds("locations-picker-row-1").is_some());
    assert!(cx.debug_bounds("locations-picker-row-4000").is_none());
    // Arrows wrap to the last row, which scrolls into view.
    keys("up", cx);
    assert_eq!(selected(&wb, cx), Some(("big.rs".into(), pt(ROWS - 1, 3))));
    assert!(cx.debug_bounds("locations-picker-row-5000").is_some());
    // The filter reads the shown part of the huge line.
    cx.simulate_input("needle");
    settle(cx);
    assert_eq!(rows(&wb, cx).len(), 2);
    assert_eq!(selected(&wb, cx), Some(("big.rs".into(), pt(0, 200_001))));
}

#[gpui::test]
fn quick_open_closes_the_picker(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, view, cx) = setup(TRUSTED, pt(1, 8), three, cx);
    let editor_focus = cx.update(|_, cx| view.read(cx).focus_handle(cx));
    cx.update(|window, cx| window.focus(&editor_focus, cx));
    run(&wb, "editor.action.revealDefinition", cx);
    run(&wb, "workbench.action.quickOpen", cx);
    assert!(picker(&wb, cx).is_none());
    assert!(cx.update(|_, cx| wb.read(cx).quick_open().is_some()));
    // One Esc closes Quick Open and gives the editor its focus back.
    keys("escape", cx);
    assert!(cx.update(|_, cx| wb.read(cx).quick_open().is_none()));
    assert!(cx.update(|window, _| editor_focus.is_focused(window)));
}

#[gpui::test]
fn range_ending_at_column_zero_highlights_its_rows_only(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(
        TRUSTED,
        pt(1, 8),
        |dir| {
            // Row 1 of b.rs whole, ending at the start of row 2.
            let whole = json!({ "uri": uri(dir, "b.rs"), "range": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 2, "character": 0 } } });
            FakeServer::new().on_request(DEFINITION, move |_| Ok(whole.clone()))
        },
        cx,
    );
    run(&wb, "editor.action.peekDefinition", cx);
    assert_eq!(preview(&wb, cx), ("b.rs".into(), pt(1, 0), vec![(1, 2)]));
}

#[gpui::test]
fn jump_survives_a_preview_dropped_before_the_tab_opens(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(TRUSTED, pt(1, 8), three, cx);
    // Only a preview holds b.rs, as when Enter is pressed in the picker.
    let docs = cx.update(|_, cx| wb.read(cx).documents().clone());
    let path = cx.update(|_, cx| wb.read(cx).root().join("src/b.rs"));
    let preview = docs.update(cx, |d, cx| d.open_preview(Arc::from(path.as_path()), cx));
    settle(cx);
    // The preview goes before the tab's view is made (on the next draw).
    cx.update(|_, cx| {
        wb.update(cx, |wb, cx| {
            let to = Arc::from(path.as_path());
            wb.jump_to(to, pt(2, 7), lucerna_workbench::OpenMode::Preview, cx)
        });
        let views = docs.read(cx).views_of(&path);
        assert_eq!(views, std::slice::from_ref(&preview), "no tab view yet");
        drop(preview);
    });
    settle(cx);
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert_eq!(cursor(&wb, cx), pt(2, 7));
}

fn split(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| {
            m.split_active();
        })
    });
    settle(cx);
}

fn focus_group(wb: &Entity<Workbench>, index: usize, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| {
            assert!(m.focus_group(index));
        })
    });
    settle(cx);
}

#[gpui::test]
fn navigation_history_stays_with_its_group_when_groups_shift(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, _view, cx) = setup(
        TRUSTED,
        pt(1, 8),
        |dir| {
            let target = location(dir, "b.rs", 1, 7, 13);
            FakeServer::new().on_request(DEFINITION, move |_| Ok(target.clone()))
        },
        cx,
    );
    // [A, B]: B jumps main.rs → b.rs, so B has history and A has none.
    split(&wb, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    assert_eq!(active_file(&wb, cx), "b.rs");

    // Splitting A inserts a group between A and B: [A, new, B].
    focus_group(&wb, 0, cx);
    split(&wb, cx);
    let view = active_view(&wb, cx);
    view.update(cx, |v, cx| v.set_cursor(pt(0, 0), cx));
    settle(cx);
    run(&wb, "workbench.action.navigateBack", cx);
    assert_eq!(active_file(&wb, cx), "main.rs");
    assert_eq!(cursor(&wb, cx), pt(0, 0), "the new group has no history");

    focus_group(&wb, 2, cx);
    assert_eq!(active_file(&wb, cx), "b.rs");
    run(&wb, "workbench.action.navigateBack", cx);
    assert_eq!(active_file(&wb, cx), "main.rs", "B keeps its own history");
}

fn single(dir: &Path) -> FakeServer {
    let target = location(dir, "b.rs", 1, 7, 13);
    FakeServer::new().on_request(DEFINITION, move |_| Ok(target.clone()))
}

#[gpui::test]
fn f12_single_result_keeps_editor_text_focus(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, view, cx) = setup(TRUSTED, pt(1, 8), single, cx);
    focus_view(&view, cx);
    assert!(editor_text_focus(&wb, cx));
    run(&wb, "editor.action.revealDefinition", cx);
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert!(editor_text_focus(&wb, cx), "editor keys still work");
}

#[gpui::test]
fn picker_enter_keeps_editor_text_focus(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, view, cx) = setup(TRUSTED, pt(1, 8), three, cx);
    focus_view(&view, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    keys("down", cx);
    keys("enter", cx);
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert!(editor_text_focus(&wb, cx));
}

#[gpui::test]
fn goto_and_peek_esc_keeps_editor_text_focus(cx: &mut TestAppContext) {
    let settings = r#"{ "session.trustAllWorktrees": true,
                        "editor.gotoLocation.multipleDefinitions": "gotoAndPeek" }"#;
    let (_dir, _fake, wb, view, cx) = setup(settings, pt(1, 8), three, cx);
    focus_view(&view, cx);
    run(&wb, "editor.action.revealDefinition", cx);
    assert_eq!(active_file(&wb, cx), "b.rs");
    keys("escape", cx);
    assert!(picker(&wb, cx).is_none());
    assert!(editor_text_focus(&wb, cx));
}

#[gpui::test]
fn cmd_click_keeps_editor_text_focus(cx: &mut TestAppContext) {
    let (_dir, _fake, wb, view, cx) = setup(TRUSTED, pt(0, 0), linking, cx);
    let cmd = secondary(false, cx);
    move_over(&view, pt(1, 8), cmd, cx);
    click(&view, pt(1, 8), cmd, cx);
    assert_eq!(active_file(&wb, cx), "b.rs");
    assert!(editor_text_focus(&wb, cx));
}
