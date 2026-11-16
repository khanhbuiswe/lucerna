//! Editing the tree from the Explorer: New File…, New Folder…, Rename…
//! and Delete.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{
    Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point,
    TestAppContext, VisualTestContext,
};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::system_shell::{SystemShell, SystemShellGlobal};
use lucerna_workbench::{AppState, OpenMode, Selection, Workbench};
use tempfile::TempDir;

/// Trusted, so opening a file raises no trust modal; no language servers,
/// whose threads would make the GPUI test scheduler non-deterministic.
const TRUSTED: &str = r#"{ "session.trustAllWorktrees": true, "lucerna.lsp.enabled": false }"#;

fn init(cx: &mut TestAppContext) {
    init_with(TRUSTED, cx);
}

fn init_with(user_settings: &str, cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    settings.set_layer(Layer::User, user_settings).unwrap();
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
    let message = cx.update(|window, cx| {
        lucerna_workbench::status_message::current(window.window_handle().window_id(), cx)
    });
    let message = message.expect("a status message");
    assert!(message.contains("x.rs"), "{message}");
    assert!(!src.join("x.rs").exists());
}

/// Scrolling the edit row out of the list stops drawing the input, which
/// GPUI reports as a blur: that must not confirm the half-typed name.
#[gpui::test]
fn scrolling_edit_row_out_of_view_keeps_editing(cx: &mut TestAppContext) {
    init(cx);
    let dir = long_project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-root", cx);
    run(&wb, "explorer.newFile", cx);
    type_text(cx, "half.rs");
    assert!(cx.debug_bounds("explorer-edit-input").is_some());
    scroll_list_away(cx);
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

/// A project of 300 files, so the Explorer list scrolls.
fn long_project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..300 {
        std::fs::write(dir.path().join(format!("f{i:03}.txt")), "").unwrap();
    }
    dir
}

/// Scrolls the Explorer list far down, so a row at the top is not drawn.
fn scroll_list_away(cx: &mut VisualTestContext) {
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
}

/// A row scrolled out of the list gets no blur when the focus moves on
/// later; the edit must still end then.
#[gpui::test]
fn scrolled_out_edit_ends_when_focus_moves_elsewhere(cx: &mut TestAppContext) {
    init(cx);
    let dir = long_project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-root", cx);
    run(&wb, "explorer.newFile", cx);
    type_text(cx, "kept.rs");
    scroll_list_away(cx);
    assert!(editing(&wb, cx));
    click_editor_area(cx);
    let root = root_of(&wb, cx);
    wait_until(cx, |_| root.join("kept.rs").is_file());
    wait_until(cx, |cx| !editing(&wb, cx));
    run(&wb, "filesExplorer.openContextMenu", cx);
    let open = cx.update(|_, cx| wb.read(cx).explorer().read(cx).context_menu_open());
    assert!(open, "Explorer commands run again");
}

#[gpui::test]
fn scrolled_out_edit_with_error_cancels_when_focus_moves(cx: &mut TestAppContext) {
    init(cx);
    let dir = long_project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-root", cx);
    run(&wb, "explorer.newFile", cx);
    type_text(cx, "f000.txt");
    scroll_list_away(cx);
    click_editor_area(cx);
    draw(cx);
    assert!(!editing(&wb, cx));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 300);
}

fn input_state(
    wb: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> (String, std::ops::Range<usize>) {
    cx.update(|_, cx| {
        let explorer = wb.read(cx).explorer().read(cx);
        let input = explorer.name_input().expect("a name input").read(cx);
        (input.text().to_string(), input.selected_range())
    })
}

/// The names in `dir`, as on disk.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn open_file(
    wb: &Entity<Workbench>,
    path: &str,
    cx: &mut VisualTestContext,
) -> Entity<lucerna_workbench::code_view::CodeView> {
    wb.update(cx, |wb, cx| {
        wb.open_path(
            Arc::from(Path::new(path)),
            lucerna_workbench::OpenMode::Pinned,
            cx,
        )
    });
    draw(cx);
    let root = root_of(wb, cx);
    let abs = root.join(path);
    wait_until(cx, |cx| {
        wb.read_with(cx, |wb, cx| {
            wb.documents().read(cx).snapshot_of(&abs).is_some()
        })
    });
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("a tab body")
            .downcast()
            .expect("a code view")
    })
}

#[gpui::test]
fn rename_selects_stem(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-row-a.rs", cx);
    keys("enter", cx);
    assert!(editing(&wb, cx));
    assert_eq!(input_state(&wb, cx), ("a.rs".to_string(), 0..1));
}

