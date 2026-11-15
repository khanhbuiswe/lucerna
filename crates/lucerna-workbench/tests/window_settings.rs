//! `WindowSettings`: one window's `.lucerna/settings.json` over the
//! app's default and user layers.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, Entity, TestAppContext};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, ConfigPaths, WindowSettings, reload_config, watch_config};
use tempfile::TempDir;

/// The app state with the user configuration in `user`.
fn init(user: &Path, cx: &mut TestAppContext) {
    let (keymap, _) = Keymap::load(Keymap::default_bindings(), None);
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
    let paths = ConfigPaths::user_only_with_user_dir(user);
    cx.update(|cx| watch_config(paths, cx));
}

/// A folder whose `.lucerna/settings.json` holds `text`.
fn folder(text: &str) -> TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".lucerna")).unwrap();
    write_workspace(root.path(), text);
    root
}

fn write_workspace(root: &Path, text: &str) {
    std::fs::write(root.join(".lucerna").join("settings.json"), text).unwrap();
}

fn window_settings(root: Option<&Path>, cx: &mut TestAppContext) -> Entity<WindowSettings> {
    let root = root.map(Path::to_path_buf);
    cx.new(|cx| WindowSettings::new(root.as_deref(), cx))
}

fn tab_size(settings: &Entity<WindowSettings>, cx: &mut TestAppContext) -> Option<u32> {
    cx.update(|cx| settings.read(cx).get("editor.tabSize", None, cx))
}

