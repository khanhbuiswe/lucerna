//! Settings, keybindings and theme files: hot reload, error reporting in
//! the status bar, the open-file commands and the theme picker. Every test
//! points the user directory at a temp dir; none touches
//! `~/.config/lucerna`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, ConfigPaths, ConfigState, ConfigWatcher, OpenMode, ThemePicker, Workbench,
    reload_config, watch_config,
};
use tempfile::TempDir;

/// A project with `a.txt` and `debug.log`, and an empty user directory.
struct Dirs {
    root: TempDir,
    user: TempDir,
}

impl Dirs {
    fn new() -> Dirs {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(root.path().join("debug.log"), "log\n").unwrap();
        Dirs {
            root,
            user: tempfile::tempdir().unwrap(),
        }
    }

    fn paths(&self) -> ConfigPaths {
        ConfigPaths::with_user_dir(self.user.path(), self.root.path())
    }

    fn settings(&self) -> PathBuf {
        self.user.path().join("settings.json")
    }

    fn keybindings(&self) -> PathBuf {
        self.user.path().join("keybindings.json")
    }
}

fn init(dirs: &Dirs, cx: &mut TestAppContext) {
    init_with(dirs.paths(), cx);
}

fn init_with(paths: ConfigPaths, cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
    });
    cx.update(|cx| watch_config(paths, cx));
}

/// Opens the workbench once its scan finished.
fn open<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let root = root.to_path_buf();
    let (wb, cx) = cx.add_window_view(move |window, cx| Workbench::new(root, window, cx));
    wait_for_scan(&wb, cx);
    (wb, cx)
}

fn wait_for_scan(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
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

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    draw(cx);
}

/// Writes `text` to `path` and delivers the change as the watcher would.
fn write(path: &Path, text: &str, cx: &mut VisualTestContext) {
    std::fs::write(path, text).unwrap();
    let changed = path.to_path_buf();
    cx.update(|_, cx| reload_config(&[changed], cx));
    draw(cx);
}

fn font_size(cx: &mut VisualTestContext) -> Option<f32> {
    cx.update(|_, cx| {
        cx.global::<AppState>()
            .settings
            .get("editor.fontSize", None)
    })
}

fn theme_name(cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| cx.global::<AppState>().theme.name.clone())
}

fn error_texts(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        cx.global::<ConfigState>()
            .errors()
            .map(|e| e.status_text())
            .collect()
    })
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

fn active_tab_path(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<PathBuf> {
    cx.update(|_, cx| {
        let model = wb.read(cx).editor_groups().read(cx).model();
        model.active_tab().map(|t| t.path.to_path_buf())
    })
}

fn theme_picker(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<Entity<ThemePicker>> {
    cx.update(|_, cx| wb.read(cx).theme_picker().cloned())
}

#[gpui::test]
fn user_settings_change_applies(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("a.txt")), OpenMode::Pinned, cx)
    });
    draw(cx);
    let view = active_view(&wb, cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).font_size()), 13.);
    let generation = cx.update(|_, cx| cx.global::<AppState>().theme_generation);

    write(
        &dirs.settings(),
        "// mine\n{ \"editor.fontSize\": 20, \"workbench.colorTheme\": \"One Light\" }\n",
        cx,
    );

    assert_eq!(font_size(cx), Some(20.));
    assert_eq!(
        cx.update(|_, cx| view.read(cx).font_size()),
        20.,
        "the open view takes the new font size"
    );
    assert_eq!(theme_name(cx), "One Light");
    assert!(
        cx.update(|_, cx| cx.global::<AppState>().theme_generation) > generation,
        "a theme switch bumps the generation the shape caches key on"
    );

    // Workspace settings override user settings.
    let workspace = dirs.root.path().join(".lucerna");
    std::fs::create_dir(&workspace).unwrap();
    write(
        &workspace.join("settings.json"),
        "{ \"editor.fontSize\": 16 }",
        cx,
    );
    assert_eq!(cx.update(|_, cx| view.read(cx).font_size()), 16.);
}

