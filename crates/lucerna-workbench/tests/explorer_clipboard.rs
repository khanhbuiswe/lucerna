//! The Explorer's clipboard: Cut, Copy and Paste of files and folders.

use std::ops::Range;
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
use lucerna_workbench::{AppState, ClipOp, Selection, Workbench, ui};
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

/// A project with `src/main.rs`, `a.rs` and `b.rs`.
fn project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    for file in ["src/main.rs", "a.rs", "b.rs"] {
        std::fs::write(dir.path().join(file), "fn main() {}\n").unwrap();
    }
    dir
}

fn wait_scan(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
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

fn open<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let root = root.to_path_buf();
    let (wb, cx) = cx.add_window_view(move |window, cx| Workbench::new(root, window, cx));
    cx.update(|window, _| window.activate_window());
    wait_scan(&wb, cx);
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

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let position = center(selector, cx);
    cx.simulate_click(position, Modifiers::none());
    draw(cx);
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

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    draw(cx);
}

fn type_text(cx: &mut VisualTestContext, text: &str) {
    cx.simulate_input(text);
    draw(cx);
}

fn root_of(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> PathBuf {
    cx.update(|_, cx| wb.read(cx).root().to_path_buf())
}

fn selection(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Selection {
    cx.update(|_, cx| wb.read(cx).explorer().read(cx).selection().clone())
}

fn p(path: &str) -> Arc<Path> {
    Arc::from(Path::new(path))
}

fn clipboard(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<(ClipOp, PathBuf)> {
    cx.update(|_, cx| {
        wb.read(cx)
            .explorer_clipboard()
            .map(|c| (c.op, c.path.to_path_buf()))
    })
}

fn status(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| lucerna_workbench::status_message::current(cx))
        .map(|s| s.to_string())
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

/// Waits until the worktree no longer has `path`, then delivers `Removed`.
fn deliver_removed(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) {
    wait_until(cx, |cx| {
        wb.read_with(cx, |wb, _| {
            !wb.worktree().snapshot().contains(Path::new(path))
        })
    });
    let removed = Change {
        path: p(path),
        kind: ChangeKind::Removed,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![removed]), cx)
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

/// `(command, disabled)` of every entry of the open menu.
fn menu(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<(String, bool)> {
    cx.update(|_, cx| {
        let explorer = wb.read(cx).explorer().read(cx);
        let Some(menu) = explorer.context_menu() else {
            return Vec::new();
        };
        menu.read(cx)
            .items()
            .iter()
            .filter_map(|item| match item {
                ui::MenuItem::Entry {
                    command, disabled, ..
                } => Some((command.to_string(), *disabled)),
                ui::MenuItem::Separator => None,
            })
            .collect()
    })
}

fn menu_has(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) -> bool {
    menu(wb, cx).iter().any(|(c, _)| c == command)
}

fn open_file(
    wb: &Entity<Workbench>,
    path: &str,
    cx: &mut VisualTestContext,
) -> Entity<lucerna_workbench::code_view::CodeView> {
    wb.update(cx, |wb, cx| {
        wb.open_path(p(path), lucerna_workbench::OpenMode::Pinned, cx)
    });
    draw(cx);
    let abs = root_of(wb, cx).join(path);
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
fn copy_paste_into_folder(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-c", cx);
    assert_eq!(clipboard(&wb, cx), Some((ClipOp::Copy, root.join("a.rs"))));
    click("explorer-row-src", cx);
    keys("cmd-v", cx);
    let copy = root.join("src/a.rs");
    wait_until(cx, |_| copy.is_file());
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "fn main() {}\n");
    assert!(root.join("a.rs").is_file(), "a copy keeps the source");
    deliver_added(&wb, &["src/a.rs"], cx);
    assert_eq!(selection(&wb, cx), Selection::Path(p("src/a.rs")));
    // The click on `a.rs` opened it in the preview tab.
    wait_until(cx, |cx| {
        tabs(&wb, cx).contains(&(PathBuf::from("src/a.rs"), true))
    });
    assert!(!decorated(&wb, "src/a.rs", cx), "Lucerna's own change");
    assert_eq!(
        clipboard(&wb, cx),
        Some((ClipOp::Copy, root.join("a.rs"))),
        "Copy keeps the clipboard"
    );
}

#[gpui::test]
fn paste_twice_names_copy_2(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-c", cx);
    keys("cmd-v", cx);
    wait_until(cx, |_| root.join("a copy.rs").is_file());
    deliver_added(&wb, &["a copy.rs"], cx);
    keys("cmd-v", cx);
    wait_until(cx, |_| root.join("a copy 2.rs").is_file());
    deliver_added(&wb, &["a copy 2.rs"], cx);
    assert_eq!(selection(&wb, cx), Selection::Path(p("a copy 2.rs")));
}

/// A second paste before the tree knows the first copy still picks a
/// free name: the write fails as taken, and the name is found again on
/// disk.
#[gpui::test]
fn paste_race_retries_on_disk(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-c", cx);
    // Taken on disk, unknown to the tree's snapshot of the moment.
    std::fs::write(root.join("a copy.rs"), "other\n").unwrap();
    keys("cmd-v", cx);
    wait_until(cx, |_| root.join("a copy 2.rs").is_file());
    assert_eq!(
        std::fs::read_to_string(root.join("a copy.rs")).unwrap(),
        "other\n",
        "never overwritten"
    );
}

#[gpui::test]
fn paste_on_file_uses_its_folder(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-b.rs", cx);
    keys("cmd-c", cx);
    click("explorer-row-src", cx);
    click("explorer-row-src/main.rs", cx);
    right_click("explorer-row-src/main.rs", cx);
    click("context-menu-item-filesExplorer.paste", cx);
    wait_until(cx, |_| root.join("src/b.rs").is_file());
}

#[gpui::test]
fn copy_folder_recursively(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-src", cx);
    keys("cmd-c", cx);
    click("explorer-root", cx);
    keys("cmd-v", cx);
    wait_until(cx, |_| root.join("src copy/main.rs").is_file());
    deliver_added(&wb, &["src copy", "src copy/main.rs"], cx);
    assert_eq!(selection(&wb, cx), Selection::Path(p("src copy")));
    assert!(tabs(&wb, cx).is_empty(), "a folder is not opened");
}

#[gpui::test]
fn cut_paste_moves_file(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-x", cx);
    assert_eq!(clipboard(&wb, cx), Some((ClipOp::Cut, root.join("a.rs"))));
    click("explorer-row-src", cx);
    keys("cmd-v", cx);
    wait_until(cx, |_| root.join("src/a.rs").is_file());
    assert!(!root.join("a.rs").exists());
    assert_eq!(clipboard(&wb, cx), None, "a cut is pasted once");
    deliver_added(&wb, &["src/a.rs"], cx);
    assert_eq!(selection(&wb, cx), Selection::Path(p("src/a.rs")));
    assert!(!decorated(&wb, "src/a.rs", cx));
}

#[gpui::test]
fn cut_paste_onto_taken_name_never_overwrites(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    std::fs::write(dir.path().join("src/a.rs"), "taken\n").unwrap();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-x", cx);
    click("explorer-row-src", cx);
    keys("cmd-v", cx);
    wait_until(cx, |_| root.join("src/a copy.rs").is_file());
    assert_eq!(
        std::fs::read_to_string(root.join("src/a.rs")).unwrap(),
        "taken\n"
    );
    assert!(!root.join("a.rs").exists());
}

#[gpui::test]
fn cut_paste_moves_dirty_tab(cx: &mut TestAppContext) {
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
    let dirty = |path: &Path, cx: &mut VisualTestContext| {
        wb.read_with(cx, |wb, cx| wb.documents().read(cx).is_dirty(path))
    };
    assert!(dirty(&root.join("a.rs"), cx));

    click("explorer-row-a.rs", cx);
    keys("cmd-x", cx);
    click("explorer-row-src", cx);
    keys("cmd-v", cx);
    let moved = root.join("src/a.rs");
    wait_until(cx, |_| moved.is_file());
    wait_until(cx, |cx| {
        tabs(&wb, cx) == [(PathBuf::from("src/a.rs"), true)]
    });
    assert!(dirty(&moved, cx));
    assert_eq!(
        &*view.read_with(cx, |v, _| v.path().clone()),
        moved.as_path()
    );

    deliver_removed(&wb, "a.rs", cx);
    deliver_added(&wb, &["src/a.rs"], cx);
    let state = wb.read_with(cx, |wb, cx| wb.documents().read(cx).state(&moved).cloned());
    assert_eq!(state, Some(lucerna_workbench::DocState::Ready));
    cx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(view.read(cx), cx);
        window.focus(&focus, cx);
    });
    draw(cx);
    keys("cmd-s", cx);
    wait_until(cx, |_| {
        std::fs::read_to_string(&moved).unwrap().starts_with("xy")
    });
    assert!(!root.join("a.rs").exists());
}

#[gpui::test]
fn cut_into_same_folder_is_noop(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-src", cx);
    click("explorer-row-src/main.rs", cx);
    keys("cmd-x", cx);
    keys("cmd-v", cx);
    click("explorer-row-src", cx);
    keys("cmd-v", cx);
    draw(cx);
    let mut names: Vec<String> = std::fs::read_dir(root.join("src"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["main.rs"]);
    assert_eq!(status(cx), None);
}

#[gpui::test]
fn paste_folder_into_itself_refused(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-src", cx);
    keys("cmd-c", cx);
    keys("cmd-v", cx);
    assert_eq!(
        status(cx).as_deref(),
        Some("Cannot paste 'src' into itself.")
    );
    // A cut folder pasted onto a file inside it.
    keys("cmd-x", cx);
    cx.update(|window, cx| {
        let args = serde_json::json!({ "path": "src/main.rs" });
        wb.update(cx, |wb, cx| {
            wb.run_command("filesExplorer.paste", Some(&args), window, cx)
        });
    });
    draw(cx);
    assert!(!root.join("src/src").exists());
    assert!(!root.join("src/src copy").exists());
    assert!(root.join("src/main.rs").is_file());
}

#[gpui::test]
fn paste_disabled_when_clipboard_empty(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-src", cx);
    let entries = menu(&wb, cx);
    let commands: Vec<&str> = entries.iter().map(|(c, _)| c.as_str()).collect();
    let find = commands
        .iter()
        .position(|c| *c == "filesExplorer.findInFolder")
        .unwrap();
    assert_eq!(
        &commands[find + 1..find + 4],
        [
            "filesExplorer.cut",
            "filesExplorer.copy",
            "filesExplorer.paste"
        ]
    );
    assert!(entries.contains(&("filesExplorer.paste".into(), true)));
    keys("escape", cx);
    // The menu's Copy then enables the menu's Paste.
    right_click("explorer-row-a.rs", cx);
    click("context-menu-item-filesExplorer.copy", cx);
    right_click("explorer-row-src", cx);
    assert!(menu(&wb, cx).contains(&("filesExplorer.paste".into(), false)));
}

#[gpui::test]
fn cut_row_is_dimmed_and_escape_cancels(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-row-a.rs", cx);
    assert!(cx.debug_bounds("explorer-row-cut-a.rs").is_none());
    keys("cmd-x", cx);
    assert!(cx.debug_bounds("explorer-row-cut-a.rs").is_some());
    keys("escape", cx);
    assert!(cx.debug_bounds("explorer-row-cut-a.rs").is_none());
    assert_eq!(clipboard(&wb, cx), None);
    // A copy is not dimmed, and Escape leaves it.
    keys("cmd-c", cx);
    assert!(cx.debug_bounds("explorer-row-cut-a.rs").is_none());
    keys("escape", cx);
    assert!(clipboard(&wb, cx).is_some());
}

#[gpui::test]
fn paste_vanished_source_reports_status(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-c", cx);
    std::fs::remove_file(root.join("a.rs")).unwrap();
    click("explorer-row-src", cx);
    keys("cmd-v", cx);
    wait_until(cx, |cx| status(cx).is_some());
    let message = status(cx).unwrap();
    assert!(message.starts_with("Unable to paste 'a.rs': "), "{message}");
    assert!(!root.join("src/a.rs").exists());
}

#[gpui::test]
fn editor_copy_paste_keys_unchanged(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let view = open_file(&wb, "a.rs", cx);
    cx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(view.read(cx), cx);
        window.focus(&focus, cx);
    });
    draw(cx);
    keys("cmd-c", cx);
    keys("cmd-x", cx);
    assert_eq!(
        clipboard(&wb, cx),
        None,
        "keys in the editor stay the editor's"
    );
}

#[gpui::test]
fn root_and_single_file_menus(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-root", cx);
    assert!(!menu_has(&wb, "filesExplorer.cut", cx));
    assert!(!menu_has(&wb, "filesExplorer.copy", cx));
    assert!(
        menu_has(&wb, "filesExplorer.paste", cx),
        "pastes into the root"
    );
    assert!(!menu_has(&wb, "filesExplorer.duplicate", cx));
    keys("escape", cx);
    right_click("explorer-row-a.rs", cx);
    for command in [
        "filesExplorer.cut",
        "filesExplorer.copy",
        "filesExplorer.paste",
        "filesExplorer.duplicate",
    ] {
        assert!(menu_has(&wb, command, cx), "{command}");
    }
    keys("escape", cx);
    // Neither key acts on the root.
    click("explorer-root", cx);
    keys("cmd-c", cx);
    keys("cmd-x", cx);
    assert_eq!(clipboard(&wb, cx), None);
}

/// Single-file mode: Copy only; nothing that edits the tree.
#[gpui::test]
fn single_file_menu_has_copy_only(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Downloads");
    std::fs::create_dir(&root).unwrap();
    let file = root.join("notes.md");
    std::fs::write(&file, "hi\n").unwrap();
    let (wb, cx) = {
        let file = file.clone();
        cx.add_window_view(move |window, cx| Workbench::single_file(root, &file, None, window, cx))
    };
    wait_scan(&wb, cx);
    right_click("explorer-row-notes.md", cx);
    assert!(menu_has(&wb, "filesExplorer.copy", cx));
    assert!(!menu_has(&wb, "filesExplorer.cut", cx));
    assert!(!menu_has(&wb, "filesExplorer.paste", cx));
    assert!(!menu_has(&wb, "filesExplorer.duplicate", cx));
}

fn editing(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| wb.read(cx).explorer().read(cx).model().edit().is_some())
}

