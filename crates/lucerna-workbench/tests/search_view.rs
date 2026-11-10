//! The Search tab: debounced, cancellable, streaming project search with
//! context lines, driven through GPUI's test platform with real keystrokes.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, Focusable, Modifiers, TestAppContext, VisualTestContext};
use lucerna_project::{Excerpt, LineMatch, SearchEvent, WorktreeEvent};
use lucerna_settings::{Keymap, SettingsStore, parse_when};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, GroupState, GroupsState, OpenMode, SEARCH_TAB_STATE, SearchState, SearchView,
    SidebarView, TabItem, Workbench, WorkspaceState, save_state, state_path,
};
use tempfile::TempDir;

const DEBOUNCE: Duration = Duration::from_millis(150);

fn init(cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
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

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    draw(cx);
}

fn search(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<SearchView> {
    cx.update(|_, cx| wb.read(cx).search_view().clone())
}

/// `(path, matching lines)` of each file, in the order shown.
fn files(view: &Entity<SearchView>, cx: &mut VisualTestContext) -> Vec<(PathBuf, usize)> {
    cx.update(|_, cx| {
        view.read(cx)
            .results()
            .files()
            .iter()
            .map(|f| (f.path.to_path_buf(), f.match_count()))
            .collect()
    })
}

fn row_count(view: &Entity<SearchView>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |v, _| v.results().rows().len())
}

fn summary(view: &Entity<SearchView>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| view.read(cx).summary())
}

fn query(view: &Entity<SearchView>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| view.read(cx).query_input().read(cx).text().to_owned())
}

fn holds(wb: &Entity<Workbench>, when: &str, cx: &mut VisualTestContext) -> bool {
    let ctx = cx.update(|window, cx| wb.read(cx).context_keys(window, cx));
    parse_when(when).unwrap().eval(&ctx)
}

fn p(path: &str) -> PathBuf {
    PathBuf::from(path)
}

fn active_item(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<TabItem> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .active_tab()
            .map(|t| t.item.clone())
    })
}

fn search_tabs(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .groups()
            .iter()
            .flat_map(|g| &g.tabs)
            .filter(|t| t.item == TabItem::Search)
            .count()
    })
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

fn needles() -> TempDir {
    project(&[
        ("b.rs", "needle one\nnothing\nneedle two needle\n"),
        ("a.rs", "a needle\n"),
        ("sub/c.rs", "NEEDLE\n"),
        // `search.exclude` has `**/target` on by default.
        ("target/x.rs", "needle\n"),
    ])
}

#[gpui::test]
fn find_in_files_opens_search_tab(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    assert_eq!(active_item(&wb, cx), Some(TabItem::Search));
    assert!(holds(&wb, "searchViewletFocus && textInputFocus", cx));
    assert!(cx.debug_bounds("search-view").is_some());
    let tab = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups.model().active_tab().cloned().unwrap()
    });
    assert!(tab.pinned, "never a preview");
    assert_eq!(tab.title(), "Search");
    assert!(cx.debug_bounds("tab-icon-0-0").is_some(), "the search icon");

    // Again, from another tab: still one Search tab, focused.
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("a.rs")), OpenMode::Pinned, cx)
    });
    draw(cx);
    keys("cmd-shift-f", cx);
    assert_eq!(search_tabs(&wb, cx), 1);
    assert_eq!(active_item(&wb, cx), Some(TabItem::Search));
    assert!(holds(&wb, "searchViewletFocus && textInputFocus", cx));
    // The sidebar is not involved.
    assert_eq!(
        wb.read_with(cx, |wb, _| wb.active_sidebar()),
        SidebarView::Explorer
    );
}

#[gpui::test]
fn search_view_streams_and_groups(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    let view = search(&wb, cx);

    type_text("needle", cx);
    assert!(
        files(&view, cx).is_empty(),
        "the search waits for the debounce"
    );
    settle_search(cx);
    assert_eq!(
        files(&view, cx),
        vec![(p("a.rs"), 1), (p("b.rs"), 2), (p("sub/c.rs"), 1)],
        "files are grouped and sorted by path whatever order they arrive in"
    );
    assert_eq!(summary(&view, cx).as_deref(), Some("4 results in 3 files"));
    // Three headers; b.rs has one excerpt of three lines (one context).
    assert_eq!(row_count(&view, cx), 8);

    click("search-file-1", cx);
    assert_eq!(row_count(&view, cx), 5, "b.rs collapsed");
    click("search-file-1", cx);
    assert_eq!(row_count(&view, cx), 8);

    // Case-sensitive drops `NEEDLE`.
    click("search-toggle-case", cx);
    settle_search(cx);
    assert_eq!(files(&view, cx), vec![(p("a.rs"), 1), (p("b.rs"), 2)]);
    assert_eq!(summary(&view, cx).as_deref(), Some("3 results in 2 files"));
}

