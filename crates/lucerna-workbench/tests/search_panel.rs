//! The Search panel: debounced, cancellable, streaming project search
//! driven through GPUI's test platform with real keystrokes.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, SettingsStore, parse_when};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, SearchPanel, SidebarView, Workbench};
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

/// Opens the workbench once its scan finished.
fn workbench<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let root = root.to_path_buf();
    let (wb, cx) = cx.add_window_view(move |window, cx| Workbench::new(root, window, cx));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !wb.read_with(cx, |wb, _| wb.worktree().snapshot().scan_complete()) {
        assert!(Instant::now() < deadline, "worktree scan did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::ScanComplete, cx)
    });
    draw(cx);
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

fn panel(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<SearchPanel> {
    cx.update(|_, cx| wb.read(cx).search_panel().clone())
}

/// `(path, results)` of each file, in the order shown.
fn files(panel: &Entity<SearchPanel>, cx: &mut VisualTestContext) -> Vec<(PathBuf, usize)> {
    cx.update(|_, cx| {
        panel
            .read(cx)
            .files()
            .map(|(path, n)| (path.to_path_buf(), n))
            .collect()
    })
}

fn summary(panel: &Entity<SearchPanel>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| panel.read(cx).summary())
}

fn query(panel: &Entity<SearchPanel>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| panel.read(cx).query_input().read(cx).text().to_owned())
}

fn holds(wb: &Entity<Workbench>, when: &str, cx: &mut VisualTestContext) -> bool {
    let ctx = cx.update(|window, cx| wb.read(cx).context_keys(window, cx));
    parse_when(when).unwrap().eval(&ctx)
}

fn p(path: &str) -> PathBuf {
    PathBuf::from(path)
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
fn search_panel_streams_and_groups(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    wb.read_with(cx, |wb, _| {
        assert!(wb.sidebar_visible());
        assert_eq!(wb.active_sidebar(), SidebarView::Search);
    });
    assert!(holds(&wb, "searchViewletFocus && textInputFocus", cx));
    let panel = panel(&wb, cx);

    type_text("needle", cx);
    assert!(
        files(&panel, cx).is_empty(),
        "the search waits for the debounce"
    );
    settle_search(cx);
    assert_eq!(
        files(&panel, cx),
        vec![(p("a.rs"), 1), (p("b.rs"), 2), (p("sub/c.rs"), 1)],
        "files are grouped and sorted by path whatever order they arrive in"
    );
    assert_eq!(summary(&panel, cx).as_deref(), Some("4 results in 3 files"));
    // Three file rows and four line rows.
    assert_eq!(panel.read_with(cx, |p, _| p.row_count()), 7);

    click("search-file-1", cx);
    assert_eq!(
        panel.read_with(cx, |p, _| p.row_count()),
        5,
        "b.rs collapsed"
    );
    click("search-file-1", cx);
    assert_eq!(panel.read_with(cx, |p, _| p.row_count()), 7);

    // Case-sensitive drops `NEEDLE`.
    click("search-toggle-case", cx);
    settle_search(cx);
    assert_eq!(files(&panel, cx), vec![(p("a.rs"), 1), (p("b.rs"), 2)]);
    assert_eq!(summary(&panel, cx).as_deref(), Some("3 results in 2 files"));
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
    let panel = panel(&wb, cx);
    type_text("alpha", cx);

    // The alpha search is in flight when the query changes.
    cx.update(|window, cx| {
        panel.update(cx, |p, cx| p.run_search(cx));
        let input = panel.read(cx).query_input().clone();
        input.update(cx, |input, cx| input.set_text("beta", window, cx));
    });
    draw(cx);
    assert!(
        files(&panel, cx).is_empty(),
        "no result of the old query arrives after the change"
    );

    settle_search(cx);
    assert_eq!(files(&panel, cx), vec![(p("beta.txt"), 1)]);
    // Nothing of the old search shows up later either.
    cx.executor().advance_clock(Duration::from_secs(1));
    draw(cx);
    assert_eq!(files(&panel, cx), vec![(p("beta.txt"), 1)]);
    assert_eq!(summary(&panel, cx).as_deref(), Some("1 result in 1 file"));
}

#[gpui::test]
fn search_truncation_message(cx: &mut TestAppContext) {
    init(cx);
    let lines = "x\n".repeat(8_000);
    let dir = project(&[("1.txt", &lines), ("2.txt", &lines), ("3.txt", &lines)]);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    let panel = panel(&wb, cx);
    type_text("x", cx);
    settle_search(cx);

    let files = files(&panel, cx);
    let total: usize = files.iter().map(|(_, n)| n).sum();
    assert_eq!(total, 20_000);
    assert_eq!(
        summary(&panel, cx),
        Some(format!(
            "20000 results in {} files (truncated at 20,000)",
            files.len()
        ))
    );
    assert_eq!(
        panel.read_with(cx, |p, _| p.row_count()),
        files.len() + 20_000
    );
}

#[gpui::test]
fn click_result_opens_and_positions(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.rs", "first\n  let needle = 1;\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    type_text("needle", cx);
    settle_search(cx);
    let panel = panel(&wb, cx);
    assert_eq!(files(&panel, cx), vec![(p("a.rs"), 1)]);

    click("search-match-0-0", cx);
    let tabs: Vec<(PathBuf, bool)> = cx.update(|_, cx| {
        let model = wb.read(cx).editor_groups().read(cx).model();
        model.groups()[0]
            .tabs
            .iter()
            .map(|t| (t.path.to_path_buf(), t.pinned))
            .collect()
    });
    assert_eq!(
        tabs,
        vec![(dir.path().join("a.rs"), false)],
        "opened as a preview"
    );
    let view = active_view(&wb, cx);
    let (cursor, highlights, active) = view.read_with(cx, |v, _| {
        (v.cursor(), v.match_highlights().to_vec(), v.active_match())
    });
    assert_eq!(cursor, Point { row: 1, column: 6 });
    assert_eq!(highlights, vec![12..18]);
    assert_eq!(active, Some(0));

    // Clicking again on the open file moves within it.
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("a.rs")), OpenMode::Pinned, cx)
    });
    view.update(cx, |v, cx| v.go_to(Point { row: 0, column: 0 }, cx));
    draw(cx);
    click("search-match-0-0", cx);
    let cursor = active_view(&wb, cx).read_with(cx, |v, _| v.cursor());
    assert_eq!(cursor, Point { row: 1, column: 6 });
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
    let view = active_view(&wb, cx);
    let at = |row, column| Point { row, column };
    view.update(cx, |v, cx| v.select(at(1, 6)..at(1, 12), cx));

    keys("cmd-shift-f", cx);
    let panel = panel(&wb, cx);
    assert_eq!(query(&panel, cx), "needle");
    assert!(holds(&wb, "searchViewletFocus && textInputFocus", cx));
    settle_search(cx);
    assert_eq!(files(&panel, cx), vec![(p("a.rs"), 1)]);

    // A selection over two lines is not used.
    view.update(cx, |v, cx| v.select(at(0, 1)..at(1, 2), cx));
    keys("cmd-shift-f", cx);
    assert_eq!(query(&panel, cx), "needle");
}

