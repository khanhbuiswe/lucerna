//! The read-only Explorer commands: Open to the Side, Reveal, Open in
//! Terminal, Copy Path and Copy Relative Path.

use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{
    Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point,
    TestAppContext, VisualTestContext,
};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_terminal::TerminalOptions;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::system_shell::{SystemShell, SystemShellGlobal};
use lucerna_workbench::{AppState, TabItem, TerminalSpawner, Workbench};
use serde_json::json;
use tempfile::TempDir;

/// Language servers off: opening `a.rs` would otherwise start the real
/// `rust-analyzer` from `PATH`, whose threads make the test flaky.
const TRUSTED: &str = r#"{ "session.trustAllWorktrees": true, "lucerna.lsp.enabled": false }"#;

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

    fn trash(&self, path: &Path) -> Result<(), String> {
        Err(format!("unexpected trash of {}", path.display()))
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
        Some(
            r#"{ "session.trustAllWorktrees": true, "terminal.explorerKind": "external", "terminal.external.osxExec": "iTerm.app" }"#,
        ),
    );
    let calls = install_shell(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-a.rs", cx);
    click("context-menu-item-openInTerminal", cx);
    let expected = Call::Terminal(root_of(&wb, cx), "iTerm.app".into());
    assert_eq!(*calls.lock().unwrap(), vec![expected]);
}

/// Records the folder each terminal would start in; the start itself
/// fails, which still adds a tab.
fn recording_spawner(
    wb: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> Rc<RefCell<Vec<PathBuf>>> {
    let cwds = Rc::new(RefCell::new(Vec::new()));
    let spawner: TerminalSpawner = {
        let cwds = cwds.clone();
        Rc::new(move |opts: TerminalOptions| {
            cwds.borrow_mut().push(opts.cwd.clone());
            Err(io::Error::other("test"))
        })
    };
    wb.update(cx, |wb, _| wb.set_terminal_spawner(spawner));
    cwds
}

fn menu_has(command: &str, cx: &mut VisualTestContext) -> bool {
    cx.debug_bounds(Box::leak(
        format!("context-menu-item-{command}").into_boxed_str(),
    ))
    .is_some()
}

/// VS Code's default (`terminal.explorerKind: "integrated"`): the menu
/// opens the integrated terminal in the folder, or a file's folder.
#[gpui::test]
fn integrated_terminal_opens_in_the_folder(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let calls = install_shell(false, cx);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let cwds = recording_spawner(&wb, cx);
    right_click("explorer-row-a.rs", cx);
    assert!(
        !menu_has("openInTerminal", cx),
        "no external entry by default"
    );
    click("context-menu-item-openInIntegratedTerminal", cx);
    assert_eq!(*cwds.borrow(), vec![root_of(&wb, cx)], "a file's folder");
    assert!(calls.lock().unwrap().is_empty(), "no external terminal");
    let shown = cx.update(|_, cx| {
        let wb = wb.read(cx);
        (
            wb.terminal_panel().read(cx).tab_count(),
            wb.panel_state().visible,
        )
    });
    assert_eq!(shown, (1, true), "a new tab, panel shown");
}

#[gpui::test]
fn explorer_kind_picks_the_menu_entries(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "session.trustAllWorktrees": true, "terminal.explorerKind": "external" }"#),
    );
    install_shell(false, cx);
    let dir = project();
    let (_wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-a.rs", cx);
    assert!(menu_has("openInTerminal", cx));
    assert!(!menu_has("openInIntegratedTerminal", cx));
}

#[gpui::test]
fn explorer_kind_both_shows_both(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "session.trustAllWorktrees": true, "terminal.explorerKind": "both" }"#),
    );
    install_shell(false, cx);
    let dir = project();
    let (_wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-a.rs", cx);
    assert!(menu_has("openInTerminal", cx));
    assert!(menu_has("openInIntegratedTerminal", cx));
}

