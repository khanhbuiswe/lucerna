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

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use lucerna_lsp::position::PositionEncoding;
use lucerna_lsp::symbols::symbols_from_response;
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_syntax::{Associations, ExtensionLanguage, LanguageRegistry, Symbol, SymbolKind};
use lucerna_text::Point;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, BreadcrumbMode, DocumentEvent, EMPTY_SYMBOLS_TTL, QuickOpen, Segment, SymbolState,
    Workbench, breadcrumb_segments,
};
use serde_json::{Value, json};
use support::lsp::{
    TRUSTED, count, draw, editor_text_focus, focus_view, init_with, language_servers,
    lsp_workbench_with, open, settle,
};
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
    setup_with(fake, cursor, TRUSTED, cx)
}

/// [`setup`] with `settings` as the user settings.
fn setup_with<'a>(
    fake: &FakeServer,
    cursor: Point,
    settings: &str,
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<CodeView>,
    &'a mut VisualTestContext,
) {
    let dir = rust_project();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher, settings, cx);
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

fn symbol(name: &str, kind: SymbolKind) -> Symbol {
    Symbol {
        name: name.into(),
        detail: None,
        kind,
        range: pt(0, 0)..pt(0, 0),
        selection_range: pt(0, 0)..pt(0, 0),
        children: Vec::new(),
    }
}

fn dir(name: &str) -> Segment {
    Segment::Dir(name.into())
}

fn file(name: &str) -> Segment {
    Segment::File(name.into())
}

fn sym(name: &str, kind: SymbolKind) -> Segment {
    Segment::Symbol {
        kind,
        name: name.into(),
    }
}

#[test]
fn segments_split_dirs_and_symbols() {
    let (imp, get) = (
        symbol("impl Foo", SymbolKind::Object),
        symbol("get", SymbolKind::Method),
    );
    let on = BreadcrumbMode::On;
    assert_eq!(
        breadcrumb_segments("crates/lucerna-lsp/src/server.rs", &[&imp, &get], on, on),
        vec![
            dir("crates"),
            dir("lucerna-lsp"),
            dir("src"),
            file("server.rs"),
            sym("impl Foo", SymbolKind::Object),
            sym("get", SymbolKind::Method),
        ]
    );
    // Absolute and home paths: no empty segment for the leading `/`.
    assert_eq!(
        breadcrumb_segments("/etc/hosts", &[], on, on),
        vec![dir("etc"), file("hosts")]
    );
    assert_eq!(
        breadcrumb_segments("~/notes.md", &[], on, on),
        vec![dir("~"), file("notes.md")]
    );
    assert_eq!(
        breadcrumb_segments("README.md", &[], on, on),
        vec![file("README.md")]
    );
}

#[test]
fn file_path_last_only() {
    let get = symbol("get", SymbolKind::Method);
    let (on, off, last) = (
        BreadcrumbMode::On,
        BreadcrumbMode::Off,
        BreadcrumbMode::Last,
    );
    assert_eq!(
        breadcrumb_segments("src/deep/main.rs", &[&get], last, on),
        vec![file("main.rs"), sym("get", SymbolKind::Method)]
    );
    assert_eq!(
        breadcrumb_segments("src/deep/main.rs", &[&get], off, on),
        vec![sym("get", SymbolKind::Method)]
    );
}

#[test]
fn symbol_path_off() {
    let (imp, get) = (
        symbol("impl Foo", SymbolKind::Object),
        symbol("get", SymbolKind::Method),
    );
    let (on, off, last) = (
        BreadcrumbMode::On,
        BreadcrumbMode::Off,
        BreadcrumbMode::Last,
    );
    assert_eq!(
        breadcrumb_segments("src/main.rs", &[&imp, &get], on, off),
        vec![dir("src"), file("main.rs")]
    );
    assert_eq!(
        breadcrumb_segments("src/main.rs", &[&imp, &get], on, last),
        vec![dir("src"), file("main.rs"), sym("get", SymbolKind::Method)]
    );
    assert_eq!(
        breadcrumb_segments("src/main.rs", &[], on, last),
        vec![dir("src"), file("main.rs")]
    );
}