#[gpui::test]
fn results_include_context_lines(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "one\ntwo\nneedle\nfour\nfive\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    type_text("needle", cx);
    settle_search(cx);
    let view = search(&wb, cx);
    let rows: Vec<String> = view.read_with(cx, |v, _| {
        let results = v.results();
        results
            .rows()
            .iter()
            .map(|row| match row {
                lucerna_workbench::SearchRow::Header(f) => format!("header {f}"),
                lucerna_workbench::SearchRow::Line(r) => {
                    format!("line {}", results.line(*r).unwrap().line_number)
                }
                lucerna_workbench::SearchRow::Separator { .. } => "separator".into(),
            })
            .collect()
    });
    assert_eq!(rows, ["header 0", "line 2", "line 3", "line 4"]);
    assert!(cx.debug_bounds("search-line-0-0-0").is_some());
    assert!(cx.debug_bounds("search-line-0-0-2").is_some());
}

#[gpui::test]
fn excerpts_are_separated(cx: &mut TestAppContext) {
    init(cx);
    let text = "needle\n2\n3\n4\n5\n6\nneedle\n";
    let dir = project(&[("a.txt", text)]);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    type_text("needle", cx);
    settle_search(cx);
    let view = search(&wb, cx);
    // Header, 2 lines, separator, 2 lines.
    assert_eq!(row_count(&view, cx), 6);
    assert!(cx.debug_bounds("search-separator-0-1").is_some());
}

#[gpui::test]
fn search_change_cancels_previous(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[
        ("alpha1.txt", "alpha\n"),
        ("alpha2.txt", "alpha alpha\n"),
        ("beta.txt", "beta\n"),
    ]);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    let view = search(&wb, cx);
    type_text("alpha", cx);

    // The alpha search is in flight when the query changes.
    cx.update(|window, cx| {
        view.update(cx, |v, cx| v.run_search(cx));
        let input = view.read(cx).query_input().clone();
        input.update(cx, |input, cx| input.set_text("beta", window, cx));
    });
    draw(cx);
    assert!(
        files(&view, cx).is_empty(),
        "no result of the old query arrives after the change"
    );

    settle_search(cx);
    assert_eq!(files(&view, cx), vec![(p("beta.txt"), 1)]);
    // Nothing of the old search shows up later either.
    cx.executor().advance_clock(Duration::from_secs(1));
    draw(cx);
    assert_eq!(files(&view, cx), vec![(p("beta.txt"), 1)]);
    assert_eq!(summary(&view, cx).as_deref(), Some("1 result in 1 file"));
}

#[gpui::test]
fn search_truncation_message(cx: &mut TestAppContext) {
    init(cx);
    let lines = "x\n".repeat(8_000);
    let dir = project(&[("1.txt", &lines), ("2.txt", &lines), ("3.txt", &lines)]);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    let view = search(&wb, cx);
    type_text("x", cx);
    settle_search(cx);

    let files = files(&view, cx);
    let total: usize = files.iter().map(|(_, n)| n).sum();
    assert_eq!(total, 20_000);
    assert_eq!(
        summary(&view, cx),
        Some(format!(
            "20,000 results in {} files (truncated at 20,000)",
            files.len()
        ))
    );
    // Every line matches: one excerpt per file, no context lines.
    assert_eq!(row_count(&view, cx), files.len() + 20_000);
}

