//! Replace in project: the Search tab's replace bar, per-line and per-file
//! replace and dismiss, driven through GPUI's test platform with real
//! keystrokes.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, Entity, Focusable, Modifiers, TestAppContext, VisualTestContext};
use lucerna_project::{
    Change, ChangeKind, Excerpt, LineKind, LineMatch, SearchQuery, WorktreeEvent,
};
use lucerna_settings::{Keymap, Layer, SettingsStore, parse_when};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, Documents, ExcerptList, ListSelection, OpenMode, ReplacePreview, SearchLineRef,
    SearchView, WindowSettings, Workbench,
};
use tempfile::TempDir;

const DEBOUNCE: Duration = Duration::from_millis(150);

fn init(cx: &mut TestAppContext) {
    init_with(cx, r#"{ "lucerna.lsp.enabled": false }"#);
}

fn init_with(cx: &mut TestAppContext, user_settings: &str) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
    // No language servers: the trust modal (opened by the first `.rs`
    // file) would take the focus these tests follow.
    let mut settings = SettingsStore::with_defaults();
    settings
        .set_layer(Layer::User, user_settings)
        .expect("valid user settings");
    cx.set_global(AppState {
        settings,
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

/// A project with the given `(path, text)` files.
fn project(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    dir
}

/// Waits for the worktree scan of `wb`, then delivers its completion.
fn finish_scan(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !wb.read_with(cx, |wb, _| wb.worktree().snapshot().scan_complete()) {
        assert!(Instant::now() < deadline, "worktree scan did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::ScanComplete, cx)
    });
    draw(cx);
}

/// Opens the workbench once its scan finished.
fn workbench<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let root = root.to_path_buf();
    let (wb, cx) = cx.add_window_view(move |window, cx| Workbench::new(root, window, cx));
    finish_scan(&wb, cx);
    (wb, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    draw(cx);
}

fn type_text(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    draw(cx);
}

/// Lets the debounce run out and the search finish.
fn settle_search(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(DEBOUNCE);
    draw(cx);
}

fn search(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<SearchView> {
    cx.update(|_, cx| wb.read(cx).search_view().clone())
}

fn needles() -> TempDir {
    project(&[
        ("b.rs", "needle one\nnothing\nneedle two needle\n"),
        ("a.rs", "a needle\n"),
        ("sub/c.rs", "NEEDLE\n"),
        // `search.exclude` has `**/target` on by default.
        ("target/x.rs", "needle\n"),
    ])
}

/// Searches "needle" in [`needles`] and returns the Search tab's view.
fn searched(
    cx: &mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<SearchView>,
    &mut VisualTestContext,
) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    let view = search(&wb, cx);
    type_text("needle", cx);
    settle_search(cx);
    (dir, wb, view, cx)
}

fn selection(view: &Entity<SearchView>, cx: &mut VisualTestContext) -> Option<ListSelection> {
    view.read_with(cx, |v, cx| v.list_selection(cx))
}

#[gpui::test]
fn arrows_stop_on_headers_in_search(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = searched(cx);
    keys("escape", cx);
    assert_eq!(selection(&view, cx), None);
    let line = |file, excerpt, line| {
        Some(ListSelection::Line(lucerna_workbench::SearchLineRef {
            file,
            excerpt,
            line,
        }))
    };
    keys("down", cx);
    assert_eq!(selection(&view, cx), Some(ListSelection::Header(0)));
    keys("down", cx);
    assert_eq!(selection(&view, cx), line(0, 0, 0));
    keys("down", cx);
    assert_eq!(selection(&view, cx), Some(ListSelection::Header(1)));
    keys("down", cx);
    assert_eq!(selection(&view, cx), line(1, 0, 0));
    keys("up", cx);
    assert_eq!(selection(&view, cx), Some(ListSelection::Header(1)));
    keys("up up", cx);
    assert_eq!(selection(&view, cx), Some(ListSelection::Header(0)));
    keys("up", cx);
    assert_eq!(
        selection(&view, cx),
        Some(ListSelection::Header(0)),
        "stops"
    );
}

#[gpui::test]
fn enter_on_a_header_toggles_its_fold(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = searched(cx);
    keys("escape down", cx);
    let rows = |cx: &mut VisualTestContext| view.read_with(cx, |v, cx| v.results(cx).rows().len());
    let before = rows(cx);
    keys("enter", cx);
    assert!(rows(cx) < before, "a.rs folded");
    keys("enter", cx);
    assert_eq!(rows(cx), before);
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    draw(cx);
}

fn holds(wb: &Entity<Workbench>, when: &str, cx: &mut VisualTestContext) -> bool {
    let ctx = cx.update(|window, cx| wb.read(cx).context_keys(window, cx));
    parse_when(when).unwrap().eval(&ctx)
}

fn query(view: &Entity<SearchView>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| view.read(cx).query_input().read(cx).text().to_owned())
}

fn query_focused(view: &Entity<SearchView>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        view.read(cx)
            .query_input()
            .focus_handle(cx)
            .is_focused(window)
    })
}

fn replace_focused(view: &Entity<SearchView>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        view.read(cx)
            .replace_input()
            .focus_handle(cx)
            .is_focused(window)
    })
}

fn row_shown(cx: &mut VisualTestContext) -> bool {
    cx.debug_bounds("search-replace-row").is_some()
}

fn active_view(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<CodeView> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("the active tab has a body")
            .downcast::<CodeView>()
            .expect("the active tab shows a file")
    })
}

fn open(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) -> Entity<CodeView> {
    let path: Arc<Path> = Arc::from(Path::new(path));
    wb.update(cx, |wb, cx| wb.open_path(path, OpenMode::Pinned, cx));
    draw(cx);
    active_view(wb, cx)
}

