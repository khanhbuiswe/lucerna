//! The status bar's warning when a file watcher could not start (the
//! worktree's or the `.git` directory's).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, Workbench};
use tempfile::TempDir;

/// Trusted, so opening a file raises no trust modal; no language servers,
/// whose threads would make the GPUI test scheduler non-deterministic.
const TRUSTED: &str = r#"{ "session.trustAllWorktrees": true, "lucerna.lsp.enabled": false }"#;

fn init(cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    settings.set_layer(Layer::User, TRUSTED).unwrap();
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
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

/// A project with `a.rs`, and its folder's name.
fn project() -> (TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
    let folder = dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    (dir, folder)
}

fn open<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let root = root.to_path_buf();
    let (wb, cx) = cx.add_window_view(move |window, cx| Workbench::new(root, window, cx));
    cx.update(|window, _| window.activate_window());
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
}

fn tooltip(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<String> {
    wb.read_with(cx, |wb, cx| wb.watch_status().read(cx).tooltip_text(cx))
}

fn watch_failed(wb: &Entity<Workbench>, reason: &str, cx: &mut VisualTestContext) {
    let ev = WorktreeEvent::WatchFailed(reason.into());
    wb.update(cx, |wb, cx| wb.on_worktree_event(&ev, cx));
}

#[gpui::test]
fn watch_off_item_hidden_by_default(cx: &mut TestAppContext) {
    init(cx);
    let (dir, _) = project();
    let (wb, cx) = open(dir.path(), cx);
    assert!(cx.debug_bounds("watch-off").is_none());
    assert_eq!(tooltip(&wb, cx), None);
}

#[gpui::test]
fn watch_off_item_shows_worktree_failure(cx: &mut TestAppContext) {
    init(cx);
    let (dir, folder) = project();
    let (wb, cx) = open(dir.path(), cx);
    watch_failed(&wb, "No path was found.", cx);
    draw(cx);
    assert!(cx.debug_bounds("watch-off").is_some());
    assert_eq!(
        tooltip(&wb, cx),
        Some(format!(
            "File watching is off for {folder}: No path was found. Changes made outside Lucerna won't show."
        ))
    );
}

#[gpui::test]
fn watch_off_item_shows_each_failure_once(cx: &mut TestAppContext) {
    init(cx);
    let (dir, _) = project();
    let (wb, cx) = open(dir.path(), cx);
    watch_failed(&wb, "No path was found.", cx);
    watch_failed(&wb, "again", cx);
    let git = wb.read_with(cx, |wb, _| wb.git().clone());
    git.update(cx, |g, cx| {
        g.fail_watch_for_test("OS file watch limit reached.", cx)
    });
    draw(cx);
    let text = tooltip(&wb, cx).expect("a warning");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].contains("again"), "{text}");
    assert!(!lines[0].contains("No path"), "{text}");
    assert_eq!(
        lines[1],
        "File watching is off for .git: OS file watch limit reached. Changes made outside Lucerna won't show."
    );
}
