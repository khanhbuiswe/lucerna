//! Go Back / Go Forward through GPUI's test platform with real keystrokes.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, Workbench};
use tempfile::TempDir;

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

/// `src/main.rs`, `src/lib.rs`, `README.md` and a 100-line `long.txt`.
/// Three 100-line files.
/// Opens the workbench once its scan finished.
fn open<'a>(
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

/// Lets background work finish and draws, so the focused input has
/// registered its input handler.
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

fn project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for file in ["a.rs", "b.rs", "c.rs"] {
        let text: String = (1..=100).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.path().join(file), text).unwrap();
    }
    dir
}

fn p(name: &str) -> Arc<Path> {
    Arc::from(Path::new(name))
}

fn jump(wb: &Entity<Workbench>, name: &str, row: u32, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| {
        wb.jump_to(p(name), Point { row, column: 0 }, OpenMode::Pinned, cx)
    });
    draw(cx);
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

/// File name, cursor and scroll row of the active editor.
fn here(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> (String, Point, u32) {
    let view = active_view(wb, cx);
    cx.update(|_, cx| {
        let v = view.read(cx);
        (
            v.path().file_name().unwrap().to_string_lossy().into_owned(),
            v.cursor(),
            v.scroll_top_row(),
        )
    })
}

fn row(r: u32) -> Point {
    Point { row: r, column: 0 }
}

#[gpui::test]
fn back_and_forward_across_files(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);

    jump(&wb, "a.rs", 0, cx);
    jump(&wb, "b.rs", 39, cx);
    let (name, cursor, b_scroll) = here(&wb, cx);
    assert_eq!((name.as_str(), cursor), ("b.rs", row(39)));
    assert!(b_scroll > 0, "row 40 is scrolled into view");

    keys("ctrl--", cx);
    let (name, cursor, _) = here(&wb, cx);
    assert_eq!((name.as_str(), cursor), ("a.rs", row(0)));

    keys("ctrl-shift--", cx);
    assert_eq!(here(&wb, cx), ("b.rs".into(), row(39), b_scroll));

    // Nothing further forward; Back and Forward stay put at the ends.
    keys("ctrl-shift--", cx);
    assert_eq!(here(&wb, cx), ("b.rs".into(), row(39), b_scroll));
    keys("ctrl--", cx);
    keys("ctrl--", cx);
    assert_eq!(here(&wb, cx).0, "a.rs");
}

#[gpui::test]
fn same_file_jumps_are_recorded_past_ten_rows(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);

    jump(&wb, "a.rs", 0, cx);
    jump(&wb, "a.rs", 5, cx);
    jump(&wb, "a.rs", 50, cx);
    keys("ctrl--", cx);
    assert_eq!(here(&wb, cx).1, row(5), "the 5-row hop was not recorded");
    keys("ctrl--", cx);
    assert_eq!(here(&wb, cx).1, row(5), "nothing is left to go back to");
}

#[gpui::test]
fn back_opens_a_pinned_tab(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);

    jump(&wb, "a.rs", 0, cx);
    wb.update(cx, |wb, cx| {
        wb.jump_to(p("b.rs"), row(0), OpenMode::Preview, cx)
    });
    draw(cx);
    keys("ctrl--", cx);
    let pinned: Vec<(String, bool)> = cx.update(|_, cx| {
        wb.read(cx).editor_groups().read(cx).model().groups()[0]
            .tabs
            .iter()
            .map(|t| (t.title(), t.pinned))
            .collect()
    });
    assert!(pinned.contains(&("a.rs".into(), true)), "{pinned:?}");
}

#[gpui::test]
fn back_skips_deleted_file(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);

    jump(&wb, "a.rs", 20, cx);
    jump(&wb, "b.rs", 0, cx);
    jump(&wb, "c.rs", 0, cx);
    std::fs::remove_file(dir.path().join("b.rs")).unwrap();

    keys("ctrl--", cx);
    assert_eq!(here(&wb, cx).0, "a.rs", "b.rs is gone, so Back skips it");
    keys("ctrl-shift--", cx);
    assert_eq!(here(&wb, cx).0, "c.rs");
}

#[gpui::test]
fn go_to_line_is_recorded(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);

    jump(&wb, "a.rs", 0, cx);
    keys("ctrl-g", cx);
    cx.simulate_input("50");
    keys("enter", cx);
    assert_eq!(here(&wb, cx).1, row(49));

    keys("ctrl--", cx);
    assert_eq!(here(&wb, cx).1, row(0));
}

#[gpui::test]
fn quick_open_file_is_recorded(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);

    jump(&wb, "a.rs", 30, cx);
    keys("cmd-p", cx);
    cx.simulate_input("b.rs");
    keys("enter", cx);
    assert_eq!(here(&wb, cx).0, "b.rs");

    keys("ctrl--", cx);
    assert_eq!(
        (here(&wb, cx).0.as_str(), here(&wb, cx).1),
        ("a.rs", row(30))
    );
}