#[gpui::test]
fn click_result_opens_and_positions(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.rs", "first\n  let needle = 1;\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    type_text("needle", cx);
    settle_search(cx);
    let view = search(&wb, cx);
    assert_eq!(files(&view, cx), vec![(p("a.rs"), 1)]);

    // Line 0 of the excerpt is the context line `first`.
    click("search-line-0-0-1", cx);
    let tabs: Vec<(Option<PathBuf>, bool)> = cx.update(|_, cx| {
        let model = wb.read(cx).editor_groups().read(cx).model();
        model.groups()[0]
            .tabs
            .iter()
            .map(|t| (t.path().map(|p| p.to_path_buf()), t.pinned))
            .collect()
    });
    assert_eq!(
        tabs,
        vec![(None, true), (Some(dir.path().join("a.rs")), false)],
        "opened as a preview next to the Search tab"
    );
    let code = active_view(&wb, cx);
    let (cursor, highlights, active) = code.read_with(cx, |v, _| {
        (v.cursor(), v.match_highlights().to_vec(), v.active_match())
    });
    assert_eq!(cursor, Point { row: 1, column: 6 });
    assert_eq!(highlights, vec![12..18]);
    assert_eq!(active, Some(0));
    // The Search tab is hidden behind the file: keys go to the editor.
    assert!(holds(&wb, "editorFocus && !searchViewletFocus", cx));

    // Clicking again on the open file moves within it.
    keys("cmd-shift-f", cx);
    code.update(cx, |v, cx| v.go_to(Point { row: 0, column: 0 }, cx));
    draw(cx);
    click("search-line-0-0-1", cx);
    let cursor = active_view(&wb, cx).read_with(cx, |v, _| v.cursor());
    assert_eq!(cursor, Point { row: 1, column: 6 });
}

#[gpui::test]
fn click_context_line_opens_at_that_line(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.rs", "first\n  let needle = 1;\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    type_text("needle", cx);
    settle_search(cx);

    click("search-line-0-0-0", cx);
    let code = active_view(&wb, cx);
    let (cursor, highlights) =
        code.read_with(cx, |v, _| (v.cursor(), v.match_highlights().to_vec()));
    assert_eq!(cursor, Point { row: 0, column: 0 });
    assert!(highlights.is_empty(), "a context line highlights nothing");
}

#[gpui::test]
fn find_in_files_prefills_selection(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.rs", "first\n  let needle = 1;\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("a.rs")), OpenMode::Pinned, cx)
    });
    draw(cx);
    let code = active_view(&wb, cx);
    let at = |row, column| Point { row, column };
    code.update(cx, |v, cx| v.select(at(1, 6)..at(1, 12), cx));

    keys("cmd-shift-f", cx);
    let view = search(&wb, cx);
    assert_eq!(query(&view, cx), "needle");
    assert!(holds(&wb, "searchViewletFocus && textInputFocus", cx));
    settle_search(cx);
    assert_eq!(files(&view, cx), vec![(p("a.rs"), 1)]);

    // A selection over two lines is not used.
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("a.rs")), OpenMode::Pinned, cx)
    });
    draw(cx);
    code.update(cx, |v, cx| v.select(at(0, 1)..at(1, 2), cx));
    keys("cmd-shift-f", cx);
    assert_eq!(query(&view, cx), "needle");
}

#[gpui::test]
fn search_invalid_regex_shows_error(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    let view = search(&wb, cx);
    click("search-toggle-regex", cx);
    // Clicking a toggle leaves focus where it was.
    assert!(holds(&wb, "textInputFocus", cx));
    type_text("(", cx);
    settle_search(cx);
    let error = view.read_with(cx, |v, _| v.error().map(str::to_owned));
    assert!(error.is_some(), "the regex error is reported");
    assert!(cx.debug_bounds("search-error").is_some());
    assert_eq!(summary(&view, cx), None);
    assert!(files(&view, cx).is_empty());

    keys("backspace", cx);
    type_text("ne+dle", cx);
    settle_search(cx);
    assert!(view.read_with(cx, |v, _| v.error().is_none()));
    assert!(cx.debug_bounds("search-error").is_none());
    assert_eq!(files(&view, cx).len(), 3);
}

#[gpui::test]
fn search_include_exclude_filters(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    let view = search(&wb, cx);
    type_text("needle", cx);
    settle_search(cx);
    assert_eq!(files(&view, cx).len(), 3);

    // The filter row shows once its button is on.
    assert!(cx.debug_bounds("search-filter-row").is_none());
    click("search-filters", cx);
    assert!(cx.debug_bounds("search-filter-row").is_some());

    let (include, exclude) = view.read_with(cx, |v, _| {
        (v.include_input().clone(), v.exclude_input().clone())
    });
    cx.update(|window, cx| include.update(cx, |i, cx| i.set_text("sub/**, a.rs", window, cx)));
    settle_search(cx);
    assert_eq!(files(&view, cx), vec![(p("a.rs"), 1), (p("sub/c.rs"), 1)]);

    cx.update(|window, cx| exclude.update(cx, |i, cx| i.set_text("sub", window, cx)));
    settle_search(cx);
    assert_eq!(files(&view, cx), vec![(p("a.rs"), 1)]);
}

