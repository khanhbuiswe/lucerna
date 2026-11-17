//! Quick Open inside the workbench: files, commands and go to line,
//! driven through GPUI's test platform with real keystrokes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Context, Keymap, SettingsStore, parse_when};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, QuickOpen, Workbench};
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

/// `src/main.rs`, `src/lib.rs`, `README.md` and a 100-line `long.txt`.
fn project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    for file in ["src/main.rs", "src/lib.rs", "README.md"] {
        std::fs::write(dir.path().join(file), "fn main() {}\n").unwrap();
    }
    let long: String = (1..=100).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.path().join("long.txt"), long).unwrap();
    dir
}

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

fn type_text(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    draw(cx);
}

fn quick_open(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<Entity<QuickOpen>> {
    cx.update(|_, cx| wb.read(cx).quick_open().cloned())
}

fn rows(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    let qo = quick_open(wb, cx).expect("quick open is shown");
    cx.update(|_, cx| qo.read(cx).row_texts())
}

fn query(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> String {
    let qo = quick_open(wb, cx).expect("quick open is shown");
    cx.update(|_, cx| qo.read(cx).query(cx))
}

fn context(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Context {
    cx.update(|window, cx| wb.read(cx).context_keys(window, cx))
}

fn holds(ctx: &Context, when: &str) -> bool {
    parse_when(when).unwrap().eval(ctx)
}

/// `(group, path, pinned)` of every tab.
fn tabs(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<(usize, PathBuf, bool)> {
    cx.update(|_, cx| {
        let model = wb.read(cx).editor_groups().read(cx).model();
        model
            .groups()
            .iter()
            .enumerate()
            .flat_map(|(g, group)| {
                group
                    .tabs
                    .iter()
                    .map(move |t| (g, t.path().unwrap().to_path_buf(), t.pinned))
            })
            .collect()
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
            .expect("tab bodies are code views")
    })
}

fn cursor(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Point {
    let view = active_view(wb, cx);
    cx.update(|_, cx| view.read(cx).cursor())
}

fn open_long_txt(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("long.txt")), OpenMode::Pinned, cx)
    });
    draw(cx);
}

#[gpui::test]
fn cmd_p_opens_and_enter_opens_preview(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);

    keys("cmd-p", cx);
    assert_eq!(query(&wb, cx), "");
    // An empty query lists every file.
    assert_eq!(rows(&wb, cx).len(), 4, "{:?}", rows(&wb, cx));

    type_text("mainrs", cx);
    assert_eq!(rows(&wb, cx)[0], "main.rs src");
    keys("enter", cx);

    assert!(quick_open(&wb, cx).is_none(), "Enter closes the picker");
    let main = dir.path().join("src/main.rs");
    assert_eq!(tabs(&wb, cx), vec![(0, main.clone(), false)]);
    assert_eq!(
        cx.update(|_, cx| wb.read(cx).recent_files().to_vec()),
        vec![Arc::<Path>::from(main)],
        "recent files are absolute"
    );

    // The recently opened file now comes first for an empty query.
    keys("cmd-p", cx);
    assert_eq!(rows(&wb, cx)[0], "main.rs src");
}

#[gpui::test]
fn up_down_wrap_and_click_activates(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);

    keys("cmd-p", cx);
    type_text("rs", cx);
    let listed = rows(&wb, cx);
    assert_eq!(listed.len(), 2, "{listed:?}");
    let selected = |cx: &mut VisualTestContext| {
        let qo = quick_open(&wb, cx).unwrap();
        cx.update(|_, cx| qo.read(cx).selected_index())
    };
    assert_eq!(selected(cx), 0);
    keys("up", cx);
    assert_eq!(selected(cx), 1, "up from the first row wraps to the last");
    keys("down", cx);
    assert_eq!(selected(cx), 0, "down from the last row wraps to the first");

    let row = cx
        .debug_bounds("quick-open-row-1")
        .expect("row 1 is drawn")
        .center();
    cx.simulate_click(row, Modifiers::none());
    draw(cx);
    assert!(quick_open(&wb, cx).is_none());
    let second = listed[1].split(' ').next().unwrap().to_string();
    let opened = tabs(&wb, cx);
    assert_eq!(opened.len(), 1);
    assert!(opened[0].1.ends_with(&second), "{opened:?} vs {second}");
}

#[gpui::test]
fn cmd_enter_opens_to_side(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("README.md")), OpenMode::Pinned, cx)
    });
    draw(cx);

    keys("cmd-p", cx);
    type_text("lib", cx);
    keys("cmd-enter", cx);

    assert!(quick_open(&wb, cx).is_none());
    assert_eq!(
        tabs(&wb, cx),
        vec![
            (0, dir.path().join("README.md"), true),
            (1, dir.path().join("src/lib.rs"), true),
        ]
    );
}

#[gpui::test]
fn commands_mode_runs_toggle_sidebar(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    assert!(wb.read_with(cx, |wb, _| wb.sidebar_visible()));

    keys("cmd-shift-p", cx);
    assert_eq!(query(&wb, cx), ">");
    let all = rows(&wb, cx);
    assert!(
        all.iter().all(|r| !r.contains("Close Quick Open")),
        "closeQuickOpen is not listed: {all:?}"
    );
    // The keybinding label sits on the right of a bound command.
    assert!(
        all.iter()
            .any(|r| r == "View: Toggle Primary Side Bar Visibility ⌘B"),
        "{all:?}"
    );

    type_text("primary side bar", cx);
    assert_eq!(
        rows(&wb, cx)[0],
        "View: Toggle Primary Side Bar Visibility ⌘B"
    );
    keys("enter", cx);

    assert!(quick_open(&wb, cx).is_none());
    assert!(!wb.read_with(cx, |wb, _| wb.sidebar_visible()));
}