#[test]
fn more_than_12_collapses() {
    let on = BreadcrumbMode::On;
    let symbols: Vec<Symbol> = (0..3)
        .map(|i| symbol(&format!("s{i}"), SymbolKind::Function))
        .collect();
    let refs: Vec<&Symbol> = symbols.iter().collect();
    // 10 folders, the file and 3 symbols: 14 segments.
    let path = "d0/d1/d2/d3/d4/d5/d6/d7/d8/d9/f.rs";
    assert_eq!(
        breadcrumb_segments(path, &refs, on, on),
        vec![
            dir("d0"),
            dir("d1"),
            dir("d2"),
            dir("d3"),
            dir("d4"),
            dir("d5"),
            Segment::Ellipsis,
            dir("d8"),
            dir("d9"),
            file("f.rs"),
            sym("s0", SymbolKind::Function),
            sym("s1", SymbolKind::Function),
            sym("s2", SymbolKind::Function),
        ]
    );
    // Exactly 12 stay whole.
    let twelve = breadcrumb_segments("d0/d1/d2/d3/d4/d5/d6/d7/f.rs", &refs, on, on);
    assert_eq!(twelve.len(), 12);
    assert!(!twelve.contains(&Segment::Ellipsis));
}

/// The breadcrumbs row of group `group` as text.
fn crumbs(wb: &Entity<Workbench>, group: usize, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .breadcrumbs_text(group, cx)
    })
}

#[gpui::test]
fn breadcrumbs_follow_cursor(cx: &mut TestAppContext) {
    let fake = symbols_server();
    let (_dir, wb, view, cx) = setup(&fake, pt(8, 8), cx);
    assert!(cx.debug_bounds("breadcrumbs-0").is_some());
    assert_eq!(
        crumbs(&wb, 0, cx).as_deref(),
        Some("src › main.rs › impl Foo › get")
    );
    // Symbols have a kind icon; folders and the file do not. A short row
    // starts at the left.
    let row = cx.debug_bounds("breadcrumbs-0").expect("a breadcrumbs row");
    let icon = cx
        .debug_bounds("breadcrumb-icon-0-2")
        .expect("the icon of `impl Foo`");
    assert!(icon.left() < row.center().x, "{icon:?} in {row:?}");
    assert!(cx.debug_bounds("breadcrumb-icon-0-3").is_some());
    assert!(cx.debug_bounds("breadcrumb-icon-0-1").is_none());
    let requests = count(&fake, DOCUMENT_SYMBOL);

    // Batched: nothing changes until the 50 ms timer runs.
    view.update(cx, |v, cx| v.set_cursor(pt(91, 4), cx));
    cx.run_until_parked();
    assert_eq!(
        crumbs(&wb, 0, cx).as_deref(),
        Some("src › main.rs › impl Foo › get")
    );
    cx.executor().advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    assert_eq!(crumbs(&wb, 0, cx).as_deref(), Some("src › main.rs › main"));

    view.update(cx, |v, cx| v.set_cursor(pt(50, 0), cx));
    settle(cx);
    assert_eq!(crumbs(&wb, 0, cx).as_deref(), Some("src › main.rs"));
    // From the cached tree: the cursor moves ask the server nothing.
    assert_eq!(count(&fake, DOCUMENT_SYMBOL), requests);
}

#[gpui::test]
fn click_opens_symbol_quick_open(cx: &mut TestAppContext) {
    let fake = symbols_server();
    let (_dir, wb, _view, cx) = setup(&fake, pt(5, 8), cx);
    // Two groups; the right one is focused.
    run(&wb, "workbench.action.splitEditor", cx);
    let active_group = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).model().active_group())
    };
    assert_eq!(active_group(cx), 1);
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.focus_sidebar(window, cx)));
    settle(cx);

    let row = cx.debug_bounds("breadcrumbs-0").expect("a breadcrumbs row");
    cx.simulate_click(row.center(), Modifiers::none());
    settle_symbols(cx);
    assert_eq!(active_group(cx), 0, "the clicked row's group is focused");
    let qo = quick_open(&wb, cx).expect("quick open is shown");
    assert_eq!(cx.update(|_, cx| qo.read(cx).query(cx)), "@");
    assert_eq!(rows(&wb, cx).len(), 6);
    keys("escape", cx);
    assert!(quick_open(&wb, cx).is_none());

    // ⇧⌘. does the same from the editor.
    keys("shift-cmd-.", cx);
    let qo = quick_open(&wb, cx).expect("quick open is shown");
    assert_eq!(cx.update(|_, cx| qo.read(cx).query(cx)), "@");
}