#[gpui::test]
fn terminal_error_shows_status(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "session.trustAllWorktrees": true, "terminal.explorerKind": "external" }"#),
    );
    install_shell(true, cx);
    let dir = project();
    let (_wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-a.rs", cx);
    click("context-menu-item-openInTerminal", cx);
    let message = cx.update(|window, cx| {
        lucerna_workbench::status_message::current(window.window_handle().window_id(), cx)
    });
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

fn include_text(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        let search = wb.read(cx).search_view().read(cx);
        search.include_input().read(cx).text().to_string()
    })
}

#[gpui::test]
fn find_in_folder_fills_include_and_focuses_query(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-src", cx);
    click("context-menu-item-filesExplorer.findInFolder", cx);
    let search_active = cx.update(|_, cx| {
        let model = wb.read(cx).editor_groups().read(cx).model();
        model
            .active_tab()
            .is_some_and(|t| t.item == TabItem::Search)
    });
    assert!(search_active);
    assert_eq!(include_text(&wb, cx), "src");
    let focused = cx.update(|window, cx| {
        let search = wb.read(cx).search_view().read(cx);
        search
            .query_input()
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(focused);
}

#[gpui::test]
fn find_in_folder_on_root_clears_include(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    cx.update(|window, cx| {
        let search = wb.read(cx).search_view().clone();
        search.update(cx, |s, cx| s.set_include("x", window, cx));
    });
    assert_eq!(include_text(&wb, cx), "x");
    right_click("explorer-root", cx);
    click("context-menu-item-filesExplorer.findInFolder", cx);
    assert_eq!(include_text(&wb, cx), "");
}

/// The include `src` must search the files under `src`, not just match the
/// directory entry.
#[gpui::test]
fn find_in_folder_searches_only_that_folder(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-src", cx);
    click("context-menu-item-filesExplorer.findInFolder", cx);
    cx.simulate_input("main");
    cx.executor().advance_clock(Duration::from_millis(150));
    draw(cx);
    let files = cx.update(|_, cx| {
        let search = wb.read(cx).search_view().read(cx);
        search
            .results(cx)
            .files()
            .iter()
            .map(|f| f.path.to_path_buf())
            .collect::<Vec<_>>()
    });
    assert_eq!(files, vec![PathBuf::from("src/main.rs")]);
}

/// Runs Find in Folder on `folder` (root-relative), searches `main` and
/// returns the files found.
fn find_in(wb: &Entity<Workbench>, folder: &str, cx: &mut VisualTestContext) -> Vec<PathBuf> {
    let args = json!({ "path": folder });
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("filesExplorer.findInFolder", Some(&args), window, cx)
        })
    });
    draw(cx);
    cx.simulate_input("main");
    cx.executor().advance_clock(Duration::from_millis(150));
    draw(cx);
    cx.update(|_, cx| {
        let search = wb.read(cx).search_view().read(cx);
        let mut files: Vec<PathBuf> = search
            .results(cx)
            .files()
            .iter()
            .map(|f| f.path.to_path_buf())
            .collect();
        files.sort();
        files
    })
}

/// Glob characters and commas in a folder name are escaped: `[slug]`
/// is that folder, not a class matching `s`, `l`, `u` or `g`.
#[gpui::test]
fn find_in_folder_escapes_glob_characters(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = tempfile::tempdir().unwrap();
    for file in [
        "app/[slug]/page.rs",
        "app/s/other.rs",
        "a,b/x.rs",
        "a/y.rs",
        "b/z.rs",
        "{x}/w.rs",
    ] {
        let path = dir.path().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "fn main() {}\n").unwrap();
    }
    let (wb, cx) = open(dir.path(), cx);
    assert_eq!(
        find_in(&wb, "app/[slug]", cx),
        [PathBuf::from("app/[slug]/page.rs")]
    );
    assert_eq!(find_in(&wb, "a,b", cx), [PathBuf::from("a,b/x.rs")]);
    assert_eq!(find_in(&wb, "{x}", cx), [PathBuf::from("{x}/w.rs")]);
}