#[gpui::test]
fn search_invalid_regex_shows_error(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    let panel = panel(&wb, cx);
    click("search-toggle-regex", cx);
    // Clicking a toggle leaves focus where it was.
    assert!(holds(&wb, "textInputFocus", cx));
    type_text("(", cx);
    settle_search(cx);
    let error = panel.read_with(cx, |p, _| p.error().map(str::to_owned));
    assert!(error.is_some(), "the regex error is reported");
    assert!(cx.debug_bounds("search-error").is_some());
    assert!(files(&panel, cx).is_empty());

    keys("backspace", cx);
    type_text("ne+dle", cx);
    settle_search(cx);
    assert!(panel.read_with(cx, |p, _| p.error().is_none()));
    assert!(cx.debug_bounds("search-error").is_none());
    assert_eq!(files(&panel, cx).len(), 3);
}

#[gpui::test]
fn search_include_exclude_filters(cx: &mut TestAppContext) {
    init(cx);
    let dir = needles();
    let (wb, cx) = workbench(dir.path(), cx);
    keys("cmd-shift-f", cx);
    let panel = panel(&wb, cx);
    type_text("needle", cx);
    settle_search(cx);
    assert_eq!(files(&panel, cx).len(), 3);

    let (include, exclude) = panel.read_with(cx, |p, _| {
        (p.include_input().clone(), p.exclude_input().clone())
    });
    cx.update(|window, cx| include.update(cx, |i, cx| i.set_text("sub/**, a.rs", window, cx)));
    settle_search(cx);
    assert_eq!(files(&panel, cx), vec![(p("a.rs"), 1), (p("sub/c.rs"), 1)]);

    cx.update(|window, cx| exclude.update(cx, |i, cx| i.set_text("sub", window, cx)));
    settle_search(cx);
    assert_eq!(files(&panel, cx), vec![(p("a.rs"), 1)]);
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
    let panel = panel(&wb, cx);
    assert_eq!(files(&panel, cx), vec![(p("a.txt"), 1)]);
    click("search-match-0-0", cx);
    let view = active_view(&wb, cx);
    view.read_with(cx, |v, _| (v.cursor(), v.match_highlights().to_vec()))
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
fn escape_in_search_panel_keeps_find_widget(cx: &mut TestAppContext) {
    init(cx);
    let dir = project(&[("a.rs", "needle\n")]);
    let (wb, cx) = workbench(dir.path(), cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("a.rs")), OpenMode::Pinned, cx)
    });
    draw(cx);
    keys("cmd-f", cx);
    let view = active_view(&wb, cx);
    let shown = |cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.find_widget().is_some());
    assert!(shown(cx));

    keys("cmd-shift-f", cx);
    keys("escape", cx);
    assert!(
        shown(cx),
        "Esc in the Search panel leaves the find widget open"
    );
    assert!(holds(&wb, "searchViewletFocus && textInputFocus", cx));

    // The command alone, run with focus outside the editor, does nothing.
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("closeFindWidget", None, window, cx)
        })
    });
    draw(cx);
    assert!(shown(cx));
    assert!(holds(&wb, "searchViewletFocus && textInputFocus", cx));
}