#[gpui::test]
fn refresh_and_collapse_all_buttons(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    let view = search(&wb, cx);
    type_text("needle", cx);
    settle_search(cx);
    assert_eq!(row_count(&view, cx), 8);

    click("search-collapse-all", cx);
    assert_eq!(row_count(&view, cx), 3, "only the headers");
    click("search-collapse-all", cx);
    assert_eq!(row_count(&view, cx), 8, "expanded again");

    fs::write(dir.path().join("a.rs"), "no match\n").unwrap();
    click("search-refresh", cx);
    assert_eq!(files(&view, cx), vec![(p("b.rs"), 2), (p("sub/c.rs"), 1)]);
}

/// Searches `needle` in a project with only `file`, clicks its first
/// result and returns where the cursor and highlight landed.
fn click_needle(file: &[u8], cx: &mut TestAppContext) -> (Point, Vec<std::ops::Range<usize>>) {
    init(cx);
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), file).unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    type_text("needle", cx);
    settle_search(cx);
    let view = search(&wb, cx);
    assert_eq!(files(&view, cx), vec![(p("a.txt"), 1)]);
    click("search-line-0-0-0", cx);
    let code = active_view(&wb, cx);
    code.read_with(cx, |v, _| (v.cursor(), v.match_highlights().to_vec()))
}

#[gpui::test]
fn click_result_past_reported_text_finds_column(cx: &mut TestAppContext) {
    // The search reports at most 1,000 chars of a line, without this match.
    let line = format!("{}needle\n", "-".repeat(1500));
    let (cursor, highlights) = click_needle(line.as_bytes(), cx);
    assert_eq!(
        cursor,
        Point {
            row: 0,
            column: 1500
        }
    );
    assert_eq!(highlights, vec![1500..1506]);
}

#[gpui::test]
fn click_result_uses_decoded_columns(cx: &mut TestAppContext) {
    // windows-1252 `é`: one byte on disk, two in the decoded buffer, and a
    // three-byte U+FFFD in the search's lossy line.
    let (cursor, highlights) = click_needle(b"caf\xe9 needle\n", cx);
    assert_eq!(cursor, Point { row: 0, column: 6 });
    assert_eq!(highlights, vec![6..12]);
}

#[gpui::test]
fn escape_in_search_tab_keeps_find_widget(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.rs", "needle\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("a.rs")), OpenMode::Pinned, cx)
    });
    draw(cx);
    keys("cmd-f", cx);
    let code = active_view(&wb, cx);
    let shown = |cx: &mut VisualTestContext| code.read_with(cx, |v, _| v.find_widget().is_some());
    assert!(shown(cx));

    keys("cmd-shift-f", cx);
    keys("escape", cx);
    assert!(
        shown(cx),
        "Esc in the Search tab leaves the find widget open"
    );
    // Esc moved focus from the query to the results list.
    assert!(holds(&wb, "searchViewletFocus && !textInputFocus", cx));

    // The command alone, run with focus outside the editor, does nothing.
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("closeFindWidget", None, window, cx)
        })
    });
    draw(cx);
    assert!(shown(cx));
    assert!(holds(&wb, "searchViewletFocus && !textInputFocus", cx));
}

#[gpui::test]
fn status_bar_search_button_opens_tab(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    click("panel-toggle-search", cx);
    assert_eq!(active_item(&wb, cx), Some(TabItem::Search));
    assert!(holds(&wb, "searchViewletFocus && textInputFocus", cx));
    // Clicking it again keeps the one tab.
    click("panel-toggle-search", cx);
    assert_eq!(search_tabs(&wb, cx), 1);
    assert_eq!(active_item(&wb, cx), Some(TabItem::Search));
}

#[gpui::test]
fn view_search_command_opens_tab(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    let ran = cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("workbench.view.search", None, window, cx)
        })
    });
    assert!(ran);
    draw(cx);
    assert_eq!(active_item(&wb, cx), Some(TabItem::Search));
    assert!(holds(&wb, "searchViewletFocus && textInputFocus", cx));
}

