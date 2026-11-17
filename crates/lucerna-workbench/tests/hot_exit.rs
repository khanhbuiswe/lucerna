//! Hot exit: a window with a data dir keeps a backup of each unsaved
//! document, written 1000 ms after its last change and deleted once the
//! document is clean or closed with Don't Save.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::hot_exit::HotExit;
use lucerna_workbench::hot_exit::store::{
    Backup, WorkspaceKey, backup_dir, read_backups, text_sha256,
};
use lucerna_workbench::{
    AppState, DiskNotice, OpenMode, SaveOptions, SaveReason, Workbench, app, app_menu,
    hold_saves_for_test, save_temps,
};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext, user_settings: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    if let Some(json) = user_settings {
        settings.set_layer(Layer::User, json).unwrap();
    }
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

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn settle(cx: &mut VisualTestContext) {
    draw(cx);
    cx.run_until_parked();
    draw(cx);
}

/// A folder holding `a.rs`, and a data dir.
struct Project {
    root: TempDir,
    data: TempDir,
}

impl Project {
    fn new(workspace_settings: Option<&str>) -> Self {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("a.rs"), "fn main() {}\n").unwrap();
        if let Some(json) = workspace_settings {
            fs::create_dir(root.path().join(".lucerna")).unwrap();
            fs::write(root.path().join(".lucerna").join("settings.json"), json).unwrap();
        }
        Project {
            root,
            data: TempDir::new().unwrap(),
        }
    }

    /// The folder window's backup folder.
    fn dir(&self) -> PathBuf {
        let key = WorkspaceKey::Folder(self.root.path().to_path_buf());
        backup_dir(self.data.path(), &key)
    }

    fn backups(&self) -> Vec<Backup> {
        read_backups(&self.dir())
    }
}

/// A window on `root` that keeps its state in `data_dir`.
fn open_with_data_dir<'a>(
    root: &Path,
    data_dir: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let (root, data_dir) = (root.to_path_buf(), data_dir.to_path_buf());
    let (wb, cx) =
        cx.add_window_view(|window, cx| Workbench::with_data_dir(root, data_dir, window, cx));
    settle(cx);
    (wb, cx)
}

/// Opens `a.rs` and returns its view once loaded.
fn open_a(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<CodeView> {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("a.rs")), OpenMode::Pinned, cx)
    });
    settle(cx);
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

/// Replaces the whole text of `a.rs` with `text`, as an edit from `view`.
fn set_text(
    wb: &Entity<Workbench>,
    view: &Entity<CodeView>,
    text: &str,
    cx: &mut VisualTestContext,
) {
    edit_text(wb, view, text, cx);
    settle(cx);
}

/// [`set_text`] without running anything after it.
fn edit_text(
    wb: &Entity<Workbench>,
    view: &Entity<CodeView>,
    text: &str,
    cx: &mut VisualTestContext,
) {
    let docs = cx.update(|_, cx| wb.read(cx).documents().clone());
    let origin = view.entity_id();
    let text: Arc<str> = Arc::from(text);
    docs.update(cx, |d, cx| {
        let len = d.snapshot_of(Path::new("a.rs")).unwrap().len_bytes();
        d.apply_edits(
            Path::new("a.rs"),
            vec![(0..len, text)],
            None,
            None,
            origin,
            cx,
        )
        .expect("a.rs is loaded");
    });
}

fn hot_exit(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<HotExit> {
    cx.update(|_, cx| wb.read(cx).hot_exit().expect("a data dir").clone())
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.run_command(command, None, window, cx)));
    settle(cx);
}

fn advance(ms: u64, cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.run_until_parked();
}

#[gpui::test]
fn edit_writes_backup_after_delay(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    advance(999, cx);
    assert!(project.backups().is_empty(), "not before the delay");
    advance(2, cx);
    let backups = project.backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].text, "hello");
    assert_eq!(backups[0].header.path, project.root.path().join("a.rs"));
}

#[gpui::test]
fn save_removes_backup(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    advance(1001, cx);
    assert_eq!(project.backups().len(), 1);
    run(&wb, "workbench.action.files.save", cx);
    settle(cx);
    assert!(project.backups().is_empty());
    assert!(!project.dir().exists());
}

