//! Replace in project: the Search tab's replace bar, per-line and per-file
//! replace and dismiss, driven through GPUI's test platform with real
//! keystrokes.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, Entity, Focusable, Modifiers, TestAppContext, VisualTestContext};
use lucerna_project::{Excerpt, LineKind, LineMatch, SearchQuery, WorktreeEvent};
use lucerna_settings::{Keymap, Layer, SettingsStore, parse_when};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, ExcerptList, ListSelection, OpenMode, ReplacePreview, SearchLineRef, SearchView,
    WindowSettings, Workbench,
};
use tempfile::TempDir;

const DEBOUNCE: Duration = Duration::from_millis(150);

fn init(cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
    // No language servers: the trust modal (opened by the first `.rs`
    // file) would take the focus these tests follow.
    let mut settings = SettingsStore::with_defaults();
    settings
        .set_layer(Layer::User, r#"{ "lucerna.lsp.enabled": false }"#)
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