fn input_state(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> (String, Range<usize>) {
    cx.update(|_, cx| {
        let explorer = wb.read(cx).explorer().read(cx);
        let input = explorer.name_input().expect("a name input").read(cx);
        (input.text().to_string(), input.selected_range())
    })
}

#[gpui::test]
fn duplicate_file_then_renames(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    right_click("explorer-row-a.rs", cx);
    let commands: Vec<String> = menu(&wb, cx).into_iter().map(|(c, _)| c).collect();
    let paste = commands
        .iter()
        .position(|c| c == "filesExplorer.paste")
        .unwrap();
    assert_eq!(commands[paste + 1], "filesExplorer.duplicate");
    click("context-menu-item-filesExplorer.duplicate", cx);
    wait_until(cx, |_| root.join("a copy.rs").is_file());
    assert!(root.join("a.rs").is_file());
    deliver_added(&wb, &["a copy.rs"], cx);
    wait_until(cx, |cx| editing(&wb, cx));
    assert_eq!(input_state(&wb, cx), ("a copy.rs".to_string(), 0..6));
    assert_eq!(selection(&wb, cx), Selection::Path(p("a copy.rs")));
    assert!(!decorated(&wb, "a copy.rs", cx));
    // The rename works as any other.
    type_text(cx, "c");
    keys("enter", cx);
    wait_until(cx, |_| root.join("c.rs").is_file());
    assert!(!root.join("a copy.rs").exists());
}

#[gpui::test]
fn duplicate_folder(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    right_click("explorer-row-src", cx);
    click("context-menu-item-filesExplorer.duplicate", cx);
    wait_until(cx, |_| root.join("src copy/main.rs").is_file());
    deliver_added(&wb, &["src copy", "src copy/main.rs"], cx);
    wait_until(cx, |cx| editing(&wb, cx));
    assert_eq!(input_state(&wb, cx), ("src copy".to_string(), 0..8));
    assert!(tabs(&wb, cx).is_empty());
}

#[gpui::test]
fn duplicate_keeps_clipboard(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-b.rs", cx);
    keys("cmd-x", cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-d", cx);
    wait_until(cx, |_| root.join("a copy.rs").is_file());
    assert_eq!(clipboard(&wb, cx), Some((ClipOp::Cut, root.join("b.rs"))));
    assert!(cx.debug_bounds("explorer-row-cut-b.rs").is_some());
}

#[gpui::test]
fn cmd_d_duplicates(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-d", cx);
    wait_until(cx, |_| root.join("a copy.rs").is_file());
    deliver_added(&wb, &["a copy.rs"], cx);
    wait_until(cx, |cx| editing(&wb, cx));
    // Twice: the next free name.
    keys("escape", cx);
    assert!(!editing(&wb, cx));
    click("explorer-row-a.rs", cx);
    keys("cmd-d", cx);
    wait_until(cx, |_| root.join("a copy 2.rs").is_file());
    deliver_added(&wb, &["a copy 2.rs"], cx);
    wait_until(cx, |cx| editing(&wb, cx));
    assert_eq!(input_state(&wb, cx), ("a copy 2.rs".to_string(), 0..8));
    // Never on the root.
    keys("escape", cx);
    click("explorer-root", cx);
    let before = std::fs::read_dir(&root).unwrap().count();
    keys("cmd-d", cx);
    let name = root.file_name().unwrap().to_string_lossy().into_owned();
    let parent = root.parent().unwrap();
    assert!(!parent.join(format!("{name} copy")).exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), before);
    assert!(!editing(&wb, cx));
}

#[gpui::test]
fn clipboard_follows_rename_of_its_source(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-x", cx);
    keys("enter", cx);
    type_text(cx, "c");
    keys("enter", cx);
    wait_until(cx, |_| root.join("c.rs").is_file());
    wait_until(cx, |cx| {
        clipboard(&wb, cx) == Some((ClipOp::Cut, root.join("c.rs")))
    });
    deliver_added(&wb, &["c.rs"], cx);
    assert!(cx.debug_bounds("explorer-row-cut-c.rs").is_some());
}

#[gpui::test]
fn clipboard_cleared_when_its_source_is_deleted(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-c", cx);
    keys("alt-cmd-backspace", cx);
    cx.simulate_prompt_answer("Delete");
    draw(cx);
    wait_until(cx, |_| !root.join("a.rs").exists());
    wait_until(cx, |cx| clipboard(&wb, cx).is_none());
}

fn editor_focused(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        let ctx = wb.read(cx).context_keys(window, cx);
        lucerna_settings::parse_when("editorFocus")
            .unwrap()
            .eval(&ctx)
    })
}