#[gpui::test]
fn user_theme_file_applies(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    std::fs::write(dirs.settings(), "{ \"workbench.colorTheme\": \"Custom\" }").unwrap();
    init(&dirs, cx);
    let (_wb, cx) = open(dirs.root.path(), cx);
    assert_eq!(theme_name(cx), "One Dark", "no such theme yet");

    let themes = dirs.user.path().join("themes");
    std::fs::create_dir(&themes).unwrap();
    write(
        &themes.join("custom.json"),
        r#"{"name":"C","themes":[{"name":"Custom","appearance":"light","style":{}}]}"#,
        cx,
    );

    assert_eq!(theme_name(cx), "Custom");
    assert!(
        cx.update(|_, cx| cx.global::<AppState>().themes.names())
            .contains(&"One Light".to_string()),
        "the bundled themes stay"
    );
}

#[gpui::test]
fn invalid_settings_shows_error_keeps_previous(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    write(&dirs.settings(), "{\n  \"editor.fontSize\": 20\n}\n", cx);
    assert_eq!(font_size(cx), Some(20.));
    assert!(error_texts(cx).is_empty());
    assert!(cx.debug_bounds("settings-error").is_none());

    write(
        &dirs.settings(),
        "{\n  \"editor.fontSize\": 22,\n  \"editor.tabSize\": \n}\n",
        cx,
    );

    assert_eq!(font_size(cx), Some(20.), "the previous user layer is kept");
    let errors = error_texts(cx);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].starts_with("settings.json: line 4: "),
        "{errors:?}"
    );
    let item = cx
        .debug_bounds("settings-error")
        .expect("the status bar shows the error");

    // Clicking the error opens the file at the reported line.
    cx.simulate_click(item.center(), Modifiers::none());
    draw(cx);
    assert_eq!(active_tab_path(&wb, cx), Some(dirs.settings()));
    let view = active_view(&wb, cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).cursor()).row, 3);

    // Broken keybindings name their own file.
    write(&dirs.keybindings(), "[\n  { \"key\": 1 }\n]\n", cx);
    let errors = error_texts(cx);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(
        errors.iter().any(|e| e.starts_with("keybindings.json: ")),
        "{errors:?}"
    );
}

#[gpui::test]
fn fixing_settings_clears_error(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    init(&dirs, cx);
    let (_wb, cx) = open(dirs.root.path(), cx);
    write(&dirs.settings(), "{ \"editor.fontSize\": ", cx);
    assert_eq!(error_texts(cx).len(), 1);
    assert!(cx.debug_bounds("settings-error").is_some());

    write(&dirs.settings(), "{ \"editor.fontSize\": 18 }", cx);

    assert!(error_texts(cx).is_empty());
    assert!(cx.debug_bounds("settings-error").is_none());
    assert_eq!(font_size(cx), Some(18.));
}

#[gpui::test]
fn keybindings_change_applies(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    assert!(wb.read_with(cx, |wb, _| wb.sidebar_visible()));

    write(
        &dirs.keybindings(),
        "// mine\n[\n  { \"key\": \"cmd+shift+y\", \"command\": \"workbench.action.toggleSidebarVisibility\" }\n]\n",
        cx,
    );
    keys("cmd-shift-y", cx);
    assert!(!wb.read_with(cx, |wb, _| wb.sidebar_visible()));

    // A broken file keeps the bindings in use.
    write(&dirs.keybindings(), "[ { \"key\": ", cx);
    keys("cmd-shift-y", cx);
    assert!(wb.read_with(cx, |wb, _| wb.sidebar_visible()));
}