#[gpui::test]
fn commands_match_ids(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    keys("cmd-shift-p", cx);
    type_text("toggleWordWrap", cx);
    assert_eq!(rows(&wb, cx)[0], "View: Toggle Word Wrap ⌥Z");
}

#[gpui::test]
fn goto_line_moves_view(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    open_long_txt(&wb, cx);

    keys("ctrl-g", cx);
    assert_eq!(query(&wb, cx), ":");
    assert_eq!(rows(&wb, cx), vec!["Type a line number…"]);
    type_text("50", cx);
    assert_eq!(rows(&wb, cx), vec!["Go to line 50"]);
    keys("enter", cx);

    assert!(quick_open(&wb, cx).is_none());
    assert_eq!(cursor(&wb, cx), Point { row: 49, column: 0 });
    let view = active_view(&wb, cx);
    let top = cx.update(|_, cx| view.read(cx).scroll_top_row());
    assert!(top > 0 && top <= 49, "line 50 is scrolled into view: {top}");

    keys("ctrl-g", cx);
    type_text("3:5", cx);
    assert_eq!(rows(&wb, cx), vec!["Go to line 3, column 5"]);
    keys("enter", cx);
    assert_eq!(cursor(&wb, cx), Point { row: 2, column: 4 });

    // Out-of-range lines and columns clamp.
    keys("ctrl-g", cx);
    type_text("999:99", cx);
    keys("enter", cx);
    assert_eq!(
        cursor(&wb, cx),
        Point {
            row: 99,
            column: "line 100".len() as u32
        }
    );
}

#[gpui::test]
fn goto_line_negative_and_comma(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    open_long_txt(&wb, cx);

    keys("ctrl-g", cx);
    type_text("-1", cx);
    assert_eq!(rows(&wb, cx), vec!["Go to line 100"]);
    keys("enter", cx);
    assert_eq!(cursor(&wb, cx), Point { row: 99, column: 0 });

    keys("ctrl-g", cx);
    type_text("12,5", cx);
    assert_eq!(rows(&wb, cx), vec!["Go to line 12, column 5"]);
    keys("enter", cx);
    assert_eq!(cursor(&wb, cx), Point { row: 11, column: 4 });

    keys("ctrl-g", cx);
    type_text("-500", cx);
    assert_eq!(rows(&wb, cx), vec!["Go to line 1"]);
    keys("escape", cx);

    keys("ctrl-g", cx);
    type_text("0", cx);
    assert_eq!(rows(&wb, cx), vec!["Go to line 1"]);
}

#[gpui::test]
fn goto_line_without_editor(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    keys("ctrl-g", cx);
    type_text("5", cx);
    assert_eq!(rows(&wb, cx), vec!["Open a file to go to a line"]);
    keys("enter", cx);
    assert!(tabs(&wb, cx).is_empty());
}

#[gpui::test]
fn escape_closes_and_context_key_resets(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    assert!(holds(&context(&wb, cx), "editorFocus && !inQuickOpen"));

    keys("cmd-p", cx);
    let ctx = context(&wb, cx);
    assert!(holds(&ctx, "inQuickOpen && textInputFocus && !editorFocus"));

    keys("escape", cx);
    assert!(quick_open(&wb, cx).is_none());
    let ctx = context(&wb, cx);
    assert!(holds(
        &ctx,
        "!inQuickOpen && !textInputFocus && editorFocus"
    ));
}

#[gpui::test]
fn click_outside_closes(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    keys("cmd-p", cx);
    assert!(quick_open(&wb, cx).is_some());
    // The bottom-left corner of the window, far from the popover.
    cx.simulate_click(point(px(5.), px(590.)), Modifiers::none());
    draw(cx);
    assert!(quick_open(&wb, cx).is_none());
    assert!(!holds(&context(&wb, cx), "inQuickOpen"));
}

#[gpui::test]
fn reopening_switches_mode(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    keys("cmd-p", cx);
    type_text("abc", cx);
    keys("cmd-shift-p", cx);
    assert_eq!(query(&wb, cx), ">");
}

#[gpui::test]
fn stale_file_results_are_dropped(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    keys("cmd-p", cx);
    let qo = quick_open(&wb, cx).unwrap();
    let input = cx.update(|_, cx| qo.read(cx).input().clone());
    // The file match for "main" is still running on the background
    // executor when the query switches to commands.
    input.update_in(cx, |input, window, cx| {
        input.set_text("main", window, cx);
        input.set_text(">word wrap", window, cx);
    });
    draw(cx);
    assert_eq!(rows(&wb, cx)[0], "View: Toggle Word Wrap ⌥Z");
}

#[gpui::test]
fn click_elsewhere_keeps_the_clicked_focus(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    keys("cmd-p", cx);
    // Focus moved by something other than Quick Open, then it closes.
    wb.update_in(cx, |wb, window, cx| {
        wb.focus_sidebar(window, cx);
        wb.close_quick_open(window, cx);
    });
    draw(cx);
    assert!(holds(&context(&wb, cx), "sideBarFocus && !editorFocus"));
}

/// Files opened by absolute path (`luc file`, search results) rank first
/// too: recents are absolute, the tree is root-relative.
#[gpui::test]
fn files_opened_by_absolute_path_are_recent(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let lib: Arc<Path> = Arc::from(dir.path().join("src/lib.rs").as_path());
    wb.update(cx, |wb, cx| wb.open_path(lib, OpenMode::Pinned, cx));
    draw(cx);
    keys("cmd-p", cx);
    assert_eq!(rows(&wb, cx)[0], "lib.rs src");
}