#[gpui::test]
fn revert_removes_backup(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    advance(1001, cx);
    assert_eq!(project.backups().len(), 1);
    run(&wb, "workbench.action.files.revert", cx);
    settle(cx);
    assert!(project.backups().is_empty());
}

#[gpui::test]
fn close_tab_dont_save_removes_backup(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    advance(1001, cx);
    assert_eq!(project.backups().len(), 1);
    // Only the tab holds the view, so closing it closes the document.
    drop(view);
    run(&wb, "workbench.action.closeActiveEditor", cx);
    cx.simulate_prompt_answer("Don't Save");
    settle(cx);
    assert!(project.backups().is_empty());
}

#[gpui::test]
fn save_during_backup_write_leaves_no_backup(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    // The timer is due; its write starts with whatever runs next.
    cx.executor().advance_clock(Duration::from_millis(1000));
    run(&wb, "workbench.action.files.save", cx);
    settle(cx);
    advance(2000, cx);
    assert!(project.backups().is_empty());
    assert!(!project.dir().exists());
}

#[gpui::test]
fn backup_writes_keep_order(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    let hot_exit = hot_exit(&wb, cx);
    // Both writes are queued before either runs; the executor may run
    // them in either order.
    edit_text(&wb, &view, "a", cx);
    let first = hot_exit.update(cx, |h, cx| h.flush(cx));
    edit_text(&wb, &view, "ab", cx);
    let second = hot_exit.update(cx, |h, cx| h.flush(cx));
    settle(cx);
    assert_eq!(first.now_or_never(), Some(Vec::new()));
    assert_eq!(second.now_or_never(), Some(Vec::new()));
    let backups = project.backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].text, "ab");
}

#[gpui::test]
fn save_while_edits_are_in_flight_leaves_a_backup_with_the_new_base(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    let hot_exit = hot_exit(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    let docs = cx.update(|_, cx| wb.read(cx).documents().clone());
    let opts = SaveOptions {
        reason: SaveReason::Explicit,
        participants: false,
    };
    let save = cx
        .update(|window, cx| docs.update(cx, |d, cx| d.save(Path::new("a.rs"), opts, window, cx)));
    // Typed while the save writes, and backed up before the save ends: the
    // backup still has the old base.
    edit_text(&wb, &view, "hello world", cx);
    let flushed = hot_exit.update(cx, |h, cx| h.flush(cx));
    settle(cx);
    assert_eq!(save.now_or_never().map(|r| r.is_ok()), Some(true));
    assert_eq!(flushed.now_or_never(), Some(Vec::new()));
    assert_eq!(
        fs::read_to_string(project.root.path().join("a.rs")).unwrap(),
        "hello"
    );
    // Quitting now writes what is not current.
    hot_exit.update(cx, |h, cx| h.flush_blocking(cx));
    let backups = project.backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].text, "hello world");
    assert_eq!(
        backups[0].header.base_sha256,
        Some(text_sha256("hello")),
        "the base is the text just saved"
    );
    let meta = fs::metadata(project.root.path().join("a.rs")).unwrap();
    assert_eq!(backups[0].header.size, Some(meta.len()));
}

