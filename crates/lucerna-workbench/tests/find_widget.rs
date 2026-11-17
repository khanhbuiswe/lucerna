//! Find in file: the pure matcher and the find widget over a code view,
//! driven through GPUI's test platform with real keystrokes.

use std::fs;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, Focusable, Modifiers, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{
    Context, ContextValue, Keymap, Resolution, SettingsStore, parse_keys, parse_when,
};
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

#[gpui::test]
fn find_widget_is_a_full_width_bar_above_the_text(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, _view, cx) = open_file("a.txt", "foo\nbar foo\n", cx);
    let text_before = cx.debug_bounds("code-text-area").expect("text area");
    assert!(cx.debug_bounds("find-widget").is_none());

    keys("cmd-f", cx);
    let code = cx.debug_bounds("code-view").expect("code view");
    let bar = cx.debug_bounds("find-widget").expect("find widget");
    let text = cx.debug_bounds("code-text-area").expect("text area");
    assert!(
        (bar.size.width - code.size.width).abs() < gpui::px(1.),
        "the bar spans the code view: {bar:?} vs {code:?}"
    );
    assert!((bar.origin.x - code.origin.x).abs() < gpui::px(1.));
    assert!(
        (bar.origin.y - code.origin.y).abs() < gpui::px(1.),
        "the bar is at the top: {bar:?} vs {code:?}"
    );
    assert!(
        text.origin.y >= bar.bottom() - gpui::px(1.),
        "the text starts below the bar: {text:?} vs {bar:?}"
    );
    assert!(text.origin.y > text_before.origin.y, "the bar pushes down");
}

fn match_label(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    let find = widget(view, cx).expect("the find widget is shown");
    cx.update(|_, cx| find.read(cx).match_label())
}

#[gpui::test]
fn find_bar_counter_and_error_border(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo\nbar foo\n", cx);
    keys("cmd-f", cx);
    assert_eq!(match_label(&view, cx), "0/0", "no query");
    assert!(cx.debug_bounds("find-query").is_some());
    assert!(cx.debug_bounds("find-query-invalid").is_none());

    type_text("foo", cx);
    assert_eq!(match_label(&view, cx), "1/2");
    click("find-next", cx);
    assert_eq!(match_label(&view, cx), "2/2", "› selects the next match");
    click("find-previous", cx);
    assert_eq!(
        match_label(&view, cx),
        "1/2",
        "‹ selects the previous match"
    );

    type_text("x", cx);
    assert_eq!(match_label(&view, cx), "0/0", "no match");
    assert!(
        cx.debug_bounds("find-query-invalid").is_some(),
        "no match: the box border shows the error color"
    );
    assert!(cx.debug_bounds("find-query").is_none());
}

#[gpui::test]
fn find_bar_matches_search_bar_metrics(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, _view, cx) = open_file("a.txt", "foo\n", cx);
    keys("cmd-f", cx);
    let bar = cx.debug_bounds("find-widget").expect("find bar");
    let query = cx.debug_bounds("find-query").expect("query box");
    let close = cx.debug_bounds("find-close").expect("close");
    let next = cx.debug_bounds("find-next").expect("next");
    // 40px row plus the 1px bottom border.
    assert!(
        (bar.size.height - gpui::px(41.)).abs() < gpui::px(1.),
        "{bar:?}"
    );
    assert!(
        (query.size.height - gpui::px(28.)).abs() < gpui::px(1.),
        "{query:?}"
    );
    assert!(
        query.size.width < bar.size.width * 0.75 && query.size.width > bar.size.width * 0.5,
        "the box takes about two thirds: {query:?} in {bar:?}"
    );
    assert!(next.origin.x > query.right(), "‹ › after the box");
    assert!(close.origin.x > next.right(), "× at the far right");
    assert!(bar.right() - close.right() < gpui::px(12.));
}

