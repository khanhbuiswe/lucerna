//! The document pipeline driven through GPUI's test platform: background
//! loads, live reloads from synthetic worktree events, the deleted and
//! error states, and the status bar.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::{LoadError, Point};
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, DocState, DocumentEvent, Documents, OpenMode, Workbench};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
        languages_generation: 0,
        icon_themes: lucerna_theme::IconThemeRegistry::build(&[]).0,
        icons: lucerna_theme::IconThemeRegistry::build(&[])
            .0
            .resolve("Lucide"),
    });
}

fn p(name: &str) -> Arc<Path> {
    Arc::from(Path::new(name))
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// Draws, then lets background loads and parses finish and draws again.
fn settle(cx: &mut VisualTestContext) {
    draw(cx);
    cx.run_until_parked();
    draw(cx);
}

fn workbench<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let root = root.to_path_buf();
    cx.add_window_view(|window, cx| Workbench::new(root, window, cx))
}

fn documents(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<Documents> {
    cx.update(|_, cx| wb.read(cx).documents().clone())
}

fn state(wb: &Entity<Workbench>, path: &Path, cx: &mut VisualTestContext) -> Option<DocState> {
    let docs = documents(wb, cx);
    cx.update(|_, cx| docs.read(cx).state(path).cloned())
}

fn active_view(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<CodeView> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("the active tab has a body")
            .downcast::<CodeView>()
            .expect("tab bodies are code views")
    })
}

/// Opens `path` as a pinned tab and lets it load.
fn open(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) -> Entity<CodeView> {
    wb.update(cx, |wb, cx| wb.open_path(p(path), OpenMode::Pinned, cx));
    settle(cx);
    active_view(wb, cx)
}

fn text(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, _| {
        let s = v.snapshot();
        s.text_for_range(0..s.len_bytes())
    })
}

fn event(wb: &Entity<Workbench>, ev: WorktreeEvent, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| wb.on_worktree_event(&ev, cx));
    settle(cx);
}

fn changed(wb: &Entity<Workbench>, path: &str, kind: ChangeKind, cx: &mut VisualTestContext) {
    let change = Change {
        path: p(path),
        kind,
    };
    event(wb, WorktreeEvent::Changed(vec![change]), cx);
}

fn lines(n: usize) -> String {
    (0..n).map(|i| format!("line {i}\n")).collect()
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    settle(cx);
}

fn pt(row: u32, column: u32) -> Point {
    Point { row, column }
}

#[gpui::test]
fn opens_and_highlights(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    let source = "fn main() {\n    let x = 1;\n}\n";
    fs::write(dir.path().join("src/main.rs"), source).unwrap();
    let (wb, cx) = workbench(dir.path(), cx);

    wb.update(cx, |wb, cx| {
        wb.open_path(p("src/main.rs"), OpenMode::Pinned, cx)
    });
    // The tab body is made, and its load started, when it is first drawn.
    draw(cx);
    assert_eq!(
        state(&wb, Path::new("src/main.rs"), cx),
        Some(DocState::Loading)
    );
    settle(cx);

    // Root-relative and absolute paths name the same document.
    let abs = dir.path().join("src/main.rs");
    assert_eq!(state(&wb, &abs, cx), Some(DocState::Ready));
    assert_eq!(
        state(&wb, Path::new("src/main.rs"), cx),
        Some(DocState::Ready)
    );
    let view = active_view(&wb, cx);
    assert_eq!(text(&view, cx), source);
    view.read_with(cx, |v, _| {
        let syntax = v.syntax().expect("a Rust file gets syntax");
        assert!(syntax.tree().is_some(), "the tree is parsed");
        let spans = syntax.highlights(v.snapshot(), 0..3, &|_| true);
        assert!(
            spans.iter().any(|s| s.capture.starts_with("keyword")),
            "`fn` is highlighted: {spans:?}"
        );
    });
}

