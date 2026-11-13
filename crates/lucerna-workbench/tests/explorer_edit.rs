//! Inline name editing in the Explorer: New File… and New Folder….

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point, TestAppContext,
    VisualTestContext,
};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, Selection, Workbench};
use tempfile::TempDir;

const TRUSTED: &str = r#"{ "session.trustAllWorktrees": true }"#;

fn init(cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    settings.set_layer(Layer::User, TRUSTED).unwrap();
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

/// A project with `src/main.rs`, `a.rs` and `b.rs`.
fn project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    for file in ["src/main.rs", "a.rs", "b.rs"] {
        std::fs::write(dir.path().join(file), "fn main() {}\n").unwrap();
    }
    dir
}

fn open<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let root = root.to_path_buf();
    let (wb, cx) = cx.add_window_view(move |window, cx| Workbench::new(root, window, cx));
    // Focus events (the input's blur) only fire in an active window.
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

fn center(selector: &'static str, cx: &mut VisualTestContext) -> Point<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center()
}

fn click_at(position: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_click(position, Modifiers::none());
    draw(cx);
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let position = center(selector, cx);
    click_at(position, cx);
}

fn right_click(selector: &'static str, cx: &mut VisualTestContext) {
    let position = center(selector, cx);
    cx.simulate_event(MouseDownEvent {
        position,
        modifiers: Modifiers::none(),
        button: MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        modifiers: Modifiers::none(),
        button: MouseButton::Right,
        click_count: 1,
    });
    draw(cx);
}

fn type_text(cx: &mut VisualTestContext, text: &str) {
    cx.simulate_input(text);
    draw(cx);
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    draw(cx);
}

/// Clicks the editor area, to the right of the sidebar.
fn click_editor_area(cx: &mut VisualTestContext) {
    let explorer = cx.debug_bounds("explorer").expect("the explorer is drawn");
    let position = Point::new(explorer.right() + gpui::px(200.), explorer.center().y);
    click_at(position, cx);
}

fn root_of(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> PathBuf {
    cx.update(|_, cx| wb.read(cx).root().to_path_buf())
}

fn editing(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| wb.read(cx).explorer().read(cx).model().edit().is_some())
}

fn selection(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Selection {
    cx.update(|_, cx| wb.read(cx).explorer().read(cx).selection().clone())
}

fn expanded(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .model()
            .visible_rows()
            .iter()
            .any(|r| &*r.path == Path::new(path) && r.expanded && !r.edit)
    })
}

/// Waits for `cond`, drawing in between, for at most 10 s.
fn wait_until(cx: &mut VisualTestContext, mut cond: impl FnMut(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond(cx) {
        assert!(Instant::now() < deadline, "condition not met in time");
        std::thread::sleep(Duration::from_millis(10));
        draw(cx);
    }
}

/// Waits until the worktree has `paths`, then delivers their `Added` event.
fn deliver_added(wb: &Entity<Workbench>, paths: &[&str], cx: &mut VisualTestContext) {
    wait_until(cx, |cx| {
        wb.read_with(cx, |wb, _| {
            let snapshot = wb.worktree().snapshot();
            paths.iter().all(|p| snapshot.contains(Path::new(p)))
        })
    });
    let changes = paths
        .iter()
        .map(|p| Change {
            path: Arc::from(Path::new(p)),
            kind: ChangeKind::Added,
        })
        .collect();
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(changes), cx)
    });
    draw(cx);
}

fn run(wb: &Entity<Workbench>, command: &'static str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| wb.run_command(command, None, window, cx));
    });
    draw(cx);
}

/// `(path, pinned)` of each tab in the active group, relative to the root.
fn tabs(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<(PathBuf, bool)> {
    cx.update(|_, cx| {
        let root = wb.read(cx).root().to_path_buf();
        let model = wb.read(cx).editor_groups().read(cx).model();
        model.groups()[model.active_group()]
            .tabs
            .iter()
            .map(|t| {
                (
                    t.path().unwrap().strip_prefix(&root).unwrap().to_path_buf(),
                    t.pinned,
                )
            })
            .collect()
    })
}

#[gpui::test]
fn new_file_creates_and_opens(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-src", cx);
    click("context-menu-item-explorer.newFile", cx);
    assert!(editing(&wb, cx));
    assert!(cx.debug_bounds("explorer-edit-input").is_some());
    type_text(cx, "util.rs");
    keys("enter", cx);
    let root = root_of(&wb, cx);
    wait_until(cx, |_| root.join("src/util.rs").is_file());
    deliver_added(&wb, &["src/util.rs"], cx);
    assert!(!editing(&wb, cx));
    assert_eq!(tabs(&wb, cx), [(PathBuf::from("src/util.rs"), true)]);
    assert_eq!(
        selection(&wb, cx),
        Selection::Path(Arc::from(Path::new("src/util.rs")))
    );
    let changed = cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .model()
            .is_recently_changed(Path::new("src/util.rs"), Instant::now())
    });
    assert!(!changed);
}

#[gpui::test]
fn new_folder_multi_level(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-root", cx);
    run(&wb, "explorer.newFolder", cx);
    type_text(cx, "a/b");
    keys("enter", cx);
    let root = root_of(&wb, cx);
    wait_until(cx, |_| root.join("a/b").is_dir());
    deliver_added(&wb, &["a", "a/b"], cx);
    assert!(!editing(&wb, cx));
    assert!(expanded(&wb, "a", cx));
    assert!(expanded(&wb, "a/b", cx));
    assert_eq!(
        selection(&wb, cx),
        Selection::Path(Arc::from(Path::new("a/b")))
    );
}