#[gpui::test]
fn cmd_shift_h_opens_replace_row(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.rs", "let foo = 1;\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-h", cx);
    let view = search(&wb, cx);
    assert!(row_shown(cx));
    assert!(view.read_with(cx, |v, _| v.replace_visible()));
    assert_eq!(query(&view, cx), "");
    assert!(query_focused(&view, cx), "an empty query takes the focus");
    assert!(holds(
        &wb,
        "searchViewletFocus && !replaceInputBoxFocus",
        cx
    ));

    // From a selection: the query is filled, the cursor goes to Replace.
    let code = open(&wb, "a.rs", cx);
    let at = |row, column| Point { row, column };
    code.update(cx, |v, cx| v.select(at(0, 4)..at(0, 7), cx));
    keys("cmd-shift-h", cx);
    assert_eq!(query(&view, cx), "foo");
    assert!(row_shown(cx));
    assert!(replace_focused(&view, cx));
    assert!(holds(&wb, "searchViewletFocus && replaceInputBoxFocus", cx));

    // Again in the Search tab: still shown, not toggled.
    keys("cmd-shift-h", cx);
    assert!(row_shown(cx));
    assert!(replace_focused(&view, cx));
}

#[gpui::test]
fn toggle_replace_button_and_escape(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = searched(cx);
    assert!(!row_shown(cx), "hidden by default");
    click("search-toggle-replace", cx);
    assert!(row_shown(cx));
    assert!(replace_focused(&view, cx));
    type_text("pin", cx);
    click("search-toggle-replace", cx);
    assert!(!row_shown(cx));
    assert!(query_focused(&view, cx));

    // The Replace text is kept while hidden.
    keys("cmd-shift-h", cx);
    assert!(row_shown(cx));
    assert_eq!(
        cx.update(|_, cx| view.read(cx).replace_input().read(cx).text().to_owned()),
        "pin"
    );
    assert!(holds(&wb, "replaceInputBoxFocus", cx));
    keys("escape", cx);
    assert!(!row_shown(cx));
    assert!(query_focused(&view, cx));
    assert!(!holds(&wb, "replaceInputBoxFocus", cx));
}

#[gpui::test]
fn tab_moves_between_query_and_replace(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-h", cx);
    let view = search(&wb, cx);
    assert!(query_focused(&view, cx));
    keys("tab", cx);
    assert!(replace_focused(&view, cx));
    keys("shift-tab", cx);
    assert!(query_focused(&view, cx));
}

#[gpui::test]
fn preview_strikes_old_and_shows_new(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.rs", "let user_id = 1;\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-h", cx);
    let view = search(&wb, cx);
    type_text(r"(\w+)_id", cx);
    click("search-toggle-regex", cx);
    settle_search(cx);
    assert_eq!(view.read_with(cx, |v, cx| v.results(cx).match_count()), 1);
    assert!(
        cx.debug_bounds("search-preview-0-0-0").is_none(),
        "no text yet"
    );
    click("search-toggle-replace", cx);
    click("search-toggle-replace", cx);
    assert!(replace_focused(&view, cx));
    assert!(view.read_with(cx, |v, cx| v.replace_active(cx)));
    type_text("$1Id", cx);
    assert!(cx.debug_bounds("search-preview-0-0-0").is_some());
    let replacements = view.read_with(cx, |v, cx| {
        let preview = v.replace_preview(cx).expect("a preview");
        let r = SearchLineRef {
            file: 0,
            excerpt: 0,
            line: 0,
        };
        preview.replacements(v.results(cx).line(r).unwrap())
    });
    assert_eq!(replacements, vec![Some("userId".to_string())]);

    // Preserve Case is applied by the preview too, without searching.
    let generation = view.read_with(cx, |v, _| v.search_generation());
    click("search-toggle-preserve-case", cx);
    assert!(view.read_with(cx, |v, _| v.preserve_case()));
    assert_eq!(view.read_with(cx, |v, _| v.search_generation()), generation);
    assert!(cx.debug_bounds("search-preview-0-0-0").is_some());

    // Hidden, the results show no preview.
    click("search-toggle-replace", cx);
    assert!(cx.debug_bounds("search-preview-0-0-0").is_none());
}

/// An empty project folder and a window showing a diff list over it.
fn diff_list(cx: &mut TestAppContext) -> (TempDir, Entity<ExcerptList>, &mut VisualTestContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let (list, cx) = cx.add_window_view(move |_, cx| {
        let settings = cx.new(|cx| WindowSettings::new(None, cx));
        ExcerptList::new(root, "diff", settings, cx)
    });
    (dir, list, cx)
}

#[gpui::test]
fn diff_view_has_no_preview(cx: &mut TestAppContext) {
    let (_dir, list, cx) = diff_list(cx);
    let line = LineMatch {
        line_number: 1,
        line_text: "let user_id = 1;".into(),
        ranges: std::iter::once(4..11).collect(),
        kind: LineKind::Added,
    };
    let files = vec![(
        Arc::from(Path::new("a.rs")),
        vec![Excerpt { lines: vec![line] }],
    )];
    list.update(cx, |l, cx| l.set_files(files, cx));
    draw(cx);
    assert!(cx.debug_bounds("diff-line-0-0-0").is_some());
    assert!(
        cx.debug_bounds("diff-preview-0-0-0").is_none(),
        "off by default"
    );

    // The option draws it when a host turns it on.
    let query = SearchQuery {
        pattern: r"(\w+)_id".into(),
        is_regex: true,
        ..SearchQuery::default()
    };
    let preview = ReplacePreview::new(&query, "$1Id", false).expect("a valid query");
    list.update(cx, |l, cx| {
        l.set_replace_preview(Some(Arc::new(preview)), cx)
    });
    draw(cx);
    assert!(cx.debug_bounds("diff-preview-0-0-0").is_some());
    list.update(cx, |l, cx| l.set_replace_preview(None, cx));
    draw(cx);
    assert!(cx.debug_bounds("diff-preview-0-0-0").is_none());
}

#[gpui::test]
fn typing_replace_does_not_search(cx: &mut TestAppContext) {
    let (_dir, _wb, view, cx) = searched(cx);
    keys("cmd-shift-h", cx);
    assert!(replace_focused(&view, cx), "the query has text");
    let generation = view.read_with(cx, |v, _| v.search_generation());
    type_text("pin", cx);
    settle_search(cx);
    assert_eq!(view.read_with(cx, |v, _| v.search_generation()), generation);
    assert!(cx.debug_bounds("search-preview-0-0-0").is_some());

    // Enter in Replace searches now.
    keys("enter", cx);
    assert_ne!(view.read_with(cx, |v, _| v.search_generation()), generation);
}