#[gpui::test]
fn inactive_tabs_load_when_activated(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "alpha\n").unwrap();
    fs::write(dir.path().join("b.txt"), "beta\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);

    // Both tabs open before the first draw, as restored tabs are.
    wb.update(cx, |wb, cx| {
        wb.open_path(p("a.txt"), OpenMode::Pinned, cx);
        wb.open_path(p("b.txt"), OpenMode::Pinned, cx);
    });
    settle(cx);
    assert_eq!(state(&wb, Path::new("b.txt"), cx), Some(DocState::Ready));
    assert_eq!(state(&wb, Path::new("a.txt"), cx), None, "not loaded yet");

    let a = cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).model().groups()[0].tabs[0].id);
    wb.update(cx, |wb, cx| wb.update_editor_groups(cx, |m| m.activate(a)));
    settle(cx);
    assert_eq!(state(&wb, Path::new("a.txt"), cx), Some(DocState::Ready));
    let view = active_view(&wb, cx);
    assert_eq!(text(&view, cx), "alpha\n");
}

#[gpui::test]
fn modify_reload_keeps_scroll_when_change_below(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("f.txt");
    fs::write(&file, lines(200)).unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "f.txt", cx);
    view.update(cx, |v, cx| v.set_scroll_top_row(50, cx));
    // A selection away from the change stays on its text.
    view.update(cx, |v, cx| v.select(pt(60, 0)..pt(61, 2), cx));

    let new = lines(200).replace("line 150\n", "changed 150\n");
    fs::write(&file, &new).unwrap();
    changed(&wb, "f.txt", ChangeKind::Modified, cx);
    assert_eq!(text(&view, cx), new);
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 50);
    assert_eq!(
        view.read_with(cx, |v, _| v.selection()),
        Some(pt(60, 0)..pt(61, 2))
    );

    view.update(cx, |v, cx| v.select(pt(149, 0)..pt(151, 1), cx));
    fs::write(&file, new.replace("changed 150\n", "again 150\n")).unwrap();
    changed(&wb, "f.txt", ChangeKind::Modified, cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.selection()),
        Some(pt(149, 0)..pt(151, 1)),
        "a selection over changed rows follows the text around it"
    );
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 50);
}

#[gpui::test]
fn insert_above_shifts_scroll(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("f.rs");
    fs::write(&file, lines(200)).unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "f.rs", cx);
    view.update(cx, |v, cx| v.set_scroll_top_row(50, cx));

    let new = format!("{}{}", "added\n".repeat(5), lines(200));
    fs::write(&file, &new).unwrap();
    changed(&wb, "f.rs", ChangeKind::Modified, cx);
    assert_eq!(text(&view, cx), new);
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 55);
    assert!(
        view.read_with(cx, |v, _| v.syntax().is_some()),
        "the reloaded file is parsed again"
    );
}

#[gpui::test]
fn truncate_to_empty_clamps_scroll_no_panic(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("f.txt");
    fs::write(&file, lines(300)).unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "f.txt", cx);
    view.update(cx, |v, cx| v.set_scroll_top_row(250, cx));

    fs::write(&file, "").unwrap();
    changed(&wb, "f.txt", ChangeKind::Modified, cx);
    assert_eq!(text(&view, cx), "");
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 0);
    assert_eq!(state(&wb, Path::new("f.txt"), cx), Some(DocState::Ready));
}

