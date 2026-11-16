//! The icon theme picker (`workbench.action.selectIconTheme`) and drawing
//! icons from files. Every test points the user directory at a temp dir.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Entity, ImgResourceLoader, Resource, TestAppContext, VisualTestContext, div, img, prelude::*,
    px,
};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::{DEFAULT_ICON_THEME, IconSource, IconThemeRegistry, ThemeRegistry};
use lucerna_workbench::{
    AppState, ConfigPaths, ConfigState, ThemePicker, Workbench, reload_config, watch_config,
};
use tempfile::TempDir;

struct Dirs {
    root: TempDir,
    user: TempDir,
    icons: TempDir,
}

impl Dirs {
    fn new() -> Dirs {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.json"), "{}\n").unwrap();
        Dirs {
            root,
            user: tempfile::tempdir().unwrap(),
            icons: tempfile::tempdir().unwrap(),
        }
    }

    fn settings(&self) -> PathBuf {
        self.user.path().join("settings.json")
    }

    /// Seti, Lucide plus "Ext Icons", whose `json` files are drawn from a file.
    fn registry(&self) -> IconThemeRegistry {
        let root = self.icons.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("icons")).unwrap();
        std::fs::write(root.join("icons/json.svg"), "<svg/>").unwrap();
        std::fs::write(
            root.join("icons.json"),
            r#"{"name":"Ext","author":"t","themes":[{"name":"Ext Icons","appearance":"dark",
            "file_suffixes":{"json":"json"},"file_icons":{"json":{"path":"./icons/json.svg"}}}]}"#,
        )
        .unwrap();
        let (registry, warnings) =
            IconThemeRegistry::build(&[("ext".to_string(), root.join("icons.json"), root)]);
        assert!(warnings.is_empty(), "{warnings:?}");
        registry
    }
}

fn init(dirs: &Dirs, settings: Option<&str>, cx: &mut TestAppContext) {
    if let Some(text) = settings {
        std::fs::write(dirs.settings(), text).unwrap();
    }
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
    let icon_themes = dirs.registry();
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
        languages_generation: 0,
        icons: icon_themes.resolve(DEFAULT_ICON_THEME),
        icon_themes,
    });
    let paths = ConfigPaths::user_only_with_user_dir(dirs.user.path());
    cx.update(|cx| watch_config(paths, cx));
}

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
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    draw(cx);
}

fn icon_name(cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| cx.global::<AppState>().icons.name().to_string())
}

fn picker(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<Entity<ThemePicker>> {
    cx.update(|_, cx| wb.read(cx).theme_picker().cloned())
}

fn show_picker(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<ThemePicker> {
    let ran = cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("workbench.action.selectIconTheme", None, window, cx)
        })
    });
    assert!(ran, "the command is registered");
    draw(cx);
    picker(wb, cx).expect("the picker is shown")
}

#[gpui::test]
fn img_loads_svg_from_a_path(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.svg");
    std::fs::write(
        &path,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" fill="red"/></svg>"#,
    )
    .unwrap();
    let shown = path.clone();
    let (_, cx) = cx.add_window_view(move |_, _| Plain(shown));
    draw(cx);
    let source = Resource::Path(Arc::from(path.as_path()));
    let loaded = cx.update(|_, cx| cx.fetch_asset::<ImgResourceLoader>(&source));
    assert!(
        matches!(loaded, Some(Ok(_))),
        "{:?}",
        loaded.map(|r| r.err())
    );
}

struct Plain(PathBuf);

impl gpui::Render for Plain {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().child(img(self.0.clone()).size(px(14.)))
    }
}

#[gpui::test]
fn icon_theme_picker_previews_and_persists(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    let original = "// mine\n{\n  \"editor.fontSize\": 14 // px\n}\n";
    init(&dirs, Some(original), cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    assert_eq!(icon_name(cx), "Seti");

    let p = show_picker(&wb, cx);
    assert_eq!(
        cx.update(|_, cx| p.read(cx).row_texts()),
        vec![
            "Seti".to_string(),
            "Lucide".to_string(),
            "Ext Icons".to_string()
        ]
    );
    keys("down down", cx);
    assert_eq!(icon_name(cx), "Ext Icons", "moving the selection previews");

    // A reload while previewing leaves the preview alone.
    std::fs::write(dirs.settings(), "{ \"editor.fontSize\": 15 }").unwrap();
    let changed = dirs.settings();
    cx.update(|_, cx| reload_config(&[changed], cx));
    draw(cx);
    assert_eq!(icon_name(cx), "Ext Icons");

    // Escape goes back to the configured theme and writes nothing.
    keys("escape", cx);
    assert!(picker(&wb, cx).is_none());
    assert_eq!(icon_name(cx), "Seti");
    assert_eq!(
        std::fs::read_to_string(dirs.settings()).unwrap(),
        "{ \"editor.fontSize\": 15 }"
    );

    // Enter writes workbench.iconTheme, keeping the rest.
    std::fs::write(dirs.settings(), original).unwrap();
    let changed = dirs.settings();
    cx.update(|_, cx| reload_config(&[changed], cx));
    show_picker(&wb, cx);
    keys("down down", cx);
    keys("enter", cx);
    assert!(picker(&wb, cx).is_none());
    assert_eq!(icon_name(cx), "Ext Icons");
    assert_eq!(
        std::fs::read_to_string(dirs.settings()).unwrap(),
        "// mine\n{\n  \"editor.fontSize\": 14, // px\n  \"workbench.iconTheme\": \"Ext Icons\"\n}\n"
    );
    let configured: Option<String> = cx.update(|_, cx| {
        cx.global::<AppState>()
            .settings
            .get("workbench.iconTheme", None)
    });
    assert_eq!(configured.as_deref(), Some("Ext Icons"));
    assert!(cx.update(|_, cx| cx.has_global::<ConfigState>()));

    // The Explorer draws the file icon of the new theme.
    let icon = cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .row_icon(Path::new("a.json"), cx)
    });
    assert!(matches!(icon, Some(IconSource::File(_))), "{icon:?}");
}