#[gpui::test]
fn find_bar_border_waits_for_background_search(cx: &mut TestAppContext) {
    init(cx);
    // Over 1 MB: searched on the background executor.
    let text = ("x".repeat(63) + "\n").repeat(20_000) + "the needle\n";
    let (_dir, _wb, view, cx) = open_file("big.txt", &text, cx);
    keys("cmd-f", cx);
    type_text("zzz", cx);
    assert!(cx.debug_bounds("find-query-invalid").is_some(), "no match");
    let find = widget(&view, cx).unwrap();
    // Change the query without letting the background search finish.
    cx.update(|window, cx| {
        let input = find.read(cx).input().clone();
        input.update(cx, |input, cx| input.set_text("needle", window, cx));
    });
    assert!(
        !find.read_with(cx, |f, _| f.is_invalid()),
        "the old query's empty result does not paint the new one red"
    );
    draw(cx);
    assert!(!find.read_with(cx, |f, _| f.is_invalid()));
    assert_eq!(match_label(&view, cx), "1/1");
}

fn find(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Entity<FindWidget> {
    widget(view, cx).expect("the find widget is shown")
}

fn replace_visible(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> bool {
    find(view, cx).read_with(cx, |f, _| f.replace_visible())
}

fn replace_text(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    let find = find(view, cx);
    cx.update(|_, cx| find.read(cx).replace_input().read(cx).text().to_owned())
}

fn find_text(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    let find = find(view, cx);
    cx.update(|_, cx| find.read(cx).input().read(cx).text().to_owned())
}

#[gpui::test]
fn alt_cmd_f_shows_replace_row(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo bar\n", cx);
    keys("alt-cmd-f", cx);
    assert!(replace_visible(&view, cx));
    assert!(cx.debug_bounds("find-replace-row").is_some());
    assert!(cx.debug_bounds("find-replace-input").is_some());
    assert!(cx.debug_bounds("find-toggle-preserve-case").is_some());
    assert!(cx.debug_bounds("find-replace-next").is_some());
    assert!(cx.debug_bounds("find-replace-all").is_some());
    // No selection: focus is in the find input.
    type_text("foo", cx);
    assert_eq!(find_text(&view, cx), "foo");
    assert_eq!(counter(&view, cx), "1 of 1");
    keys("escape", cx);
    keys("cmd-f", cx);
    assert!(
        !replace_visible(&view, cx),
        "cmd-f on a hidden bar has no replace row"
    );
    assert!(cx.debug_bounds("find-replace-row").is_none());
}

#[gpui::test]
fn cmd_f_keeps_open_replace_row(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo bar\n", cx);
    keys("alt-cmd-f", cx);
    keys("cmd-f", cx);
    assert!(replace_visible(&view, cx));
    assert!(cx.debug_bounds("find-replace-row").is_some());
}

#[gpui::test]
fn alt_cmd_f_with_selection_focuses_replace(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo bar\n", cx);
    view.update(cx, |v, cx| v.select(pt(0, 4)..pt(0, 7), cx));
    keys("alt-cmd-f", cx);
    assert_eq!(find_text(&view, cx), "bar");
    type_text("x", cx);
    assert_eq!(replace_text(&view, cx), "x");
    assert_eq!(find_text(&view, cx), "bar");
}

#[gpui::test]
fn alt_cmd_f_again_moves_focus_to_replace(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo bar\n", cx);
    keys("alt-cmd-f", cx);
    type_text("foo", cx);
    keys("alt-cmd-f", cx);
    type_text("x", cx);
    assert_eq!(find_text(&view, cx), "foo");
    assert_eq!(replace_text(&view, cx), "x");
}

#[gpui::test]
fn toggle_replace_button_and_tab(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo bar\n", cx);
    keys("cmd-f", cx);
    assert!(cx.debug_bounds("find-replace-row").is_none());
    click("find-toggle-replace", cx);
    assert!(cx.debug_bounds("find-replace-row").is_some());
    type_text("a", cx);
    assert_eq!(
        (replace_text(&view, cx), find_text(&view, cx)),
        ("a".into(), "".into())
    );
    keys("shift-tab", cx);
    type_text("b", cx);
    assert_eq!(
        (replace_text(&view, cx), find_text(&view, cx)),
        ("a".into(), "b".into())
    );
    keys("tab", cx);
    type_text("c", cx);
    assert_eq!(
        (replace_text(&view, cx), find_text(&view, cx)),
        ("ac".into(), "b".into())
    );
    click("find-toggle-replace", cx);
    assert!(cx.debug_bounds("find-replace-row").is_none());
    type_text("d", cx);
    assert_eq!(find_text(&view, cx), "bd");
    assert_eq!(replace_text(&view, cx), "ac");
}

#[gpui::test]
fn replace_text_survives_close(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo bar\n", cx);
    keys("alt-cmd-f", cx);
    keys("alt-cmd-f", cx);
    type_text("x", cx);
    assert_eq!(replace_text(&view, cx), "x");
    keys("escape", cx);
    assert!(widget(&view, cx).is_none(), "escape closes the bar");
    keys("alt-cmd-f", cx);
    assert_eq!(replace_text(&view, cx), "x");
}

#[gpui::test]
fn readonly_view_has_no_replace(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo bar\n", cx);
    view.update(cx, |v, _| v.set_passive(true));
    keys("alt-cmd-f", cx);
    assert!(widget(&view, cx).is_some(), "the bar still shows");
    assert!(!replace_visible(&view, cx));
    assert!(cx.debug_bounds("find-toggle-replace").is_none());
    assert!(cx.debug_bounds("find-replace-row").is_none());
}

#[gpui::test]
fn replace_input_focused_context_key(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, wb, _view, cx) = open_file("a.txt", "foo bar\n", cx);
    let key = |wb: &Entity<Workbench>, cx: &mut VisualTestContext| {
        let ctx = cx.update(|window, cx| wb.read(cx).context_keys(window, cx));
        parse_when("replaceInputFocussed").unwrap().eval(&ctx)
    };
    keys("alt-cmd-f", cx);
    assert!(!key(&wb, cx));
    keys("alt-cmd-f", cx);
    assert!(key(&wb, cx));
    keys("shift-tab", cx);
    assert!(!key(&wb, cx));
}

/// Focuses the Replace input and types `text` into it.
fn fill_replace(view: &Entity<CodeView>, text: &str, cx: &mut VisualTestContext) {
    let find = find(view, cx);
    cx.update(|window, cx| {
        let focus = find.read(cx).replace_input().focus_handle(cx);
        window.focus(&focus, cx);
    });
    type_text(text, cx);
}

/// Sets the Replace input's text without moving focus.
fn set_replace(view: &Entity<CodeView>, text: &str, cx: &mut VisualTestContext) {
    let find = find(view, cx);
    cx.update(|window, cx| {
        let input = find.read(cx).replace_input().clone();
        input.update(cx, |input, cx| input.set_text(text, window, cx));
    });
    draw(cx);
}

fn buffer_text(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        let s = view.read(cx).snapshot();
        s.text_for_range(0..s.len_bytes())
    })
}

