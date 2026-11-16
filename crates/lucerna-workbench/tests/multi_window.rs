//! Several windows in one process: each reads its own folder's
//! `.lucerna/settings.json` over the app's default and user layers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    BorrowAppContext as _, Entity, Global, TestAppContext, VisualTestContext, WindowHandle,
};
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::fonts::EDITOR_FONT_FAMILY;
use lucerna_workbench::{
    AppState, FileView, GitState, LanguageServers, MarkdownPreview, OpenMode, Workbench,
};
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

/// The User layer: what the app's `settings.json` holds.
fn set_user(cx: &mut TestAppContext, json: &str) {
    init(cx);
    cx.update(|cx| {
        cx.update_global::<AppState, _>(|state, _| {
            state.settings.set_layer(Layer::User, json).unwrap()
        })
    });
}

/// A folder with `a.rs`, `a.md` and `x.foo` whose `.lucerna/settings.json` holds `workspace`.
fn folder(workspace: &str) -> TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.path().join("a.md"), "# A\n\n```rust\nfn a() {}\n```\n").unwrap();
    std::fs::write(root.path().join("x.foo"), "fn x() {}\n").unwrap();
    std::fs::create_dir(root.path().join(".lucerna")).unwrap();
    std::fs::write(
        root.path().join(".lucerna").join("settings.json"),
        workspace,
    )
    .unwrap();
    root
}

/// Two windows, each a [`Workbench`] on its own folder: `ws_a` and `ws_b`
/// are their `.lucerna/settings.json`. Each folder has `a.rs` and `a.md`, and lives
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
    open_file(handle, "a.rs", cx)
}