#[gpui::test]
fn unknown_icon_theme_setting_kept(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    let text = "{ \"workbench.iconTheme\": \"Gone\" }";
    init(&dirs, Some(text), cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    assert_eq!(icon_name(cx), "Seti", "Seti answers for an unknown theme");

    let p = show_picker(&wb, cx);
    assert_eq!(cx.update(|_, cx| p.read(cx).selected_index()), 0);
    keys("down", cx);
    assert_eq!(icon_name(cx), "Lucide");
    keys("escape", cx);
    assert_eq!(icon_name(cx), "Seti");
    assert_eq!(std::fs::read_to_string(dirs.settings()).unwrap(), text);
    let configured: Option<String> = cx.update(|_, cx| {
        cx.global::<AppState>()
            .settings
            .get("workbench.iconTheme", None)
    });
    assert_eq!(configured.as_deref(), Some("Gone"));
}

#[gpui::test]
fn lucide_setting_kept(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    init(&dirs, Some("{ \"workbench.iconTheme\": \"Lucide\" }"), cx);
    let (_wb, cx) = open(dirs.root.path(), cx);
    assert_eq!(icon_name(cx), "Lucide");

    let changed = dirs.settings();
    cx.update(|_, cx| reload_config(&[changed], cx));
    draw(cx);
    assert_eq!(icon_name(cx), "Lucide");
}

/// A window on `root` beside others, once its scan finished.
fn open_window(root: &Path, cx: &mut TestAppContext) -> gpui::WindowHandle<Workbench> {
    let root = root.to_path_buf();
    let handle = cx.add_window(move |window, cx| Workbench::new(root, window, cx));
    let mut vcx = VisualTestContext::from_window(handle.into(), cx);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !handle
        .read_with(&vcx, |wb, _| wb.worktree().snapshot().scan_complete())
        .unwrap()
    {
        assert!(Instant::now() < deadline, "worktree scan did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    handle
        .update(&mut vcx, |wb, _, cx| {
            wb.on_worktree_event(&WorktreeEvent::ScanComplete, cx)
        })
        .unwrap();
    draw(&mut vcx);
    handle
}

/// Closing a window while its icon theme picker previews puts the
/// configured icon theme back, and later settings apply (M3-1). With
/// `button`, the close button asks first; otherwise the window just goes.
fn closing_window_mid_icon_preview_reverts(button: bool, cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    init(&dirs, Some("{}"), cx);
    let other = tempfile::tempdir().unwrap();
    let b = open_window(other.path(), cx);
    let a = open_window(dirs.root.path(), cx);
    {
        let mut vcx = VisualTestContext::from_window(a.into(), cx);
        // Held only here, so removing the window releases the workbench.
        let wb = a.root(&mut vcx).unwrap();
        show_picker(&wb, &mut vcx);
        drop(wb);
        keys("down down", &mut vcx);
        assert_eq!(icon_name(&mut vcx), "Ext Icons", "the preview");
        if button {
            assert!(vcx.simulate_close(), "nothing to ask about");
        }
        vcx.update(|window, _| window.remove_window());
    }
    cx.run_until_parked();

    let mut vcx = VisualTestContext::from_window(b.into(), cx);
    assert_eq!(icon_name(&mut vcx), "Seti", "the configured icon theme");
    std::fs::write(dirs.settings(), "{ \"workbench.iconTheme\": \"Lucide\" }").unwrap();
    let changed = dirs.settings();
    vcx.update(|_, cx| reload_config(&[changed], cx));
    draw(&mut vcx);
    assert_eq!(icon_name(&mut vcx), "Lucide", "settings apply again");
}

#[gpui::test]
fn close_button_mid_icon_preview_reverts(cx: &mut TestAppContext) {
    closing_window_mid_icon_preview_reverts(true, cx);
}

#[gpui::test]
fn released_window_mid_icon_preview_reverts(cx: &mut TestAppContext) {
    closing_window_mid_icon_preview_reverts(false, cx);
}
