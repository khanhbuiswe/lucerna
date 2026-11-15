//! Several windows in one process: each reads its own folder's
//! `.lucerna/settings.json` over the app's default and user layers.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, Global, TestAppContext, VisualTestContext, WindowHandle};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, Workbench};
use tempfile::TempDir;

/// The folders the windows of a test show, dropped with the test's app.
struct Folders(Vec<TempDir>);

impl Global for Folders {}

/// The app state with the default settings and no user file.
fn init(cx: &mut TestAppContext) {
    if cx.update(|cx| cx.has_global::<AppState>()) {
        return;
    }
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
    cx.set_global(Folders(Vec::new()));
}

/// A folder with `a.rs` whose `.lucerna/settings.json` holds `workspace`.
fn folder(workspace: &str) -> TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.rs"), "fn main() {}\n").unwrap();
    std::fs::create_dir(root.path().join(".lucerna")).unwrap();
    std::fs::write(
        root.path().join(".lucerna").join("settings.json"),
        workspace,
    )
    .unwrap();
    root
}

/// Two windows, each a [`Workbench`] on its own folder: `ws_a` and `ws_b`
/// are their `.lucerna/settings.json`. Each folder has `a.rs`, and lives
/// as long as the test's app.
fn open_two(
    cx: &mut TestAppContext,
    ws_a: &str,
    ws_b: &str,
) -> (WindowHandle<Workbench>, WindowHandle<Workbench>) {
    init(cx);
    let a = open_window(folder(ws_a), cx);
    let b = open_window(folder(ws_b), cx);
    (a, b)
}

/// A window on `root`, once its scan finished.
fn open_window(root: TempDir, cx: &mut TestAppContext) -> WindowHandle<Workbench> {
    let path = root.path().to_path_buf();
    cx.update(|cx| cx.global_mut::<Folders>().0.push(root));
    let handle = cx.add_window(move |window, cx| Workbench::new(path, window, cx));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !handle
        .read_with(cx, |wb, _| wb.worktree().snapshot().scan_complete())
        .unwrap()
    {
        assert!(Instant::now() < deadline, "worktree scan did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    handle
        .update(cx, |wb, _, cx| {
            wb.on_worktree_event(&WorktreeEvent::ScanComplete, cx)
        })
        .unwrap();
    draw(handle, cx);
    handle
}

fn draw(handle: WindowHandle<Workbench>, cx: &mut TestAppContext) {
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// Opens `a.rs` in the window and returns its code view once loaded.
fn open_a(handle: WindowHandle<Workbench>, cx: &mut TestAppContext) -> Entity<CodeView> {
    handle
        .update(cx, |wb, _, cx| {
            wb.open_path(Arc::from(Path::new("a.rs")), OpenMode::Pinned, cx)
        })
        .unwrap();
    draw(handle, cx);
    handle
        .read_with(cx, |wb, cx| {
            wb.editor_groups()
                .read(cx)
                .active_body()
                .expect("the active tab has a body")
                .downcast::<CodeView>()
                .expect("tab bodies are code views")
        })
        .unwrap()
}

fn tab_size(view: &Entity<CodeView>, cx: &mut TestAppContext) -> u32 {
    cx.update(|cx| view.read(cx).tab_size())
}

/// The indentation `Documents` detected for `a.rs`, from its settings.
fn detected_tab_size(handle: WindowHandle<Workbench>, cx: &mut TestAppContext) -> Option<u32> {
    handle
        .read_with(cx, |wb, cx| {
            wb.documents()
                .read(cx)
                .indentation(Path::new("a.rs"))
                .map(|i| i.tab_size)
        })
        .unwrap()
}

#[gpui::test]
fn editor_settings_per_window(cx: &mut TestAppContext) {
    let (a, b) = open_two(
        cx,
        r#"{"editor.tabSize": 2, "editor.wordWrap": "on"}"#,
        r#"{"editor.tabSize": 8}"#,
    );
    let view_a = open_a(a, cx);
    let view_b = open_a(b, cx);

    assert_eq!(detected_tab_size(a, cx), Some(2));
    assert_eq!(detected_tab_size(b, cx), Some(8));
    assert_eq!(tab_size(&view_a, cx), 2);
    assert_eq!(tab_size(&view_b, cx), 8);
    assert!(cx.update(|cx| view_a.read(cx).word_wrap()), "A wraps");
    assert!(!cx.update(|cx| view_b.read(cx).word_wrap()), "B does not");
}

#[gpui::test]
fn workspace_change_stays_in_window(cx: &mut TestAppContext) {
    let (a, b) = open_two(cx, r#"{"editor.tabSize": 2}"#, r#"{"editor.tabSize": 8}"#);
    let view_a = open_a(a, cx);
    let view_b = open_a(b, cx);
    assert_eq!(tab_size(&view_a, cx), 2);
    assert_eq!(tab_size(&view_b, cx), 8);

    let settings_a = a.read_with(cx, |wb, _| wb.settings().clone()).unwrap();
    settings_a
        .update(cx, |s, cx| {
            s.set_workspace_json(r#"{"editor.tabSize": 3}"#, cx)
        })
        .unwrap();
    draw(a, cx);
    draw(b, cx);

    assert_eq!(detected_tab_size(a, cx), Some(3), "A's documents follow");
    assert_eq!(tab_size(&view_a, cx), 3, "A's view follows");
    assert_eq!(detected_tab_size(b, cx), Some(8), "B's documents stay");
    assert_eq!(tab_size(&view_b, cx), 8, "B's view stays");
}
