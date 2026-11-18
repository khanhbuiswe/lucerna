//! Replace in project: the Search tab's replace bar, per-line and per-file
//! replace and dismiss, driven through GPUI's test platform with real
//! keystrokes.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, ListSelection, SearchView, Workbench};
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
