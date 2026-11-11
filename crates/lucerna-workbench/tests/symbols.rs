//! Document symbols (spec §7.6): from the language server, else from the
//! language's `outline.scm`, listed by Quick Open `@` with a preview of
//! the selected symbol in the editor.

mod support;

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_lsp::position::PositionEncoding;
use lucerna_lsp::symbols::symbols_from_response;
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_syntax::{Associations, ExtensionLanguage, LanguageRegistry};
use lucerna_text::Point;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, DocumentEvent, QuickOpen, SymbolState, Workbench};
use serde_json::{Value, json};
use support::lsp::{TRUSTED, init_with, language_servers, lsp_workbench_with, open, settle};
use tempfile::TempDir;

const DOCUMENT_SYMBOL: &str = "textDocument/documentSymbol";

/// `Foo` and its impl on rows 0–10, comments, then `main` on row 90.
fn main_rs() -> String {
    let mut text = String::from(
        "struct Foo {\n    a: u32,\n}\nimpl Foo {\n    fn new() -> Foo {\n        Foo { a: 0 }\n    }\n    fn get(&self) -> u32 {\n        self.a\n    }\n}\n",
    );
    for row in 11..90 {
        text.push_str(&format!("// {row}\n"));
    }
    text.push_str("fn main() {\n    let f = Foo::new();\n}\n");
    text
}

fn range(l1: u32, c1: u32, l2: u32, c2: u32) -> Value {
    json!({ "start": { "line": l1, "character": c1 }, "end": { "line": l2, "character": c2 } })
}

/// What rust-analyzer would answer for [`main_rs`].
fn main_symbols() -> Value {
    json!([
        { "name": "Foo", "kind": 23, "range": range(0, 0, 2, 1),
          "selectionRange": range(0, 7, 0, 10),
          "children": [
            { "name": "a", "kind": 8, "range": range(1, 4, 1, 10),
              "selectionRange": range(1, 4, 1, 5) }
          ] },
        { "name": "impl Foo", "kind": 19, "range": range(3, 0, 10, 1),
          "selectionRange": range(3, 5, 3, 8),
          "children": [
            { "name": "new", "detail": "fn() -> Foo", "kind": 6,
              "range": range(4, 4, 6, 5), "selectionRange": range(4, 7, 4, 10) },
            { "name": "get", "kind": 6, "range": range(7, 4, 9, 5),
              "selectionRange": range(7, 7, 7, 10) }
          ] },
        { "name": "main", "detail": "fn()", "kind": 12, "range": range(90, 0, 92, 1),
          "selectionRange": range(90, 3, 90, 7) }
    ])
}

fn pt(row: u32, column: u32) -> Point {
    Point { row, column }
}

fn rust_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), main_rs()).unwrap();
    fs::write(dir.path().join("notes.txt"), "just text\n").unwrap();
    dir
}

/// Settles, then runs the `documentSymbol` debounce (300 ms).
fn settle_symbols(cx: &mut VisualTestContext) {
    settle(cx);
    cx.executor().advance_clock(Duration::from_millis(400));
    settle(cx);
}

/// A workbench on the Rust project whose server is `fake`, with main.rs
/// open and the cursor at `cursor`.
fn setup<'a>(
    fake: &FakeServer,
    cursor: Point,
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<CodeView>,
    &'a mut VisualTestContext,
) {
    let dir = rust_project();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher, TRUSTED, cx);
    let view = open(&wb, "src/main.rs", cx);
    view.update(cx, |v, cx| v.set_cursor(cursor, cx));
    settle_symbols(cx);
    (dir, wb, view, cx)
}

fn symbols_server() -> FakeServer {
    FakeServer::new().on_request(DOCUMENT_SYMBOL, |_| Ok(main_symbols()))
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(wb.run_command(command, None, window, cx), "{command}");
        })
    });
    settle_symbols(cx);
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    settle(cx);
}

fn type_text(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    settle(cx);
}