#[gpui::test]
fn disabled_shows_plain_path(cx: &mut TestAppContext) {
    let fake = symbols_server();
    let settings = r#"{ "session.trustAllWorktrees": true, "breadcrumbs.enabled": false }"#;
    let (_dir, wb, _view, cx) = setup_with(&fake, pt(8, 8), settings, cx);
    assert_eq!(crumbs(&wb, 0, cx).as_deref(), Some("src/main.rs"));
    assert!(cx.debug_bounds("breadcrumbs-0").is_none());
    assert!(cx.debug_bounds("path-bar-0").is_some());
}

#[gpui::test]
fn file_path_last_and_icons_settings(cx: &mut TestAppContext) {
    let fake = symbols_server();
    let settings = r#"{ "session.trustAllWorktrees": true, "breadcrumbs.filePath": "last", "breadcrumbs.symbolPath": "last", "breadcrumbs.icons": false }"#;
    let (_dir, wb, _view, cx) = setup_with(&fake, pt(8, 8), settings, cx);
    assert_eq!(crumbs(&wb, 0, cx).as_deref(), Some("main.rs › get"));
    assert!(cx.debug_bounds("breadcrumb-icon-0-1").is_none());
}

#[gpui::test]
fn long_rows_are_cut_on_the_left(cx: &mut TestAppContext) {
    let fake = symbols_server();
    let (dir, wb, _view, cx) = setup(&fake, pt(8, 8), cx);
    let deep = "src/a_rather_long_folder_name/another_rather_long_folder_name/yet_another_folder";
    fs::create_dir_all(dir.path().join(deep)).unwrap();
    fs::write(dir.path().join(deep).join("main.rs"), main_rs()).unwrap();
    let view = open(&wb, &format!("{deep}/main.rs"), cx);
    view.update(cx, |v, cx| v.set_cursor(pt(8, 8), cx));
    cx.simulate_resize(gpui::size(gpui::px(500.), gpui::px(400.)));
    settle_symbols(cx);
    assert_eq!(
        crumbs(&wb, 0, cx).as_deref(),
        Some(format!("{} › main.rs › impl Foo › get", deep.replace('/', " › ")).as_str())
    );
    let row = cx.debug_bounds("breadcrumbs-0").expect("a breadcrumbs row");
    // The symbol at the cursor stays in view; the folders are cut.
    let last = cx
        .debug_bounds("breadcrumb-icon-0-6")
        .expect("the icon of `get`");
    assert!(last.right() <= row.right(), "{last:?} in {row:?}");
    assert!(last.left() >= row.left(), "{last:?} in {row:?}");
    let parent = cx
        .debug_bounds("breadcrumb-icon-0-5")
        .expect("the icon of `impl Foo`");
    assert!(parent.left() > row.left());
}

#[gpui::test]
fn breadcrumbs_do_not_poll_empty_symbols(cx: &mut TestAppContext) {
    let fake = FakeServer::new().on_request(DOCUMENT_SYMBOL, |_| Ok(json!([])));
    let (_dir, wb, _view, cx) = setup(&fake, pt(8, 8), cx);
    assert_eq!(crumbs(&wb, 0, cx).as_deref(), Some("src › main.rs"));
    let requests = count(&fake, DOCUMENT_SYMBOL);
    // The empty answer expires after 2 s; only Quick Open asks again.
    for _ in 0..4 {
        cx.executor().advance_clock(EMPTY_SYMBOLS_TTL);
        settle_symbols(cx);
    }
    assert_eq!(count(&fake, DOCUMENT_SYMBOL), requests);
}

#[gpui::test]
fn tab_switch_and_split_show_symbols_at_once(cx: &mut TestAppContext) {
    let fake = symbols_server();
    let (_dir, wb, _view, cx) = setup(&fake, pt(8, 8), cx);
    let crumbs_now = |cx: &mut VisualTestContext, group| {
        // No clock advance: the 50 ms batch does not run.
        cx.run_until_parked();
        draw(cx);
        crumbs(&wb, group, cx)
    };
    let main_tab = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups.model().active_tab().expect("main.rs").id
    });
    open(&wb, "notes.txt", cx);
    assert_eq!(crumbs(&wb, 0, cx).as_deref(), Some("notes.txt"));

    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| m.activate(main_tab))
    });
    assert_eq!(
        crumbs_now(cx, 0).as_deref(),
        Some("src › main.rs › impl Foo › get")
    );

    // The split's editor starts with its own cursor, at the top.
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| m.split_active())
    });
    assert_eq!(crumbs_now(cx, 1).as_deref(), Some("src › main.rs › Foo"));
}