#[gpui::test]
fn hot_exit_off_writes_nothing(cx: &mut TestAppContext) {
    init(cx, Some(r#"{"files.hotExit":"off"}"#));
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    advance(2000, cx);
    assert!(!project.dir().exists());
}

#[gpui::test]
fn workspace_value_is_ignored(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(Some(r#"{"files.hotExit":"off"}"#));
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    advance(1001, cx);
    let backups = project.backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].text, "hello");
}

// ---- Restore (spec §5.4) ----

/// The document of `path` (relative to the root, or absolute).
fn docs(
    wb: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> Entity<lucerna_workbench::Documents> {
    cx.update(|_, cx| wb.read(cx).documents().clone())
}

/// Replaces bytes `range` of `path`'s text with `text`, as an edit from
/// `view`.
fn edit_range(
    wb: &Entity<Workbench>,
    view: &Entity<CodeView>,
    path: &Path,
    range: std::ops::Range<usize>,
    text: &str,
    cx: &mut VisualTestContext,
) {
    let docs = docs(wb, cx);
    let origin = view.entity_id();
    let text: Arc<str> = Arc::from(text);
    docs.update(cx, |d, cx| {
        d.apply_edits(path, vec![(range, text)], None, None, origin, cx)
            .expect("the document is loaded");
    });
    settle(cx);
}

/// Opens `path` and returns its view once loaded.
fn open(wb: &Entity<Workbench>, path: &Path, cx: &mut VisualTestContext) -> Entity<CodeView> {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(path), OpenMode::Pinned, cx)
    });
    settle(cx);
    active_view(wb, cx)
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

/// The paths of every tab, in every group.
fn tab_paths(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<PathBuf> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .groups()
            .iter()
            .flat_map(|g| g.tabs.iter().filter_map(|t| t.path()))
            .map(|p| p.to_path_buf())
            .collect()
    })
}

fn text_of(wb: &Entity<Workbench>, path: &Path, cx: &mut VisualTestContext) -> String {
    let docs = docs(wb, cx);
    docs.read_with(cx, |d, _| {
        let s = d.snapshot_of(path).expect("the document is loaded");
        s.text_for_range(0..s.len_bytes())
    })
}

/// Quits the window the way Cmd+Q does once its backups are written: the
/// backups stay.
fn quit(wb: Entity<Workbench>, cx: &mut VisualTestContext) {
    let hot_exit = hot_exit(&wb, cx);
    hot_exit.update(cx, |h, _| h.set_quitting());
    let flushed = hot_exit.update(cx, |h, cx| h.flush(cx));
    settle(cx);
    assert_eq!(flushed.now_or_never(), Some(Vec::new()));
    drop((wb, hot_exit));
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
}

/// Opens the project's folder window again, with the same data dir.
fn reopen<'a>(
    project: &Project,
    cx: &'a mut VisualTestContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let app: &'a mut TestAppContext = cx;
    open_with_data_dir(project.root.path(), project.data.path(), app)
}

fn is_dirty(wb: &Entity<Workbench>, path: &Path, cx: &mut VisualTestContext) -> bool {
    let docs = docs(wb, cx);
    docs.read_with(cx, |d, _| d.is_dirty(path))
}

#[gpui::test]
fn reopen_restores_dirty_tab(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "mine", cx);
    drop(view);
    quit(wb, cx);
    let (wb, cx) = reopen(&project, cx);
    let a = project.root.path().join("a.rs");
    let tabs = tab_paths(&wb, cx);
    assert_eq!(
        tabs.iter().filter(|p| **p == a).count(),
        1,
        "one tab: {tabs:?}"
    );
    assert!(is_dirty(&wb, &a, cx));
    assert_eq!(text_of(&wb, &a, cx), "mine");
    assert_eq!(project.backups().len(), 1, "the backup stays until saved");
}

#[gpui::test]
fn restored_inactive_tab_keeps_its_document(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let a = project.root.path().join("a.rs");
    let b = project.root.path().join("b.rs");
    fs::write(&b, "fn b() {}\n").unwrap();
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open(&wb, &a, cx);
    edit_range(&wb, &view, &a, 0..2, "pub fn", cx);
    let view_b = open(&wb, &b, cx);
    edit_range(&wb, &view_b, &b, 0..2, "pub fn", cx);
    drop((view, view_b));
    quit(wb, cx);
    let (wb, cx) = reopen(&project, cx);
    // Draws: only one restored tab is active, the other is never drawn.
    settle(cx);
    advance(2000, cx);
    let active = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups
            .model()
            .active_tab()
            .and_then(|t| t.path())
            .map(|p| p.to_path_buf())
    });
    let inactive = if active.as_deref() == Some(a.as_path()) {
        &b
    } else {
        &a
    };
    assert!(is_dirty(&wb, inactive, cx), "{inactive:?} is still unsaved");
    let expected = if inactive == &a {
        "pub fn main() {}\n"
    } else {
        "pub fn b() {}\n"
    };
    assert_eq!(text_of(&wb, inactive, cx), expected);
    let backups = project.backups();
    assert!(
        backups.iter().any(|backup| &backup.header.path == inactive),
        "{inactive:?} keeps its backup"
    );
    assert_eq!(backups.len(), 2);

    // Closing the never-drawn tab with Save closes its document too.
    let id = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        let tab = groups.model().groups().iter().flat_map(|g| &g.tabs);
        tab.clone()
            .find(|t| t.path().is_some_and(|p| **p == **inactive))
            .map(|t| t.id)
            .expect("a tab of the inactive file")
    });
    let groups = cx.update(|_, cx| wb.read(cx).editor_groups().clone());
    cx.update(|window, cx| groups.update(cx, |g, cx| g.request_close(id, window, cx)));
    cx.simulate_prompt_answer("Save");
    settle(cx);
    advance(2000, cx);
    assert_eq!(fs::read_to_string(inactive).unwrap(), expected);
    assert!(!tab_paths(&wb, cx).contains(inactive));
    let docs = docs(&wb, cx);
    assert!(
        docs.read_with(cx, |d, _| d.snapshot_of(inactive).is_none()),
        "no document is left open without a tab"
    );
    assert_eq!(project.backups().len(), 1);
}