#[gpui::test]
fn removed_marks_deleted_and_keeps_content(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("gone.txt");
    fs::write(&file, "last words\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "gone.txt", cx);
    assert!(cx.debug_bounds("tab-deleted-0-0").is_none());

    fs::remove_file(&file).unwrap();
    changed(&wb, "gone.txt", ChangeKind::Removed, cx);
    assert_eq!(state(&wb, &file, cx), Some(DocState::Deleted));
    assert_eq!(text(&view, cx), "last words\n");
    assert!(
        cx.debug_bounds("tab-deleted-0-0").is_some(),
        "the tab title is struck through"
    );
    assert!(
        cx.debug_bounds("doc-error").is_none(),
        "content stays visible"
    );
}

#[gpui::test]
fn readded_returns_ready(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("back.txt");
    fs::write(&file, "first\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "back.txt", cx);
    fs::remove_file(&file).unwrap();
    changed(&wb, "back.txt", ChangeKind::Removed, cx);
    assert_eq!(state(&wb, &file, cx), Some(DocState::Deleted));

    fs::write(&file, "second\n").unwrap();
    changed(&wb, "back.txt", ChangeKind::Added, cx);
    assert_eq!(state(&wb, &file, cx), Some(DocState::Ready));
    assert_eq!(text(&view, cx), "second\n");
    assert!(cx.debug_bounds("tab-deleted-0-0").is_none());
}

#[gpui::test]
fn binary_shows_open_as_text_then_opens(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("blob.bin");
    fs::write(&file, b"head\0tail\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    open(&wb, "blob.bin", cx);
    assert_eq!(
        state(&wb, &file, cx),
        Some(DocState::Error(LoadError::Binary))
    );
    assert!(cx.debug_bounds("doc-error").is_some());
    assert!(cx.debug_bounds("doc-retry").is_none());

    click("doc-open-as-text", cx);
    assert_eq!(state(&wb, &file, cx), Some(DocState::Ready));
    let view = active_view(&wb, cx);
    assert_eq!(text(&view, cx), "head\0tail\n");
    assert!(cx.debug_bounds("doc-error").is_none());
}

#[gpui::test]
fn permission_denied_retry(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("secret.txt");
    fs::write(&file, "classified\n").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    open(&wb, "secret.txt", cx);
    assert_eq!(
        state(&wb, &file, cx),
        Some(DocState::Error(LoadError::PermissionDenied))
    );
    assert!(cx.debug_bounds("doc-open-as-text").is_none());

    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    click("doc-retry", cx);
    assert_eq!(state(&wb, &file, cx), Some(DocState::Ready));
    let view = active_view(&wb, cx);
    assert_eq!(text(&view, cx), "classified\n");
}

#[gpui::test]
fn rescan_reloads_changed_open_files(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let (a, b, c) = (
        dir.path().join("a.txt"),
        dir.path().join("b.txt"),
        dir.path().join("c.txt"),
    );
    fs::write(&a, "a1\n").unwrap();
    fs::write(&b, "b1\n").unwrap();
    fs::write(&c, "c1\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view_a = open(&wb, "a.txt", cx);
    open(&wb, "b.txt", cx);
    let view_c = open(&wb, "c.txt", cx);

    // `a` changes, `b` goes away, `c` is rewritten with the same stamp.
    fs::write(&a, "a2\n").unwrap();
    let later = SystemTime::now() + Duration::from_secs(10);
    fs::File::options()
        .write(true)
        .open(&a)
        .unwrap()
        .set_modified(later)
        .unwrap();
    fs::remove_file(&b).unwrap();
    event(&wb, WorktreeEvent::Rescanned, cx);

    assert_eq!(text(&view_a, cx), "a2\n");
    assert_eq!(state(&wb, &a, cx), Some(DocState::Ready));
    assert_eq!(state(&wb, &b, cx), Some(DocState::Deleted));
    assert_eq!(text(&view_c, cx), "c1\n");
}

#[gpui::test]
fn same_path_in_two_groups_shares_one_document(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("f.txt");
    fs::write(&file, lines(200)).unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let left = open(&wb, "f.txt", cx);
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| m.split_active())
    });
    settle(cx);
    let right = active_view(&wb, cx);
    assert_ne!(
        left.entity_id(),
        right.entity_id(),
        "each tab has its own view"
    );
    assert_eq!(text(&right, cx), lines(200));

    left.update(cx, |v, cx| v.set_scroll_top_row(10, cx));
    right.update(cx, |v, cx| v.set_scroll_top_row(100, cx));
    let new = format!("top\n{}", lines(200));
    fs::write(&file, &new).unwrap();
    changed(&wb, "f.txt", ChangeKind::Modified, cx);
    assert_eq!(text(&left, cx), new);
    assert_eq!(text(&right, cx), new);
    assert_eq!(left.read_with(cx, |v, _| v.scroll_top_row()), 11);
    assert_eq!(right.read_with(cx, |v, _| v.scroll_top_row()), 101);
}

#[gpui::test]
fn reload_storm_ends_on_latest_text(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("f.txt");
    fs::write(&file, "v0\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "f.txt", cx);

    // Events arrive faster than reloads finish.
    for i in 1..=5 {
        fs::write(&file, format!("v{i}\n")).unwrap();
        let change = Change {
            path: p("f.txt"),
            kind: ChangeKind::Modified,
        };
        wb.update(cx, |wb, cx| {
            wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
        });
    }
    settle(cx);
    assert_eq!(text(&view, cx), "v5\n");
    assert_eq!(state(&wb, &file, cx), Some(DocState::Ready));
}

#[gpui::test]
fn status_shows_cursor_encoding_eol_and_language(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("m.rs"), "fn a() {}\r\n// héllo\r\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "m.rs", cx);
    // Byte 6 of `// héllo` follows `é`, the fifth char: column 6.
    view.update(cx, |v, cx| v.go_to(pt(1, 6), cx));
    settle(cx);
    let docs = documents(&wb, cx);
    let status = cx.update(|_, cx| docs.read(cx).status(view.read(cx)));
    let status = status.expect("a loaded document has a status");
    assert_eq!(status.cursor, "Ln 2, Col 6");
    assert_eq!(status.encoding, "UTF-8");
    assert_eq!(status.line_ending, "CRLF");
    assert_eq!(&*status.language, "Rust");
    assert!(cx.debug_bounds("status-cursor").is_some());
}

#[gpui::test]
fn open_path_at_jumps_once_loaded(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("f.txt"), lines(500)).unwrap();
    let (wb, cx) = workbench(dir.path(), cx);

    wb.update(cx, |wb, cx| {
        wb.open_path_at(p("f.txt"), OpenMode::Preview, pt(300, 2), cx)
    });
    settle(cx);
    let view = active_view(&wb, cx);
    assert_eq!(view.read_with(cx, |v, _| v.cursor()), pt(300, 2));

    // An already-open document jumps at once.
    wb.update(cx, |wb, cx| {
        wb.open_path_at(p("f.txt"), OpenMode::Preview, pt(10, 0), cx)
    });
    settle(cx);
    assert_eq!(view.read_with(cx, |v, _| v.cursor()), pt(10, 0));
}

#[gpui::test]
fn absolute_tab_path_and_relative_change_meet(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("abs.txt");
    fs::write(&file, "one\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let abs: PathBuf = file.clone();
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(abs.as_path()), OpenMode::Pinned, cx)
    });
    settle(cx);
    let view = active_view(&wb, cx);
    fs::write(&file, "two\n").unwrap();
    changed(&wb, "abs.txt", ChangeKind::Modified, cx);
    assert_eq!(text(&view, cx), "two\n");
}