#[gpui::test]
fn files_exclude_change_rescans(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    let has_log = |cx: &mut VisualTestContext| {
        wb.read_with(cx, |wb, _| {
            wb.worktree().snapshot().contains(Path::new("debug.log"))
        })
    };
    assert!(has_log(cx));

    write(
        &dirs.settings(),
        "{ \"files.exclude\": { \"**/*.log\": true } }",
        cx,
    );

    // R26: the user's globs merge with the defaults.
    assert_eq!(
        wb.read_with(cx, |wb, _| wb.scan_options().exclude.clone()),
        ["**/.git", "**/.DS_Store", "**/*.log"]
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if !has_log(cx) && wb.read_with(cx, |wb, _| wb.worktree().snapshot().scan_complete()) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the rescan did not drop debug.log"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[gpui::test]
fn open_settings_creates_missing_file(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    // The user directory does not exist yet: it is made on first open.
    let user_dir = dirs.user.path().join("lucerna");
    let paths = ConfigPaths::with_user_dir(&user_dir, dirs.root.path());
    init_with(paths, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    assert!(!dirs.root.path().join(".lucerna").exists());

    keys("cmd-,", cx);
    let settings = user_dir.join("settings.json");
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        "// Lucerna user settings\n{\n}\n"
    );
    assert_eq!(active_tab_path(&wb, cx), Some(settings.clone()));

    wb.update_in(cx, |wb, window, cx| {
        wb.run_command(
            "workbench.action.openGlobalKeybindingsFile",
            None,
            window,
            cx,
        )
    });
    draw(cx);
    let keybindings = user_dir.join("keybindings.json");
    assert_eq!(
        std::fs::read_to_string(&keybindings).unwrap(),
        "// Lucerna user keybindings\n[\n]\n"
    );
    assert_eq!(active_tab_path(&wb, cx), Some(keybindings));

    wb.update_in(cx, |wb, window, cx| {
        wb.run_command(
            "workbench.action.openWorkspaceSettingsFile",
            None,
            window,
            cx,
        )
    });
    draw(cx);
    let workspace = dirs.root.path().join(".lucerna/settings.json");
    assert_eq!(
        std::fs::read_to_string(&workspace).unwrap(),
        "// Lucerna workspace settings\n{\n}\n"
    );
    assert_eq!(active_tab_path(&wb, cx), Some(workspace));

    // An existing file is opened as it is.
    std::fs::write(&settings, "{ \"editor.tabSize\": 2 }").unwrap();
    keys("cmd-,", cx);
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        "{ \"editor.tabSize\": 2 }"
    );
    assert!(
        wb.read_with(cx, |wb, _| wb
            .commands()
            .all()
            .any(|(id, title)| id == "lucerna.openLogsFolder"
                && title == "Lucerna: Open Logs Folder"))
    );
}

#[gpui::test]
fn theme_picker_writes_setting_preserving_comments(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    let original = "// my settings\n{\n  // bigger\n  \"editor.fontSize\": 14 // px\n}\n";
    std::fs::write(dirs.settings(), original).unwrap();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);

    keys("cmd-k cmd-t", cx);
    let picker = theme_picker(&wb, cx).expect("cmd-k cmd-t shows the theme picker");
    assert_eq!(
        cx.update(|_, cx| picker.read(cx).row_texts()),
        vec!["One Dark".to_string(), "One Light".to_string()]
    );
    assert_eq!(cx.update(|_, cx| picker.read(cx).selected_index()), 0);
    cx.simulate_input("light");
    draw(cx);
    keys("enter", cx);

    assert!(theme_picker(&wb, cx).is_none(), "Enter closes the picker");
    assert_eq!(theme_name(cx), "One Light");
    assert_eq!(
        std::fs::read_to_string(dirs.settings()).unwrap(),
        "// my settings\n{\n  // bigger\n  \"editor.fontSize\": 14, // px\n  \"workbench.colorTheme\": \"One Light\"\n}\n"
    );

    // Picking again replaces the value in place.
    keys("cmd-k cmd-t", cx);
    let picker = theme_picker(&wb, cx).unwrap();
    assert_eq!(
        cx.update(|_, cx| picker.read(cx).selected_index()),
        1,
        "the active theme is selected"
    );
    keys("up", cx);
    keys("enter", cx);
    assert_eq!(theme_name(cx), "One Dark");
    assert_eq!(
        std::fs::read_to_string(dirs.settings()).unwrap(),
        "// my settings\n{\n  // bigger\n  \"editor.fontSize\": 14, // px\n  \"workbench.colorTheme\": \"One Dark\"\n}\n"
    );
    let leftovers: Vec<_> = std::fs::read_dir(dirs.user.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, vec![std::ffi::OsString::from("settings.json")]);
}

#[gpui::test]
fn theme_picker_escape_reverts(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    let generation = cx.update(|_, cx| cx.global::<AppState>().theme_generation);

    keys("cmd-k cmd-t", cx);
    keys("down", cx);
    assert_eq!(theme_name(cx), "One Light", "moving the selection previews");
    keys("escape", cx);

    assert!(theme_picker(&wb, cx).is_none());
    assert_eq!(theme_name(cx), "One Dark");
    assert!(cx.update(|_, cx| cx.global::<AppState>().theme_generation) > generation);
    assert!(!dirs.settings().exists(), "Escape writes nothing");
}