#[gpui::test]
fn restore_keeps_encoding_and_line_ending(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let a = project.root.path().join("a.txt");
    let utf16 = |s: &str| -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        bytes
    };
    fs::write(&a, utf16("x\r\ny\r\n")).unwrap();
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open(&wb, &a, cx);
    edit_range(&wb, &view, &a, 0..1, "z", cx);
    drop(view);
    quit(wb, cx);
    let (wb, cx) = reopen(&project, cx);
    assert_eq!(text_of(&wb, &a, cx), "z\ny\n");
    run(&wb, "workbench.action.files.save", cx);
    settle(cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(fs::read(&a).unwrap(), utf16("z\r\ny\r\n"));
    assert!(!is_dirty(&wb, &a, cx));
}

#[gpui::test]
fn changed_on_disk_while_closed_asks_on_save(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let a = project.root.path().join("a.rs");
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "mine", cx);
    drop(view);
    quit(wb, cx);
    fs::write(&a, "agent").unwrap();
    let (wb, cx) = reopen(&project, cx);
    assert_eq!(text_of(&wb, &a, cx), "mine");
    assert!(is_dirty(&wb, &a, cx));
    assert!(!cx.has_pending_prompt(), "no notice at restore");
    run(&wb, "workbench.action.files.save", cx);
    assert!(
        cx.has_pending_prompt(),
        "the file changed on disk: asks first"
    );
    assert_eq!(fs::read_to_string(&a).unwrap(), "agent");
}

#[gpui::test]
fn unchanged_disk_auto_merges_later(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let a = project.root.path().join("a.rs");
    let lines: Vec<String> = (1..=12).map(|i| format!("line {i}")).collect();
    let original = lines.join("\n") + "\n";
    fs::write(&a, &original).unwrap();
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    edit_range(&wb, &view, &a, 0.."line 1".len(), "mine", cx);
    drop(view);
    quit(wb, cx);
    let (wb, cx) = reopen(&project, cx);
    let theirs = original.replace("line 10\n", "agent\n");
    fs::write(&a, &theirs).unwrap();
    let change = lucerna_project::Change {
        path: Arc::from(Path::new("a.rs")),
        kind: lucerna_project::ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&lucerna_project::WorktreeEvent::Changed(vec![change]), cx)
    });
    settle(cx);
    let docs = docs(&wb, cx);
    assert!(!docs.read_with(cx, |d, _| d.has_conflict(&a)));
    assert_eq!(
        text_of(&wb, &a, cx),
        theirs.replacen("line 1\n", "mine\n", 1)
    );
    assert!(is_dirty(&wb, &a, cx));
}

#[gpui::test]
fn deleted_while_closed_recreates_on_save(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let a = project.root.path().join("a.rs");
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "mine", cx);
    drop(view);
    quit(wb, cx);
    fs::remove_file(&a).unwrap();
    let (wb, cx) = reopen(&project, cx);
    assert!(tab_paths(&wb, cx).contains(&a));
    assert_eq!(text_of(&wb, &a, cx), "mine");
    run(&wb, "workbench.action.files.save", cx);
    settle(cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(fs::read_to_string(&a).unwrap(), "mine");
}