/// The root's menu fixes its target too: every entry carries
/// `{"path": ""}`, which names the root itself.
#[gpui::test]
fn root_menu_entries_carry_root_path(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-root", cx);
    let args: Vec<Option<serde_json::Value>> = cx.update(|_, cx| {
        let explorer = wb.read(cx).explorer().read(cx);
        explorer
            .context_menu()
            .unwrap()
            .read(cx)
            .items()
            .iter()
            .filter_map(|item| match item {
                lucerna_workbench::ui::MenuItem::Entry { args, .. } => Some(args.clone()),
                lucerna_workbench::ui::MenuItem::Separator => None,
            })
            .collect()
    });
    assert!(!args.is_empty());
    assert!(
        args.iter().all(|a| a == &Some(json!({ "path": "" }))),
        "{args:?}"
    );
    click("context-menu-item-copyFilePath", cx);
    assert_eq!(
        clipboard(cx),
        Some(root_of(&wb, cx).to_string_lossy().into_owned()),
        "the root, without a trailing separator"
    );
}

/// Palette titles say what the commands act on: the Explorer's selection,
/// and the platform's file manager.
#[gpui::test]
fn palette_titles_name_the_explorer_and_platform(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let title = |id: &str, cx: &mut VisualTestContext| {
        wb.read_with(cx, |wb, _| {
            wb.commands()
                .all()
                .find(|(i, _)| *i == id)
                .map(|(_, t)| t.to_string())
                .unwrap()
        })
    };
    assert_eq!(title("copyFilePath", cx), "Explorer: Copy Path");
    assert_eq!(
        title("copyRelativeFilePath", cx),
        "Explorer: Copy Relative Path"
    );
    let reveal = if cfg!(target_os = "macos") {
        "File: Reveal in Finder"
    } else if cfg!(target_os = "windows") {
        "File: Reveal in File Explorer"
    } else {
        "File: Open Containing Folder"
    };
    assert_eq!(title("revealFileInOS", cx), reveal);
}

/// The active tab's item, if any.
fn active_item(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<TabItem> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .active_tab()
            .map(|t| t.item.clone())
    })
}

#[gpui::test]
fn open_preview_from_menu_opens_markdown_preview(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = project();
    std::fs::write(dir.path().join("README.md"), "# Title\n").unwrap();
    let (wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-README.md", cx);
    assert!(menu_has("markdown.showPreview", cx));
    click("context-menu-item-markdown.showPreview", cx);
    let readme: Arc<Path> = Arc::from(root_of(&wb, cx).join("README.md"));
    assert_eq!(active_item(&wb, cx), Some(TabItem::MarkdownPreview(readme)));
    let pinned = cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .active_tab()
            .is_some_and(|t| t.pinned)
    });
    assert!(pinned);
}

#[gpui::test]
fn no_open_preview_for_rust_file(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    let dir = project();
    let (_wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-a.rs", cx);
    assert!(!menu_has("markdown.showPreview", cx));
}

#[gpui::test]
fn open_preview_follows_files_associations(cx: &mut TestAppContext) {
    init(cx, Some(TRUSTED));
    // What the extension service does with `files.associations`.
    let map = std::collections::BTreeMap::from([("*.notes".to_string(), "markdown".to_string())]);
    cx.update(|cx| {
        let state = cx.global_mut::<AppState>();
        state.languages = Arc::new(
            state
                .languages
                .with_associations(lucerna_syntax::Associations::new(&map)),
        );
    });
    let dir = project();
    std::fs::write(dir.path().join("todo.notes"), "- a\n").unwrap();
    let (_wb, cx) = open(dir.path(), cx);
    right_click("explorer-row-todo.notes", cx);
    assert!(menu_has("markdown.showPreview", cx));
}