fn undo(view: &Entity<CodeView>, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| {
        v.undo(true, cx);
    });
    draw(cx);
}

#[gpui::test]
fn replace_one_replaces_active_and_moves_on(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo\nbar foo\nfoo\n", cx);
    keys("alt-cmd-f", cx);
    type_text("foo", cx);
    keys("tab", cx);
    type_text("X", cx);
    assert_eq!(replace_text(&view, cx), "X");
    keys("enter", cx);
    assert_eq!(buffer_text(&view, cx), "X\nbar foo\nfoo\n");
    assert_eq!(counter(&view, cx), "1 of 2");
    assert_eq!(
        view.read_with(cx, |v, _| v.selection()),
        Some(pt(1, 4)..pt(1, 7))
    );
    undo(&view, cx);
    assert_eq!(
        buffer_text(&view, cx),
        "foo\nbar foo\nfoo\n",
        "one undo step"
    );
}

#[gpui::test]
fn replace_one_wraps_to_first(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo a foo", cx);
    view.update(cx, |v, cx| v.go_to(pt(0, 4), cx));
    keys("alt-cmd-f", cx);
    type_text("foo", cx);
    assert_eq!(counter(&view, cx), "2 of 2");
    keys("tab", cx);
    type_text("X", cx);
    keys("enter", cx);
    assert_eq!(buffer_text(&view, cx), "foo a X");
    assert_eq!(counter(&view, cx), "1 of 1");
    let (ranges, active) = highlights(&view, cx);
    assert_eq!(
        (ranges.len(), ranges.first(), active),
        (1, Some(&(0..3)), Some(0))
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.selection()),
        Some(pt(0, 0)..pt(0, 3))
    );
}