// ---------------------------------------------------------------------------
// Quick Open `#`: workspace symbols.

const WORKSPACE_SYMBOL: &str = "workspace/symbol";
const RESOLVE_SYMBOL: &str = "workspaceSymbol/resolve";

/// Waits (real time) for the worktree scan, so Quick Open knows which
/// files are in the project.
fn wait_for_scan(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !wb.read_with(cx, |wb, _| wb.worktree().snapshot().scan_complete()) {
        assert!(
            std::time::Instant::now() < deadline,
            "worktree scan did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::ScanComplete, cx)
    });
    settle(cx);
}

/// Types `text` one char at a time, `gap` of clock apart (also after the
/// last char), without running the `#` debounce otherwise.
fn type_slowly(text: &str, gap: Duration, cx: &mut VisualTestContext) {
    for ch in text.chars() {
        cx.simulate_input(&ch.to_string());
        cx.run_until_parked();
        cx.executor().advance_clock(gap);
        cx.run_until_parked();
    }
}

fn uri(path: &Path) -> Value {
    serde_json::to_value(lucerna_lsp::position::path_to_uri(path)).unwrap()
}

/// A `SymbolInformation` for a function `name` at `line`, `character`.
fn info(name: &str, container: Option<&str>, path: &Path, line: u32, character: u32) -> Value {
    let end = character + name.len() as u32;
    json!({
        "name": name, "kind": 12, "containerName": container,
        "location": { "uri": uri(path), "range": range(line, character, line, end) },
    })
}

