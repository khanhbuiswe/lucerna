//! The Explorer marks a file that another program changed (bold name and a
//! dot), but never for Lucerna's own saves, a rewrite with the same bytes,
//! or the file in the active tab.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, Workbench};
use tempfile::TempDir;

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

/// A workbench on a temporary folder holding `a.txt` (`a`) and `b.txt`.
fn workbench<'a>(
    a: &str,
    cx: &'a mut TestAppContext,
) -> (TempDir, Entity<Workbench>, &'a mut VisualTestContext) {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), a).unwrap();
    fs::write(dir.path().join("b.txt"), "b\n").unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::ScanComplete, cx)
    });
    settle(cx);
    (dir, wb, cx)
}

/// Opens `name` pinned and focuses its view.
fn open(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new(name)), OpenMode::Pinned, cx)
    });
    settle(cx);
    let view = cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("the active tab has a body")
            .downcast::<CodeView>()
            .expect("tab bodies are code views")
    });
    cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    settle(cx);
}

/// The watcher reports that `name` was modified.
fn report(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) {
    let change = Change {
        path: Arc::from(Path::new(name)),
        kind: ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
}

/// Another program writes `text` to `name`, and the watcher reports it.
fn external_write(
    wb: &Entity<Workbench>,
    dir: &TempDir,
    name: &str,
    text: &str,
    cx: &mut VisualTestContext,
) {
    fs::write(dir.path().join(name), text).unwrap();
    report(wb, name, cx);
    settle(cx);
}

fn recently_changed(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .model()
            .is_recently_changed(Path::new(name), Instant::now())
    })
}

#[gpui::test]
fn own_save_is_not_marked(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, cx) = workbench("a\n", cx);
    open(&wb, "a.txt", cx);
    cx.simulate_input("x");
    settle(cx);
    cx.simulate_keystrokes("cmd-s");
    settle(cx);
    report(&wb, "a.txt", cx);
    settle(cx);
    assert!(!recently_changed(&wb, "a.txt", cx));
}

#[gpui::test]
fn own_save_of_background_file_is_not_marked(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, cx) = workbench("a\n", cx);
    open(&wb, "a.txt", cx);
    cx.simulate_input("x");
    settle(cx);
    cx.simulate_keystrokes("cmd-s");
    settle(cx);
    open(&wb, "b.txt", cx);
    report(&wb, "a.txt", cx);
    settle(cx);
    assert!(!recently_changed(&wb, "a.txt", cx));
}

#[gpui::test]
fn agent_change_to_background_open_file_is_marked(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = workbench("a\n", cx);
    open(&wb, "a.txt", cx);
    open(&wb, "b.txt", cx);
    external_write(&wb, &dir, "a.txt", "other\n", cx);
    assert!(recently_changed(&wb, "a.txt", cx));
}

#[gpui::test]
fn same_bytes_rewrite_of_open_file_is_not_marked(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = workbench("a\n", cx);
    open(&wb, "a.txt", cx);
    open(&wb, "b.txt", cx);
    external_write(&wb, &dir, "a.txt", "a\n", cx);
    assert!(!recently_changed(&wb, "a.txt", cx));
}

#[gpui::test]
fn agent_change_merging_into_dirty_background_file_is_marked(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = workbench("a\nb\nc\nd\ne\n", cx);
    open(&wb, "a.txt", cx);
    cx.simulate_input("x");
    settle(cx);
    open(&wb, "b.txt", cx);
    external_write(&wb, &dir, "a.txt", "a\nb\nc\nd\nE\n", cx);
    assert!(recently_changed(&wb, "a.txt", cx));
}

#[gpui::test]
fn agent_change_to_active_file_is_not_marked(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = workbench("a\n", cx);
    open(&wb, "a.txt", cx);
    external_write(&wb, &dir, "a.txt", "other\n", cx);
    assert!(!recently_changed(&wb, "a.txt", cx));
}

#[gpui::test]
fn changed_on_disk_respects_setting(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "explorer.decorations.externalChanges": false }"#),
    );
    let (dir, wb, cx) = workbench("a\n", cx);
    open(&wb, "a.txt", cx);
    open(&wb, "b.txt", cx);
    external_write(&wb, &dir, "a.txt", "other\n", cx);
    assert!(!recently_changed(&wb, "a.txt", cx));
}

#[gpui::test]
fn unopened_file_is_marked_at_once(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = workbench("a\n", cx);
    fs::write(dir.path().join("b.txt"), "other\n").unwrap();
    report(&wb, "b.txt", cx);
    assert!(recently_changed(&wb, "b.txt", cx));
}

#[gpui::test]
fn own_changes_are_not_decorated(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = workbench("a\n", cx);
    fs::write(dir.path().join("new.rs"), "x\n").unwrap();
    fs::write(dir.path().join("other.rs"), "y\n").unwrap();
    wb.update(cx, |wb, _| {
        wb.expect_own_changes([Arc::from(Path::new("new.rs"))])
    });
    let added = |name: &str| Change {
        path: Arc::from(Path::new(name)),
        kind: ChangeKind::Added,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(
            &WorktreeEvent::Changed(vec![added("new.rs"), added("other.rs")]),
            cx,
        )
    });
    settle(cx);
    assert!(!recently_changed(&wb, "new.rs", cx));
    assert!(recently_changed(&wb, "other.rs", cx));
}

#[gpui::test]
fn own_directory_covers_paths_under_it(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = workbench("a\n", cx);
    fs::create_dir_all(dir.path().join("d/e")).unwrap();
    wb.update(cx, |wb, _| {
        wb.expect_own_changes([Arc::from(Path::new("d"))])
    });
    let added = |name: &str| Change {
        path: Arc::from(Path::new(name)),
        kind: ChangeKind::Added,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![added("d"), added("d/e")]), cx)
    });
    settle(cx);
    assert!(!recently_changed(&wb, "d/e", cx));
}