#[gpui::test]
fn slow_parse_finishes_in_background(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    // Far more than the 1 ms foreground budget can parse.
    let source: String = (0..20_000)
        .map(|i| format!("fn f{i}() {{ let x = {i}; }}\n"))
        .collect();
    fs::write(dir.path().join("big.rs"), &source).unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "big.rs", cx);
    view.read_with(cx, |v, _| {
        assert!(!v.is_large());
        let syntax = v.syntax().expect("the background parse delivered syntax");
        let spans = syntax.highlights(v.snapshot(), 19_990..20_000, &|_| true);
        assert!(!spans.is_empty(), "the end of the file is highlighted");
    });
}

/// The status reads the column from the rope; a 1 MB line is not copied
/// per frame (no timing asserted, only the column).
#[gpui::test]
fn status_column_on_a_one_megabyte_line(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let line = format!("{}x\n", "é".repeat(500_000));
    fs::write(dir.path().join("wide.txt"), line).unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "wide.txt", cx);
    view.update(cx, |v, cx| v.go_to(pt(0, 1_000_000), cx));
    settle(cx);
    let docs = documents(&wb, cx);
    let status = cx.update(|_, cx| docs.read(cx).status(view.read(cx)));
    assert_eq!(status.expect("loaded").cursor, "Ln 1, Col 500001");
}