/// The real watcher: a write to `settings.json` arrives as one debounced
/// batch naming it.
#[test]
fn watcher_reports_settings_write() {
    let dirs = Dirs::new();
    let (tx, rx) = crossbeam_channel::unbounded();
    let _watcher = ConfigWatcher::start(&dirs.paths(), move |batch| {
        let _ = tx.send(batch);
    })
    .unwrap();
    // FSEvents may report events from before the watch started; let them pass.
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}

    std::fs::write(dirs.settings(), "{}").unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let batch: Vec<PathBuf> = rx.recv_timeout(left).expect("a batch arrives");
        if batch.iter().any(|p| p.ends_with("settings.json")) {
            break;
        }
    }
}

#[test]
fn config_paths_layout() {
    let paths = ConfigPaths::with_user_dir("/u", Path::new("/r"));
    assert_eq!(paths.user_dir, PathBuf::from("/u"));
    assert_eq!(
        paths.workspace_settings,
        PathBuf::from("/r/.lucerna/settings.json")
    );
    assert_eq!(paths.user_settings(), PathBuf::from("/u/settings.json"));
    assert_eq!(paths.keybindings(), PathBuf::from("/u/keybindings.json"));
    assert_eq!(paths.themes_dir(), PathBuf::from("/u/themes"));
    let home = ConfigPaths::for_root(Path::new("/r"));
    assert!(
        home.user_dir.ends_with(".config/lucerna"),
        "{:?}",
        home.user_dir
    );
}

#[gpui::test]
fn reload_mid_preview_keeps_preview(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);

    keys("cmd-k cmd-t", cx);
    keys("down", cx);
    assert_eq!(theme_name(cx), "One Light");
    // Unrelated reloads arrive while the preview is shown.
    write(&dirs.settings(), "{ \"editor.fontSize\": 15 }", cx);
    write(&dirs.keybindings(), "[]", cx);
    assert_eq!(theme_name(cx), "One Light", "the preview survives reloads");
    assert!(theme_picker(&wb, cx).is_some());

    // Escape goes back to the configured theme, as last loaded.
    write(
        &dirs.settings(),
        "{ \"workbench.colorTheme\": \"One Light\" }",
        cx,
    );
    keys("up", cx);
    assert_eq!(theme_name(cx), "One Dark");
    keys("escape", cx);
    assert_eq!(theme_name(cx), "One Light");
    // Closed: reloads apply the configured theme again.
    write(
        &dirs.settings(),
        "{ \"workbench.colorTheme\": \"One Dark\" }",
        cx,
    );
    assert_eq!(theme_name(cx), "One Dark");
}

#[gpui::test]
fn failed_theme_save_keeps_picker_open(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    let broken = "{ \"editor.fontSize\": ";
    std::fs::write(dirs.settings(), broken).unwrap();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    keys("cmd-k cmd-t", cx);
    keys("down", cx);
    assert_eq!(theme_name(cx), "One Light");
    keys("enter", cx);

    assert!(theme_picker(&wb, cx).is_some(), "the picker stays open");
    assert_eq!(theme_name(cx), "One Dark", "the preview is reverted");
    assert_eq!(std::fs::read_to_string(dirs.settings()).unwrap(), broken);
    let errors = error_texts(cx);
    assert!(
        errors.iter().any(|e| e.starts_with("settings.json: ")),
        "{errors:?}"
    );
    assert!(cx.debug_bounds("settings-error").is_some());
}

#[gpui::test]
fn keymap_reload_drops_pending_chord(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    keys("cmd-k", cx);
    assert!(wb.read_with(cx, |wb, _| wb.pending_chord_label()).is_some());

    write(&dirs.keybindings(), "[]", cx);

    assert_eq!(wb.read_with(cx, |wb, _| wb.pending_chord_label()), None);
}

#[gpui::test]
fn settings_change_keeps_scroll_position(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    let long: String = (1..=300).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dirs.root.path().join("long.txt"), long).unwrap();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("long.txt")), OpenMode::Pinned, cx)
    });
    draw(cx);
    let view = active_view(&wb, cx);
    view.update(cx, |v, cx| v.set_scroll_top_row(120, cx));
    draw(cx);

    write(&dirs.settings(), "{ \"editor.fontSize\": 18 }", cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).font_size()), 18.);
    assert_eq!(cx.update(|_, cx| view.read(cx).scroll_top_row()), 120);

    write(
        &dirs.settings(),
        "{ \"editor.fontSize\": 18, \"editor.minimap.enabled\": false }",
        cx,
    );
    assert_eq!(cx.update(|_, cx| view.read(cx).scroll_top_row()), 120);
}
