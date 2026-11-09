//! Find in file: the pure matcher and the find widget over a code view,
//! driven through GPUI's test platform with real keystrokes.

use std::fs;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, SettingsStore, parse_when};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::{Buffer, BufferSnapshot, Point};
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, FindWidget, OpenMode, Workbench, find_in_snapshot};
use tempfile::TempDir;

fn snap(text: &str) -> BufferSnapshot {
    Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot()
}

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

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
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

/// Writes `name` with `text` into a new project, opens it and lets it load.
fn open_file<'a>(
    name: &str,
    text: &str,
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<CodeView>,
    &'a mut VisualTestContext,
) {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join(name), text).unwrap();
    let (wb, cx) = workbench(dir.path(), cx);
    let path: Arc<Path> = Arc::from(Path::new(name));
    wb.update(cx, |wb, cx| wb.open_path(path, OpenMode::Pinned, cx));
    draw(cx);
    let view = active_view(&wb, cx);
    (dir, wb, view, cx)
}

fn widget(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Option<Entity<FindWidget>> {
    cx.update(|_, cx| view.read(cx).find_widget().cloned())
}

fn counter(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    let find = widget(view, cx).expect("the find widget is shown");
    cx.update(|_, cx| find.read(cx).counter())
}

fn highlights(
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
) -> (Vec<Range<usize>>, Option<usize>) {
    cx.update(|_, cx| {
        let v = view.read(cx);
        (v.match_highlights().to_vec(), v.active_match())
    })
}

fn find_widget_visible(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    let ctx = cx.update(|window, cx| wb.read(cx).context_keys(window, cx));
    parse_when("findWidgetVisible").unwrap().eval(&ctx)
}

fn pt(row: u32, column: u32) -> Point {
    Point { row, column }
}

#[test]
fn find_in_snapshot_literal_regex_case_word() {
    let s = snap("Foo foo food a.b axb\nfoo\n");
    let find = |pattern: &str, regex: bool, case: bool, word: bool| {
        find_in_snapshot(&s, pattern, regex, case, word).unwrap()
    };
    // Literal, case-insensitive by default.
    assert_eq!(
        find("foo", false, false, false),
        vec![0..3, 4..7, 8..11, 21..24]
    );
    assert_eq!(find("foo", false, true, false), vec![4..7, 8..11, 21..24]);
    assert_eq!(find("foo", false, false, true), vec![0..3, 4..7, 21..24]);
    assert_eq!(find("foo", false, true, true), vec![4..7, 21..24]);
    // A literal escapes regex syntax; a regex does not.
    assert_eq!(find("a.b", false, false, false), vec![13..16]);
    assert_eq!(find("a.b", true, false, false), vec![13..16, 17..20]);
    // `^` matches at every line start.
    assert_eq!(find("^foo", true, true, false), vec![21..24]);
    assert_eq!(find("^foo", true, false, false), vec![0..3, 21..24]);
    // Nothing to find, and empty matches are skipped.
    assert!(find("", false, false, false).is_empty());
    assert!(find("x*", true, false, false).iter().all(|r| !r.is_empty()));
    assert_eq!(find("x*", true, false, false), vec![18..19]);
}

#[test]
fn find_invalid_regex_err() {
    let s = snap("a (b) c\n");
    let err = find_in_snapshot(&s, "(", true, false, false).unwrap_err();
    assert!(!err.is_empty());
    // The same text as a literal is fine.
    assert_eq!(
        find_in_snapshot(&s, "(", false, false, false).unwrap(),
        vec![2..3]
    );
}

#[gpui::test]
fn find_widget_next_prev_wraps(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, wb, view, cx) = open_file("a.txt", "foo\nbar foo\nbaz\nfoo end\n", cx);
    assert!(!find_widget_visible(&wb, cx));

    keys("cmd-f", cx);
    assert!(widget(&view, cx).is_some(), "cmd-f shows the find widget");
    assert!(find_widget_visible(&wb, cx));
    type_text("foo", cx);
    assert_eq!(highlights(&view, cx), (vec![0..3, 8..11, 16..19], Some(0)));
    assert_eq!(counter(&view, cx), "1 of 3");

    keys("enter", cx);
    assert_eq!(counter(&view, cx), "2 of 3");
    let selection = view.read_with(cx, |v, _| v.selection());
    assert_eq!(selection, Some(pt(1, 4)..pt(1, 7)), "the match is selected");
    keys("enter", cx);
    assert_eq!(counter(&view, cx), "3 of 3");
    assert_eq!(view.read_with(cx, |v, _| v.cursor()), pt(3, 3));
    keys("enter", cx);
    assert_eq!(counter(&view, cx), "1 of 3", "next wraps to the first");
    keys("shift-enter", cx);
    assert_eq!(counter(&view, cx), "3 of 3", "previous wraps to the last");
    assert_eq!(highlights(&view, cx).1, Some(2));
    keys("shift-enter", cx);
    assert_eq!(counter(&view, cx), "2 of 3");

    keys("escape", cx);
    assert!(widget(&view, cx).is_none(), "escape closes the widget");
    assert!(!find_widget_visible(&wb, cx));
    assert_eq!(highlights(&view, cx), (vec![], None));
}