#[gpui::test]
fn rename_keeps_unsaved_edits(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    let view = open_file(&wb, "a.rs", cx);
    cx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(view.read(cx), cx);
        window.focus(&focus, cx);
    });
    draw(cx);
    type_text(cx, "xy");
    let cursor = view.read_with(cx, |v, _| v.cursor());
    assert_eq!(cursor.column, 2);
    let a = root.join("a.rs");
    let dirty = |path: &Path, cx: &mut VisualTestContext| {
        wb.read_with(cx, |wb, cx| wb.documents().read(cx).is_dirty(path))
    };
    assert!(dirty(&a, cx));

    click("explorer-row-a.rs", cx);
    keys("enter", cx);
    type_text(cx, "c");
    keys("enter", cx);
    let c = root.join("c.rs");
    wait_until(cx, |_| c.is_file());
    wait_until(cx, |cx| !editing(&wb, cx));
    assert_eq!(tabs(&wb, cx), [(PathBuf::from("c.rs"), true)]);
    assert!(dirty(&c, cx));
    assert_eq!(view.read_with(cx, |v, _| v.cursor()), cursor);
    assert_eq!(&*view.read_with(cx, |v, _| v.path().clone()), c.as_path());

    let changes = vec![
        Change {
            path: Arc::from(Path::new("a.rs")),
            kind: ChangeKind::Removed,
        },
        Change {
            path: Arc::from(Path::new("c.rs")),
            kind: ChangeKind::Added,
        },
    ];
    wait_until(cx, |cx| {
        wb.read_with(cx, |wb, _| {
            wb.worktree().snapshot().contains(Path::new("c.rs"))
        })
    });
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(changes), cx)
    });
    draw(cx);
    let state = wb.read_with(cx, |wb, cx| wb.documents().read(cx).state(&c).cloned());
    assert_eq!(state, Some(lucerna_workbench::DocState::Ready));
    assert_eq!(
        selection(&wb, cx),
        Selection::Path(Arc::from(Path::new("c.rs")))
    );

    keys("cmd-s", cx);
    wait_until(cx, |_| {
        std::fs::read_to_string(&c).unwrap().starts_with("xy")
    });
    assert!(!a.exists());
}

#[gpui::test]
fn rename_folder_moves_open_tabs(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    open_file(&wb, "src/main.rs", cx);
    click("explorer-row-src", cx);
    keys("enter", cx);
    assert_eq!(input_state(&wb, cx), ("src".to_string(), 0..3));
    type_text(cx, "lib");
    keys("enter", cx);
    wait_until(cx, |_| root.join("lib/main.rs").is_file());
    wait_until(cx, |cx| !editing(&wb, cx));
    assert_eq!(tabs(&wb, cx), [(PathBuf::from("lib/main.rs"), true)]);
    let lib_main = root.join("lib/main.rs");
    let views = wb.read_with(cx, |wb, cx| {
        wb.documents().read(cx).views_of(&lib_main).len()
    });
    assert_eq!(views, 1);
}

#[gpui::test]
fn rename_to_existing_shows_error(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-row-a.rs", cx);
    keys("enter", cx);
    type_text(cx, "b");
    let error = cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .edit_error()
            .map(str::to_string)
    });
    assert!(
        error
            .as_deref()
            .is_some_and(|e| e.contains("A file or folder b.rs already exists")),
        "{error:?}"
    );
    keys("enter", cx);
    assert!(editing(&wb, cx));
    assert_eq!(names(dir.path()), ["a.rs", "b.rs", "src"]);
}

#[gpui::test]
fn case_only_rename(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-row-a.rs", cx);
    keys("enter", cx);
    type_text(cx, "A");
    assert!(
        cx.update(|_, cx| wb.read(cx).explorer().read(cx).edit_error().is_none()),
        "a case-only rename is allowed"
    );
    keys("enter", cx);
    wait_until(cx, |_| names(dir.path()) == ["A.rs", "b.rs", "src"]);
    wait_until(cx, |cx| !editing(&wb, cx));
}

/// True if the file system holding `dir` ignores case (APFS by default).
fn ignores_case(dir: &Path) -> bool {
    std::fs::write(dir.join("case-probe-a"), "").unwrap();
    let ignores = std::fs::symlink_metadata(dir.join("CASE-PROBE-A")).is_ok();
    std::fs::remove_file(dir.join("case-probe-a")).unwrap();
    ignores
}

/// After a case-only rename of `old` to `new` (root-relative, top level):
/// waits until the worktree has only the new spelling, then delivers the
/// watcher's events for both, with what is below them.
fn deliver_case_rename(
    wb: &Entity<Workbench>,
    old: &[&str],
    new: &[&str],
    cx: &mut VisualTestContext,
) {
    wait_until(cx, |cx| {
        wb.read_with(cx, |wb, _| {
            let snapshot = wb.worktree().snapshot();
            new.iter().all(|p| snapshot.contains(Path::new(p)))
                && !old.iter().any(|p| snapshot.contains(Path::new(p)))
        })
    });
    let change = |p: &&str, kind| Change {
        path: Arc::from(Path::new(p)),
        kind,
    };
    let changes = old
        .iter()
        .map(|p| change(p, ChangeKind::Removed))
        .chain(new.iter().map(|p| change(p, ChangeKind::Added)))
        .collect();
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(changes), cx)
    });
    draw(cx);
}

/// Root-relative paths of the Explorer's top-level rows.
fn top_rows(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .model()
            .visible_rows()
            .iter()
            .filter(|r| r.path.components().count() == 1)
            .map(|r| r.path.to_string_lossy().into_owned())
            .collect()
    })
}

/// T2-2: renaming the open `a.rs` to `A.rs` leaves one tab and one
/// document, at `A.rs`, and no `a.rs` row in the tree.
#[gpui::test]
fn case_only_rename_of_open_file_leaves_one_tab(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    if !ignores_case(dir.path()) {
        return;
    }
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    open_file(&wb, "a.rs", cx);
    click("explorer-row-a.rs", cx);
    keys("enter", cx);
    type_text(cx, "A");
    keys("enter", cx);
    wait_until(cx, |_| names(dir.path()) == ["A.rs", "b.rs", "src"]);
    wait_until(cx, |cx| !editing(&wb, cx));
    deliver_case_rename(&wb, &["a.rs"], &["A.rs"], cx);

    assert_eq!(tabs(&wb, cx), [(PathBuf::from("A.rs"), true)]);
    let open = wb.read_with(cx, |wb, cx| wb.documents().read(cx).open_paths());
    assert_eq!(open, [Arc::from(root.join("A.rs"))]);
    assert_eq!(top_rows(&wb, cx), ["src", "A.rs", "b.rs"]);
}