#[gpui::test]
fn alt_cmd_p_toggles_preserve_case_in_search_only(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = searched(cx);
    keys("cmd-shift-h", cx);
    keys("alt-cmd-p", cx);
    assert!(view.read_with(cx, |v, _| v.preserve_case()));
    keys("alt-cmd-p", cx);
    assert!(!view.read_with(cx, |v, _| v.preserve_case()));
    keys("alt-cmd-p", cx);

    // In a code view the find bar's toggle runs instead.
    let code = open(&wb, "a.rs", cx);
    keys("cmd-f", cx);
    assert!(holds(&wb, "editorFocus && !searchViewletFocus", cx));
    keys("alt-cmd-p", cx);
    let find = cx
        .update(|_, cx| code.read(cx).find_widget().cloned())
        .expect("the find bar");
    assert!(find.read_with(cx, |f, _| f.preserve_case()));
    assert!(view.read_with(cx, |v, _| v.preserve_case()), "unchanged");
}

// Replace All across files.

fn disk(dir: &TempDir, name: &str) -> String {
    fs::read_to_string(dir.path().join(name)).unwrap()
}

fn documents(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<Documents> {
    cx.update(|_, cx| wb.read(cx).documents().clone())
}

fn doc_text(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) -> String {
    let docs = documents(wb, cx);
    docs.read_with(cx, |d, _| {
        let s = d
            .snapshot_of(Path::new(name))
            .expect("the document is loaded");
        s.text_for_range(0..s.len_bytes())
    })
}

fn is_dirty(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) -> bool {
    let docs = documents(wb, cx);
    docs.read_with(cx, |d, _| d.is_dirty(Path::new(name)))
}

fn has_conflict(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) -> bool {
    let docs = documents(wb, cx);
    docs.read_with(cx, |d, _| d.has_conflict(Path::new(name)))
}

fn focus_view(view: &Entity<CodeView>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    draw(cx);
}

fn replace_message(view: &Entity<SearchView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| v.last_replace_message().map(str::to_owned))
}

/// Opens the Search tab with its Replace row, searches `query` and types
/// `value` in Replace.
fn replace_ready(
    wb: &Entity<Workbench>,
    query: &str,
    value: &str,
    cx: &mut VisualTestContext,
) -> Entity<SearchView> {
    keys("cmd-shift-h", cx);
    let view = search(wb, cx);
    assert!(query_focused(&view, cx));
    type_text(query, cx);
    settle_search(cx);
    keys("tab", cx);
    assert!(replace_focused(&view, cx));
    if !value.is_empty() {
        type_text(value, cx);
    }
    view
}

/// `⌥⌘Enter`, then answers the confirmation with `answer`.
fn replace_all(answer: &str, cx: &mut VisualTestContext) {
    keys("alt-cmd-enter", cx);
    assert!(cx.has_pending_prompt(), "Replace All asks first");
    cx.simulate_prompt_answer(answer);
    draw(cx);
    assert!(!cx.has_pending_prompt());
}

#[gpui::test]
fn replace_all_asks_then_writes(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo foo\n"), ("b.txt", "foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let view = replace_ready(&wb, "foo", "bar", cx);
    assert!(holds(&wb, "searchViewletFocus && replaceActive", cx));

    keys("alt-cmd-enter", cx);
    assert_eq!(
        cx.pending_prompt().map(|(m, _)| m).as_deref(),
        Some("Replace 3 occurrences across 2 files with 'bar'?")
    );
    cx.simulate_prompt_answer("Cancel");
    draw(cx);
    assert_eq!(disk(&dir, "a.txt"), "foo foo\n");
    assert_eq!(disk(&dir, "b.txt"), "foo\n");
    assert_eq!(view.read_with(cx, |v, cx| v.results(cx).match_count()), 2);
    assert_eq!(replace_message(&view, cx), None);

    replace_all("Replace", cx);
    assert_eq!(disk(&dir, "a.txt"), "bar bar\n");
    assert_eq!(disk(&dir, "b.txt"), "bar\n");
    assert!(view.read_with(cx, |v, cx| v.results(cx).files().is_empty()));
    assert!(!view.read_with(cx, |v, _| v.is_replacing()));
    assert_eq!(
        replace_message(&view, cx).as_deref(),
        Some("Replaced 3 occurrences across 2 files with 'bar'.")
    );
    assert!(cx.debug_bounds("search-replace-message").is_some());

    // The next search clears the message.
    keys("enter", cx);
    assert_eq!(replace_message(&view, cx), None);
}