/// While a reloaded file is parsed in the background, the old tree
/// (edited to the new text) keeps the colours on screen.
#[gpui::test]
fn reload_keeps_highlights_until_the_background_parse_lands(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    // Far more than the 1 ms foreground budget can parse.
    let source: String = (0..20_000)
        .map(|i| format!("fn f{i}() {{ let x = {i}; }}\n"))
        .collect();
    fs::write(dir.path().join("big.rs"), &source).unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "big.rs", cx);
    let before = view.read_with(cx, |v, _| v.revision());
    let parsed = view
        .read_with(cx, |v, _| v.syntax().cloned())
        .expect("parsed");

    fs::write(dir.path().join("big.rs"), format!("// edited\n{source}")).unwrap();
    wb.update(cx, |wb, cx| {
        let change = Change {
            path: p("big.rs"),
            kind: ChangeKind::Modified,
        };
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    // Step until the reload reaches the view, not further: the parse of
    // the new text is still queued.
    while view.read_with(cx, |v, _| v.revision()) == before {
        assert!(cx.executor().tick(), "the reload finishes");
    }
    view.read_with(cx, |v, _| {
        let syntax = v
            .syntax()
            .expect("the old syntax stays until the parse lands");
        assert!(!Arc::ptr_eq(syntax, &parsed), "edited to the new text");
        let spans = syntax.highlights(v.snapshot(), 100..110, &|_| true);
        assert!(!spans.is_empty(), "rows keep their colours");
    });
    settle(cx);
    view.read_with(cx, |v, _| {
        let syntax = v.syntax().expect("the new parse landed");
        assert!(!syntax.highlights(v.snapshot(), 0..1, &|_| true).is_empty());
    });
}

/// `touch` or a rewrite with the same text changes nothing on screen.
#[gpui::test]
fn same_content_rewrite_does_not_bump_the_revision(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("f.rs"), "fn a() {}\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "f.rs", cx);
    let before = view.read_with(cx, |v, _| (v.revision(), v.syntax().cloned()));
    fs::write(dir.path().join("f.rs"), "fn a() {}\n").unwrap();
    changed(&wb, "f.rs", ChangeKind::Modified, cx);
    let after = view.read_with(cx, |v, _| (v.revision(), v.syntax().cloned()));
    assert_eq!(after.0, before.0, "the revision stays");
    assert!(
        Arc::ptr_eq(after.1.as_ref().unwrap(), before.1.as_ref().unwrap()),
        "and so does the syntax"
    );
    assert_eq!(state(&wb, Path::new("f.rs"), cx), Some(DocState::Ready));
}

#[gpui::test]
fn failed_grammar_shows_plain_text_not_loading(cx: &mut TestAppContext) {
    init(cx);
    let broken = lucerna_syntax::Language::from_wasm_for_test_with(
        "broken",
        Path::new("/nonexistent/broken.wasm"),
        "(x) @x",
        false,
        &["zzz"],
    );
    cx.update(|cx| {
        let state = cx.global_mut::<AppState>();
        state.languages = Arc::new(LanguageRegistry::bundled_plus_for_test(broken));
    });
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.zzz"), "hello\nworld\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "a.zzz", cx);
    assert_eq!(
        state(&wb, &dir.path().join("a.zzz"), cx),
        Some(DocState::Ready)
    );
    assert_eq!(text(&view, cx), "hello\nworld\n");
    view.read_with(cx, |v, _| assert!(v.syntax().is_none()));
}

fn edit_at(
    wb: &Entity<Workbench>,
    view: &Entity<CodeView>,
    path: &str,
    range: std::ops::Range<usize>,
    text: &str,
    cx: &mut VisualTestContext,
) {
    let docs = documents(wb, cx);
    let origin = view.entity_id();
    let edit = (range, Arc::<str>::from(text));
    docs.update(cx, |d, cx| {
        d.apply_edits(Path::new(path), vec![edit], None, None, origin, cx)
    })
    .expect("the document is loaded");
    settle(cx);
}

fn is_dirty(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) -> bool {
    let docs = documents(wb, cx);
    cx.update(|_, cx| docs.read(cx).is_dirty(Path::new(path)))
}

fn has_conflict(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) -> bool {
    let docs = documents(wb, cx);
    cx.update(|_, cx| docs.read(cx).has_conflict(Path::new(path)))
}

/// Every event `Documents` emits from now on.
fn record_events(
    wb: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> (Rc<RefCell<Vec<DocumentEvent>>>, gpui::Subscription) {
    let docs = documents(wb, cx);
    let events = Rc::new(RefCell::new(Vec::new()));
    let sub = cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&docs, move |_, ev: &DocumentEvent, _| {
            events.borrow_mut().push(ev.clone())
        })
    });
    (events, sub)
}