/// A single-file window on `file` (in the canonical `root`) that keeps
/// its state in `data_dir`, with `file` opened.
fn open_single_file<'a>(
    root: &Path,
    file: &Path,
    data_dir: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let (root, file, data_dir) = (
        root.to_path_buf(),
        file.to_path_buf(),
        data_dir.to_path_buf(),
    );
    let single = file.clone();
    let (wb, cx) = cx.add_window_view(move |window, cx| {
        Workbench::single_file(root, &single, Some(data_dir), window, cx)
    });
    settle(cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(file.as_path()), OpenMode::Pinned, cx)
    });
    settle(cx);
    (wb, cx)
}

#[gpui::test]
fn deleted_single_file_window_restores_as_unsaved(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let root = fs::canonicalize(project.root.path()).unwrap();
    let a = root.join("a.rs");
    let (wb, cx) = open_single_file(&root, &a, project.data.path(), cx);
    let view = active_view(&wb, cx);
    set_text(&wb, &view, "mine", cx);
    drop(view);
    quit(wb, cx);
    fs::remove_file(&a).unwrap();
    let app: &mut TestAppContext = cx;
    let (wb, cx) = open_single_file(&root, &a, project.data.path(), app);
    assert_eq!(tab_paths(&wb, cx), vec![a.clone()]);
    assert!(is_dirty(&wb, &a, cx));
    assert_eq!(text_of(&wb, &a, cx), "mine");
    let docs = docs(&wb, cx);
    assert_eq!(docs.read_with(cx, |d, _| d.disk_notice(&a)), None);
    assert!(!cx.has_pending_prompt());
    run(&wb, "workbench.action.files.save", cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(fs::read_to_string(&a).unwrap(), "mine");
}

#[gpui::test]
fn outside_root_file_restored(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let outside = TempDir::new().unwrap();
    let b = outside.path().join("b.rs");
    fs::write(&b, "fn b() {}\n").unwrap();
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open(&wb, &b, cx);
    edit_range(&wb, &view, &b, 0..2, "pub fn", cx);
    drop(view);
    quit(wb, cx);
    let (wb, cx) = reopen(&project, cx);
    assert!(tab_paths(&wb, cx).contains(&b));
    assert!(is_dirty(&wb, &b, cx));
    assert_eq!(text_of(&wb, &b, cx), "pub fn b() {}\n");
}

// ---- Invalid bytes (spec §6.2) ----

/// `a.rs` with a BOM and a byte that is not valid UTF-8.
fn write_lossy_a(project: &Project) -> PathBuf {
    let a = project.root.path().join("a.rs");
    fs::write(&a, b"\xEF\xBB\xBFfn \xE9\n").unwrap();
    a
}

#[gpui::test]
fn lossy_flag_survives_restore_and_save_asks_again(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let a = write_lossy_a(&project);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open(&wb, &a, cx);
    edit_range(&wb, &view, &a, 0..0, "x", cx);
    drop(view);
    quit(wb, cx);
    assert!(project.backups()[0].header.lossy);
    let (wb, cx) = reopen(&project, cx);
    let docs = docs(&wb, cx);
    assert_eq!(
        docs.read_with(cx, |d, _| d.disk_notice(&a)),
        Some(DiskNotice::Lossy { encoding: "UTF-8" })
    );
    run(&wb, "workbench.action.files.save", cx);
    settle(cx);
    let (title, _) = cx.pending_prompt().expect("the first save asks");
    assert!(
        title.starts_with("Save 'a.rs' and replace invalid bytes?"),
        "{title}"
    );
}

#[gpui::test]
fn quit_with_auto_save_backs_up_a_lossy_file_without_asking(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{"files.autoSave":"afterDelay","files.autoSaveDelay":60000}"#),
    );
    let project = Project::new(None);
    let a = write_lossy_a(&project);
    let before = fs::read(&a).unwrap();
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open(&wb, &a, cx);
    edit_range(&wb, &view, &a, 0..0, "x", cx);
    quit_app(cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(quits(cx), 1);
    assert_eq!(fs::read(&a).unwrap(), before, "the file is not saved");
    let backups = project.backups();
    assert_eq!(backups.len(), 1);
    assert!(backups[0].header.lossy);
}

// ---- Quit (spec §5.3) ----

/// Cmd+Q through the app menu, with quits counted instead of done.
fn quit_app(cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        app_menu::install(cx);
        app::count_quits_for_test(cx);
        cx.dispatch_action(&app_menu::Quit);
    });
    cx.run_until_parked();
}