/// Review Focus 3: a case-only rename of a folder holding a tab with
/// unsaved changes. The tab follows with its edits; no second document.
#[gpui::test]
fn case_only_rename_of_folder_with_unsaved_tab(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    if !ignores_case(dir.path()) {
        return;
    }
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    let view = open_file(&wb, "src/main.rs", cx);
    cx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(view.read(cx), cx);
        window.focus(&focus, cx);
    });
    draw(cx);
    type_text(cx, "xy");
    click("explorer-row-src", cx);
    keys("enter", cx);
    type_text(cx, "Src");
    keys("enter", cx);
    wait_until(cx, |_| names(dir.path()) == ["Src", "a.rs", "b.rs"]);
    wait_until(cx, |cx| !editing(&wb, cx));
    deliver_case_rename(&wb, &["src", "src/main.rs"], &["Src", "Src/main.rs"], cx);

    assert_eq!(tabs(&wb, cx), [(PathBuf::from("Src/main.rs"), true)]);
    let main = root.join("Src/main.rs");
    let (open, dirty, state) = wb.read_with(cx, |wb, cx| {
        let documents = wb.documents().read(cx);
        (
            documents.open_paths(),
            documents.is_dirty(&main),
            documents.state(&main).cloned(),
        )
    });
    assert_eq!(open, [Arc::from(main.as_path())]);
    assert!(dirty);
    assert_eq!(state, Some(lucerna_workbench::DocState::Ready));
    assert_eq!(
        &*view.read_with(cx, |v, _| v.path().clone()),
        main.as_path()
    );
    assert_eq!(top_rows(&wb, cx), ["Src", "a.rs", "b.rs"]);

    keys("cmd-s", cx);
    wait_until(cx, |_| {
        std::fs::read_to_string(&main).unwrap().starts_with("xy")
    });
    assert_eq!(names(dir.path()), ["Src", "a.rs", "b.rs"]);
}

/// Delivers the worktree's real events (the app's event pump does not run
/// on the test scheduler) until `done` holds, for at most 10 s.
fn pump_until(
    wb: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    mut done: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let events = wb.read_with(cx, |wb, _| wb.worktree().events());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(cx) {
        assert!(Instant::now() < deadline, "condition not met in time");
        match events.try_recv() {
            Ok(ev) => wb.update(cx, |wb, cx| wb.on_worktree_event(&ev, cx)),
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
        draw(cx);
    }
}

fn text_of(
    view: &Entity<lucerna_workbench::code_view::CodeView>,
    cx: &mut VisualTestContext,
) -> String {
    view.read_with(cx, |v, _| {
        let s = v.snapshot();
        s.text_for_range(0..s.len_bytes())
    })
}

/// Another program (an agent's `mv` / `git mv`) renames the open, dirty
/// `path`'s file or folder `old` to `new`, which differ only in case: the
/// tab and its document follow with the edits, and saving keeps `new`.
fn external_case_rename_moves_dirty_tab(path: &str, old: &str, new: &str, cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    if !ignores_case(dir.path()) {
        return;
    }
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    let view = open_file(&wb, path, cx);
    cx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(view.read(cx), cx);
        window.focus(&focus, cx);
    });
    draw(cx);
    type_text(cx, "xy");
    let text = text_of(&view, cx);

    std::fs::rename(root.join(old), root.join(new)).unwrap();
    let rest = Path::new(path).strip_prefix(old).unwrap();
    let moved = if rest.as_os_str().is_empty() {
        root.join(new)
    } else {
        root.join(new).join(rest)
    };
    let open_paths = |cx: &mut VisualTestContext| {
        wb.read_with(cx, |wb, cx| wb.documents().read(cx).open_paths())
    };
    pump_until(&wb, cx, |cx| {
        wb.read_with(cx, |wb, _| {
            let snapshot = wb.worktree().snapshot();
            snapshot.contains(Path::new(new)) && !snapshot.contains(Path::new(old))
        }) && open_paths(cx).iter().all(|p| **p == *moved)
    });

    let rel = moved.strip_prefix(&root).unwrap().to_path_buf();
    assert_eq!(tabs(&wb, cx), [(rel, true)]);
    assert_eq!(open_paths(cx), [Arc::from(moved.as_path())]);
    let (dirty, state) = wb.read_with(cx, |wb, cx| {
        let documents = wb.documents().read(cx);
        (documents.is_dirty(&moved), documents.state(&moved).cloned())
    });
    assert!(dirty);
    assert_eq!(state, Some(lucerna_workbench::DocState::Ready));
    assert_eq!(text_of(&view, cx), text);
    assert_eq!(
        &*view.read_with(cx, |v, _| v.path().clone()),
        moved.as_path()
    );

    keys("cmd-s", cx);
    wait_until(cx, |_| {
        std::fs::read_to_string(&moved).unwrap().starts_with("xy")
    });
    let mut want = vec![new.to_string(), "b.rs".to_string()];
    want.push(if old == "src" { "a.rs" } else { "src" }.to_string());
    want.sort();
    assert_eq!(names(dir.path()), want, "the new spelling stays");
}

#[gpui::test]
fn external_case_only_rename_of_file_moves_dirty_tab(cx: &mut TestAppContext) {
    external_case_rename_moves_dirty_tab("a.rs", "a.rs", "A.rs", cx);
}