#[gpui::test]
fn find_widget_counts_none_and_shows_regex_error(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "Foo foo food\n", cx);
    keys("cmd-f", cx);
    assert_eq!(counter(&view, cx), "");
    type_text("foo", cx);
    assert_eq!(counter(&view, cx), "1 of 3");

    click("find-toggle-case", cx);
    assert_eq!(counter(&view, cx), "1 of 2");
    click("find-toggle-word", cx);
    assert_eq!(counter(&view, cx), "1 of 1");
    assert_eq!(highlights(&view, cx).0, vec![4..7]);
    click("find-toggle-word", cx);
    click("find-toggle-case", cx);

    type_text("x", cx);
    assert_eq!(counter(&view, cx), "No results");

    click("find-toggle-regex", cx);
    keys("backspace backspace backspace backspace", cx);
    type_text("(", cx);
    let find = widget(&view, cx).unwrap();
    let error = cx.update(|_, cx| find.read(cx).error().map(str::to_owned));
    assert!(error.is_some(), "an invalid regex reports its error");
    assert!(
        cx.debug_bounds("find-error").is_some(),
        "the error is shown under the input"
    );
    assert!(highlights(&view, cx).0.is_empty());
    keys("backspace", cx);
    type_text("fo+", cx);
    // The search starts at the selection: the `foo` the toggles selected.
    assert_eq!(counter(&view, cx), "2 of 3");
    assert!(cx.debug_bounds("find-error").is_none());
}

#[gpui::test]
fn find_widget_prefills_single_line_selection(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo\nbar foo\nbaz\n", cx);
    view.update(cx, |v, cx| v.select(pt(1, 0)..pt(1, 3), cx));
    keys("cmd-f", cx);
    let find = widget(&view, cx).unwrap();
    let text = cx.update(|_, cx| find.read(cx).input().read(cx).text().to_owned());
    assert_eq!(text, "bar");
    assert_eq!(counter(&view, cx), "1 of 1");

    // A selection over several lines leaves the query as it was.
    keys("escape", cx);
    view.update(cx, |v, cx| v.select(pt(0, 1)..pt(2, 1), cx));
    keys("cmd-f", cx);
    let find = widget(&view, cx).unwrap();
    let text = cx.update(|_, cx| find.read(cx).input().read(cx).text().to_owned());
    assert_eq!(text, "bar");
}

#[gpui::test]
fn find_matches_follow_reload(cx: &mut TestAppContext) {
    init(cx);
    let (dir, wb, view, cx) = open_file("a.txt", "foo\nfoo\n", cx);
    keys("cmd-f", cx);
    type_text("foo", cx);
    assert_eq!(counter(&view, cx), "1 of 2");

    fs::write(dir.path().join("a.txt"), "foo\nfoo\nfoo foo\n").unwrap();
    let change = Change {
        path: Arc::from(Path::new("a.txt")),
        kind: ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    draw(cx);
    assert_eq!(counter(&view, cx), "1 of 4");
    assert_eq!(highlights(&view, cx).0, vec![0..3, 4..7, 8..11, 12..15]);
}

#[gpui::test]
fn find_large_file_searches_off_the_main_thread(cx: &mut TestAppContext) {
    init(cx);
    // Over 1 MB: searched on the background executor.
    let mut text = "x".repeat(63) + "\n";
    text = text.repeat(20_000);
    text.push_str("the needle\n");
    let (_dir, _wb, view, cx) = open_file("big.txt", &text, cx);
    keys("cmd-f", cx);
    type_text("needle", cx);
    assert_eq!(counter(&view, cx), "1 of 1");
    let start = 64 * 20_000 + 4;
    assert_eq!(highlights(&view, cx).0, vec![start..start + 6]);
}