/// Opens `path` (root-relative) in the window and returns its code view
/// once loaded.
fn open_file(
    handle: WindowHandle<Workbench>,
    path: &str,
    cx: &mut TestAppContext,
) -> Entity<CodeView> {
    handle
        .update(cx, |wb, _, cx| {
            wb.open_path(Arc::from(Path::new(path)), OpenMode::Pinned, cx)
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

/// Opens the static preview of `a.md` in the window.
fn open_preview(
    handle: WindowHandle<Workbench>,
    cx: &mut TestAppContext,
) -> Entity<MarkdownPreview> {
    let groups = handle
        .read_with(cx, |wb, _| wb.editor_groups().clone())
        .unwrap();
    let path: Arc<Path> = Arc::from(
        handle
            .read_with(cx, |wb, _| wb.root().join("a.md"))
            .unwrap(),
    );
    cx.update(|cx| {
        groups.update(cx, |groups, cx| {
            groups.update_model(cx, |m| {
                m.open_file(path, FileView::Preview, OpenMode::Pinned, true, false)
            })
        })
    });
    draw(handle, cx);
    cx.update(|cx| {
        groups
            .read(cx)
            .active_body()
            .expect("drawn")
            .downcast::<MarkdownPreview>()
            .expect("a markdown preview")
    })
}

#[gpui::test]
fn markdown_code_font_per_window(cx: &mut TestAppContext) {
    let (a, b) = open_two(cx, r#"{"editor.fontFamily": "Menlo"}"#, "{}");
    let preview_a = open_preview(a, cx);
    let preview_b = open_preview(b, cx);
    assert_eq!(
        cx.update(|cx| preview_a.read(cx).code_font_family(cx)),
        "Menlo"
    );
    assert_eq!(
        cx.update(|cx| preview_b.read(cx).code_font_family(cx)),
        EDITOR_FONT_FAMILY,
        "B has no workspace font: the default"
    );
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

fn language_servers(
    handle: WindowHandle<Workbench>,
    cx: &mut TestAppContext,
) -> Entity<LanguageServers> {
    handle
        .read_with(cx, |wb, _| wb.language_servers().clone())
        .unwrap()
}

/// A launcher whose `rust-analyzer` is an in-process fake, for the window.
fn fake_rust_analyzer(
    handle: WindowHandle<Workbench>,
    cx: &mut TestAppContext,
) -> Arc<FakeLauncher> {
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", FakeServer::new()));
    let servers = language_servers(handle, cx);
    let set = launcher.clone();
    cx.update(|cx| servers.update(cx, |servers, _| servers.set_launcher(set)));
    launcher
}

/// Draws, lets server traffic and its batches finish, and draws again.
fn settle(handle: WindowHandle<Workbench>, cx: &mut TestAppContext) {
    draw(handle, cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    draw(handle, cx);
}

#[gpui::test]
fn lsp_settings_per_window(cx: &mut TestAppContext) {
    set_user(cx, r#"{ "session.trustAllWorktrees": true }"#);
    let (a, b) = open_two(cx, r#"{"lucerna.lsp.enabled": false}"#, "{}");
    let launcher_a = fake_rust_analyzer(a, cx);
    let launcher_b = fake_rust_analyzer(b, cx);
    open_a(a, cx);
    open_a(b, cx);
    settle(a, cx);
    settle(b, cx);
    assert!(
        launcher_a.launches().is_empty(),
        "A turns language servers off"
    );
    assert_eq!(launcher_b.launches().len(), 1, "B starts rust-analyzer");
}

/// `lsp.<server>.binary.path` names what runs: a folder's own settings
/// choose it only once that folder is trusted, and only in its window.
#[gpui::test]
fn lsp_binary_ignores_workspace(cx: &mut TestAppContext) {
    let (a, b) = open_two(
        cx,
        r#"{"lsp": {"rust-analyzer": {"binary": {"path": "/bin/false-path"}}}}"#,
        "{}",
    );
    let binary = |handle: WindowHandle<Workbench>, cx: &mut TestAppContext| -> Option<PathBuf> {
        let servers = language_servers(handle, cx);
        cx.update(|cx| {
            servers
                .read(cx)
                .table()
                .get("rust-analyzer")
                .and_then(|def| def.binary.as_ref()?.path.clone())
        })
    };
    let servers_a = language_servers(a, cx);
    assert!(!cx.update(|cx| servers_a.read(cx).is_trusted()));
    assert_eq!(
        binary(a, cx),
        None,
        "untrusted: the folder's path is ignored"
    );

    cx.update(|cx| servers_a.update(cx, |servers, cx| servers.set_trusted(true, cx)));
    draw(a, cx);
    assert_eq!(binary(a, cx), Some(PathBuf::from("/bin/false-path")));
    let servers_b = language_servers(b, cx);
    cx.update(|cx| servers_b.update(cx, |servers, cx| servers.set_trusted(true, cx)));
    draw(b, cx);
    assert_eq!(binary(b, cx), None, "B's folder sets no path");
}

#[gpui::test]
fn associations_per_window(cx: &mut TestAppContext) {
    let (a, b) = open_two(cx, r#"{"files.associations": {"*.foo": "rust"}}"#, "{}");
    let view_a = open_file(a, "x.foo", cx);
    let view_b = open_file(b, "x.foo", cx);
    assert_eq!(
        cx.update(|cx| view_a.read(cx).language_id().map(str::to_owned)),
        Some("rust".to_owned())
    );
    assert_eq!(
        cx.update(|cx| view_b.read(cx).language_id().map(str::to_owned)),
        None,
        "B: plain text"
    );

    // A change of the folder's associations applies to open files.
    let settings_b = b.read_with(cx, |wb, _| wb.settings().clone()).unwrap();
    settings_b
        .update(cx, |s, cx| {
            s.set_workspace_json(r#"{"files.associations": {"*.foo": "rust"}}"#, cx)
        })
        .unwrap();
    draw(b, cx);
    assert_eq!(
        cx.update(|cx| view_b.read(cx).language_id().map(str::to_owned)),
        Some("rust".to_owned())
    );
}

#[gpui::test]
fn git_disabled_per_window(cx: &mut TestAppContext) {
    set_user(cx, r#"{ "session.trustAllWorktrees": true }"#);
    let (a, b) = open_two(cx, r#"{"git.enabled": false}"#, "{}");
    let state = |handle: WindowHandle<Workbench>, cx: &mut TestAppContext| {
        handle
            .read_with(cx, |wb, cx| {
                let git = wb.git().read(cx);
                (git.is_enabled(), git.state())
            })
            .unwrap()
    };
    assert_eq!(state(a, cx), (false, GitState::Off), "A runs no git");
    let (enabled, b_state) = state(b, cx);
    assert!(enabled);
    assert_ne!(b_state, GitState::Off, "B looks for git and a repository");
}