#[gpui::test]
fn sidebar_has_only_explorer(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let data = TempDir::new().unwrap();
    // A state file from before the Search panel moved into a tab.
    let state = WorkspaceState {
        version: 1,
        sidebar_visible: false,
        sidebar_width: 300.,
        sidebar_view: "search".into(),
        groups: GroupsState {
            groups: vec![GroupState {
                tabs: Vec::new(),
                active: None,
            }],
            active_group: 0,
        },
        scroll: Default::default(),
        group_widths: Vec::new(),
        search: Default::default(),
    };
    save_state(&state_path(data.path(), dir.path()), &state).unwrap();
    let (root, data_dir) = (dir.path().to_path_buf(), data.path().to_path_buf());
    let (wb, cx) =
        cx.add_window_view(move |window, cx| Workbench::with_data_dir(root, data_dir, window, cx));
    finish_scan(&wb, cx);
    let sidebar = wb.read_with(cx, |wb, _| wb.active_sidebar());
    // Exhaustive: the Explorer is the only sidebar view.
    match sidebar {
        SidebarView::Explorer => {}
    }
    assert!(!wb.read_with(cx, |wb, _| wb.sidebar_visible()));

    keys("cmd-shift-e", cx);
    wb.read_with(cx, |wb, _| {
        assert!(wb.sidebar_visible());
        assert_eq!(wb.active_sidebar(), SidebarView::Explorer);
    });
    assert!(holds(&wb, "explorerViewletFocus", cx));
    assert!(cx.debug_bounds("sidebar-header").is_none());
}

#[gpui::test]
fn closing_focused_search_tab_focuses_editor_area(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    assert!(holds(&wb, "searchViewletFocus", cx));
    keys("cmd-w", cx);
    assert_eq!(search_tabs(&wb, cx), 0);
    assert!(holds(&wb, "editorFocus && !searchViewletFocus", cx));
    assert!(!holds(&wb, "textInputFocus", cx));
}

/// R29: in single-file mode the folder is never walked, so Find in Files
/// sends no search and says so.
#[gpui::test]
fn search_is_unavailable_in_single_file_mode(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "needle\n"), ("b.txt", "needle\n")]);
    let (root, file) = (dir.path().to_path_buf(), dir.path().join("a.txt"));
    let (wb, cx) =
        cx.add_window_view(move |window, cx| Workbench::single_file(root, &file, None, window, cx));
    draw(cx);
    keys("cmd-shift-f", cx);
    assert_eq!(active_item(&wb, cx), Some(TabItem::Search));
    let view = search(&wb, cx);
    type_text("needle", cx);
    settle_search(cx);
    keys("enter", cx);
    settle_search(cx);
    assert!(files(&view, cx).is_empty(), "no search ran");
    assert_eq!(
        summary(&view, cx).as_deref(),
        Some("Search is unavailable when a single file is opened")
    );
    assert!(cx.debug_bounds("search-summary").is_some());
}

fn highlights_of(
    view: &Entity<SearchView>,
    path: &str,
    cx: &mut VisualTestContext,
) -> Option<Vec<(u32, usize)>> {
    cx.update(|_, cx| {
        view.read(cx).highlights(Path::new(path)).map(|h| {
            let mut lines: Vec<_> = h.iter().map(|(n, s)| (*n, s.len())).collect();
            lines.sort();
            lines
        })
    })
}

#[gpui::test]
fn visible_rust_result_gets_highlights(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("main.rs", "fn main() {}\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    let view = search(&wb, cx);
    type_text("main", cx);
    settle_search(cx);
    draw(cx);
    let lines = highlights_of(&view, "main.rs", cx).expect("the visible file was highlighted");
    assert!(
        lines.iter().any(|(n, spans)| *n == 1 && *spans > 0),
        "{lines:?}"
    );
}

#[gpui::test]
fn new_search_drops_old_highlights(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.rs", "fn alpha() {}\n"), ("b.rs", "fn beta() {}\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    let view = search(&wb, cx);
    type_text("alpha", cx);
    settle_search(cx);
    draw(cx);
    assert!(highlights_of(&view, "a.rs", cx).is_some());

    // Typing again clears the results and the colors with them; the
    // highlights of the first search never show up for the second.
    keys("backspace", cx);
    keys("backspace", cx);
    keys("backspace", cx);
    keys("backspace", cx);
    keys("backspace", cx);
    type_text("beta", cx);
    cx.executor().advance_clock(DEBOUNCE);
    cx.update(|_, _| {});
    // Before the frame that draws the new rows: nothing for a.rs.
    assert!(highlights_of(&view, "a.rs", cx).is_none());
    draw(cx);
    assert!(highlights_of(&view, "a.rs", cx).is_none());
    assert!(highlights_of(&view, "b.rs", cx).is_some());
}

#[gpui::test]
fn at_most_three_highlight_jobs_run(cx: &mut TestAppContext) {
    init(cx);
    let files: Vec<(String, String)> = (0..10)
        .map(|i| (format!("f{i}.rs"), format!("fn needle{i}() {{}}\n")))
        .collect();
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let dir = project(&refs);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    let view = search(&wb, cx);
    type_text("needle", cx);
    settle_search(cx);
    draw(cx);
    let (peak, now) = view.read_with(cx, |v, _| {
        (v.peak_highlight_jobs(), v.highlight_jobs_in_flight())
    });
    assert_eq!(peak, 3, "three at once, never more");
    assert_eq!(now, 0, "all done once settled");
    for i in 0..10 {
        assert!(highlights_of(&view, &format!("f{i}.rs"), cx).is_some());
    }
}

/// The active tab's file, relative to the project `dir`.
fn active_path(
    dir: &TempDir,
    wb: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> Option<PathBuf> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .active_tab()
            .and_then(|t| {
                t.path()
                    .map(|p| p.strip_prefix(dir.path()).unwrap_or(p).to_path_buf())
            })
    })
}

