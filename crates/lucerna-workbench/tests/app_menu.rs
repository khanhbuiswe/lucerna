//! The menu bar's actions, driven through GPUI's test platform.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{Entity, Focusable, OwnedMenu, OwnedMenuItem, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::app_menu::{self, DisabledCommand, RunCommand};
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::recent::{self, RecentEntry, RecentKind, RecentStore};
use lucerna_workbench::{AppState, OpenMode, WelcomeView, Workbench};
use tempfile::TempDir;

const TOGGLE_SIDEBAR: &str = "workbench.action.toggleSidebarVisibility";

fn init(cx: &mut TestAppContext) {
    app_state(cx);
    cx.update(app_menu::install);
}

fn app_state(cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
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

fn open(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut VisualTestContext) {
    cx.add_window_view(|window, cx| Workbench::new(PathBuf::from("/tmp/project"), window, cx))
}

fn sidebar_visible(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    wb.read_with(cx, |wb, _| wb.sidebar_visible())
}

#[gpui::test]
fn run_command_reaches_the_workbench(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    assert!(sidebar_visible(&wb, cx));
    cx.dispatch_action(RunCommand {
        id: TOGGLE_SIDEBAR.into(),
    });
    cx.run_until_parked();
    assert!(!sidebar_visible(&wb, cx));
}

#[gpui::test]
fn menu_binding_does_not_double_run(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    // The menu shows ⌘B through a GPUI binding of its own.
    let bound = cx.update(|_, cx| {
        let action = RunCommand {
            id: TOGGLE_SIDEBAR.into(),
        };
        cx.key_bindings()
            .borrow()
            .bindings_for_action(&action)
            .count()
    });
    assert_eq!(bound, 1);
    cx.simulate_keystrokes("cmd-b");
    cx.run_until_parked();
    assert!(
        !sidebar_visible(&wb, cx),
        "cmd-b ran an even number of times"
    );
}

/// AppKit enables a menu item when GPUI says its action is available, so
/// what a Welcome window cannot run uses an action nothing handles.
#[gpui::test]
fn welcome_window_cannot_run_workbench_items(cx: &mut TestAppContext) {
    init(cx);
    cx.set_global(RecentStore { data_dir: None });
    let (_, cx) = cx.add_window_view(WelcomeView::new);
    cx.run_until_parked();
    let (disabled, open_recent) = cx.update(|_, cx| {
        let save = DisabledCommand {
            id: "workbench.action.files.save".into(),
        };
        let open_recent = RunCommand {
            id: "workbench.action.openRecent".into(),
        };
        (
            cx.is_action_available(&save),
            cx.is_action_available(&open_recent),
        )
    });
    assert!(!disabled);
    assert!(open_recent);
}

fn settle(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// A workbench on a temporary folder with `file.txt` open and focused,
/// after typing `typed` into it.
fn editor<'a>(
    typed: &str,
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    Entity<Workbench>,
    Entity<CodeView>,
    &'a mut VisualTestContext,
) {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("file.txt"), "").unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("file.txt")), OpenMode::Pinned, cx)
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
    cx.simulate_input(typed);
    settle(cx);
    (dir, wb, view, cx)
}

fn text_of(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, _| {
        let s = v.snapshot();
        s.text_for_range(0..s.len_bytes())
    })
}

fn menu_command(id: &str, cx: &mut VisualTestContext) {
    cx.dispatch_action(RunCommand { id: id.into() });
    settle(cx);
}

/// A menu item runs its command only where the command's key would: not
/// on the document while the find box has focus.
#[gpui::test]
fn menu_command_respects_when_in_text_input(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = editor("abc", cx);
    cx.simulate_keystrokes("cmd-f");
    settle(cx);
    menu_command("undo", cx);
    assert_eq!(text_of(&view, cx), "abc", "undo ran in the find box");
    menu_command("editor.action.clipboardCutAction", cx);
    assert_eq!(text_of(&view, cx), "abc", "cut ran in the find box");

    // Back in the editor, the menu works.
    cx.simulate_keystrokes("escape");
    settle(cx);
    menu_command("undo", cx);
    assert_eq!(text_of(&view, cx), "");
}

/// A command with a binding that has no `when` (or one with no binding
/// at all) runs from the menu wherever focus is.
#[gpui::test]
fn menu_command_without_when_still_runs(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, wb, _view, cx) = editor("abc", cx);
    cx.simulate_keystrokes("cmd-f");
    settle(cx);
    assert!(sidebar_visible(&wb, cx));
    menu_command(TOGGLE_SIDEBAR, cx);
    assert!(!sidebar_visible(&wb, cx));
}