#[gpui::test]
fn replace_one_only_selects_when_selection_moved(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo\nbar foo\nfoo\n", cx);
    keys("alt-cmd-f", cx);
    type_text("foo", cx);
    set_replace(&view, "X", cx);
    assert_eq!(counter(&view, cx), "1 of 3");
    view.update(cx, |v, cx| v.go_to(pt(0, 1), cx));
    draw(cx);
    keys("shift-cmd-1", cx);
    assert_eq!(buffer_text(&view, cx), "foo\nbar foo\nfoo\n");
    assert_eq!(counter(&view, cx), "2 of 3");
    assert_eq!(
        view.read_with(cx, |v, _| v.selection()),
        Some(pt(1, 4)..pt(1, 7))
    );
    // Now the selection is the match: the next press replaces it.
    keys("shift-cmd-1", cx);
    assert_eq!(buffer_text(&view, cx), "foo\nbar X\nfoo\n");
}

#[gpui::test]
fn replace_one_ignores_stale_match(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo bar foo", cx);
    keys("alt-cmd-f", cx);
    type_text("foo", cx);
    set_replace(&view, "X", cx);
    assert_eq!(counter(&view, cx), "1 of 2");
    let find = find(&view, cx);
    cx.update(|_, cx| {
        // An edit the widget has not searched yet; the selection (the
        // active match 0..3) stays.
        view.update(cx, |v, cx| {
            let selections = v.selections().clone();
            let plan = lucerna_editor::EditPlan::new(
                vec![(11..11, Arc::from("!"))],
                selections,
                lucerna_editor::EditKind::Other,
            );
            assert!(v.transact(plan, cx));
        });
        find.update(cx, |f, cx| f.replace_one(cx));
    });
    draw(cx);
    assert_eq!(buffer_text(&view, cx), "foo bar foo!", "nothing replaced");
}

#[gpui::test]
fn replace_all_is_one_undo_step(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo foo foo", cx);
    view.update(cx, |v, cx| {
        v.go_to(pt(0, 11), cx);
        v.type_text("z", cx);
    });
    draw(cx);
    assert_eq!(buffer_text(&view, cx), "foo foo fooz");
    keys("alt-cmd-f", cx);
    type_text("foo", cx);
    keys("tab", cx);
    type_text("b", cx);
    keys("cmd-enter", cx);
    assert_eq!(buffer_text(&view, cx), "b b bz");
    assert_eq!(counter(&view, cx), "No results");
    undo(&view, cx);
    assert_eq!(buffer_text(&view, cx), "foo foo fooz");
}

#[gpui::test]
fn alt_cmd_enter_and_shift_cmd_1_from_find_input(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo foo foo\n", cx);
    keys("alt-cmd-f", cx);
    set_replace(&view, "X", cx);
    type_text("foo", cx);
    assert_eq!(find_text(&view, cx), "foo", "focus is in the find input");
    keys("shift-cmd-1", cx);
    assert_eq!(buffer_text(&view, cx), "X foo foo\n");
    assert_eq!(counter(&view, cx), "1 of 2");
    keys("alt-cmd-enter", cx);
    assert_eq!(buffer_text(&view, cx), "X X X\n");
    assert_eq!(find_text(&view, cx), "foo");
    assert_eq!(replace_text(&view, cx), "X");
}