fn cursor_row(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> u32 {
    active_view(wb, cx).read_with(cx, |v, _| v.cursor().row)
}

/// Searches `needle` in [`needles`] and returns the Search tab's view.
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
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(move |window, cx| Workbench::new(root, window, cx));
    finish_scan(&wb, cx);
    keys("cmd-shift-f", cx);
    let view = search(&wb, cx);
    type_text("needle", cx);
    settle_search(cx);
    (dir, wb, view, cx)
}

#[gpui::test]
fn alt_cmd_c_toggles_case_in_search_tab(cx: &mut TestAppContext) {
    let (dir, wb, view, cx) = searched(cx);
    assert_eq!(files(&view, cx).len(), 3);
    keys("alt-cmd-c", cx);
    settle_search(cx);
    assert!(view.read_with(cx, |v, _| v.case_sensitive()));
    assert_eq!(files(&view, cx), vec![(p("a.rs"), 1), (p("b.rs"), 2)]);
    keys("alt-cmd-w", cx);
    keys("alt-cmd-r", cx);
    assert!(view.read_with(cx, |v, _| v.whole_word() && v.is_regex()));

    // The same keys in an editor leave the search alone.
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("a.rs")), OpenMode::Pinned, cx)
    });
    draw(cx);
    assert!(holds(&wb, "editorFocus && !searchViewletFocus", cx));
    keys("alt-cmd-c", cx);
    keys("alt-cmd-w", cx);
    keys("alt-cmd-r", cx);
    assert!(view.read_with(cx, |v, _| v.case_sensitive()
        && v.whole_word()
        && v.is_regex()));
    drop(dir);
}

#[gpui::test]
fn shift_cmd_j_shows_filter_row(cx: &mut TestAppContext) {
    let (_dir, _wb, _view, cx) = searched(cx);
    assert!(cx.debug_bounds("search-include").is_none());
    keys("shift-cmd-j", cx);
    assert!(cx.debug_bounds("search-include").is_some());
    keys("shift-cmd-j", cx);
    assert!(cx.debug_bounds("search-include").is_none());
}

#[gpui::test]
fn f4_walks_matches_across_files_and_wraps(cx: &mut TestAppContext) {
    let (dir, wb, _view, cx) = searched(cx);
    // Match lines: a.rs:0, b.rs:0, b.rs:2, sub/c.rs:0.
    let expect = [
        ("a.rs", 0),
        ("b.rs", 0),
        ("b.rs", 2),
        ("sub/c.rs", 0),
        ("a.rs", 0),
    ];
    for (file, row) in expect {
        keys("f4", cx);
        assert_eq!(active_path(&dir, &wb, cx), Some(p(file)));
        assert_eq!(cursor_row(&wb, cx), row, "{file}");
    }
    keys("shift-f4", cx);
    assert_eq!(active_path(&dir, &wb, cx), Some(p("sub/c.rs")));
}

#[gpui::test]
fn f4_expands_collapsed_file(cx: &mut TestAppContext) {
    let (dir, wb, view, cx) = searched(cx);
    click("search-collapse-all", cx);
    assert_eq!(row_count(&view, cx), 3);
    keys("f4", cx);
    assert_eq!(active_path(&dir, &wb, cx), Some(p("a.rs")));
    assert_eq!(row_count(&view, cx), 4, "a.rs expanded, the others not");
    assert!(view.read_with(cx, |v, _| v.selected().is_some()));
}