/// The queries of the `workspace/symbol` requests the server got.
fn symbol_queries(fake: &FakeServer) -> Vec<String> {
    support::lsp::sent(fake, WORKSPACE_SYMBOL)
        .into_iter()
        .map(|params| params["query"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// `src/lib.rs`: `helper` on row 1, after a comment with a two-byte
/// char, so its UTF-16 column (11) is not its byte column (12).
const LIB_RS: &str = "// lib\n/* é */ fn helper() {}\n";

#[gpui::test]
fn hash_debounces_and_cancels(cx: &mut TestAppContext) {
    let fake = symbols_server().hang_on(WORKSPACE_SYMBOL);
    let (_dir, wb, _view, cx) = setup(&fake, pt(0, 0), cx);
    run(&wb, "workbench.action.showAllSymbols", cx);
    let qo = quick_open(&wb, cx).expect("quick open is shown");
    assert_eq!(cx.update(|_, cx| qo.read(cx).query(cx)), "#");

    type_slowly("Foo", Duration::from_millis(30), cx);
    assert!(symbol_queries(&fake).is_empty(), "still debouncing");
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    assert_eq!(symbol_queries(&fake), vec!["Foo"], "one request");
    let first = fake.hung_ids(WORKSPACE_SYMBOL)[0].clone();

    cx.simulate_input("x");
    cx.run_until_parked();
    let cancels = support::lsp::sent(&fake, "$/cancelRequest");
    assert!(
        cancels.contains(&json!({ "id": first })),
        "the pending request is cancelled: {cancels:?}"
    );
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    assert_eq!(symbol_queries(&fake), vec!["Foo", "Foox"]);
}

#[gpui::test]
fn project_results_first_max_100(cx: &mut TestAppContext) {
    let outside = TempDir::new().unwrap();
    let ext = outside.path().join("ext.rs");
    fs::write(&ext, "fn x() {}\n".repeat(200)).unwrap();
    let root = rust_project();
    fs::write(root.path().join(".gitignore"), "target/\n").unwrap();
    fs::create_dir(root.path().join("target")).unwrap();
    let gen_rs = root.path().join("target/gen.rs");
    fs::write(&gen_rs, "fn foo() {}\n").unwrap();
    let main = root.path().join("src/main.rs");

    // The project's symbols match worst; the server sends everything.
    let mut answer = vec![
        info("fxoxo", Some("Foo"), &main, 4, 7),
        info("fyoyo", None, &main, 90, 3),
        info("foo", None, &gen_rs, 0, 3),
        info("bar", None, &ext, 0, 3),
    ];
    for i in 0..120 {
        answer.push(info(&format!("foo_ext_{i}"), None, &ext, i, 3));
    }
    let answer = Value::Array(answer);
    let fake = symbols_server().on_request(WORKSPACE_SYMBOL, move |_| Ok(answer.clone()));
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let (wb, cx) = lsp_workbench_with(root.path(), launcher, TRUSTED, cx);
    wait_for_scan(&wb, cx);
    open(&wb, "src/main.rs", cx);
    settle_symbols(cx);

    run(&wb, "workbench.action.showAllSymbols", cx);
    type_text("foo", cx);
    let rows = rows(&wb, cx);
    assert_eq!(rows.len(), 100, "at most 100");
    assert_eq!(rows[0], "fxoxo Foo src/main.rs:5", "{rows:?}");
    assert_eq!(rows[1], "fyoyo src/main.rs:91");
    assert_eq!(rows[2], "foo target/gen.rs:1", "ignored files are external");
    let external = format!("{}:", ext.display());
    assert!(rows[3..].iter().all(|r| r.contains(&external)), "{rows:?}");
    assert!(!rows.iter().any(|r| r.starts_with("bar")), "filtered");
}

/// [`setup`] (cursor on row 91) with `src/lib.rs` ([`LIB_RS`]) added.
fn lib_project<'a>(
    fake: &FakeServer,
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<CodeView>,
    &'a mut VisualTestContext,
) {
    let (dir, wb, view, cx) = setup(fake, pt(91, 4), cx);
    fs::write(dir.path().join("src/lib.rs"), LIB_RS).unwrap();
    (dir, wb, view, cx)
}

fn active_path(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> PathBuf {
    let view = support::lsp::active_view(wb, cx);
    cx.update(|_, cx| view.read(cx).path().to_path_buf())
}

#[gpui::test]
fn enter_opens_symbol(cx: &mut TestAppContext) {
    let lib = Arc::new(std::sync::Mutex::new(PathBuf::new()));
    let lib_for_server = lib.clone();
    let fake = symbols_server().on_request(WORKSPACE_SYMBOL, move |_| {
        let lib = lib_for_server.lock().unwrap().clone();
        Ok(json!([info("helper", Some("lib"), &lib, 1, 11)]))
    });
    let (dir, wb, _view, cx) = lib_project(&fake, cx);
    *lib.lock().unwrap() = dir.path().join("src/lib.rs");

    run(&wb, "workbench.action.showAllSymbols", cx);
    type_text("help", cx);
    assert_eq!(rows(&wb, cx), vec!["helper lib src/lib.rs:2"]);
    keys("down", cx);
    assert!(
        active_path(&wb, cx).ends_with("src/main.rs"),
        "no preview while moving"
    );
    keys("enter", cx);
    assert!(quick_open(&wb, cx).is_none());
    settle(cx);
    assert!(active_path(&wb, cx).ends_with("src/lib.rs"));
    let view = support::lsp::active_view(&wb, cx);
    assert_eq!(place(&view, cx).0, pt(1, 12), "UTF-16 column 11");

    run(&wb, "workbench.action.navigateBack", cx);
    assert!(active_path(&wb, cx).ends_with("src/main.rs"), "history");

    // ⌘Enter opens it in the group to the right.
    run(&wb, "workbench.action.showAllSymbols", cx);
    type_text("help", cx);
    keys("cmd-enter", cx);
    settle(cx);
    let groups = cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).model().groups().len());
    assert_eq!(groups, 2);
    assert!(active_path(&wb, cx).ends_with("src/lib.rs"));
    let view = support::lsp::active_view(&wb, cx);
    assert_eq!(place(&view, cx).0, pt(1, 12));
}

#[gpui::test]
fn uri_only_symbol_resolves_on_confirm(cx: &mut TestAppContext) {
    use lsp_types::{
        OneOf, PositionEncodingKind, ServerCapabilities, TextDocumentSyncCapability,
        TextDocumentSyncKind, WorkspaceSymbolOptions,
    };
    let lib = Arc::new(std::sync::Mutex::new(PathBuf::new()));
    let (lib_a, lib_b) = (lib.clone(), lib.clone());
    let fake = FakeServer::new()
        .capabilities(ServerCapabilities {
            position_encoding: Some(PositionEncodingKind::UTF16),
            text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
            workspace_symbol_provider: Some(OneOf::Right(WorkspaceSymbolOptions {
                resolve_provider: Some(true),
                work_done_progress_options: Default::default(),
            })),
            ..Default::default()
        })
        .on_request(WORKSPACE_SYMBOL, move |_| {
            let lib = lib_a.lock().unwrap().clone();
            Ok(json!([{ "name": "helper", "kind": 12,
                        "location": { "uri": uri(&lib) }, "data": 7 }]))
        })
        .on_request(RESOLVE_SYMBOL, move |params| {
            let lib = lib_b.lock().unwrap().clone();
            assert_eq!(params["data"], json!(7), "the symbol is sent back");
            Ok(info("helper", None, &lib, 1, 11))
        });
    let (dir, wb, _view, cx) = lib_project(&fake, cx);
    *lib.lock().unwrap() = dir.path().join("src/lib.rs");

    run(&wb, "workbench.action.showAllSymbols", cx);
    type_text("helper", cx);
    assert_eq!(rows(&wb, cx), vec!["helper src/lib.rs"], "no line yet");
    keys("enter", cx);
    settle(cx);
    assert_eq!(count(&fake, RESOLVE_SYMBOL), 1);
    assert!(active_path(&wb, cx).ends_with("src/lib.rs"));
    let view = support::lsp::active_view(&wb, cx);
    assert_eq!(place(&view, cx).0, pt(1, 12));
}

#[gpui::test]
fn no_server_message(cx: &mut TestAppContext) {
    let dir = rust_project();
    let launcher = Arc::new(FakeLauncher::new());
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher, TRUSTED, cx);
    open(&wb, "src/main.rs", cx);
    run(&wb, "workbench.action.showAllSymbols", cx);
    type_text("Foo", cx);
    assert_eq!(rows(&wb, cx), vec!["No language server is running"]);
}