#[gpui::test]
fn enter_in_find_input_still_steps(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo\nbar foo\nfoo\n", cx);
    keys("alt-cmd-f", cx);
    set_replace(&view, "X", cx);
    type_text("foo", cx);
    keys("enter", cx);
    assert_eq!(counter(&view, cx), "2 of 3");
    assert_eq!(buffer_text(&view, cx), "foo\nbar foo\nfoo\n");
}

#[gpui::test]
fn replace_regex_groups(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "let a = 1;", cx);
    keys("alt-cmd-f", cx);
    click("find-toggle-regex", cx);
    let find = find(&view, cx);
    cx.update(|window, cx| {
        let input = find.read(cx).input().clone();
        input.update(cx, |input, cx| input.set_text(r"(\w+) = (\d)", window, cx));
    });
    draw(cx);
    assert_eq!(counter(&view, cx), "1 of 1");
    fill_replace(&view, "$2 = $1", cx);
    keys("cmd-enter", cx);
    assert_eq!(buffer_text(&view, cx), "let 1 = a;");
}

#[gpui::test]
fn replace_preserve_case(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "Foo foo", cx);
    keys("alt-cmd-f", cx);
    type_text("foo", cx);
    click("find-toggle-preserve-case", cx);
    assert!(find(&view, cx).read_with(cx, |f, _| f.preserve_case()));
    fill_replace(&view, "bar", cx);
    keys("cmd-enter", cx);
    assert_eq!(buffer_text(&view, cx), "Bar bar");
}

#[gpui::test]
fn readonly_view_does_not_replace(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo foo\n", cx);
    keys("cmd-f", cx);
    type_text("foo", cx);
    set_replace(&view, "X", cx);
    view.update(cx, |v, _| v.set_passive(true));
    keys("alt-cmd-enter", cx);
    keys("shift-cmd-1", cx);
    let find = find(&view, cx);
    cx.update(|_, cx| {
        find.update(cx, |f, cx| {
            f.replace_one(cx);
            f.replace_all(cx);
        })
    });
    draw(cx);
    assert_eq!(buffer_text(&view, cx), "foo foo\n");
}

#[gpui::test]
fn replace_all_large_file_off_main_thread(cx: &mut TestAppContext) {
    init(cx);
    let text = ("x".repeat(63) + "\n").repeat(20_000) + "the needle\n";
    let len = text.len();
    let (_dir, _wb, view, cx) = open_file("big.txt", &text, cx);
    keys("alt-cmd-f", cx);
    type_text("needle", cx);
    assert_eq!(counter(&view, cx), "1 of 1");
    fill_replace(&view, "pin", cx);
    keys("cmd-enter", cx);
    let after = buffer_text(&view, cx);
    assert!(
        after.ends_with("the pin\n"),
        "{:?}",
        &after[after.len() - 20..]
    );
    assert_eq!(after.len(), len - 3);
    assert_eq!(counter(&view, cx), "No results");
}

#[gpui::test]
fn toggle_keys_flip_find_options(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "Foo foo\n", cx);
    keys("cmd-f", cx);
    type_text("foo", cx);
    assert_eq!(counter(&view, cx), "1 of 2");
    keys("alt-cmd-c", cx);
    assert_eq!(counter(&view, cx), "1 of 1");
    keys("alt-cmd-c", cx);
    // The selection sits on the second match, which stays the nearest.
    assert_eq!(counter(&view, cx), "2 of 2");
    let find = find(&view, cx);
    let state = |cx: &mut VisualTestContext| {
        find.read_with(cx, |f, _| {
            (
                f.case_sensitive(),
                f.whole_word(),
                f.is_regex(),
                f.preserve_case(),
            )
        })
    };
    assert_eq!(state(cx), (false, false, false, false));
    keys("alt-cmd-w", cx);
    assert_eq!(state(cx), (false, true, false, false));
    keys("alt-cmd-r", cx);
    assert_eq!(state(cx), (false, true, true, false));
    keys("alt-cmd-p", cx);
    assert_eq!(state(cx), (false, true, true, true));
    keys("alt-cmd-w alt-cmd-r alt-cmd-p", cx);
    assert_eq!(state(cx), (false, false, false, false));
}