#[gpui::test]
fn replace_all_button_disabled_without_results(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let view = replace_ready(&wb, "zzz", "bar", cx);
    assert_eq!(view.read_with(cx, |v, cx| v.results(cx).match_count()), 0);
    click("search-replace-all", cx);
    assert!(!cx.has_pending_prompt(), "nothing to replace");
    keys("alt-cmd-enter", cx);
    assert!(!cx.has_pending_prompt());

    // With results, the button asks.
    keys("shift-tab", cx);
    keys("cmd-a", cx);
    type_text("foo", cx);
    settle_search(cx);
    click("search-replace-all", cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    draw(cx);
    assert_eq!(disk(&dir, "a.txt"), "foo foo\n");
}

#[gpui::test]
fn replace_all_open_clean_document_undoable(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo foo\n"), ("b.txt", "foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    let view = replace_ready(&wb, "foo", "bar", cx);
    replace_all("Replace", cx);
    assert_eq!(disk(&dir, "a.txt"), "bar bar\n");
    assert_eq!(disk(&dir, "b.txt"), "bar\n");
    assert_eq!(doc_text(&wb, "a.txt", cx), "bar bar\n");
    assert!(!is_dirty(&wb, "a.txt", cx), "saved");
    assert_eq!(
        replace_message(&view, cx).as_deref(),
        Some("Replaced 3 occurrences across 2 files with 'bar'.")
    );

    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    keys("cmd-z", cx);
    assert_eq!(doc_text(&wb, "a.txt", cx), "foo foo\n", "one undo step");
}

#[gpui::test]
fn replace_all_dirty_document_not_saved(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    type_text("foo", cx);
    keys("enter", cx);
    assert_eq!(doc_text(&wb, "a.txt", cx), "foo\nfoo foo\n");
    assert!(is_dirty(&wb, "a.txt", cx));

    let view = replace_ready(&wb, "foo", "bar", cx);
    replace_all("Replace", cx);
    assert_eq!(doc_text(&wb, "a.txt", cx), "bar\nbar bar\n");
    assert!(is_dirty(&wb, "a.txt", cx), "never saved");
    assert_eq!(disk(&dir, "a.txt"), "foo foo\n");
    let message = replace_message(&view, cx).expect("a message");
    assert!(
        message.contains("Replaced 3 occurrences across 1 file with 'bar'."),
        "{message}"
    );
    assert!(
        message.contains("1 file with unsaved changes was not saved."),
        "{message}"
    );

    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    keys("cmd-z", cx);
    assert_eq!(doc_text(&wb, "a.txt", cx), "foo\nfoo foo\n");
    assert!(is_dirty(&wb, "a.txt", cx));
}

#[gpui::test]
fn replace_all_skips_conflicted_document(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo foo\n"), ("b.txt", "foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    type_text("x", cx);
    fs::write(dir.path().join("a.txt"), "foo\n").unwrap();
    let change = Change {
        path: Arc::from(Path::new("a.txt")),
        kind: ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    draw(cx);
    assert!(has_conflict(&wb, "a.txt", cx));

    let view = replace_ready(&wb, "foo", "bar", cx);
    replace_all("Replace", cx);
    assert_eq!(doc_text(&wb, "a.txt", cx), "xfoo foo\n", "left alone");
    assert_eq!(disk(&dir, "a.txt"), "foo\n");
    assert_eq!(disk(&dir, "b.txt"), "bar\n");
    let message = replace_message(&view, cx).expect("a message");
    assert!(
        message.contains("Skipped 1 file: a.txt (has a conflict with the file on disk)"),
        "{message}"
    );
}

#[gpui::test]
fn replace_all_skips_file_changed_since_search(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo foo\n"), ("b.txt", "foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let view = replace_ready(&wb, "foo", "bar", cx);
    fs::write(dir.path().join("b.txt"), "x\nfoo\n").unwrap();
    replace_all("Replace", cx);
    assert_eq!(disk(&dir, "a.txt"), "bar bar\n");
    assert_eq!(disk(&dir, "b.txt"), "x\nfoo\n");
    let message = replace_message(&view, cx).expect("a message");
    assert!(
        message.contains("Replaced 2 occurrences across 1 file with 'bar'."),
        "{message}"
    );
    assert!(
        message.contains("Skipped 1 file: b.txt (changed since the search)"),
        "{message}"
    );
}

#[gpui::test]
fn replace_all_empty_value_message(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo foo\n"), ("b.txt", "foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let view = replace_ready(&wb, "foo", "", cx);
    keys("alt-cmd-enter", cx);
    assert_eq!(
        cx.pending_prompt().map(|(m, _)| m).as_deref(),
        Some("Replace 3 occurrences across 2 files?")
    );
    cx.simulate_prompt_answer("Replace");
    draw(cx);
    assert_eq!(disk(&dir, "a.txt"), " \n");
    assert_eq!(disk(&dir, "b.txt"), "\n");
    assert_eq!(
        replace_message(&view, cx).as_deref(),
        Some("Replaced 3 occurrences across 2 files.")
    );
}

#[gpui::test]
fn new_search_during_replace_keeps_its_results(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo foo\nkeep\n"), ("b.txt", "foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let view = replace_ready(&wb, "foo", "bar", cx);
    keys("alt-cmd-enter", cx);
    cx.simulate_prompt_answer("Replace");
    let mut ticks = 0;
    while !view.read_with(cx, |v, _| v.is_replacing()) {
        assert!(cx.executor().tick(), "the replace starts");
        ticks += 1;
        assert!(ticks < 10_000);
    }
    assert!(
        view.read_with(cx, |v, cx| v.match_label(cx))
            .starts_with("Replacing… 0/2 files"),
    );
    // A new search starts while the replace runs.
    let input = view.read_with(cx, |v, _| v.query_input().clone());
    cx.update(|window, cx| input.update(cx, |i, cx| i.set_text("keep", window, cx)));
    draw(cx);
    settle_search(cx);
    assert!(!view.read_with(cx, |v, _| v.is_replacing()));
    assert_eq!(disk(&dir, "a.txt"), "bar bar\nkeep\n");
    assert_eq!(disk(&dir, "b.txt"), "bar\n");
    assert_eq!(replace_message(&view, cx), None);
    let files = view.read_with(cx, |v, cx| {
        v.results(cx)
            .files()
            .iter()
            .map(|f| f.path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    });
    assert_eq!(files, vec!["a.txt".to_string()], "the new search's results");
}

#[gpui::test]
fn find_bar_alt_cmd_enter_still_works_in_editor(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo foo\n"), ("b.txt", "foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let view = replace_ready(&wb, "foo", "bar", cx);

    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    keys("alt-cmd-f", cx);
    type_text("foo", cx);
    keys("tab", cx);
    type_text("X", cx);
    keys("alt-cmd-enter", cx);
    assert!(!cx.has_pending_prompt(), "the find bar replaces");
    assert_eq!(doc_text(&wb, "a.txt", cx), "X X\n");
    assert_eq!(disk(&dir, "b.txt"), "foo\n");

    // Back in the Search tab, with the find bar still open in a.txt.
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| wb.open_search(window, cx));
        let focus = view.read(cx).query_input().focus_handle(cx);
        window.focus(&focus, cx);
    });
    draw(cx);
    keys("alt-cmd-enter", cx);
    assert!(cx.has_pending_prompt(), "the Search tab's Replace All");
    cx.simulate_prompt_answer("Cancel");
    draw(cx);
    assert_eq!(doc_text(&wb, "a.txt", cx), "X X\n");
}