#[gpui::test]
fn empty_query_message(cx: &mut TestAppContext) {
    let fake = symbols_server().on_request(WORKSPACE_SYMBOL, |_| Ok(json!([])));
    let (_dir, wb, _view, cx) = setup(&fake, pt(0, 0), cx);
    run(&wb, "workbench.action.showAllSymbols", cx);
    assert_eq!(rows(&wb, cx), vec!["Type to search symbols"]);
    type_text("zzz", cx);
    assert_eq!(rows(&wb, cx), vec!["No symbols found"]);
    keys("backspace backspace backspace", cx);
    assert_eq!(rows(&wb, cx), vec!["Type to search symbols"]);
    assert_eq!(
        symbol_queries(&fake),
        vec!["zzz"],
        "empty queries ask nothing"
    );
}

#[gpui::test]
fn at_enter_keeps_editor_text_focus(cx: &mut TestAppContext) {
    let fake = symbols_server();
    let (_dir, wb, view, cx) = setup(&fake, pt(91, 4), cx);
    focus_view(&view, cx);
    run(&wb, "workbench.action.gotoSymbol", cx);
    type_text("get", cx);
    keys("enter", cx);
    assert!(quick_open(&wb, cx).is_none());
    assert!(editor_text_focus(&wb, cx));
}

#[gpui::test]
fn hash_enter_keeps_editor_text_focus(cx: &mut TestAppContext) {
    let lib = Arc::new(std::sync::Mutex::new(PathBuf::new()));
    let lib_for_server = lib.clone();
    let fake = symbols_server().on_request(WORKSPACE_SYMBOL, move |_| {
        let lib = lib_for_server.lock().unwrap().clone();
        Ok(json!([info("helper", Some("lib"), &lib, 1, 11)]))
    });
    let (dir, wb, view, cx) = lib_project(&fake, cx);
    *lib.lock().unwrap() = dir.path().join("src/lib.rs");
    focus_view(&view, cx);
    run(&wb, "workbench.action.showAllSymbols", cx);
    type_text("help", cx);
    keys("enter", cx);
    settle(cx);
    assert!(active_path(&wb, cx).ends_with("src/lib.rs"));
    assert!(editor_text_focus(&wb, cx));
}