#[gpui::test]
fn existing_name_shows_error_and_enter_does_nothing(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-root", cx);
    click("context-menu-item-explorer.newFile", cx);
    type_text(cx, "a.rs");
    assert!(cx.debug_bounds("explorer-edit-error").is_some());
    let error = cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .edit_error()
            .map(str::to_string)
    });
    let error = error.expect("an error");
    assert!(
        error.contains("A file or folder a.rs already exists"),
        "{error}"
    );
    keys("enter", cx);
    assert!(editing(&wb, cx));
    let entries = std::fs::read_dir(dir.path()).unwrap().count();
    assert_eq!(entries, 3, "nothing created");
}

#[gpui::test]
fn escape_cancels(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-src", cx);
    click("context-menu-item-explorer.newFile", cx);
    type_text(cx, "x.rs");
    keys("escape", cx);
    assert!(!editing(&wb, cx));
    assert!(cx.debug_bounds("explorer-edit-input").is_none());
    assert!(!dir.path().join("src/x.rs").exists());
}

#[gpui::test]
fn blur_with_error_cancels(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-root", cx);
    click("context-menu-item-explorer.newFile", cx);
    type_text(cx, "a.rs");
    click_editor_area(cx);

    assert!(!editing(&wb, cx));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
}

#[gpui::test]
fn blur_with_valid_name_creates(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-root", cx);
    click("context-menu-item-explorer.newFile", cx);
    type_text(cx, "c.rs");
    click_editor_area(cx);
    let root = root_of(&wb, cx);
    wait_until(cx, |_| root.join("c.rs").is_file());
    wait_until(cx, |cx| !editing(&wb, cx));
}

#[gpui::test]
fn menu_has_new_entries_only_for_folders(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (_wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-src", cx);
    assert!(
        cx.debug_bounds("context-menu-item-explorer.newFile")
            .is_some()
    );
    assert!(
        cx.debug_bounds("context-menu-item-explorer.newFolder")
            .is_some()
    );
    keys("escape", cx);
    right_click("explorer-row-a.rs", cx);
    assert!(
        cx.debug_bounds("context-menu-item-explorer.newFile")
            .is_none()
    );
}

/// While a name is being typed, Explorer keys do not run.
#[gpui::test]
fn explorer_keys_are_ignored_while_editing(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-root", cx);
    click("context-menu-item-explorer.newFile", cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("before".into()));
    keys("alt-cmd-c", cx);
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("before")
    );
    run(&wb, "copyFilePath", cx);
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("before")
    );
    assert!(editing(&wb, cx));
}

/// Gives a directory back its write permission when dropped, also when the
/// test panics.
#[cfg(unix)]
struct Writable(PathBuf);

#[cfg(unix)]
impl Drop for Writable {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

#[cfg(unix)]
#[gpui::test]
fn create_error_shows_status(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    init(cx);
    let dir = project();
    let src = dir.path().join("src");
    std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o555)).unwrap();
    let _writable = Writable(src.clone());
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-src", cx);
    click("context-menu-item-explorer.newFile", cx);
    type_text(cx, "x.rs");
    keys("enter", cx);
    wait_until(cx, |cx| !editing(&wb, cx));
    let message = cx.update(|_, cx| lucerna_workbench::status_message::current(cx));
    let message = message.expect("a status message");
    assert!(message.contains("x.rs"), "{message}");
    assert!(!src.join("x.rs").exists());
}

/// Scrolling the edit row out of the list stops drawing the input, which
/// GPUI reports as a blur: that must not confirm the half-typed name.
#[gpui::test]
fn scrolling_edit_row_out_of_view_keeps_editing(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    for i in 0..300 {
        std::fs::write(dir.path().join(format!("f{i:03}.txt")), "").unwrap();
    }
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-root", cx);
    run(&wb, "explorer.newFile", cx);
    type_text(cx, "half.rs");
    assert!(cx.debug_bounds("explorer-edit-input").is_some());
    let list = cx.debug_bounds("explorer-list").unwrap();
    cx.simulate_event(gpui::ScrollEvent {
        position: list.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-5000.))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
        source: gpui::ScrollSource::Pointer,
    });
    draw(cx);
    draw(cx);
    assert!(
        cx.debug_bounds("explorer-edit-input").is_none(),
        "the edit row scrolled out of the list"
    );
    assert!(editing(&wb, cx));
    assert!(!dir.path().join("half.rs").exists());
}

/// The folder a name is typed in is deleted: the edit ends and the folder
/// is not re-created.
#[gpui::test]
fn deleting_parent_cancels_edit(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-src", cx);
    click("context-menu-item-explorer.newFile", cx);
    type_text(cx, "x.rs");
    let root = root_of(&wb, cx);
    std::fs::remove_dir_all(root.join("src")).unwrap();
    wait_until(cx, |cx| {
        wb.read_with(cx, |wb, _| {
            !wb.worktree().snapshot().contains(Path::new("src"))
        })
    });
    let removed = Change {
        path: Arc::from(Path::new("src")),
        kind: ChangeKind::Removed,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![removed]), cx)
    });
    draw(cx);
    draw(cx);
    assert!(!editing(&wb, cx));
    assert!(cx.debug_bounds("explorer-edit-input").is_none());
    assert!(!root.join("src").exists());
}