#[gpui::test]
fn external_case_only_rename_of_folder_moves_dirty_tab(cx: &mut TestAppContext) {
    external_case_rename_moves_dirty_tab("src/main.rs", "src", "Src", cx);
}

#[gpui::test]
fn unchanged_name_just_ends(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-row-a.rs", cx);
    keys("enter", cx);
    keys("enter", cx);
    assert!(!editing(&wb, cx));
    assert_eq!(names(dir.path()), ["a.rs", "b.rs", "src"]);
}

#[gpui::test]
fn enter_on_root_does_nothing(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-root", cx);
    keys("enter", cx);
    assert!(!editing(&wb, cx));
    run(&wb, "renameFile", cx);
    assert!(!editing(&wb, cx));
}

#[gpui::test]
fn menu_has_rename_except_on_root(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (_wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-a.rs", cx);
    assert!(cx.debug_bounds("context-menu-item-renameFile").is_some());
    keys("escape", cx);
    right_click("explorer-row-src", cx);
    assert!(cx.debug_bounds("context-menu-item-renameFile").is_some());
    keys("escape", cx);
    right_click("explorer-root", cx);
    assert!(cx.debug_bounds("context-menu-item-renameFile").is_none());
}

/// A folder cannot move into itself: the error shows while typing, and a
/// blur with that name cancels instead of trying (and looping) forever.
#[gpui::test]
fn rename_into_itself_shows_error_and_blur_cancels(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-row-src", cx);
    keys("enter", cx);
    type_text(cx, "src/x");
    let error = cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .edit_error()
            .map(str::to_string)
    });
    assert!(
        error.as_deref().is_some_and(|e| e.contains("is not valid")),
        "{error:?}"
    );
    click_editor_area(cx);
    // Settles: no redraw loop keeps the edit alive.
    cx.run_until_parked();
    draw(cx);
    cx.run_until_parked();
    assert!(!editing(&wb, cx));
    assert_eq!(names(dir.path()), ["a.rs", "b.rs", "src"]);
    assert_eq!(names(&dir.path().join("src")), ["main.rs"]);
}

/// The tab of a deleted file holds its name: renaming another file onto
/// it is refused while typing, so two tabs never share a path.
#[gpui::test]
fn rename_onto_open_deleted_file_shows_error(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    open_file(&wb, "b.rs", cx);
    std::fs::remove_file(dir.path().join("b.rs")).unwrap();
    wait_until(cx, |cx| {
        wb.read_with(cx, |wb, _| {
            !wb.worktree().snapshot().contains(Path::new("b.rs"))
        })
    });
    let removed = Change {
        path: Arc::from(Path::new("b.rs")),
        kind: ChangeKind::Removed,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![removed]), cx)
    });
    draw(cx);
    click("explorer-row-a.rs", cx);
    keys("enter", cx);
    type_text(cx, "b");
    let error = cx.update(|_, cx| wb.read(cx).explorer().read(cx).edit_error().is_some());
    assert!(error);
    keys("enter", cx);
    assert!(editing(&wb, cx));
    assert!(dir.path().join("a.rs").exists());
}

/// Records `trash` calls. On success it removes the path, as the Trash
/// would; on failure it leaves it.
struct TrashShell {
    trashed: Arc<Mutex<Vec<PathBuf>>>,
    fail: bool,
}

impl SystemShell for TrashShell {
    fn reveal(&self, _path: &Path, _cx: &gpui::App) {}

    fn open_terminal(&self, _dir: &Path, _exec: &str) -> std::io::Result<()> {
        Ok(())
    }

    fn trash(&self, path: &Path) -> Result<(), String> {
        self.trashed.lock().unwrap().push(path.to_path_buf());
        if self.fail {
            return Err("no Trash on this volume".to_string());
        }
        lucerna_project::fs_ops::delete_permanently(path).map_err(|e| e.to_string())
    }
}

fn install_trash(fail: bool, cx: &mut TestAppContext) -> Arc<Mutex<Vec<PathBuf>>> {
    let trashed = Arc::new(Mutex::new(Vec::new()));
    cx.set_global(SystemShellGlobal(Arc::new(TrashShell {
        trashed: trashed.clone(),
        fail,
    })));
    trashed
}

const CONFIRM_OFF: &str = r#"{ "session.trustAllWorktrees": true, "lucerna.lsp.enabled": false, "explorer.confirmDelete": false }"#;