#[gpui::test]
fn reads_workspace_file(cx: &mut TestAppContext) {
    let user = tempfile::tempdir().unwrap();
    init(user.path(), cx);
    let root = folder(r#"{"editor.tabSize": 2}"#);
    let settings = window_settings(Some(root.path()), cx);
    assert_eq!(tab_size(&settings, cx), Some(2));
    assert_eq!(
        cx.update(|cx| settings.read(cx).settings_path().map(Path::to_path_buf)),
        Some(root.path().join(".lucerna").join("settings.json"))
    );
    assert!(cx.update(|cx| settings.read(cx).error().is_none()));
    // The app's store alone has no workspace layer.
    assert_eq!(
        cx.update(|cx| cx
            .global::<AppState>()
            .settings
            .get::<u32>("editor.tabSize", None)),
        Some(4)
    );
    // Trusted lookups skip it.
    assert_eq!(
        cx.update(|cx| settings
            .read(cx)
            .get_trusted::<u32>("editor.tabSize", None, cx)),
        Some(4)
    );
}

#[gpui::test]
fn user_change_notifies(cx: &mut TestAppContext) {
    let user = tempfile::tempdir().unwrap();
    init(user.path(), cx);
    let root = folder(r#"{"editor.tabSize": 2}"#);
    let settings = window_settings(Some(root.path()), cx);
    let notified = Rc::new(Cell::new(0));
    let count = notified.clone();
    cx.update(|cx| {
        cx.observe(&settings, move |_, _| count.set(count.get() + 1))
            .detach()
    });

    let user_settings = user.path().join("settings.json");
    std::fs::write(
        &user_settings,
        r#"{"editor.fontSize": 20, "editor.tabSize": 8}"#,
    )
    .unwrap();
    cx.update(|cx| reload_config(&[user_settings], cx));
    cx.run_until_parked();

    assert!(
        notified.get() > 0,
        "a user change notifies the window settings"
    );
    assert_eq!(
        cx.update(|cx| settings.read(cx).get::<u32>("editor.fontSize", None, cx)),
        Some(20)
    );
    assert_eq!(tab_size(&settings, cx), Some(2), "the workspace still wins");
}

#[gpui::test]
fn reload_picks_up_edit(cx: &mut TestAppContext) {
    let user = tempfile::tempdir().unwrap();
    init(user.path(), cx);
    let root = folder(r#"{"editor.tabSize": 2}"#);
    let settings = window_settings(Some(root.path()), cx);
    let generation = cx.update(|cx| settings.read(cx).generation());

    write_workspace(root.path(), r#"{"editor.tabSize": 3}"#);
    settings.update(cx, |s, cx| s.reload(cx));
    cx.run_until_parked();

    assert_eq!(tab_size(&settings, cx), Some(3));
    assert!(cx.update(|cx| settings.read(cx).generation()) > generation);

    // A removed file is an empty layer.
    std::fs::remove_file(root.path().join(".lucerna/settings.json")).unwrap();
    settings.update(cx, |s, cx| s.reload(cx));
    cx.run_until_parked();
    assert_eq!(tab_size(&settings, cx), Some(4));
    assert!(cx.update(|cx| settings.read(cx).error().is_none()));
}

#[gpui::test]
fn broken_file_keeps_previous_and_reports(cx: &mut TestAppContext) {
    let user = tempfile::tempdir().unwrap();
    init(user.path(), cx);
    let root = folder(r#"{"editor.tabSize": 2}"#);
    let settings = window_settings(Some(root.path()), cx);

    write_workspace(root.path(), "{\n  \"editor.tabSize\": 3,\n  \"x\": \n}\n");
    settings.update(cx, |s, cx| s.reload(cx));
    cx.run_until_parked();

    assert_eq!(tab_size(&settings, cx), Some(2), "the previous layer stays");
    let error = cx
        .update(|cx| settings.read(cx).error().map(|e| e.status_text()))
        .expect("the error is kept");
    assert!(error.starts_with(".lucerna/settings.json: "), "{error}");

    // Fixing it clears the error.
    write_workspace(root.path(), r#"{"editor.tabSize": 3}"#);
    settings.update(cx, |s, cx| s.reload(cx));
    cx.run_until_parked();
    assert_eq!(tab_size(&settings, cx), Some(3));
    assert!(cx.update(|cx| settings.read(cx).error().is_none()));

    // A broken file at creation reports too, on the defaults and user.
    write_workspace(root.path(), "{ \"editor.tabSize\": ");
    let fresh = window_settings(Some(root.path()), cx);
    assert_eq!(tab_size(&fresh, cx), Some(4));
    assert!(cx.update(|cx| fresh.read(cx).error().is_some()));
}

#[gpui::test]
fn single_file_has_no_workspace(cx: &mut TestAppContext) {
    let user = tempfile::tempdir().unwrap();
    init(user.path(), cx);
    let settings = window_settings(None, cx);
    assert_eq!(tab_size(&settings, cx), Some(4));
    assert!(cx.update(|cx| settings.read(cx).settings_path().is_none()));
    settings.update(cx, |s, cx| s.reload(cx));
    cx.run_until_parked();
    assert_eq!(tab_size(&settings, cx), Some(4));
    assert!(cx.update(|cx| settings.read(cx).error().is_none()));
}

#[gpui::test]
fn two_entities_independent(cx: &mut TestAppContext) {
    let user = tempfile::tempdir().unwrap();
    init(user.path(), cx);
    let (a, b) = (
        folder(r#"{"editor.tabSize": 2}"#),
        folder(r#"{"editor.tabSize": 8}"#),
    );
    let (sa, sb) = (
        window_settings(Some(a.path()), cx),
        window_settings(Some(b.path()), cx),
    );
    assert_eq!(tab_size(&sa, cx), Some(2));
    assert_eq!(tab_size(&sb, cx), Some(8));

    sa.update(cx, |s, cx| {
        s.set_workspace_json(r#"{"editor.tabSize": 3}"#, cx)
    })
    .unwrap();
    assert_eq!(tab_size(&sa, cx), Some(3));
    assert_eq!(tab_size(&sb, cx), Some(8), "the other window is untouched");
    assert!(
        sa.update(cx, |s, cx| s.set_workspace_json("{ broken", cx))
            .is_err()
    );
    assert_eq!(tab_size(&sa, cx), Some(3));
}

#[gpui::test]
fn app_scope_keys_ignored(cx: &mut TestAppContext) {
    let user = tempfile::tempdir().unwrap();
    init(user.path(), cx);
    let root = folder(r#"{"workbench.colorTheme": "X", "editor.tabSize": 2}"#);
    let settings = window_settings(Some(root.path()), cx);
    assert_eq!(tab_size(&settings, cx), Some(2));
    let theme = cx.update(|cx| {
        settings
            .read(cx)
            .get::<String>("workbench.colorTheme", None, cx)
    });
    assert_ne!(theme.as_deref(), Some("X"));
}