/// The copy reported after the focus left the Explorer: no rename takes
/// the focus back.
#[gpui::test]
fn duplicate_rename_dropped_when_focus_left(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    keys("cmd-d", cx);
    // The click opened `a.rs` in the preview tab: focus its editor.
    cx.update(|window, cx| {
        let view: Entity<lucerna_workbench::code_view::CodeView> = wb
            .read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("a tab body")
            .downcast()
            .expect("a code view");
        let focus = gpui::Focusable::focus_handle(view.read(cx), cx);
        window.focus(&focus, cx);
    });
    draw(cx);
    wait_until(cx, |_| root.join("a copy.rs").is_file());
    deliver_added(&wb, &["a copy.rs"], cx);
    draw(cx);
    assert!(!editing(&wb, cx));
    assert!(editor_focused(&wb, cx));
}

/// A copy inside a collapsed folder (Duplicate from the palette or a
/// menu with `args.path`): the rename reveals it rather than being
/// skipped.
#[gpui::test]
fn duplicate_rename_reveals_hidden_copy(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    assert!(cx.debug_bounds("explorer-row-src/main.rs").is_none());
    cx.update(|window, cx| {
        let args = serde_json::json!({ "path": "src/main.rs" });
        wb.update(cx, |wb, cx| {
            wb.run_command("filesExplorer.duplicate", Some(&args), window, cx)
        });
    });
    draw(cx);
    wait_until(cx, |_| root.join("src/main copy.rs").is_file());
    deliver_added(&wb, &["src/main copy.rs"], cx);
    wait_until(cx, |cx| editing(&wb, cx));
    assert_eq!(input_state(&wb, cx), ("main copy.rs".to_string(), 0..9));
}

#[gpui::test]
fn duplicate_failure_reports_status(cx: &mut TestAppContext) {
    init(cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let root = root_of(&wb, cx);
    click("explorer-row-a.rs", cx);
    std::fs::remove_file(root.join("a.rs")).unwrap();
    keys("cmd-d", cx);
    wait_until(cx, |cx| status(cx).is_some());
    let message = status(cx).unwrap();
    assert!(
        message.starts_with("Unable to duplicate 'a.rs': "),
        "{message}"
    );
}