#[gpui::test]
fn clean_reload_is_undoable(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("f.txt");
    fs::write(&file, "a\nb\nc\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "f.txt", cx);

    fs::write(&file, "a\nB\nc\nd\n").unwrap();
    changed(&wb, "f.txt", ChangeKind::Modified, cx);
    assert_eq!(text(&view, cx), "a\nB\nc\nd\n");
    assert!(!is_dirty(&wb, "f.txt", cx), "a reload leaves the doc clean");

    let docs = documents(&wb, cx);
    let undone = docs.update(cx, |d, cx| d.undo(Path::new("f.txt"), cx));
    assert!(undone.is_some(), "the reload is one undo step");
    settle(cx);
    assert_eq!(text(&view, cx), "a\nb\nc\n");
    assert!(
        is_dirty(&wb, "f.txt", cx),
        "the text no longer matches disk"
    );
    let redone = docs.update(cx, |d, cx| d.redo(Path::new("f.txt"), cx));
    assert!(redone.is_some());
    settle(cx);
    assert_eq!(text(&view, cx), "a\nB\nc\nd\n");
    assert!(!is_dirty(&wb, "f.txt", cx));
}

#[gpui::test]
fn reload_keeps_cursor_on_same_line(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("f.txt");
    fs::write(&file, lines(20)).unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "f.txt", cx);
    view.update(cx, |v, cx| v.set_cursor(pt(10, 3), cx));

    let mut new: Vec<String> = (0..20).map(|i| format!("line {i}\n")).collect();
    new.insert(2, "inserted\n".into());
    fs::write(&file, new.concat()).unwrap();
    changed(&wb, "f.txt", ChangeKind::Modified, cx);
    assert_eq!(text(&view, cx), new.concat());
    assert_eq!(view.read_with(cx, |v, _| v.cursor()), pt(11, 3));
}

