//! Git change bars in the editor gutter (spec §4.6) and the next/previous
//! change commands (Alt+F5 / Shift+Alt+F5).

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, Focusable, TestAppContext, VisualTestContext};
use lucerna_git::{Hunk, HunkKind};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, Workbench};
use support::git::{git, isolated_git, repo};
use tempfile::TempDir;

/// Twelve committed lines, `l0` to `l11`.
const LINES: &str = "l0\nl1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\n";

fn init(cx: &mut TestAppContext, user_settings: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    if let Some(json) = user_settings {
        settings.set_layer(Layer::User, json).unwrap();
    }
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
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

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn settle(cx: &mut VisualTestContext) {
    draw(cx);
    cx.run_until_parked();
    draw(cx);
}

/// Lets the 50 ms hunk debounce run out and draws.
fn after_debounce(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(50));
    settle(cx);
}

/// A repo with committed `c.txt` ([`LINES`]); `prepare` runs on the repo
/// before the workbench opens. The workbench is trusted and has scanned.
fn workbench<'a>(
    settings: Option<&str>,
    prepare: impl FnOnce(&Path),
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    PathBuf,
    Entity<Workbench>,
    &'a mut VisualTestContext,
) {
    init(cx, settings);
    let (tmp, dir) = repo();
    std::fs::write(dir.join("c.txt"), LINES).unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "c"]);
    prepare(&dir);
    let root = dir.clone();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    wb.update(cx, |wb, cx| {
        wb.git().update(cx, |git, cx| {
            git.use_git_for_test(isolated_git());
            git.set_trusted(true, cx);
        });
    });
    settle(cx);
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&lucerna_project::WorktreeEvent::ScanComplete, cx)
    });
    settle(cx);
    (tmp, dir, wb, cx)
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

/// Opens `name`, focuses its view and lets its hunks arrive.
fn open(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) -> Entity<CodeView> {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new(name)), OpenMode::Pinned, cx)
    });
    settle(cx);
    let view = active_view(wb, cx);
    cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    after_debounce(cx);
    view
}

fn hunks(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Vec<Hunk> {
    cx.update(|_, cx| view.read(cx).diff_hunks().to_vec())
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.run_command(command, None, window, cx)));
    settle(cx);
}

fn rescan(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| wb.git().update(cx, |g, cx| g.rescan(cx)));
    after_debounce(cx);
}

fn set_cursor(view: &Entity<CodeView>, row: u32, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.set_cursor(Point { row, column: 1 }, cx));
    settle(cx);
}