fn quits(cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| app::quits_for_test(cx))
}

#[gpui::test]
fn quit_with_hot_exit_does_not_ask(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    // Before the 1000 ms backup delay.
    quit_app(cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(quits(cx), 1);
    let backups = project.backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].text, "hello");
}

/// With hot exit on, Cmd+Q asks no unsaved question but still asks about
/// an Explorer file operation that is running.
#[gpui::test]
fn quit_with_hot_exit_asks_while_a_file_operation_runs(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    let _op = cx.update(|_, cx| wb.read(cx).begin_file_operation());
    quit_app(cx);
    let (message, _) = cx
        .pending_prompt()
        .expect("the running operation is asked about");
    assert_eq!(message, "A file operation is still running");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(quits(cx), 0, "Cancel stops the quit");
    assert_eq!(project.backups().len(), 1, "the backup stays");
}

#[gpui::test]
fn quit_asks_for_files_whose_backup_failed(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    // A file where the backups folder goes: no backup can be written.
    fs::write(project.data.path().join("backups"), "").unwrap();
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    quit_app(cx);
    let (message, _) = cx.pending_prompt().expect("the failed file is asked about");
    assert!(message.contains("a.rs"), "{message}");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(quits(cx), 0, "Cancel stops the quit");
    assert!(is_dirty(&wb, Path::new("a.rs"), cx));
}