fn open_recent_titles(menus: &[OwnedMenu]) -> Vec<String> {
    let file = menus
        .iter()
        .find(|m| m.name.as_ref() == "File")
        .expect("a File menu");
    let open_recent = file
        .items
        .iter()
        .find_map(|item| match item {
            OwnedMenuItem::Submenu(m) if m.name.as_ref() == "Open Recent" => Some(m),
            _ => None,
        })
        .expect("an Open Recent submenu");
    open_recent
        .items
        .iter()
        .filter_map(|item| match item {
            OwnedMenuItem::Action { name, .. } => Some(name.to_string()),
            _ => None,
        })
        .collect()
}

/// File ▸ Open Recent leaves out the window's own folder.
#[gpui::test]
fn open_recent_menu_hides_the_window_folder(cx: &mut TestAppContext) {
    app_state(cx);
    let tmp = TempDir::new().unwrap();
    let (this, other) = (tmp.path().join("this"), tmp.path().join("other"));
    fs::create_dir(&this).unwrap();
    fs::create_dir(&other).unwrap();
    let data = tmp.path().join("data");
    for path in [&other, &this] {
        let entry = RecentEntry {
            path: path.clone(),
            kind: RecentKind::Folder,
        };
        recent::record(&data, entry).unwrap();
    }
    cx.set_global(RecentStore {
        data_dir: Some(data),
    });
    let root = this.clone();
    let (_wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    cx.update(|_, cx| app_menu::install(cx));
    settle(cx);
    let menus = cx.update(|_, cx| cx.get_menus()).expect("menus are set");
    let titles = open_recent_titles(&menus);
    let shown = |path: &Path| titles.iter().any(|t| t.ends_with(&*path.to_string_lossy()));
    assert!(shown(&other), "{titles:?}");
    assert!(!shown(&this), "{titles:?}");
}

/// A single-file window leaves out its file.
#[gpui::test]
fn open_recent_menu_hides_the_single_file(cx: &mut TestAppContext) {
    app_state(cx);
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("notes.md");
    let other = tmp.path().join("other.md");
    fs::write(&file, "").unwrap();
    fs::write(&other, "").unwrap();
    let data = tmp.path().join("data");
    for path in [&other, &file] {
        let entry = RecentEntry {
            path: path.clone(),
            kind: RecentKind::File,
        };
        recent::record(&data, entry).unwrap();
    }
    cx.set_global(RecentStore {
        data_dir: Some(data),
    });
    let (root, single) = (tmp.path().to_path_buf(), file.clone());
    let (_wb, cx) =
        cx.add_window_view(|window, cx| Workbench::single_file(root, &single, None, window, cx));
    cx.update(|_, cx| app_menu::install(cx));
    settle(cx);
    let menus = cx.update(|_, cx| cx.get_menus()).expect("menus are set");
    let titles = open_recent_titles(&menus);
    let shown = |path: &Path| titles.iter().any(|t| t.ends_with(&*path.to_string_lossy()));
    assert!(shown(&other), "{titles:?}");
    assert!(!shown(&file), "{titles:?}");
}

/// T3-2: in single-file mode the file's parent folder is not open, so
/// Open Recent still lists it.
#[gpui::test]
fn open_recent_menu_keeps_the_single_files_parent_folder(cx: &mut TestAppContext) {
    app_state(cx);
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("README.md");
    fs::write(&file, "").unwrap();
    let data = tmp.path().join("data");
    let entry = RecentEntry {
        path: tmp.path().to_path_buf(),
        kind: RecentKind::Folder,
    };
    recent::record(&data, entry).unwrap();
    cx.set_global(RecentStore {
        data_dir: Some(data),
    });
    let (root, single) = (tmp.path().to_path_buf(), file.clone());
    let (_wb, cx) =
        cx.add_window_view(|window, cx| Workbench::single_file(root, &single, None, window, cx));
    cx.update(|_, cx| app_menu::install(cx));
    settle(cx);
    let menus = cx.update(|_, cx| cx.get_menus()).expect("menus are set");
    let titles = open_recent_titles(&menus);
    let dir = tmp.path().to_string_lossy().into_owned();
    assert!(titles.iter().any(|t| t.ends_with(&dir)), "{titles:?}");
}