#[gpui::test]
fn delete_asks_then_trashes(cx: &mut TestAppContext) {
    init(cx);
    let trashed = install_trash(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-backspace", cx);
    assert!(cx.has_pending_prompt());
    assert!(trashed.lock().unwrap().is_empty());
    cx.simulate_prompt_answer("Move to Trash");
    draw(cx);
    wait_until(cx, |_| !trashed.lock().unwrap().is_empty());
    assert_eq!(*trashed.lock().unwrap(), [root.join("a.rs")]);
    assert!(!root.join("a.rs").exists());
}

#[gpui::test]
fn cancel_keeps_file(cx: &mut TestAppContext) {
    init(cx);
    let trashed = install_trash(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-backspace", cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    draw(cx);
    cx.run_until_parked();
    assert!(trashed.lock().unwrap().is_empty());
    assert!(root.join("a.rs").is_file());
}

#[gpui::test]
fn confirm_off_skips_prompt(cx: &mut TestAppContext) {
    init_with(CONFIRM_OFF, cx);
    let trashed = install_trash(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-backspace", cx);
    assert!(!cx.has_pending_prompt());
    wait_until(cx, |_| !trashed.lock().unwrap().is_empty());
    assert_eq!(*trashed.lock().unwrap(), [root.join("a.rs")]);
}

#[gpui::test]
fn trash_failure_offers_permanent_delete(cx: &mut TestAppContext) {
    init(cx);
    let trashed = install_trash(true, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-backspace", cx);
    cx.simulate_prompt_answer("Move to Trash");
    draw(cx);
    wait_until(cx, |cx| cx.has_pending_prompt());
    assert_eq!(*trashed.lock().unwrap(), [root.join("a.rs")]);
    // The Trash failed: nothing is gone until the user says so.
    assert!(root.join("a.rs").is_file());
    cx.simulate_prompt_answer("Delete Permanently");
    draw(cx);
    wait_until(cx, |_| !root.join("a.rs").exists());
}

#[gpui::test]
fn trash_failure_cancel_keeps_file(cx: &mut TestAppContext) {
    init(cx);
    let trashed = install_trash(true, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-backspace", cx);
    cx.simulate_prompt_answer("Move to Trash");
    draw(cx);
    wait_until(cx, |cx| cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    draw(cx);
    cx.run_until_parked();
    assert_eq!(trashed.lock().unwrap().len(), 1);
    assert!(root.join("a.rs").is_file());
}

#[gpui::test]
fn delete_permanently_always_asks(cx: &mut TestAppContext) {
    init_with(CONFIRM_OFF, cx);
    let trashed = install_trash(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("alt-cmd-backspace", cx);
    assert!(cx.has_pending_prompt());
    assert!(root.join("a.rs").is_file());
    cx.simulate_prompt_answer("Delete");
    draw(cx);
    wait_until(cx, |_| !root.join("a.rs").exists());
    // Permanently: never through the Trash.
    assert!(trashed.lock().unwrap().is_empty());
}

#[gpui::test]
fn delete_closes_clean_tabs_keeps_dirty(cx: &mut TestAppContext) {
    init(cx);
    install_trash(false, cx);
    let dir = project();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    let view = open_file(&wb, "src/main.rs", cx);
    cx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(view.read(cx), cx);
        window.focus(&focus, cx);
    });
    draw(cx);
    type_text(cx, "xy");
    open_file(&wb, "src/lib.rs", cx);
    assert_eq!(tabs(&wb, cx).len(), 2);

    click("explorer-row-src", cx);
    keys("cmd-backspace", cx);
    cx.simulate_prompt_answer("Move to Trash");
    draw(cx);
    wait_until(cx, |_| !root.join("src").exists());
    wait_until(cx, |cx| tabs(&wb, cx).len() == 1);
    assert_eq!(tabs(&wb, cx), [(PathBuf::from("src/main.rs"), true)]);

    wait_until(cx, |cx| {
        wb.read_with(cx, |wb, _| {
            !wb.worktree().snapshot().contains(Path::new("src"))
        })
    });
    let changes = ["src", "src/main.rs", "src/lib.rs"]
        .iter()
        .map(|p| Change {
            path: Arc::from(Path::new(p)),
            kind: ChangeKind::Removed,
        })
        .collect();
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(changes), cx)
    });
    draw(cx);
    let main = root.join("src/main.rs");
    let state = wb.read_with(cx, |wb, cx| wb.documents().read(cx).state(&main).cloned());
    assert_eq!(state, Some(lucerna_workbench::DocState::Deleted));
    assert!(wb.read_with(cx, |wb, cx| wb.documents().read(cx).is_dirty(&main)));
}

#[gpui::test]
fn selection_moves_to_parent_after_delete(cx: &mut TestAppContext) {
    init(cx);
    install_trash(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-src", cx);
    assert!(expanded(&wb, "src", cx));
    click("explorer-row-src/main.rs", cx);
    keys("cmd-backspace", cx);
    cx.simulate_prompt_answer("Move to Trash");
    draw(cx);
    wait_until(cx, |_| !root.join("src/main.rs").exists());
    wait_until(cx, |cx| {
        selection(&wb, cx) == Selection::Path(Arc::from(Path::new("src")))
    });
}

#[gpui::test]
fn delete_is_not_offered_for_root(cx: &mut TestAppContext) {
    init_with(CONFIRM_OFF, cx);
    let trashed = install_trash(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-a.rs", cx);
    assert!(
        cx.debug_bounds("context-menu-item-moveFileToTrash")
            .is_some()
    );
    assert!(cx.debug_bounds("context-menu-item-deleteFile").is_none());
    keys("escape", cx);
    right_click("explorer-root", cx);
    assert!(
        cx.debug_bounds("context-menu-item-moveFileToTrash")
            .is_none()
    );
    keys("escape", cx);
    click("explorer-root", cx);
    keys("cmd-backspace", cx);
    run(&wb, "moveFileToTrash", cx);
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert!(trashed.lock().unwrap().is_empty());
    assert!(dir.path().exists());
}

fn run_with(
    wb: &Entity<Workbench>,
    command: &'static str,
    args: serde_json::Value,
    cx: &mut VisualTestContext,
) {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command(command, Some(&args), window, cx)
        });
    });
    draw(cx);
}

/// Removes `path` (top-level) on disk and delivers its `Removed` event.
fn remove_and_deliver(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) {
    let root = root_of(wb, cx);
    let abs = root.join(path);
    if abs.exists() {
        lucerna_project::fs_ops::delete_permanently(&abs).unwrap();
    }
    wait_until(cx, |cx| {
        wb.read_with(cx, |wb, _| {
            !wb.worktree().snapshot().contains(Path::new(path))
        })
    });
    let removed = Change {
        path: Arc::from(Path::new(path)),
        kind: ChangeKind::Removed,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![removed]), cx)
    });
    draw(cx);
}