#[gpui::test]
fn arrows_and_enter_open_selected_line(cx: &mut TestAppContext) {
    let (dir, wb, view, cx) = searched(cx);
    keys("escape", cx);
    assert_eq!(view.read_with(cx, |v, _| v.selected()), None);
    keys("down", cx);
    let sel = |cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.selected().unwrap());
    let at = |file, line| (file, 0, line);
    let key = |r: lucerna_workbench::SearchLineRef| (r.file, r.excerpt, r.line);
    assert_eq!(key(sel(cx)), at(0, 0), "a.rs line, header skipped");
    keys("down", cx);
    assert_eq!(key(sel(cx)), at(1, 0), "b.rs, header skipped");
    keys("down", cx);
    assert_eq!(key(sel(cx)), at(1, 1), "context lines can be selected");
    keys("up", cx);
    keys("up", cx);
    assert_eq!(key(sel(cx)), at(0, 0));
    keys("up", cx);
    assert_eq!(key(sel(cx)), at(0, 0), "stops at the first line");
    keys("down down", cx);
    keys("enter", cx);
    assert_eq!(active_path(&dir, &wb, cx), Some(p("b.rs")));
    assert_eq!(cursor_row(&wb, cx), 1);
}

#[gpui::test]
fn escape_swaps_focus_between_query_and_list(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = searched(cx);
    let query_focused = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            view.read(cx)
                .query_input()
                .focus_handle(cx)
                .is_focused(window)
        })
    };
    let list_focused = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| view.read(cx).list_focus_handle().is_focused(window))
    };
    assert!(query_focused(cx));
    keys("escape", cx);
    assert!(list_focused(cx) && !query_focused(cx));
    assert!(holds(&wb, "searchViewletFocus", cx));
    keys("escape", cx);
    assert!(query_focused(cx) && !list_focused(cx));
}

#[gpui::test]
fn collapse_all_then_expand_all(cx: &mut TestAppContext) {
    let (_dir, wb, view, cx) = searched(cx);
    assert_eq!(row_count(&view, cx), 8);
    run(&wb, "search.action.collapseSearchResults", cx);
    assert_eq!(row_count(&view, cx), 3);
    click("search-collapse-all", cx);
    assert_eq!(
        row_count(&view, cx),
        8,
        "the button expands when all are folded"
    );
    run(&wb, "search.action.refreshSearchResults", cx);
    assert_eq!(files(&view, cx).len(), 3);
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.run_command(command, None, window, cx)));
    draw(cx);
}

#[gpui::test]
fn selection_follows_its_line_when_files_arrive_before_it(cx: &mut TestAppContext) {
    let (dir, _wb, view, cx) = searched(cx);
    keys("escape down down", cx);
    let sel = view.read_with(cx, |v, _| v.selected().unwrap());
    assert_eq!((sel.file, sel.line), (1, 0));
    // A new file sorting first, as streaming would insert it.
    let line = LineMatch {
        line_number: 1,
        line_text: "needle".into(),
        ranges: std::iter::once(0..6).collect(),
    };
    view.update(cx, |v, cx| {
        v.apply_event(
            SearchEvent::File {
                path: Arc::from(Path::new("0.rs")),
                excerpts: vec![Excerpt { lines: vec![line] }],
            },
            cx,
        )
    });
    let sel = view.read_with(cx, |v, _| v.selected().unwrap());
    assert_eq!((sel.file, sel.line), (2, 0), "still b.rs's first line");
    drop(dir);
}

fn tab_title(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .search_title()
            .map(|t| t.to_string())
    })
}

#[gpui::test]
fn tab_title_follows_query(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    assert_eq!(tab_title(&wb, cx), None, "an empty query says \"Search\"");
    type_text("needle", cx);
    assert_eq!(tab_title(&wb, cx).as_deref(), Some("needle"));
    keys("cmd-a backspace", cx);
    assert_eq!(tab_title(&wb, cx), None);
}

