//! The read-only Explorer commands: Open to the Side, Reveal, Open in
//! Terminal, Copy Path and Copy Relative Path.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{
    Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point, TestAppContext,
    VisualTestContext,
};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::system_shell::{SystemShell, SystemShellGlobal};
use lucerna_workbench::{AppState, Workbench};
use serde_json::json;
use tempfile::TempDir;

const TRUSTED: &str = r#"{ "session.trustAllWorktrees": true }"#;

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Reveal(PathBuf),
    Terminal(PathBuf, String),
}

struct RecordingShell {
    calls: Arc<Mutex<Vec<Call>>>,
    fail: bool,
}

impl SystemShell for RecordingShell {
    fn reveal(&self, path: &Path, _cx: &gpui::App) {
        self.calls.lock().unwrap().push(Call::Reveal(path.into()));
    }

    fn open_terminal(&self, dir: &Path, exec: &str) -> std::io::Result<()> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::Terminal(dir.into(), exec.into()));
        if self.fail {
            Err(std::io::ErrorKind::NotFound.into())
        } else {
            Ok(())
        }
    }
}

fn install_shell(fail: bool, cx: &mut TestAppContext) -> Arc<Mutex<Vec<Call>>> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    cx.set_global(SystemShellGlobal(Arc::new(RecordingShell {
        calls: calls.clone(),
        fail,
    })));
    calls
}

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

/// A project with `src/main.rs`, `a.rs` and `b.rs`.
fn project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    for file in ["src/main.rs", "a.rs", "b.rs"] {
        std::fs::write(dir.path().join(file), "fn main() {}\n").unwrap();
    }
    dir
}

/// Opens the workbench on `root` once its scan finished, then delivers
/// `ScanComplete` by hand so the test does not depend on event timing.
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

fn right_click_at(position: Point<Pixels>, cx: &mut VisualTestContext) {
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

fn right_click(selector: &'static str, cx: &mut VisualTestContext) {
    let position = center(selector, cx);
    right_click_at(position, cx);
}

fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

fn root_of(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> PathBuf {
    cx.update(|_, cx| wb.read(cx).root().to_path_buf())
}

#[gpui::test]
fn menu_copy_path_writes_absolute_path(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-a.rs", cx);
    click("context-menu-item-copyFilePath", cx);
    assert_eq!(
        clipboard(cx),
        Some(root_of(&wb, cx).join("a.rs").to_string_lossy().into_owned())
    );
}

#[gpui::test]
fn copy_relative_path_key(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = project();
    let (_wb, cx) = open(dir.path(), cx);
    click("explorer-row-src", cx);
    click("explorer-row-src/main.rs", cx);
    cx.simulate_keystrokes("alt-cmd-shift-c");
    assert_eq!(
        clipboard(cx),
        Some(format!("src{}main.rs", std::path::MAIN_SEPARATOR))
    );
}

#[gpui::test]
fn reveal_records_target(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let calls = install_shell(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-row-a.rs", cx);
    cx.simulate_keystrokes("alt-cmd-r");
    let expected = Call::Reveal(root_of(&wb, cx).join("a.rs"));
    assert_eq!(*calls.lock().unwrap(), vec![expected]);
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn terminal_opens_parent_of_file_with_setting(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "session.trustAllWorktrees": true, "terminal.external.osxExec": "iTerm.app" }"#),
    );
    let calls = install_shell(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-a.rs", cx);
    click("context-menu-item-openInTerminal", cx);
    let expected = Call::Terminal(root_of(&wb, cx), "iTerm.app".into());
    assert_eq!(*calls.lock().unwrap(), vec![expected]);
}

#[gpui::test]
fn terminal_error_shows_status(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    install_shell(true, cx);
    let dir = project();
    let (_wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-a.rs", cx);
    click("context-menu-item-openInTerminal", cx);
    let message = cx.update(|_, cx| lucerna_workbench::status_message::current(cx));
    let message = message.expect("a status message");
    assert!(message.contains("Couldn't open the terminal"), "{message}");
}

#[gpui::test]
fn open_to_side_uses_second_group(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-row-a.rs", cx);
    cx.simulate_keystrokes("ctrl-enter");
    draw(cx);
    let (groups, second) = cx.update(|_, cx| {
        let model = wb.read(cx).editor_groups().read(cx).model();
        let groups = model.groups();
        let second = groups.get(1).map(|g| {
            g.tabs
                .iter()
                .filter_map(|t| t.path().map(|p| p.to_path_buf()))
                .collect::<Vec<_>>()
        });
        (groups.len(), second)
    });
    assert_eq!(groups, 2);
    assert_eq!(second, Some(vec![root_of(&wb, cx).join("a.rs")]));
}

#[gpui::test]
fn args_path_overrides_selection(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-row-a.rs", cx);
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("copyFilePath", Some(&json!({"path": "b.rs"})), window, cx)
        })
    });
    assert_eq!(
        clipboard(cx),
        Some(root_of(&wb, cx).join("b.rs").to_string_lossy().into_owned())
    );
}

#[gpui::test]
fn copy_relative_path_respects_separator_setting(cx: &mut TestAppContext) {
    init(
        cx,
        Some(
            r#"{ "session.trustAllWorktrees": true, "explorer.copyRelativePathSeparator": "\\" }"#,
        ),
    );
    let dir = project();
    let (_wb, cx) = open(dir.path(), cx);
    click("explorer-row-src", cx);
    click("explorer-row-src/main.rs", cx);
    cx.simulate_keystrokes("alt-cmd-shift-c");
    assert_eq!(clipboard(cx), Some("src\\main.rs".to_string()));
}

#[gpui::test]
fn open_to_side_ignores_missing_path(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command(
                "explorer.openToSide",
                Some(&json!({"path": "nope.rs"})),
                window,
                cx,
            )
        })
    });
    draw(cx);
    let (groups, tabs) = cx.update(|_, cx| {
        let model = wb.read(cx).editor_groups().read(cx).model();
        (
            model.groups().len(),
            model.groups().iter().map(|g| g.tabs.len()).sum::<usize>(),
        )
    });
    assert_eq!((groups, tabs), (1, 0));
}

/// A command run from the menu may update the Explorer itself (Rename and
/// New File will): it must not hit the Explorer's own update.
#[gpui::test]
fn menu_command_may_update_the_explorer(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    static RAN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    wb.update(cx, |wb, _| {
        wb.commands_mut()
            .register("copyFilePath", "", |wb, _, _, cx| {
                wb.explorer().update(cx, |_, cx| cx.notify());
                RAN.store(true, std::sync::atomic::Ordering::SeqCst);
            });
    });
    right_click("explorer-row-a.rs", cx);
    click("context-menu-item-copyFilePath", cx);
    assert!(RAN.load(std::sync::atomic::Ordering::SeqCst));
}