/// The menu's Delete acts on the row it was opened for: when that row
/// vanished meanwhile it does nothing, rather than delete whatever the
/// selection moved to.
#[gpui::test]
fn menu_delete_of_vanished_entry_does_nothing(cx: &mut TestAppContext) {
    init(cx);
    let trashed = install_trash(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-row-src", cx);
    click("explorer-row-src/main.rs", cx);
    right_click("explorer-row-src/main.rs", cx);
    assert!(
        cx.debug_bounds("context-menu-item-moveFileToTrash")
            .is_some()
    );
    let root = root_of(&wb, cx);
    std::fs::remove_file(root.join("src/main.rs")).unwrap();
    wait_until(cx, |cx| {
        wb.read_with(cx, |wb, _| {
            !wb.worktree().snapshot().contains(Path::new("src/main.rs"))
        })
    });
    let removed = Change {
        path: Arc::from(Path::new("src/main.rs")),
        kind: ChangeKind::Removed,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![removed]), cx)
    });
    draw(cx);
    // The selection fell back to the folder.
    assert_eq!(
        selection(&wb, cx),
        Selection::Path(Arc::from(Path::new("src")))
    );
    click("context-menu-item-moveFileToTrash", cx);
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert!(trashed.lock().unwrap().is_empty());
    assert!(root.join("src").is_dir());
}

#[gpui::test]
fn selection_moves_to_next_sibling_after_delete(cx: &mut TestAppContext) {
    init_with(CONFIRM_OFF, cx);
    install_trash(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-backspace", cx);
    wait_until(cx, |_| !root.join("a.rs").exists());
    wait_until(cx, |cx| {
        selection(&wb, cx) == Selection::Path(Arc::from(Path::new("b.rs")))
    });
}

#[gpui::test]
fn selection_moves_to_previous_sibling_after_delete(cx: &mut TestAppContext) {
    init_with(CONFIRM_OFF, cx);
    install_trash(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-b.rs", cx);
    keys("cmd-backspace", cx);
    wait_until(cx, |_| !root.join("b.rs").exists());
    wait_until(cx, |cx| {
        selection(&wb, cx) == Selection::Path(Arc::from(Path::new("a.rs")))
    });
}

/// A held Delete key does not go on deleting the next entries.
#[gpui::test]
fn held_delete_key_deletes_once(cx: &mut TestAppContext) {
    init_with(CONFIRM_OFF, cx);
    let trashed = install_trash(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-backspace", cx);
    wait_until(cx, |_| !root.join("a.rs").exists());
    remove_and_deliver(&wb, "a.rs", cx);
    assert_eq!(
        selection(&wb, cx),
        Selection::Path(Arc::from(Path::new("b.rs")))
    );
    for keystroke in ["cmd-backspace", "alt-cmd-backspace"] {
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: gpui::Keystroke::parse(keystroke).unwrap(),
            is_held: true,
            prefer_character_input: false,
        });
        draw(cx);
    }
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(trashed.lock().unwrap().len(), 1);
    assert!(root.join("b.rs").is_file());
}

/// Delete never reaches the root, a path outside it or one the worktree
/// does not know, whatever `args.path` says, nor runs while a name is typed.
#[gpui::test]
fn delete_guards_hold_for_every_route(cx: &mut TestAppContext) {
    init_with(CONFIRM_OFF, cx);
    let trashed = install_trash(false, cx);
    let dir = project();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("x"), "x").unwrap();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    let paths = [
        String::new(),
        ".".to_string(),
        root.to_string_lossy().into_owned(),
        "../x".to_string(),
        outside.path().join("x").to_string_lossy().into_owned(),
    ];
    for command in ["moveFileToTrash", "deleteFile"] {
        for path in &paths {
            run_with(&wb, command, serde_json::json!({ "path": path }), cx);
            cx.run_until_parked();
            assert!(!cx.has_pending_prompt(), "{command} {path:?}");
        }
    }
    click("explorer-row-src", cx);
    run(&wb, "explorer.newFile", cx);
    assert!(editing(&wb, cx));
    for command in ["moveFileToTrash", "deleteFile"] {
        run_with(&wb, command, serde_json::json!({ "path": "a.rs" }), cx);
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt(), "{command} while editing");
    }
    assert!(trashed.lock().unwrap().is_empty());
    assert_eq!(names(&root), ["a.rs", "b.rs", "src"]);
    assert!(outside.path().join("x").is_file());
}

/// A second Delete of a path whose Delete is still running is ignored.
#[gpui::test]
fn second_delete_while_in_flight_is_ignored(cx: &mut TestAppContext) {
    init(cx);
    let trashed = install_trash(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    run_with(
        &wb,
        "moveFileToTrash",
        serde_json::json!({ "path": "a.rs" }),
        cx,
    );
    assert!(cx.has_pending_prompt());
    run_with(
        &wb,
        "moveFileToTrash",
        serde_json::json!({ "path": "a.rs" }),
        cx,
    );
    run_with(&wb, "deleteFile", serde_json::json!({ "path": "a.rs" }), cx);
    cx.simulate_prompt_answer("Move to Trash");
    draw(cx);
    wait_until(cx, |_| !root.join("a.rs").exists());
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(*trashed.lock().unwrap(), [root.join("a.rs")]);
    // Once done, another Delete runs again.
    run_with(
        &wb,
        "moveFileToTrash",
        serde_json::json!({ "path": "b.rs" }),
        cx,
    );
    assert!(cx.has_pending_prompt());
}

fn sidebar_visible(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    wb.read_with(cx, |wb, _| wb.sidebar_visible())
}

fn menu_open(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| wb.read(cx).explorer().read(cx).context_menu_open())
}