/// A state file with a Search tab and the given search inputs.
fn saved_search(root: &Path, data: &Path, search: SearchState, with_tab: bool) {
    let tabs = if with_tab {
        vec![(PathBuf::from(SEARCH_TAB_STATE), true)]
    } else {
        Vec::new()
    };
    let state = WorkspaceState {
        version: 1,
        sidebar_visible: true,
        sidebar_width: 300.,
        sidebar_view: "explorer".into(),
        groups: GroupsState {
            groups: vec![GroupState {
                active: with_tab.then_some(0),
                tabs,
            }],
            active_group: 0,
        },
        scroll: Default::default(),
        group_widths: Vec::new(),
        search,
    };
    save_state(&state_path(data, root), &state).unwrap();
}

fn needle_state(query: &str) -> SearchState {
    SearchState {
        query: query.into(),
        is_regex: true,
        include: "*.rs".into(),
        filters_visible: true,
        ..Default::default()
    }
}

#[gpui::test]
fn search_state_round_trips_and_reruns(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.rs", "needle\n"), ("b.txt", "needle\n")]);
    let data = TempDir::new().unwrap();
    let (root, data_dir) = (dir.path().to_path_buf(), data.path().to_path_buf());
    let (wb, vcx) = cx.add_window_view({
        let (root, data_dir) = (root.clone(), data_dir.clone());
        move |window, cx| Workbench::with_data_dir(root, data_dir, window, cx)
    });
    finish_scan(&wb, vcx);
    keys("cmd-shift-f", vcx);
    type_text("nee.le", vcx);
    keys("alt-cmd-r", vcx);
    keys("shift-cmd-j", vcx);
    type_text("*.rs", vcx);
    settle_search(vcx);
    vcx.update(|_, cx| wb.update(cx, |wb, cx| wb.save_workspace_state(cx)));
    let saved = wb.read_with(vcx, |wb, cx| wb.search_view().read(cx).state(cx));
    assert_eq!(saved, needle_state("nee.le"));
    let on_disk = lucerna_workbench::load_state(&state_path(&data_dir, &root)).unwrap();
    assert_eq!(on_disk.search, saved);

    let (wb, vcx) =
        cx.add_window_view(move |window, cx| Workbench::with_data_dir(root, data_dir, window, cx));
    finish_scan(&wb, vcx);
    settle_search(vcx);
    let view = search(&wb, vcx);
    assert_eq!(view.read_with(vcx, |v, cx| v.state(cx)), saved);
    assert_eq!(tab_title(&wb, vcx).as_deref(), Some("nee.le"));
    let found = files(&view, vcx);
    assert_eq!(found.len(), 1, "the include filter is back: {found:?}");
    assert!(found[0].0.ends_with("a.rs"));
}

#[gpui::test]
fn restored_search_in_single_file_mode_does_not_search(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.txt", "needle\n"), ("b.txt", "needle\n")]);
    let data = TempDir::new().unwrap();
    saved_search(dir.path(), data.path(), needle_state("needle"), true);
    let (root, file) = (dir.path().to_path_buf(), dir.path().join("a.txt"));
    let data_dir = data.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(move |window, cx| {
        Workbench::single_file(root, &file, Some(data_dir), window, cx)
    });
    draw(cx);
    settle_search(cx);
    let view = search(&wb, cx);
    assert!(files(&view, cx).is_empty(), "no search ran");
    assert_eq!(query(&view, cx), "needle");
}

#[gpui::test]
fn restored_empty_query_does_not_search(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let data = TempDir::new().unwrap();
    saved_search(dir.path(), data.path(), SearchState::default(), true);
    let (root, data_dir) = (dir.path().to_path_buf(), data.path().to_path_buf());
    let (wb, cx) =
        cx.add_window_view(move |window, cx| Workbench::with_data_dir(root, data_dir, window, cx));
    finish_scan(&wb, cx);
    settle_search(cx);
    assert_eq!(search_tabs(&wb, cx), 1);
    let view = search(&wb, cx);
    assert!(files(&view, cx).is_empty());
    assert_eq!(summary(&view, cx), None);
    assert_eq!(tab_title(&wb, cx), None);
}

#[gpui::test]
fn restored_query_without_search_tab_does_not_search(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let data = TempDir::new().unwrap();
    saved_search(dir.path(), data.path(), needle_state("needle"), false);
    let (root, data_dir) = (dir.path().to_path_buf(), data.path().to_path_buf());
    let (wb, cx) =
        cx.add_window_view(move |window, cx| Workbench::with_data_dir(root, data_dir, window, cx));
    finish_scan(&wb, cx);
    settle_search(cx);
    let view = search(&wb, cx);
    assert!(files(&view, cx).is_empty());
    assert_eq!(query(&view, cx), "needle");
}
