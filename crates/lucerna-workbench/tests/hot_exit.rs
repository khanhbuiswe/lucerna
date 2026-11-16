//! Hot exit: a window with a data dir keeps a backup of each unsaved
//! document, written 1000 ms after its last change and deleted once the
//! document is clean or closed with Don't Save.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::hot_exit::store::{Backup, WorkspaceKey, backup_dir, read_backups};
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
    settle(cx);
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
    set_text(&wb, &view, "a", cx);
    advance(1000, cx);
    set_text(&wb, &view, "ab", cx);
    advance(1000, cx);
    settle(cx);
    let backups = project.backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].text, "ab");
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