fn quick_open(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<Entity<QuickOpen>> {
    cx.update(|_, cx| wb.read(cx).quick_open().cloned())
}

fn rows(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    let qo = quick_open(wb, cx).expect("quick open is shown");
    cx.update(|_, cx| qo.read(cx).row_texts())
}

fn selected(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> usize {
    let qo = quick_open(wb, cx).expect("quick open is shown");
    cx.update(|_, cx| qo.read(cx).selected_index())
}

/// Cursor, scroll top row and line highlights of `view`.
fn place(
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
) -> (Point, u32, Vec<std::ops::Range<u32>>) {
    cx.update(|_, cx| {
        let v = view.read(cx);
        (v.cursor(), v.scroll_top_row(), v.line_highlights().to_vec())
    })
}

#[gpui::test]
fn at_lists_lsp_symbols_indented(cx: &mut TestAppContext) {
    let fake = symbols_server();
    let (_dir, wb, _view, cx) = setup(&fake, pt(5, 8), cx);
    run(&wb, "workbench.action.gotoSymbol", cx);
    assert_eq!(
        rows(&wb, cx),
        vec![
            "Foo",
            "  a",
            "impl Foo",
            "  new fn() -> Foo",
            "  get",
            "main fn()"
        ]
    );
    assert_eq!(selected(&wb, cx), 3, "the deepest symbol at the cursor");
    let qo = quick_open(&wb, cx).expect("quick open is shown");
    assert_eq!(cx.update(|_, cx| qo.read(cx).query(cx)), "@");
}

#[gpui::test]
fn filter_matches_in_name_only_for_single_word(cx: &mut TestAppContext) {
    let fake = symbols_server();
    let (_dir, wb, _view, cx) = setup(&fake, pt(0, 0), cx);
    run(&wb, "workbench.action.gotoSymbol", cx);
    type_text("foo", cx);
    let mut found = rows(&wb, cx);
    found.sort();
    // `new` and `get` are inside `impl Foo`, and `new`'s detail names
    // `Foo`: one word must match the name itself.
    assert_eq!(found, vec!["Foo", "impl Foo"]);

    // Several words match against the ancestors and the name.
    type_text(" get", cx);
    assert_eq!(rows(&wb, cx), vec!["get in impl Foo"]);
}

#[gpui::test]
fn group_by_kind_headers(cx: &mut TestAppContext) {
    let fake = symbols_server();
    let (_dir, wb, _view, cx) = setup(&fake, pt(5, 8), cx);
    run(&wb, "workbench.action.gotoSymbol", cx);
    type_text(":", cx);
    assert_eq!(
        rows(&wb, cx),
        vec![
            "methods (2)",
            "new fn() -> Foo in impl Foo",
            "get in impl Foo",
            "fields (1)",
            "a in Foo",
            "functions (1)",
            "main fn()",
            "objects (1)",
            "impl Foo",
            "structs (1)",
            "Foo",
        ]
    );
    assert_eq!(
        selected(&wb, cx),
        1,
        "the symbol at the cursor, not a header"
    );
    // Headers are skipped.
    keys("up", cx);
    assert_eq!(selected(&wb, cx), 10, "wraps past the first header");
    keys("down", cx);
    assert_eq!(selected(&wb, cx), 1);
    keys("down down", cx);
    assert_eq!(selected(&wb, cx), 4);
}

#[gpui::test]
fn preview_scrolls_and_esc_restores(cx: &mut TestAppContext) {
    let fake = symbols_server();
    let (_dir, wb, view, cx) = setup(&fake, pt(91, 4), cx);
    let (cursor, scroll, highlights) = place(&view, cx);
    assert_eq!(cursor, pt(91, 4));
    assert!(scroll > 20, "the cursor row is in view: {scroll}");
    assert!(highlights.is_empty());

    run(&wb, "workbench.action.gotoSymbol", cx);
    assert_eq!(selected(&wb, cx), 5);
    assert_eq!(place(&view, cx), (cursor, scroll, vec![]), "no preview yet");

    keys("up", cx);
    let (now_cursor, now_scroll, now_highlights) = place(&view, cx);
    assert_eq!(now_highlights, vec![7..10], "the rows of `get`");
    assert!(now_scroll <= 7, "`get` is in view: {now_scroll}");
    assert_eq!(now_cursor, cursor, "the preview leaves the cursor");

    keys("escape", cx);
    assert!(quick_open(&wb, cx).is_none());
    assert_eq!(place(&view, cx), (cursor, scroll, vec![]));
}

#[gpui::test]
fn enter_jumps_and_records_history(cx: &mut TestAppContext) {
    let fake = symbols_server();
    let (_dir, wb, view, cx) = setup(&fake, pt(91, 4), cx);
    run(&wb, "workbench.action.gotoSymbol", cx);
    type_text("get", cx);
    assert_eq!(rows(&wb, cx), vec!["get in impl Foo"]);
    keys("enter", cx);
    assert!(quick_open(&wb, cx).is_none());
    let (cursor, _, highlights) = place(&view, cx);
    assert_eq!(cursor, pt(7, 7), "the start of the selection range");
    assert!(highlights.is_empty());

    run(&wb, "workbench.action.navigateBack", cx);
    assert_eq!(place(&view, cx).0, pt(91, 4));
}

#[gpui::test]
fn no_source_message(cx: &mut TestAppContext) {
    let fake = FakeServer::new().hang_on(DOCUMENT_SYMBOL);
    let (_dir, wb, _view, cx) = setup(&fake, pt(0, 0), cx);
    run(&wb, "workbench.action.gotoSymbol", cx);
    assert_eq!(rows(&wb, cx), vec!["Loading symbols…"]);

    let id = fake
        .hung_ids(DOCUMENT_SYMBOL)
        .last()
        .cloned()
        .expect("a documentSymbol request");
    assert!(fake.answer_hung(&id, json!([])));
    settle_symbols(cx);
    assert_eq!(rows(&wb, cx), vec!["No symbols found"]);

    keys("escape", cx);
    open(&wb, "notes.txt", cx);
    settle_symbols(cx);
    run(&wb, "workbench.action.gotoSymbol", cx);
    assert_eq!(
        rows(&wb, cx),
        vec!["No language server or outline for this file"]
    );
}

/// The `json.wasm` grammar fixture of `lucerna-syntax`.
fn json_wasm() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../lucerna-syntax/tests/fixtures/wasm/json.wasm")
}

#[gpui::test]
fn at_falls_back_to_outline_without_server(cx: &mut TestAppContext) {
    let dir = TempDir::new().unwrap();
    let language_dir = dir.path().join("language");
    fs::create_dir(&language_dir).unwrap();
    fs::write(language_dir.join("highlights.scm"), "(string) @string\n").unwrap();
    // Every key of an object is a symbol, nested by its value.
    fs::write(
        language_dir.join("outline.scm"),
        "(pair key: (_) @name) @item\n",
    )
    .unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(
        project.join("data.outl"),
        "{\n  \"a\": {\n    \"b\": 1\n  },\n  \"c\": 2\n}\n",
    )
    .unwrap();
    let registry = LanguageRegistry::build(
        &[ExtensionLanguage {
            extension_id: "outline-test".into(),
            name: "Outline Test".into(),
            dir: language_dir,
            grammar: Some(("json".into(), json_wasm())),
            path_suffixes: vec!["outl".into()],
            first_line_pattern: None,
            hidden: false,
        }],
        Associations::default(),
    );

    init_with(TRUSTED, cx);
    cx.update(|cx| cx.global_mut::<AppState>().languages = Arc::new(registry));
    let root = project.clone();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    let servers = language_servers(&wb, cx);
    servers.update(cx, |servers, _| {
        servers.set_launcher(Arc::new(FakeLauncher::new()))
    });
    open(&wb, "data.outl", cx);
    settle_symbols(cx);

    run(&wb, "workbench.action.gotoSymbol", cx);
    assert_eq!(rows(&wb, cx), vec!["\"a\"", "  \"b\"", "\"c\""]);
    keys("escape", cx);

    // A reload first keeps the old tree, edited; the parse that lands
    // after it is announced, and the outline is read again from it.
    let documents = cx.update(|_, cx| wb.read(cx).documents().clone());
    let parsed = Rc::new(RefCell::new(Vec::new()));
    let _subscription = cx.update(|_, cx| {
        let parsed = parsed.clone();
        cx.subscribe(&documents, move |_, ev, _| {
            if let DocumentEvent::Parsed { path } = ev {
                parsed.borrow_mut().push(path.clone());
            }
        })
    });
    fs::write(project.join("data.outl"), "{\n  \"x\": 1\n}\n").unwrap();
    let change = Change {
        path: Arc::from(Path::new("data.outl")),
        kind: ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    settle_symbols(cx);
    assert_eq!(
        parsed.borrow().as_slice(),
        &[Arc::from(project.join("data.outl").as_path())]
    );
    // Fresh without waiting for another fetch.
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("workbench.action.gotoSymbol", None, window, cx)
        })
    });
    settle(cx);
    assert_eq!(rows(&wb, cx), vec!["\"x\""]);
}

#[gpui::test]
fn new_symbols_keep_the_selection_and_preview(cx: &mut TestAppContext) {
    let fake = symbols_server();
    let (_dir, wb, view, cx) = setup(&fake, pt(0, 0), cx);
    run(&wb, "workbench.action.gotoSymbol", cx);
    assert_eq!(selected(&wb, cx), 0);
    keys("down down", cx);
    assert_eq!(selected(&wb, cx), 2);
    let before = place(&view, cx);
    assert_eq!(before.2, vec![3..11], "`impl Foo` is previewed");

    // The same symbols again, as a refetch brings them.
    let snapshot = cx.update(|_, cx| view.read(cx).snapshot().clone());
    let tree = symbols_from_response(
        serde_json::from_value(main_symbols()).unwrap(),
        &snapshot,
        PositionEncoding::Utf16,
    );
    let qo = quick_open(&wb, cx).expect("quick open is shown");
    qo.update(cx, |qo, cx| {
        qo.set_symbol_state(SymbolState::Ready(Arc::new(tree)), cx)
    });
    settle(cx);
    assert_eq!(selected(&wb, cx), 2);
    assert_eq!(place(&view, cx), before);
}