#[gpui::test]
fn quit_hot_exit_off_asks(cx: &mut TestAppContext) {
    init(cx, Some(r#"{"files.hotExit":"off"}"#));
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    quit_app(cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(quits(cx), 0);
    assert!(!project.dir().exists());
}

#[gpui::test]
fn quit_saves_auto_save_files(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{"files.autoSave":"afterDelay","files.autoSaveDelay":60000}"#),
    );
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    quit_app(cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(quits(cx), 1);
    assert_eq!(
        fs::read_to_string(project.root.path().join("a.rs")).unwrap(),
        "hello"
    );
    assert!(project.backups().is_empty());
}

/// Dock Quit and Log Out reach only the `on_app_quit` hooks, which run in
/// the order they were registered, and the first window's language-server
/// shutdown can block for 5 s: every window's backups are written by the
/// first window's hot exit hook, before any hook registered after it.
#[gpui::test]
fn app_quit_hook_writes_every_window_first(cx: &mut TestAppContext) {
    init(cx, None);
    let (first, second) = (Project::new(None), Project::new(None));
    let mut app = cx.clone();
    let _ = open_with_data_dir(first.root.path(), first.data.path(), cx);
    // Registered after the first window's hooks, before the second's.
    let seen: Rc<RefCell<Option<Vec<String>>>> = Rc::default();
    let dir = second.dir();
    app.update(|cx| {
        let seen = seen.clone();
        cx.on_app_quit(move |_| {
            let texts = read_backups(&dir).into_iter().map(|b| b.text).collect();
            *seen.borrow_mut() = Some(texts);
            async {}
        })
        .detach();
    });
    let (wb, vcx) = open_with_data_dir(second.root.path(), second.data.path(), &mut app);
    let view = open_a(&wb, vcx);
    // Before the 1000 ms backup delay.
    set_text(&wb, &view, "hello", vcx);
    assert!(second.backups().is_empty());
    drop((wb, view));
    app.quit();
    assert_eq!(seen.take(), Some(vec!["hello".to_string()]));
}

#[gpui::test]
fn quit_waits_for_a_save_in_flight(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    let hold = cx.update(|_, cx| hold_saves_for_test(cx));
    run(&wb, "workbench.action.files.save", cx);
    quit_app(cx);
    assert_eq!(quits(cx), 0, "waits for the write");
    drop(hold);
    cx.run_until_parked();
    assert_eq!(quits(cx), 1);
    assert_eq!(
        fs::read_to_string(project.root.path().join("a.rs")).unwrap(),
        "hello"
    );
}

#[gpui::test]
fn quit_dont_save_waits_for_a_save_in_flight(cx: &mut TestAppContext) {
    init(cx, Some(r#"{"files.hotExit":"off"}"#));
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    let hold = cx.update(|_, cx| hold_saves_for_test(cx));
    run(&wb, "workbench.action.files.save", cx);
    quit_app(cx);
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert_eq!(quits(cx), 0, "waits for the write");
    drop(hold);
    cx.run_until_parked();
    assert_eq!(quits(cx), 1);
}

#[gpui::test]
fn quit_gives_up_on_a_stuck_save_after_5_seconds(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    let _hold = cx.update(|_, cx| hold_saves_for_test(cx));
    run(&wb, "workbench.action.files.save", cx);
    // Stands for the temp file of the stuck write.
    let temp = project.root.path().join(".a.rs.luc~stuck000");
    fs::write(&temp, "hel").unwrap();
    cx.update(|_, cx| save_temps(cx).track(temp.clone()));
    quit_app(cx);
    advance(4_999, cx);
    assert_eq!(quits(cx), 0);
    assert!(temp.exists());
    advance(1, cx);
    assert_eq!(quits(cx), 1);
    assert!(!temp.exists());
    assert_eq!(
        fs::read_to_string(project.root.path().join("a.rs")).unwrap(),
        "fn main() {}\n"
    );
}

/// Dock Quit and Log Out run only the `on_app_quit` hooks: they delete the
/// temp files of writes in flight without waiting for them.
#[gpui::test]
fn app_quit_hook_removes_save_temp_files(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let app = cx.clone();
    let _ = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let temp = project.root.path().join(".a.rs.luc~stuck000");
    fs::write(&temp, "hel").unwrap();
    app.update(|cx| save_temps(cx).track(temp.clone()));
    app.quit();
    assert!(!temp.exists());
}

#[gpui::test]
fn close_window_still_asks(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    advance(1001, cx);
    assert_eq!(project.backups().len(), 1);
    drop((wb, view));
    assert!(!cx.simulate_close(), "the window waits for the answer");
    settle(cx);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert!(project.backups().is_empty());
}

// ---- Don't Save while a quit or close waits on the terminal prompt ----

/// Makes every terminal a fake that never exits. Before the window opens:
/// it takes the spawner as it is made.
fn fake_terminals(cx: &mut TestAppContext) {
    cx.update(|cx| {
        lucerna_workbench::set_default_terminal_spawner(
            Rc::new(|opts: lucerna_terminal::TerminalOptions| {
                let (tx, rx) = futures::channel::mpsc::unbounded();
                // Kept open: the terminal runs until the test ends.
                std::mem::forget(tx);
                Ok((
                    lucerna_terminal::Terminal::fake(opts.rows, opts.cols, opts.scrollback),
                    rx,
                ))
            }),
            cx,
        )
    });
}

/// Opens a (fake) terminal in `wb`: with `confirmOnExit: always` a quit
/// or close asks about it.
fn open_running_terminal(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    run(wb, "workbench.action.terminal.new", cx);
}

const CONFIRM_TERMINALS: &str = r#"{"terminal.integrated.confirmOnExit":"always"}"#;

/// Cmd+Q writes the backup and waits on the terminal prompt; meanwhile the
/// tab of `a.rs` is closed with Don't Save. Returns once that is done.
fn dont_save_tab_while_quit_waits<'a>(
    project: &Project,
    cx: &'a mut TestAppContext,
) -> &'a mut VisualTestContext {
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    open_running_terminal(&wb, cx);
    // Only the tab holds the view, so closing it closes the document.
    drop(view);
    // The tab's prompt is asked first; Cmd+Q comes while it is up.
    run(&wb, "workbench.action.closeActiveEditor", cx);
    quit_app(cx);
    assert_eq!(project.backups().len(), 1, "the quit wrote the backup");
    cx.simulate_prompt_answer("Don't Save");
    settle(cx);
    assert!(cx.has_pending_prompt(), "the terminal prompt is still up");
    cx
}

#[gpui::test]
fn dont_save_during_quit_then_cancel_leaves_no_backup(cx: &mut TestAppContext) {
    init(cx, Some(CONFIRM_TERMINALS));
    fake_terminals(cx);
    let project = Project::new(None);
    let cx = dont_save_tab_while_quit_waits(&project, cx);
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert_eq!(quits(cx), 0);
    assert!(project.backups().is_empty());
}

#[gpui::test]
fn dont_save_during_quit_then_terminate_leaves_no_backup(cx: &mut TestAppContext) {
    init(cx, Some(CONFIRM_TERMINALS));
    fake_terminals(cx);
    let project = Project::new(None);
    let app = cx.clone();
    let cx = dont_save_tab_while_quit_waits(&project, cx);
    cx.simulate_prompt_answer("Terminate");
    settle(cx);
    assert_eq!(quits(cx), 1);
    // The quit hooks run as the app quits.
    app.quit();
    assert!(project.backups().is_empty());
}

/// Don't Save at the quit's prompt for a file whose backup failed: the
/// quit hook does not write it once the failure has passed.
#[gpui::test]
fn dont_save_at_failed_backup_prompt_survives_quit_hook(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    // A file where the backups folder goes: no backup can be written.
    let blocker = project.data.path().join("backups");
    fs::write(&blocker, "").unwrap();
    let app = cx.clone();
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    quit_app(cx);
    assert!(cx.has_pending_prompt(), "the failed file is asked about");
    // The failure was transient.
    fs::remove_file(&blocker).unwrap();
    cx.simulate_prompt_answer("Don't Save");
    settle(cx);
    assert_eq!(quits(cx), 1);
    drop((wb, view));
    app.quit();
    assert!(project.backups().is_empty());
}

/// Don't Save, then Cancel at the terminal prompt, as the window closes:
/// the window stays with its unsaved files, which keep their backups.
#[gpui::test]
fn close_window_dont_save_then_cancel_keeps_backup(cx: &mut TestAppContext) {
    init(cx, Some(CONFIRM_TERMINALS));
    fake_terminals(cx);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    advance(1001, cx);
    assert_eq!(project.backups().len(), 1);
    open_running_terminal(&wb, cx);
    assert!(!cx.simulate_close(), "the window waits for the answer");
    settle(cx);
    cx.simulate_prompt_answer("Don't Save");
    settle(cx);
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert!(!cx.has_pending_prompt());
    assert!(is_dirty(&wb, Path::new("a.rs"), cx));
    let backups = project.backups();
    assert_eq!(backups.len(), 1, "the window kept its unsaved file");
    assert_eq!(backups[0].text, "hello");
}

/// A discarded path is skipped by flushes only until its next edit.
#[gpui::test]
fn discarded_path_is_backed_up_again_after_an_edit(cx: &mut TestAppContext) {
    init(cx, None);
    let project = Project::new(None);
    let (wb, cx) = open_with_data_dir(project.root.path(), project.data.path(), cx);
    let view = open_a(&wb, cx);
    set_text(&wb, &view, "hello", cx);
    let hot_exit = hot_exit(&wb, cx);
    let a: Arc<Path> = Arc::from(project.root.path().join("a.rs"));
    hot_exit.update(cx, |h, cx| h.discard(&[a], cx));
    let flushed = hot_exit.update(cx, |h, cx| h.flush(cx));
    settle(cx);
    assert_eq!(flushed.now_or_never(), Some(Vec::new()));
    assert!(project.backups().is_empty(), "the flush skips it");
    set_text(&wb, &view, "hello again", cx);
    let flushed = hot_exit.update(cx, |h, cx| h.flush(cx));
    settle(cx);
    assert_eq!(flushed.now_or_never(), Some(Vec::new()));
    let backups = project.backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].text, "hello again");
}