fn cursor(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Point {
    cx.update(|_, cx| view.read(cx).cursor())
}

fn bars(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> usize {
    draw(cx);
    cx.update(|_, cx| view.read(cx).painted_hunk_bars())
}

/// `c.txt` on disk with rows 2 and 10 changed.
fn two_changes(dir: &Path) {
    let text = LINES.replace("l2\n", "x2\n").replace("l10\n", "x10\n");
    std::fs::write(dir.join("c.txt"), text).unwrap();
}

#[gpui::test]
fn edit_shows_bar_then_commit_clears_it(cx: &mut TestAppContext) {
    let (_t, dir, wb, cx) = workbench(None, |_| {}, cx);
    let view = open(&wb, "c.txt", cx);
    assert!(hunks(&view, cx).is_empty());

    set_cursor(&view, 3, cx);
    cx.simulate_input("X");
    settle(cx);
    after_debounce(cx);
    assert_eq!(
        hunks(&view, cx),
        vec![Hunk {
            kind: HunkKind::Modified,
            rows: 3..4,
            base_rows: 3..4,
        }]
    );
    assert_eq!(bars(&view, cx), 1);

    run(&wb, "workbench.action.files.save", cx);
    git(&dir, &["commit", "-q", "-am", "edit"]);
    rescan(&wb, cx);
    assert!(hunks(&view, cx).is_empty());
    assert_eq!(bars(&view, cx), 0);
}

#[gpui::test]
fn deleted_hunk_has_empty_rows(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(
        None,
        |dir| std::fs::write(dir.join("c.txt"), LINES.replace("l4\n", "")).unwrap(),
        cx,
    );
    let view = open(&wb, "c.txt", cx);
    let h = hunks(&view, cx);
    assert_eq!(h.len(), 1, "{h:?}");
    assert_eq!(h[0].kind, HunkKind::Deleted);
    assert_eq!(h[0].rows, 4..4);
    assert_eq!(bars(&view, cx), 1);
}

#[gpui::test]
fn untracked_file_has_no_hunks(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(
        None,
        |dir| std::fs::write(dir.join("new.txt"), "a\nb\n").unwrap(),
        cx,
    );
    let view = open(&wb, "new.txt", cx);
    assert!(hunks(&view, cx).is_empty());
    assert_eq!(bars(&view, cx), 0);
}

#[gpui::test]
fn next_change_wraps(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(None, two_changes, cx);
    let view = open(&wb, "c.txt", cx);
    let starts: Vec<u32> = hunks(&view, cx).iter().map(|h| h.rows.start).collect();
    assert_eq!(starts, vec![2, 10]);

    set_cursor(&view, 11, cx);
    run(&wb, "workbench.action.editor.nextChange", cx);
    assert_eq!(cursor(&view, cx), Point { row: 2, column: 0 });

    set_cursor(&view, 11, cx);
    run(&wb, "workbench.action.editor.previousChange", cx);
    assert_eq!(cursor(&view, cx), Point { row: 10, column: 0 });
    run(&wb, "workbench.action.editor.previousChange", cx);
    assert_eq!(cursor(&view, cx), Point { row: 2, column: 0 });
    run(&wb, "workbench.action.editor.previousChange", cx);
    assert_eq!(cursor(&view, cx), Point { row: 10, column: 0 });
    run(&wb, "workbench.action.editor.nextChange", cx);
    assert_eq!(cursor(&view, cx), Point { row: 2, column: 0 });
}

#[gpui::test]
fn next_change_without_hunks_says_so(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(None, |_| {}, cx);
    let view = open(&wb, "c.txt", cx);
    set_cursor(&view, 5, cx);
    assert!(!view.update(cx, |v, cx| v.next_change(true, cx)));
    run(&wb, "workbench.action.editor.nextChange", cx);
    assert_eq!(cursor(&view, cx), Point { row: 5, column: 1 });
    let message =
        cx.update(|_, cx| lucerna_workbench::status_message::current(cx).map(|m| m.to_string()));
    assert_eq!(message.as_deref(), Some("No changes"));
}

#[gpui::test]
fn alt_f5_bound(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(None, two_changes, cx);
    let view = open(&wb, "c.txt", cx);
    set_cursor(&view, 5, cx);
    cx.simulate_keystrokes("alt-f5");
    settle(cx);
    assert_eq!(cursor(&view, cx), Point { row: 10, column: 0 });
    cx.simulate_keystrokes("shift-alt-f5");
    settle(cx);
    assert_eq!(cursor(&view, cx), Point { row: 2, column: 0 });
}

#[gpui::test]
fn diff_decorations_none_hides_bars(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(
        Some(r#"{ "scm.diffDecorations": "none" }"#),
        two_changes,
        cx,
    );
    let view = open(&wb, "c.txt", cx);
    assert_eq!(hunks(&view, cx).len(), 2);
    assert_eq!(bars(&view, cx), 0);
}

#[gpui::test]
fn diff_decorations_gutter_shows_bars(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(
        Some(r#"{ "scm.diffDecorations": "gutter" }"#),
        two_changes,
        cx,
    );
    let view = open(&wb, "c.txt", cx);
    assert_eq!(bars(&view, cx), 2);
}

#[gpui::test]
fn second_view_gets_the_hunks(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(None, two_changes, cx);
    let view = open(&wb, "c.txt", cx);
    assert_eq!(hunks(&view, cx).len(), 2);
    let docs = cx.update(|_, cx| wb.read(cx).documents().clone());
    let second = docs.update(cx, |d, cx| d.open(Arc::from(Path::new("c.txt")), cx));
    assert_eq!(hunks(&second, cx).len(), 2);
}

#[gpui::test]
fn bars_follow_set_diff_hunks(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(None, |_| {}, cx);
    let view = open(&wb, "c.txt", cx);
    let hunk = |kind, rows: std::ops::Range<u32>| Hunk {
        kind,
        rows,
        base_rows: 0..0,
    };
    view.update(cx, |v, cx| {
        v.set_diff_hunks(
            Arc::from(vec![
                hunk(HunkKind::Added, 0..2),
                hunk(HunkKind::Deleted, 4..4),
                hunk(HunkKind::Modified, 6..9),
            ]),
            cx,
        )
    });
    assert_eq!(bars(&view, cx), 3);
    view.update(cx, |v, cx| v.clear_diff_hunks(cx));
    assert_eq!(bars(&view, cx), 0);
}

#[gpui::test]
fn closed_fold_shows_one_bar_for_its_changes(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(
        None,
        |dir| {
            std::fs::write(dir.join("f.txt"), "a\n  b\n  c\n  d\n  e\nz\n").unwrap();
            git(dir, &["add", "."]);
            git(dir, &["commit", "-q", "-m", "f"]);
            std::fs::write(dir.join("f.txt"), "a\n  B\n  c\n  e\nz\n").unwrap();
        },
        cx,
    );
    let view = open(&wb, "f.txt", cx);
    let kinds: Vec<HunkKind> = hunks(&view, cx).iter().map(|h| h.kind).collect();
    assert_eq!(kinds, vec![HunkKind::Modified, HunkKind::Deleted]);
    assert_eq!(bars(&view, cx), 2);
    view.update(cx, |v, cx| v.toggle_fold(0, cx));
    assert_eq!(bars(&view, cx), 1);
}

#[gpui::test]
fn vim_brackets_go_to_changes(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(Some(r#"{ "vim.enabled": true }"#), two_changes, cx);
    let view = open(&wb, "c.txt", cx);
    cx.simulate_keystrokes("] c");
    settle(cx);
    assert_eq!(cursor(&view, cx).row, 2);
    cx.simulate_keystrokes("] c");
    settle(cx);
    assert_eq!(cursor(&view, cx).row, 10);
    cx.simulate_keystrokes("[ c");
    settle(cx);
    assert_eq!(cursor(&view, cx).row, 2);
}