#[gpui::test]
fn dirty_doc_is_not_reloaded_and_conflicts(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("f.txt");
    fs::write(&file, "one\ntwo\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "f.txt", cx);
    edit_at(&wb, &view, "f.txt", 0..0, "x", cx);
    assert!(is_dirty(&wb, "f.txt", cx));
    let (events, _sub) = record_events(&wb, cx);

    fs::write(&file, "one\nTWO\n").unwrap();
    changed(&wb, "f.txt", ChangeKind::Modified, cx);
    assert_eq!(text(&view, cx), "xone\ntwo\n", "the buffer is left alone");
    assert!(has_conflict(&wb, "f.txt", cx));
    let conflicts: Vec<bool> = events
        .borrow()
        .iter()
        .filter_map(|ev| match ev {
            DocumentEvent::ConflictChanged { conflict, .. } => Some(*conflict),
            _ => None,
        })
        .collect();
    assert_eq!(conflicts, [true]);
    assert!(
        !events
            .borrow()
            .iter()
            .any(|ev| matches!(ev, DocumentEvent::Edited { .. })),
        "nothing was edited"
    );
}

#[gpui::test]
fn same_bytes_event_is_ignored(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("f.txt");
    fs::write(&file, "same\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "f.txt", cx);
    let (events, _sub) = record_events(&wb, cx);

    fs::write(&file, "same\n").unwrap();
    changed(&wb, "f.txt", ChangeKind::Modified, cx);
    assert_eq!(text(&view, cx), "same\n");
    assert!(
        !events
            .borrow()
            .iter()
            .any(|ev| matches!(ev, DocumentEvent::Edited { .. })),
        "{:?}",
        events.borrow()
    );
    assert!(!has_conflict(&wb, "f.txt", cx));
}

#[gpui::test]
fn reload_racing_an_edit_keeps_the_edit(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("f.txt");
    fs::write(&file, "one\ntwo\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "f.txt", cx);

    // The reload job starts but does not run yet.
    fs::write(&file, "one\nTWO\n").unwrap();
    let change = Change {
        path: p("f.txt"),
        kind: ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    let docs = documents(&wb, cx);
    let origin = view.entity_id();
    docs.update(cx, |d, cx| {
        d.apply_edits(
            Path::new("f.txt"),
            vec![(0..0, Arc::from("x"))],
            None,
            None,
            origin,
            cx,
        )
    })
    .expect("the document is loaded");
    settle(cx);

    assert_eq!(text(&view, cx), "xone\ntwo\n", "the user's edit survives");
    assert!(is_dirty(&wb, "f.txt", cx));
    assert!(has_conflict(&wb, "f.txt", cx));
}

#[gpui::test]
fn edit_emits_dirty_changed_once(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("f.txt"), "abc\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "f.txt", cx);
    let (events, _sub) = record_events(&wb, cx);

    edit_at(&wb, &view, "f.txt", 0..0, "x", cx);
    edit_at(&wb, &view, "f.txt", 1..1, "y", cx);
    assert_eq!(text(&view, cx), "xyabc\n");
    let dirty: Vec<bool> = events
        .borrow()
        .iter()
        .filter_map(|ev| match ev {
            DocumentEvent::DirtyChanged { dirty, .. } => Some(*dirty),
            _ => None,
        })
        .collect();
    assert_eq!(dirty, [true]);
    let edited = events
        .borrow()
        .iter()
        .filter(|ev| matches!(ev, DocumentEvent::Edited { origin: Some(o), .. } if *o == view.entity_id()))
        .count();
    assert_eq!(edited, 2);
}

/// A reload diffed against text that changed (and changed back to clean)
/// while it ran is diffed again, not applied to the wrong text.
#[gpui::test]
fn reload_racing_an_undone_edit_runs_again(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("f.txt");
    fs::write(&file, "one\ntwo\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "f.txt", cx);

    fs::write(&file, "one\nTWO\n").unwrap();
    let change = Change {
        path: p("f.txt"),
        kind: ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    let docs = documents(&wb, cx);
    let origin = view.entity_id();
    docs.update(cx, |d, cx| {
        d.apply_edits(
            Path::new("f.txt"),
            vec![(0..0, Arc::from("x"))],
            None,
            None,
            origin,
            cx,
        );
        d.undo(Path::new("f.txt"), cx)
    })
    .expect("the edit is undone");
    assert!(!is_dirty(&wb, "f.txt", cx));
    settle(cx);

    assert_eq!(text(&view, cx), "one\nTWO\n");
    assert!(!is_dirty(&wb, "f.txt", cx));
    assert!(!has_conflict(&wb, "f.txt", cx));
}

#[gpui::test]
fn rename_rekeys_dirty_document(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "a.rs", cx);
    edit_at(&wb, &view, "a.rs", 0..0, "// x\n", cx);
    assert!(is_dirty(&wb, "a.rs", cx));
    let (events, _sub) = record_events(&wb, cx);

    fs::rename(dir.path().join("a.rs"), dir.path().join("b.rs")).unwrap();
    let (from, to) = (dir.path().join("a.rs"), dir.path().join("b.rs"));
    let docs = documents(&wb, cx);
    docs.update(cx, |d, cx| d.rename(&from, &to, cx));
    settle(cx);

    assert!(is_dirty(&wb, "b.rs", cx));
    assert!(!is_dirty(&wb, "a.rs", cx));
    assert_eq!(state(&wb, Path::new("b.rs"), cx), Some(DocState::Ready));
    let views = cx.update(|_, cx| docs.read(cx).views_of(Path::new("b.rs")));
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].entity_id(), view.entity_id());
    assert_eq!(&*view.read_with(cx, |v, _| v.path().clone()), to.as_path());
    assert_eq!(text(&view, cx), "// x\nfn a() {}\n");

    let events = events.borrow().clone();
    let closed = events
        .iter()
        .position(|ev| matches!(ev, DocumentEvent::Closed { path } if **path == *from));
    let opened = events
        .iter()
        .position(|ev| matches!(ev, DocumentEvent::Opened { path, .. } if **path == *to));
    let (closed, opened) = (closed.expect("Closed{a.rs}"), opened.expect("Opened{b.rs}"));
    assert!(closed < opened, "{events:?}");

    // The undo history came along.
    let undone = docs.update(cx, |d, cx| d.undo(Path::new("b.rs"), cx));
    assert!(undone.is_some());
    settle(cx);
    assert_eq!(text(&view, cx), "fn a() {}\n");
}

/// The watcher reported the old path removed before the rename reached the
/// documents: the moved document reads its new path and is ready again,
/// unsaved changes kept.
#[gpui::test]
fn rename_after_removed_event_comes_back(cx: &mut TestAppContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/a.rs"), "fn a() {}\n").unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let view = open(&wb, "src/a.rs", cx);
    edit_at(&wb, &view, "src/a.rs", 0..0, "// x\n", cx);
    fs::rename(dir.path().join("src"), dir.path().join("lib")).unwrap();
    changed(&wb, "src/a.rs", ChangeKind::Removed, cx);
    assert_eq!(
        state(&wb, Path::new("src/a.rs"), cx),
        Some(DocState::Deleted)
    );

    let docs = documents(&wb, cx);
    let (from, to) = (dir.path().join("src"), dir.path().join("lib"));
    docs.update(cx, |d, cx| d.rename(&from, &to, cx));
    settle(cx);
    assert_eq!(state(&wb, Path::new("lib/a.rs"), cx), Some(DocState::Ready));
    assert!(is_dirty(&wb, "lib/a.rs", cx));
    assert_eq!(text(&view, cx), "// x\nfn a() {}\n");
}