#[gpui::test]
fn replace_all_in_vim_insert_session_is_its_own_undo_step(cx: &mut TestAppContext) {
    init_with(
        cx,
        r#"{ "lucerna.lsp.enabled": false, "vim.enabled": true }"#,
    );
    let dir = project(&[("a.txt", "foo foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    // Insert mode: the vim session holds undo stops back.
    keys("i", cx);
    type_text("foo ", cx);
    assert_eq!(doc_text(&wb, "a.txt", cx), "foo foo foo\n");

    let _view = replace_ready(&wb, "foo", "bar", cx);
    replace_all("Replace", cx);
    assert_eq!(doc_text(&wb, "a.txt", cx), "bar bar bar\n");
    assert_eq!(disk(&dir, "a.txt"), "foo foo\n", "dirty: not saved");

    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    keys("escape", cx);
    keys("u", cx);
    assert_eq!(
        doc_text(&wb, "a.txt", cx),
        "foo foo foo\n",
        "one undo takes back the replace only"
    );
    keys("u", cx);
    assert_eq!(doc_text(&wb, "a.txt", cx), "foo foo\n", "then the typing");
}

// Per-line and per-file replace, Dismiss.

/// The status bar's message in the test window.
fn status(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|window, cx| {
        lucerna_workbench::status_message::current(window.window_handle().window_id(), cx)
            .map(|s| s.to_string())
    })
}

/// From the Replace input, focuses the results list (by way of the query).
fn focus_list(view: &Entity<SearchView>, cx: &mut VisualTestContext) {
    keys("shift-tab", cx);
    assert!(query_focused(view, cx));
    keys("escape", cx);
}

/// The line number of the selected line.
fn selected_line_number(view: &Entity<SearchView>, cx: &mut VisualTestContext) -> Option<u32> {
    view.read_with(cx, |v, cx| match v.list_selection(cx)? {
        ListSelection::Line(r) => Some(v.results(cx).line(r)?.line_number),
        ListSelection::Header(_) => None,
    })
}

fn result_files(view: &Entity<SearchView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, cx| {
        v.results(cx)
            .files()
            .iter()
            .map(|f| f.path.to_string_lossy().into_owned())
            .collect()
    })
}

fn match_count(view: &Entity<SearchView>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |v, cx| v.results(cx).match_count())
}

#[gpui::test]
fn shift_cmd_1_replaces_selected_line(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo\nx\nfoo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let view = replace_ready(&wb, "foo", "bar", cx);
    focus_list(&view, cx);
    keys("down", cx);
    assert_eq!(selection(&view, cx), Some(ListSelection::Header(0)));
    assert!(holds(&wb, "fileMatchFocus && fileMatchOrMatchFocus", cx));
    assert!(!holds(&wb, "matchFocus", cx));
    keys("down", cx);
    assert_eq!(selected_line_number(&view, cx), Some(1));
    assert!(holds(
        &wb,
        "searchViewletFocus && replaceActive && matchFocus && fileMatchOrMatchFocus",
        cx
    ));
    assert!(!holds(&wb, "fileMatchFocus", cx));

    keys("shift-cmd-1", cx);
    assert!(!cx.has_pending_prompt(), "never asks");
    assert_eq!(disk(&dir, "a.txt"), "bar\nx\nfoo\n");
    assert_eq!(match_count(&view, cx), 1);
    assert_eq!(selected_line_number(&view, cx), Some(3), "the next match");
    assert!(!view.read_with(cx, |v, _| v.is_replacing()));
    assert_eq!(replace_message(&view, cx), None, "the results stay");

    // The last match: replaced, nothing left to select.
    keys("shift-cmd-1", cx);
    assert_eq!(disk(&dir, "a.txt"), "bar\nx\nbar\n");
    assert_eq!(match_count(&view, cx), 0);
}

#[gpui::test]
fn replace_line_selects_previous_when_last(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo\nx\ny\nz\nfoo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let view = replace_ready(&wb, "foo", "bar", cx);
    focus_list(&view, cx);
    // Header, 1, 2 (context), 4 (context), 5.
    keys("down down down down down", cx);
    assert_eq!(selected_line_number(&view, cx), Some(5));
    keys("shift-cmd-1", cx);
    assert_eq!(disk(&dir, "a.txt"), "foo\nx\ny\nz\nbar\n");
    assert_eq!(
        selected_line_number(&view, cx),
        Some(1),
        "the previous match"
    );
}

#[gpui::test]
fn replace_line_refuses_changed_line(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo\nx\nfoo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let view = replace_ready(&wb, "foo", "bar", cx);
    fs::write(dir.path().join("a.txt"), "zfoo\nx\nfoo\n").unwrap();
    focus_list(&view, cx);
    keys("down down", cx);
    keys("shift-cmd-1", cx);
    assert_eq!(disk(&dir, "a.txt"), "zfoo\nx\nfoo\n", "left alone");
    assert_eq!(
        status(cx).as_deref(),
        Some("The file has changed since the search. Search again to replace it.")
    );
    assert_eq!(match_count(&view, cx), 2, "the results stay");
}

#[gpui::test]
fn shift_cmd_1_on_header_replaces_file(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo foo\n"), ("b.txt", "x\nfoo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let view = replace_ready(&wb, "foo", "bar", cx);
    focus_list(&view, cx);
    keys("down", cx);
    assert_eq!(selection(&view, cx), Some(ListSelection::Header(0)));
    keys("shift-cmd-1", cx);
    assert!(!cx.has_pending_prompt(), "never asks");
    assert_eq!(disk(&dir, "a.txt"), "bar bar\n");
    assert_eq!(disk(&dir, "b.txt"), "x\nfoo\n");
    assert_eq!(result_files(&view, cx), vec!["b.txt".to_string()]);
    assert_eq!(
        selected_line_number(&view, cx),
        Some(2),
        "b.txt's first match"
    );

    // ⇧⌘Enter on a header does the same (above line 1, a context line).
    keys("up up", cx);
    assert_eq!(selection(&view, cx), Some(ListSelection::Header(0)));
    keys("shift-cmd-enter", cx);
    assert_eq!(disk(&dir, "b.txt"), "x\nbar\n");
    assert!(result_files(&view, cx).is_empty());
}