/// What the command left holding the focus is drawn: the name input or
/// the menu.
fn input_focused(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        let explorer = wb.read(cx).explorer().read(cx);
        explorer
            .name_input()
            .is_some_and(|input| input.focus_handle(cx).is_focused(window))
    })
}

/// From the palette with the sidebar hidden, the commands that type in
/// the tree or open its menu show the Explorer first (VS Code focuses it).
#[gpui::test]
fn explorer_commands_show_hidden_sidebar(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-row-a.rs", cx);
    for command in [
        "explorer.newFile",
        "explorer.newFolder",
        "renameFile",
        "filesExplorer.openContextMenu",
    ] {
        run(&wb, "workbench.action.toggleSidebarVisibility", cx);
        assert!(!sidebar_visible(&wb, cx), "{command}");
        run(&wb, command, cx);
        draw(cx);
        assert!(sidebar_visible(&wb, cx), "{command} shows the sidebar");
        if command == "filesExplorer.openContextMenu" {
            assert!(menu_open(&wb, cx));
            assert!(cx.debug_bounds("context-menu").is_some());
        } else {
            assert!(editing(&wb, cx), "{command}");
            assert!(cx.debug_bounds("explorer-edit-input").is_some());
            assert!(input_focused(&wb, cx), "{command} focuses the input");
        }
        // Hiding the sidebar again ends what the command started.
        run(&wb, "workbench.action.toggleSidebarVisibility", cx);
        assert!(!editing(&wb, cx), "{command}: hiding ends the edit");
        assert!(!menu_open(&wb, cx), "{command}: hiding closes the menu");
        run(&wb, "workbench.action.toggleSidebarVisibility", cx);
    }
}

#[gpui::test]
fn hiding_sidebar_cancels_name_edit(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    right_click("explorer-row-src", cx);
    click("context-menu-item-explorer.newFile", cx);
    type_text(cx, "kept.rs");
    run(&wb, "workbench.action.toggleSidebarVisibility", cx);
    draw(cx);
    assert!(!editing(&wb, cx));
    assert!(!root.join("src/kept.rs").exists(), "cancelled, not created");
    assert!(key(&wb, "editorFocus", cx), "the focus leaves the sidebar");
}

#[gpui::test]
fn hiding_sidebar_closes_menu(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-a.rs", cx);
    assert!(menu_open(&wb, cx));
    run(&wb, "workbench.action.toggleSidebarVisibility", cx);
    assert!(!menu_open(&wb, cx));
    assert!(cx.debug_bounds("context-menu").is_none());
    assert!(!key(&wb, "contextMenuVisible", cx));
}

fn key(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) -> bool {
    cx.update(|w, cx| {
        let ctx = wb.read(cx).context_keys(w, cx);
        lucerna_settings::parse_when(name).unwrap().eval(&ctx)
    })
}

/// Trashes whatever is still there and always succeeds, so a test can
/// remove the file (and report it) before the delete's result arrives.
struct LenientTrashShell;

impl SystemShell for LenientTrashShell {
    fn reveal(&self, _path: &Path, _cx: &gpui::App) {}

    fn open_terminal(&self, _dir: &Path, _exec: &str) -> std::io::Result<()> {
        Ok(())
    }

    fn trash(&self, path: &Path) -> Result<(), String> {
        let _ = lucerna_project::fs_ops::delete_permanently(path);
        Ok(())
    }
}

/// The watcher's Removed can arrive before the delete's result: the tree
/// then moves the selection to the parent first, and the delete must still
/// select the neighbour.
#[gpui::test]
fn removed_event_before_result_still_selects_next_sibling(cx: &mut TestAppContext) {
    init(cx);
    cx.set_global(SystemShellGlobal(Arc::new(LenientTrashShell)));
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-backspace", cx);
    assert!(cx.has_pending_prompt());
    std::fs::remove_file(root.join("a.rs")).unwrap();
    wait_until(cx, |cx| {
        wb.read_with(cx, |wb, _| {
            !wb.worktree().snapshot().contains(Path::new("a.rs"))
        })
    });
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(
            &WorktreeEvent::Changed(vec![Change {
                path: Arc::from(Path::new("a.rs")),
                kind: ChangeKind::Removed,
            }]),
            cx,
        )
    });
    draw(cx);
    assert_eq!(selection(&wb, cx), Selection::Root, "the tree moved it up");
    cx.simulate_prompt_answer("Move to Trash");
    draw(cx);
    wait_until(cx, |cx| {
        selection(&wb, cx) == Selection::Path(Arc::from(Path::new("b.rs")))
    });
}

fn changed(wb: &Entity<Workbench>, path: &str, kind: ChangeKind, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(
            &WorktreeEvent::Changed(vec![Change {
                path: Arc::from(Path::new(path)),
                kind,
            }]),
            cx,
        )
    });
    draw(cx);
}

fn decorated(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .model()
            .is_recently_changed(Path::new(path), Instant::now())
    })
}

/// An own change is forgotten once the worktree reports it: a later
/// change to the same path by another program is decorated.
#[gpui::test]
fn own_change_is_dropped_once_reported(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    wb.update(cx, |wb, _| {
        wb.expect_own_changes([Arc::from(Path::new("a.rs"))])
    });
    changed(&wb, "a.rs", ChangeKind::Modified, cx);
    assert!(!decorated(&wb, "a.rs", cx), "Lucerna's own change");
    changed(&wb, "a.rs", ChangeKind::Modified, cx);
    assert!(decorated(&wb, "a.rs", cx), "another program's change");
}