#[gpui::test]
fn toggle_keys_work_with_bar_hidden(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "Foo foo\n", cx);
    keys("alt-cmd-r", cx);
    assert!(
        widget(&view, cx).is_none(),
        "the toggle does not show the bar"
    );
    assert!(highlights(&view, cx).0.is_empty());
    assert_eq!(view.read_with(cx, |v, _| v.selection()), None);
    keys("cmd-f", cx);
    assert!(find(&view, cx).read_with(cx, |f, _| f.is_regex()));
    // A toggle on the shown bar keeps working after a close.
    keys("escape alt-cmd-r", cx);
    keys("cmd-f", cx);
    assert!(!find(&view, cx).read_with(cx, |f, _| f.is_regex()));
}

#[test]
fn alt_cmd_c_in_explorer_still_copies_path() {
    let (keymap, _) = Keymap::load(Keymap::default_bindings(), None);
    let resolve = |key: &str, flags: &[&str]| {
        let mut ctx = Context::default();
        for flag in flags {
            ctx.set(*flag, ContextValue::Bool(true));
        }
        match keymap.resolve(&parse_keys(key).unwrap(), &ctx) {
            Resolution::Command { command, .. } => command,
            other => panic!("expected a command, got {other:?}"),
        }
    };
    assert_eq!(
        resolve("alt+cmd+c", &["filesExplorerFocus"]),
        "copyFilePath"
    );
    assert_eq!(
        resolve("alt+cmd+r", &["filesExplorerFocus"]),
        "revealFileInOS"
    );
    assert_eq!(
        resolve("alt+cmd+c", &["editorFocus"]),
        "toggleFindCaseSensitive"
    );
    assert_eq!(
        resolve("alt+cmd+w", &["editorFocus"]),
        "toggleFindWholeWord"
    );
    assert_eq!(resolve("alt+cmd+r", &["editorFocus"]), "toggleFindRegex");
    assert_eq!(resolve("alt+cmd+p", &["editorFocus"]), "togglePreserveCase");
    assert_eq!(
        resolve("alt+cmd+c", &["searchViewletFocus"]),
        "toggleSearchCaseSensitive"
    );
}

#[gpui::test]
fn stale_pick_after_replace_does_not_leak_into_a_new_search(cx: &mut TestAppContext) {
    init(cx);
    let text = "foo\n".to_owned() + &("x".repeat(63) + "\n").repeat(20_000) + "foo\nfoo\n";
    let (_dir, _wb, view, cx) = open_file("big.txt", &text, cx);
    keys("alt-cmd-f", cx);
    type_text("foo", cx);
    assert_eq!(counter(&view, cx), "1 of 3");
    set_replace(&view, "X", cx);
    let find = find(&view, cx);
    // No parking in between: the replace's background search is still
    // running when the next search starts and drops it.
    find.update(cx, |f, cx| {
        f.replace_one(cx);
    });
    view.update(cx, |v, cx| v.go_to(pt(20_002, 0), cx));
    find.update(cx, |f, cx| f.toggle_case_sensitive(cx));
    draw(cx);
    assert_eq!(buffer_text(&view, cx).lines().next(), Some("X"));
    assert_eq!(
        find.read_with(cx, |f, _| f.active_index()),
        Some(1),
        "the match nearest the cursor, not the replace's stale offset"
    );
}