#[gpui::test]
fn cmd_backspace_dismisses_line_and_file(cx: &mut TestAppContext) {
    let (dir, wb, view, cx) = searched(cx);
    let before: Vec<String> = ["a.rs", "b.rs", "sub/c.rs"]
        .iter()
        .map(|p| disk(&dir, p))
        .collect();
    assert_eq!(match_count(&view, cx), 4);
    keys("escape down down", cx);
    assert_eq!(selected_line_number(&view, cx), Some(1));
    assert!(holds(
        &wb,
        "searchViewletFocus && fileMatchOrMatchFocus",
        cx
    ));
    keys("cmd-backspace", cx);
    assert_eq!(match_count(&view, cx), 3);
    assert_eq!(
        result_files(&view, cx),
        vec!["b.rs".to_string(), "sub/c.rs".to_string()]
    );
    assert!(
        matches!(selection(&view, cx), Some(ListSelection::Line(r)) if r.file == 0),
        "the next line"
    );

    keys("up", cx);
    assert_eq!(selection(&view, cx), Some(ListSelection::Header(0)));
    keys("cmd-backspace", cx);
    assert_eq!(result_files(&view, cx), vec!["sub/c.rs".to_string()]);
    assert_eq!(match_count(&view, cx), 1);
    assert_eq!(view.read_with(cx, |v, cx| v.match_label(cx)), "1 result");
    let after: Vec<String> = ["a.rs", "b.rs", "sub/c.rs"]
        .iter()
        .map(|p| disk(&dir, p))
        .collect();
    assert_eq!(before, after, "nothing on disk changes");
}

#[gpui::test]
fn hover_buttons_only_with_replace_row(cx: &mut TestAppContext) {
    let (dir, _wb, view, cx) = searched(cx);
    assert!(cx.debug_bounds("search-line-replace-0-0-0").is_none());
    assert!(cx.debug_bounds("search-line-dismiss-0-0-0").is_some());
    assert!(cx.debug_bounds("search-file-replace-0").is_none());
    assert!(cx.debug_bounds("search-file-dismiss-0").is_some());
    // Context lines have no buttons.
    assert!(cx.debug_bounds("search-line-dismiss-1-0-1").is_none());

    click("search-toggle-replace", cx);
    assert!(cx.debug_bounds("search-line-replace-0-0-0").is_some());
    assert!(cx.debug_bounds("search-file-replace-0").is_some());
    type_text("pin", cx);

    // The buttons do their job, and never open the line.
    click("search-line-dismiss-0-0-0", cx);
    assert_eq!(match_count(&view, cx), 3);
    assert_eq!(disk(&dir, "a.rs"), "a needle\n");
    click("search-line-replace-0-0-0", cx);
    assert_eq!(disk(&dir, "b.rs"), "pin one\nnothing\nneedle two needle\n");
    assert_eq!(match_count(&view, cx), 2);
    click("search-file-replace-0", cx);
    assert_eq!(disk(&dir, "b.rs"), "pin one\nnothing\npin two pin\n");
    assert_eq!(result_files(&view, cx), vec!["sub/c.rs".to_string()]);
    click("search-file-dismiss-0", cx);
    assert!(result_files(&view, cx).is_empty());
    assert_eq!(disk(&dir, "sub/c.rs"), "NEEDLE\n");
}

#[gpui::test]
fn replace_line_in_dirty_document_reports_unsaved(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo\nx\nfoo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    type_text("y", cx);
    assert_eq!(doc_text(&wb, "a.txt", cx), "yfoo\nx\nfoo\n");
    assert!(is_dirty(&wb, "a.txt", cx));

    let view = replace_ready(&wb, "foo", "bar", cx);
    focus_list(&view, cx);
    keys("down down", cx);
    keys("shift-cmd-1", cx);
    assert_eq!(doc_text(&wb, "a.txt", cx), "ybar\nx\nfoo\n");
    assert!(is_dirty(&wb, "a.txt", cx), "never saved");
    assert_eq!(disk(&dir, "a.txt"), "foo\nx\nfoo\n");
    assert_eq!(
        status(cx).as_deref(),
        Some("Replaced in a.txt. The file has unsaved changes.")
    );
    assert_eq!(match_count(&view, cx), 1, "searched again on the buffer");
    assert_eq!(selected_line_number(&view, cx), Some(3));
}

/// Presses `↓`, then `↑`, in the results list until line `n` is selected.
fn select_line_number(view: &Entity<SearchView>, n: u32, cx: &mut VisualTestContext) {
    for key in ["down", "up"] {
        for _ in 0..30 {
            if selected_line_number(view, cx) == Some(n) {
                return;
            }
            keys(key, cx);
        }
    }
    panic!("line {n} is never selected");
}

/// The match lines' numbers the results show, by file.
fn match_lines(view: &Entity<SearchView>, cx: &mut VisualTestContext) -> Vec<u32> {
    view.read_with(cx, |v, cx| {
        v.results(cx)
            .files()
            .iter()
            .flat_map(|f| &f.excerpts)
            .flat_map(|e| &e.lines)
            .filter(|l| !l.ranges.is_empty())
            .map(|l| l.line_number)
            .collect()
    })
}

#[gpui::test]
fn replace_line_keeps_dismissed_lines_out(cx: &mut TestAppContext) {
    init(cx);
    let text = "foo1\na\nb\nc\nfoo2\nd\ne\nf\nfoo3\n";
    let dir = project(&[("a.txt", text)]);
    let (wb, cx) = workbench(dir.path(), cx);
    let view = replace_ready(&wb, "foo", "bar", cx);
    focus_list(&view, cx);
    select_line_number(&view, 1, cx);
    keys("cmd-backspace", cx);
    assert_eq!(match_lines(&view, cx), vec![5, 9]);
    select_line_number(&view, 5, cx);
    keys("shift-cmd-1", cx);
    assert_eq!(disk(&dir, "a.txt"), "foo1\na\nb\nc\nbar2\nd\ne\nf\nfoo3\n");
    assert_eq!(match_lines(&view, cx), vec![9], "line 1 stays dismissed");

    replace_all("Replace", cx);
    assert_eq!(
        disk(&dir, "a.txt"),
        "foo1\na\nb\nc\nbar2\nd\ne\nf\nbar3\n",
        "Replace All leaves the dismissed line alone"
    );
}