/// A Removed report does not end the wait: a rename's or delete's old
/// path is reported removed, and its new path may still come.
#[gpui::test]
fn own_change_survives_removed_report(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    wb.update(cx, |wb, _| {
        wb.expect_own_changes([Arc::from(Path::new("a.rs"))])
    });
    changed(&wb, "a.rs", ChangeKind::Removed, cx);
    changed(&wb, "a.rs", ChangeKind::Added, cx);
    assert!(!decorated(&wb, "a.rs", cx));
}

/// Every Markdown preview made so far, static or dynamic.
fn previews(
    wb: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> Vec<Entity<lucerna_workbench::MarkdownPreview>> {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups
            .model()
            .groups()
            .iter()
            .flat_map(|g| &g.tabs)
            .filter(|t| t.previewed_path().is_some())
            .filter_map(|t| groups.body_of(t.id)?.downcast().ok())
            .collect()
    })
}

fn preview_path(
    preview: &Entity<lucerna_workbench::MarkdownPreview>,
    cx: &mut VisualTestContext,
) -> PathBuf {
    preview.read_with(cx, |p, _| p.path().to_path_buf())
}

/// Opens the Markdown file `path`, which shows its preview.
fn open_md(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new(path)), OpenMode::Pinned, cx)
    });
    draw(cx);
}

/// Whether a document is open at `path` (absolute).
fn has_document(wb: &Entity<Workbench>, path: &Path, cx: &mut VisualTestContext) -> bool {
    wb.read_with(cx, |wb, cx| wb.documents().read(cx).state(path).is_some())
}

/// Renames the row `selector` by typing `stem` over the selected stem.
fn rename_row(selector: &'static str, stem: &str, cx: &mut VisualTestContext) {
    click(selector, cx);
    keys("enter", cx);
    type_text(cx, stem);
    keys("enter", cx);
}

/// A preview of a renamed file shows it at its new path, and leaves no
/// document at the old one, so renaming back is allowed.
#[gpui::test]
fn rename_previewed_file_and_back(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    std::fs::write(dir.path().join("doc.md"), "# hi\n").unwrap();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    open_md(&wb, "doc.md", cx);
    let [preview] = previews(&wb, cx).try_into().expect("one preview");

    rename_row("explorer-row-doc.md", "x", cx);
    wait_until(cx, |_| root.join("x.md").is_file());
    wait_until(cx, |cx| !editing(&wb, cx));
    draw(cx);
    assert_eq!(preview_path(&preview, cx), root.join("x.md"));
    assert!(!has_document(&wb, &root.join("doc.md"), cx));

    // The watcher's report of the rename, as the app gets it.
    wait_until(cx, |cx| {
        wb.read_with(cx, |wb, _| {
            wb.worktree().snapshot().contains(Path::new("x.md"))
        })
    });
    let changes = vec![
        Change {
            path: Arc::from(Path::new("doc.md")),
            kind: ChangeKind::Removed,
        },
        Change {
            path: Arc::from(Path::new("x.md")),
            kind: ChangeKind::Added,
        },
    ];
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(changes), cx)
    });
    draw(cx);
    assert!(!has_document(&wb, &root.join("doc.md"), cx));
    click("explorer-row-x.md", cx);
    keys("enter", cx);
    type_text(cx, "doc");
    let error = cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .edit_error()
            .map(str::to_string)
    });
    assert_eq!(error, None);
    keys("enter", cx);
    wait_until(cx, |_| root.join("doc.md").is_file());
    wait_until(cx, |cx| !editing(&wb, cx));
    assert_eq!(preview_path(&preview, cx), root.join("doc.md"));
}

#[gpui::test]
fn rename_folder_moves_preview(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    std::fs::write(dir.path().join("src/guide.md"), "# hi\n").unwrap();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    open_md(&wb, "src/guide.md", cx);
    let [preview] = previews(&wb, cx).try_into().expect("one preview");

    rename_row("explorer-row-src", "lib", cx);
    wait_until(cx, |_| root.join("lib/guide.md").is_file());
    wait_until(cx, |cx| !editing(&wb, cx));
    draw(cx);
    assert_eq!(preview_path(&preview, cx), root.join("lib/guide.md"));
    assert!(!has_document(&wb, &root.join("src/guide.md"), cx));
}

#[gpui::test]
fn rename_moves_dynamic_preview(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    std::fs::write(dir.path().join("doc.md"), "# hi\n").unwrap();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    wb.update(cx, |wb, cx| {
        wb.open_source(Arc::from(Path::new("doc.md")), OpenMode::Pinned, cx)
    });
    draw(cx);
    let view = cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("a tab body")
            .downcast::<lucerna_workbench::code_view::CodeView>()
            .expect("a code view")
    });
    cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    draw(cx);
    run(&wb, "markdown.showPreviewToSide", cx);
    let preview = previews(&wb, cx)
        .into_iter()
        .find(|p| p.read_with(cx, |p, _| p.is_dynamic()))
        .expect("a dynamic preview");

    rename_row("explorer-row-doc.md", "x", cx);
    wait_until(cx, |_| root.join("x.md").is_file());
    wait_until(cx, |cx| !editing(&wb, cx));
    draw(cx);
    assert_eq!(preview_path(&preview, cx), root.join("x.md"));
    assert!(!has_document(&wb, &root.join("doc.md"), cx));
}