#[gpui::test]
fn replace_line_with_newline_renumbers_shown_lines(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo\nx\nfoo\ny\nfoo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-h", cx);
    let view = search(&wb, cx);
    type_text("foo", cx);
    keys("alt-cmd-r", cx);
    assert!(view.read_with(cx, |v, _| v.is_regex()));
    settle_search(cx);
    keys("tab", cx);
    type_text(r"a\nb", cx);
    focus_list(&view, cx);
    select_line_number(&view, 3, cx);
    keys("cmd-backspace", cx);
    assert_eq!(match_lines(&view, cx), vec![1, 5]);
    select_line_number(&view, 1, cx);
    keys("shift-cmd-1", cx);
    assert_eq!(disk(&dir, "a.txt"), "a\nb\nx\nfoo\ny\nfoo\n");
    assert_eq!(
        match_lines(&view, cx),
        vec![6],
        "line 5 is now 6; the dismissed line 3 (now 4) stays out"
    );
    assert_eq!(selected_line_number(&view, cx), Some(6));
}

// Replace All: what the final review found.

/// The message of a Replace All of `foo` with `bar` over an open clean
/// `a.txt` that an agent rewrote on disk before the replace saved it (the
/// watcher has not reported the write yet).
fn replace_over_agent_write(cx: &mut TestAppContext, user_settings: &str) {
    init_with(cx, user_settings);
    let dir = project(&[("a.txt", "foo foo\n"), ("b.txt", "foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    let view = replace_ready(&wb, "foo", "bar", cx);
    fs::write(dir.path().join("a.txt"), "the agent wrote this\n").unwrap();
    replace_all("Replace", cx);
    assert_eq!(disk(&dir, "a.txt"), "the agent wrote this\n", "kept");
    assert_eq!(disk(&dir, "b.txt"), "bar\n");
    assert_eq!(
        doc_text(&wb, "a.txt", cx),
        "bar bar\n",
        "the buffer keeps it"
    );
    assert!(has_conflict(&wb, "a.txt", cx));
    let message = replace_message(&view, cx).expect("a message");
    assert!(
        message.contains("Replaced 1 occurrence across 1 file with 'bar'."),
        "{message}"
    );
    assert!(
        message.contains("Skipped 1 file: a.txt (changed on disk)"),
        "{message}"
    );
}

#[gpui::test]
fn replace_all_save_of_clean_document_hits_disk_change(cx: &mut TestAppContext) {
    replace_over_agent_write(cx, r#"{ "lucerna.lsp.enabled": false }"#);
}

#[gpui::test]
fn replace_all_checks_disk_despite_overwrite_setting(cx: &mut TestAppContext) {
    replace_over_agent_write(
        cx,
        r#"{
            "lucerna.lsp.enabled": false,
            "files.saveConflictResolution": "overwriteFileOnDisk"
        }"#,
    );
}

#[gpui::test]
fn replace_all_leaves_dismissed_line_alone(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo\nx\nfoo\n"), ("b.txt", "foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let view = replace_ready(&wb, "foo", "bar", cx);
    focus_list(&view, cx);
    select_line_number(&view, 3, cx);
    keys("cmd-backspace", cx);
    assert_eq!(match_count(&view, cx), 2);
    replace_all("Replace", cx);
    assert_eq!(disk(&dir, "a.txt"), "bar\nx\nfoo\n", "line 3 dismissed");
    assert_eq!(disk(&dir, "b.txt"), "bar\n");
    assert_eq!(
        replace_message(&view, cx).as_deref(),
        Some("Replaced 2 occurrences across 2 files with 'bar'.")
    );
}

#[gpui::test]
fn replace_all_queued_save_never_saves_later_typing(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    // A save of the clean file is in flight (a slow disk) when the
    // replace asks for its own: that one waits its turn.
    let hold = cx.update(|_, cx| lucerna_workbench::hold_saves_for_test(cx));
    let docs = documents(&wb, cx);
    let opts = lucerna_workbench::SaveOptions {
        reason: lucerna_workbench::SaveReason::Explicit,
        participants: false,
        overwrite: false,
    };
    let _first = cx
        .update(|window, cx| docs.update(cx, |d, cx| d.save(Path::new("a.txt"), opts, window, cx)));
    let view = replace_ready(&wb, "foo", "bar", cx);
    replace_all("Replace", cx);
    assert_eq!(doc_text(&wb, "a.txt", cx), "bar bar\n");
    // The user types before the replace's save has its turn.
    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    type_text("x", cx);
    assert!(doc_text(&wb, "a.txt", cx).contains('x'));
    drop(hold);
    draw(cx);
    assert_eq!(
        disk(&dir, "a.txt"),
        "foo foo\n",
        "only the first save wrote"
    );
    assert!(is_dirty(&wb, "a.txt", cx));
    let message = replace_message(&view, cx).expect("a message");
    assert!(
        message.contains("Replaced 2 occurrences across 1 file with 'bar'."),
        "{message}"
    );
    assert!(
        message.contains("1 file with unsaved changes was not saved."),
        "{message}"
    );
}

#[gpui::test]
fn replace_all_skips_deleted_open_document(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "foo foo\n"), ("b.txt", "foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    let view = replace_ready(&wb, "foo", "bar", cx);
    fs::remove_file(dir.path().join("a.txt")).unwrap();
    let change = Change {
        path: Arc::from(Path::new("a.txt")),
        kind: ChangeKind::Removed,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    draw(cx);
    let docs = documents(&wb, cx);
    assert_eq!(
        docs.read_with(cx, |d, _| d.state(Path::new("a.txt")).cloned()),
        Some(lucerna_workbench::DocState::Deleted)
    );
    replace_all("Replace", cx);
    assert!(!dir.path().join("a.txt").exists(), "not made again");
    assert_eq!(disk(&dir, "b.txt"), "bar\n");
    let message = replace_message(&view, cx).expect("a message");
    assert!(
        message.contains("Skipped 1 file: a.txt (no longer exists)"),
        "{message}"
    );
}

// Replace All: what the merge review found.

#[cfg(unix)] // used only by the unix-only tests below
/// The text of the document at `path` (absolute or an Untitled key).
fn doc_text_at(wb: &Entity<Workbench>, path: &Path, cx: &mut VisualTestContext) -> String {
    let docs = documents(wb, cx);
    docs.read_with(cx, |d, _| {
        let s = d.snapshot_of(path).expect("the document is loaded");
        s.text_for_range(0..s.len_bytes())
    })
}

#[cfg(unix)] // `:` is not allowed in Windows file names
#[gpui::test]
fn replace_all_never_takes_a_file_named_untitled_for_the_untitled_document(
    cx: &mut TestAppContext,
) {
    init(cx);
    let name = "untitled:Untitled-1";
    let dir = project(&[(name, "foo foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let real = dir.path().join(name);
    // The real file, open with unsaved changes.
    let code = open(&wb, name, cx);
    focus_view(&code, cx);
    type_text("x", cx);
    assert_eq!(doc_text_at(&wb, &real, cx), "xfoo foo\n");
    // An Untitled tab whose key is the file's name, with the same line.
    keys("cmd-n", cx);
    type_text("xfoo foo", cx);
    let untitled = Path::new(name);
    assert_eq!(doc_text_at(&wb, untitled, cx), "xfoo foo");

    let view = replace_ready(&wb, "foo", "bar", cx);
    assert_eq!(match_count(&view, cx), 1);
    replace_all("Replace", cx);
    assert_eq!(doc_text_at(&wb, untitled, cx), "xfoo foo", "never touched");
    assert_eq!(
        doc_text_at(&wb, &real, cx),
        "xbar bar\n",
        "in its own buffer"
    );
    let docs = documents(&wb, cx);
    assert!(docs.read_with(cx, |d, _| d.is_dirty(&real)), "not saved");
    assert_eq!(disk(&dir, name), "foo foo\n");
    let message = replace_message(&view, cx).expect("a message");
    assert!(
        message.contains("Replaced 2 occurrences across 1 file with 'bar'."),
        "{message}"
    );
    assert!(
        message.contains("1 file with unsaved changes was not saved."),
        "{message}"
    );
}

#[cfg(unix)] // file permissions
#[gpui::test]
fn replace_all_failed_save_reports_unsaved_not_skipped(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt as _;
    init(cx);
    let dir = project(&[("a.txt", "foo foo\n"), ("b.txt", "foo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    let code = open(&wb, "a.txt", cx);
    focus_view(&code, cx);
    assert!(!is_dirty(&wb, "a.txt", cx));
    let view = replace_ready(&wb, "foo", "bar", cx);
    // The save of the clean document will fail.
    let a = dir.path().join("a.txt");
    fs::set_permissions(&a, fs::Permissions::from_mode(0o444)).unwrap();
    replace_all("Replace", cx);
    fs::set_permissions(&a, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(disk(&dir, "a.txt"), "foo foo\n", "not written");
    assert_eq!(disk(&dir, "b.txt"), "bar\n");
    assert_eq!(
        doc_text(&wb, "a.txt", cx),
        "bar bar\n",
        "the buffer keeps it"
    );
    assert!(is_dirty(&wb, "a.txt", cx));
    let message = replace_message(&view, cx).expect("a message");
    assert!(
        message.contains("Replaced 3 occurrences across 2 files with 'bar'."),
        "{message}"
    );
    assert!(
        message.contains("1 file with unsaved changes was not saved."),
        "{message}"
    );
    assert!(!message.contains("Skipped"), "{message}");
}

#[gpui::test]
fn new_search_during_replace_still_reports_skipped_files(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[
        ("a.txt", "foo foo\nkeep\n"),
        ("b.txt", "foo\n"),
        ("c.txt", "foo\n"),
    ]);
    let (wb, cx) = workbench(dir.path(), cx);
    let code = open(&wb, "c.txt", cx);
    focus_view(&code, cx);
    type_text("x", cx);
    assert!(is_dirty(&wb, "c.txt", cx));
    let view = replace_ready(&wb, "foo", "bar", cx);
    fs::write(dir.path().join("b.txt"), "x\nfoo\n").unwrap();
    keys("alt-cmd-enter", cx);
    cx.simulate_prompt_answer("Replace");
    let mut ticks = 0;
    while !view.read_with(cx, |v, _| v.is_replacing()) {
        assert!(cx.executor().tick(), "the replace starts");
        ticks += 1;
        assert!(ticks < 10_000);
    }
    let input = view.read_with(cx, |v, _| v.query_input().clone());
    cx.update(|window, cx| input.update(cx, |i, cx| i.set_text("keep", window, cx)));
    draw(cx);
    settle_search(cx);
    assert!(!view.read_with(cx, |v, _| v.is_replacing()));
    assert_eq!(disk(&dir, "a.txt"), "bar bar\nkeep\n");
    assert_eq!(disk(&dir, "b.txt"), "x\nfoo\n");
    assert_eq!(replace_message(&view, cx), None, "no Replaced message");
    let status = status(cx).expect("a status message");
    assert!(
        status.contains("Skipped 1 file: b.txt (changed since the search)"),
        "{status}"
    );
    assert!(
        status.contains("1 file with unsaved changes was not saved."),
        "{status}"
    );
    assert!(!status.contains("Replaced"), "{status}");
}

#[cfg(unix)] // `:` is not allowed in Windows file names
#[gpui::test]
fn replace_line_searches_a_file_named_untitled_on_its_own_text(cx: &mut TestAppContext) {
    init(cx);
    let name = "untitled:Untitled-1";
    let dir = project(&[(name, "foo\nx\nfoo\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    // An Untitled tab whose key is the file's name, with no match.
    keys("cmd-n", cx);
    type_text("zzz", cx);
    let view = replace_ready(&wb, "foo", "bar", cx);
    focus_list(&view, cx);
    select_line_number(&view, 1, cx);
    keys("shift-cmd-1", cx);
    assert_eq!(disk(&dir, name), "bar\nx\nfoo\n");
    assert_eq!(match_lines(&view, cx), vec![3], "searched on the file");
    assert_eq!(doc_text_at(&wb, Path::new(name), cx), "zzz");
}
