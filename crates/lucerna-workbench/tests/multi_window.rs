//! Several windows in one process: each reads its own folder's
//! `.lucerna/settings.json` over the app's default and user layers, and
//! the app's router opens paths among them.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnyWindowHandle, App, BorrowAppContext as _, Entity, Focusable as _, Global, OwnedMenuItem,
    QuitMode, TestAppContext, VisualTestContext, WindowHandle,
};
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_terminal::{Terminal, TerminalOptions};
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::app;
use lucerna_workbench::app_menu::{self, OpenRecentEntry, RunCommand};
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::fonts::EDITOR_FONT_FAMILY;
use lucerna_workbench::recent::{RecentEntry, RecentKind, RecentStore, recent_path};
use lucerna_workbench::router::{self, Target, WindowKind};
use lucerna_workbench::session::{self, Session, SessionWindow, session_path};
use lucerna_workbench::system_shell::{SystemShell, SystemShellGlobal};
use lucerna_workbench::{
    AppState, FileView, GitState, LanguageServers, MarkdownPreview, OpenMode, PanelPosition,
    Replacement, TerminalSpawner, WelcomeEvent, WelcomeView, Workbench, status_message,
};
use serde_json::Value;
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

/// A Trash that deletes at once and records what it was given.
struct RecordingTrash(std::sync::Mutex<Vec<PathBuf>>);

impl SystemShell for RecordingTrash {
    fn reveal(&self, _: &Path, _: &gpui::App) {}

    fn open_terminal(&self, _: &Path, _: &str) -> std::io::Result<()> {
        Ok(())
    }

    fn trash(&self, path: &Path) -> Result<(), String> {
        self.0.lock().unwrap().push(path.to_path_buf());
        lucerna_project::fs_ops::delete_permanently(path).map_err(|e| e.to_string())
    }
}

/// Starts terminals as fakes and keeps the options each was started with.
fn fake_terminals(
    handle: WindowHandle<Workbench>,
    cx: &mut TestAppContext,
) -> Rc<RefCell<Vec<TerminalOptions>>> {
    let started = Rc::new(RefCell::new(Vec::new()));
    let record = started.clone();
    let spawner: TerminalSpawner = Rc::new(move |opts: TerminalOptions| {
        record.borrow_mut().push(opts.clone());
        let (_tx, rx) = futures::channel::mpsc::unbounded();
        Ok((Terminal::fake(opts.rows, opts.cols, opts.scrollback), rx))
    });
    handle
        .update(cx, |wb, _, _| wb.set_terminal_spawner(spawner))
        .unwrap();
    started
}

fn run(
    handle: WindowHandle<Workbench>,
    command: &str,
    args: Option<Value>,
    cx: &mut TestAppContext,
) {
    handle
        .update(cx, |wb, window, cx| {
            wb.run_command(command, args.as_ref(), window, cx);
        })
        .unwrap();
    draw(handle, cx);
}

/// Inserts `x` at the start of `a.rs`, open in the window (through the
/// documents: vim, on in one window, would take typed keys).
fn edit_a(handle: WindowHandle<Workbench>, cx: &mut TestAppContext) {
    let view = open_a(handle, cx);
    let docs = handle
        .read_with(cx, |wb, _| wb.documents().clone())
        .unwrap();
    let edit = (0..0, Arc::<str>::from("x"));
    cx.update(|cx| {
        docs.update(cx, |d, cx| {
            d.apply_edits(
                Path::new("a.rs"),
                vec![edit],
                None,
                None,
                view.entity_id(),
                cx,
            )
        })
    })
    .expect("a.rs is loaded");
    draw(handle, cx);
}

#[gpui::test]
fn window_settings_reach_every_reader(cx: &mut TestAppContext) {
    let (a, b) = open_two(
        cx,
        r#"{
            "terminal.integrated.scrollback": 123,
            "explorer.confirmDelete": false,
            "workbench.panel.defaultLocation": "right",
            "vim.enabled": true,
            "files.autoSave": "afterDelay"
        }"#,
        "{}",
    );

    // Terminal: a new terminal's scrollback.
    let (started_a, started_b) = (fake_terminals(a, cx), fake_terminals(b, cx));
    run(a, "workbench.action.terminal.new", None, cx);
    run(b, "workbench.action.terminal.new", None, cx);
    assert_eq!(started_a.borrow().last().map(|o| o.scrollback), Some(123));
    assert_eq!(
        started_b.borrow().last().map(|o| o.scrollback),
        Some(10_000)
    );

    // Explorer: A moves to the Trash without asking, B asks.
    let trash = Arc::new(RecordingTrash(Default::default()));
    cx.set_global(SystemShellGlobal(trash.clone()));
    let x_foo = |handle: WindowHandle<Workbench>, cx: &mut TestAppContext| {
        handle
            .read_with(cx, |wb, _| wb.root().join("x.foo"))
            .unwrap()
    };
    let args = Some(serde_json::json!({ "path": "x.foo" }));
    run(a, "moveFileToTrash", args.clone(), cx);
    assert!(!cx.has_pending_prompt(), "A does not ask");
    let deadline = Instant::now() + Duration::from_secs(10);
    while trash.0.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "A's delete did not run");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(*trash.0.lock().unwrap(), [x_foo(a, cx)]);
    run(b, "moveFileToTrash", args, cx);
    assert!(cx.has_pending_prompt(), "B asks first");
    cx.simulate_prompt_answer("Cancel");
    draw(b, cx);
    assert!(x_foo(b, cx).is_file());

    // Panel position.
    let position = |handle: WindowHandle<Workbench>, cx: &mut TestAppContext| {
        handle
            .read_with(cx, |wb, _| wb.panel_state().position)
            .unwrap()
    };
    assert_eq!(position(a, cx), PanelPosition::Right);
    assert_eq!(position(b, cx), PanelPosition::Bottom);

    // Vim layer.
    let vim = |handle: WindowHandle<Workbench>, cx: &mut TestAppContext| {
        handle
            .read_with(cx, |wb, cx| wb.vim().read(cx).enabled())
            .unwrap()
    };
    assert!(vim(a, cx), "A turns vim on");
    assert!(!vim(b, cx), "B leaves it off");

    // Auto-save: A saves after the default 1000 ms delay, B never.
    edit_a(a, cx);
    edit_a(b, cx);
    cx.executor().advance_clock(Duration::from_millis(1500));
    cx.run_until_parked();
    let disk = |handle: WindowHandle<Workbench>, cx: &mut TestAppContext| {
        let path = handle
            .read_with(cx, |wb, _| wb.root().join("a.rs"))
            .unwrap();
        std::fs::read_to_string(path).unwrap()
    };
    assert_eq!(disk(a, cx), "xfn main() {}\n", "A auto-saved");
    assert_eq!(disk(b, cx), "fn main() {}\n", "B did not");
    let b_dirty = b
        .read_with(cx, |wb, cx| {
            wb.documents().read(cx).is_dirty(Path::new("a.rs"))
        })
        .unwrap();
    assert!(b_dirty, "B holds the edit unsaved");
}

// One router for every window (spec §6.2–6.3): `router::open` focuses a
// window already on the path, opens a file in the window whose root holds
// it, reuses a Welcome window, and opens new windows in this process.

/// The app state, and a data dir (with `recent.json` listing `recent`
/// folders, newest first) the router records into; returns the data dir.
fn routed(recent: &[&Path], cx: &mut TestAppContext) -> PathBuf {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let data = std::fs::canonicalize(dir.path()).unwrap();
    let entries: Vec<Value> = recent
        .iter()
        .map(|path| serde_json::json!({ "path": path, "kind": "folder" }))
        .collect();
    let file = serde_json::json!({ "version": 1, "entries": entries });
    std::fs::write(recent_path(&data), file.to_string()).unwrap();
    cx.update(|cx| {
        cx.global_mut::<Folders>().0.push(dir);
        cx.set_global(RecentStore {
            data_dir: Some(data.clone()),
        });
    });
    data
}

/// A canonical folder holding `src/x.rs`, kept for the test's app.
fn project(cx: &mut TestAppContext) -> PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::create_dir(path.join("src")).unwrap();
    std::fs::write(path.join("src").join("x.rs"), "fn x() {}\n").unwrap();
    cx.update(|cx| cx.global_mut::<Folders>().0.push(dir));
    path
}

fn open_routed(paths: Vec<PathBuf>, target: Target, cx: &mut TestAppContext) {
    cx.update(|cx| router::open_for_test(paths, target, cx));
    cx.run_until_parked();
}

fn routed_windows(cx: &mut TestAppContext) -> Vec<(AnyWindowHandle, WindowKind)> {
    cx.update(|cx| router::windows(cx))
}

fn kinds(cx: &mut TestAppContext) -> Vec<WindowKind> {
    routed_windows(cx).into_iter().map(|(_, k)| k).collect()
}

/// The workbench window the router shows on `kind`.
fn routed_workbench(kind: &WindowKind, cx: &mut TestAppContext) -> WindowHandle<Workbench> {
    let (handle, _) = routed_windows(cx)
        .into_iter()
        .find(|(_, k)| k == kind)
        .unwrap_or_else(|| panic!("a window on {kind:?}"));
    WindowHandle::new(handle.window_id())
}

/// The active tab's path in the workbench `handle`.
fn active_tab(handle: WindowHandle<Workbench>, cx: &mut TestAppContext) -> Option<PathBuf> {
    handle
        .read_with(cx, |wb, cx| {
            wb.editor_groups()
                .read(cx)
                .model()
                .active_tab()
                .and_then(|tab| tab.path().map(|p| p.to_path_buf()))
        })
        .unwrap()
}

#[gpui::test]
fn open_two_folders_one_app(cx: &mut TestAppContext) {
    routed(&[], cx);
    let (a, b) = (project(cx), project(cx));
    open_routed(vec![a.clone()], Target::Auto, cx);
    open_routed(vec![b.clone()], Target::Auto, cx);
    assert_eq!(
        kinds(cx),
        vec![WindowKind::Folder(b), WindowKind::Folder(a)],
        "two windows, the last opened first"
    );
    assert_eq!(cx.update(|cx| cx.windows().len()), 2);
}

#[gpui::test]
fn reopen_focuses_existing(cx: &mut TestAppContext) {
    routed(&[], cx);
    let (a, b) = (project(cx), project(cx));
    open_routed(vec![a.clone()], Target::Auto, cx);
    open_routed(vec![b.clone()], Target::Auto, cx);
    open_routed(vec![a.clone()], Target::Auto, cx);
    assert_eq!(
        kinds(cx),
        vec![WindowKind::Folder(a.clone()), WindowKind::Folder(b)],
        "still two windows; A is the most recently used"
    );
    open_routed(vec![a.clone()], Target::NewWindow, cx);
    assert_eq!(
        kinds(cx).len(),
        2,
        "a new window on an open folder focuses it"
    );
}

#[cfg(unix)]
#[gpui::test]
fn open_through_symlink_focuses(cx: &mut TestAppContext) {
    routed(&[], cx);
    let a = project(cx);
    let links = tempfile::tempdir().unwrap();
    let link = links.path().join("link");
    std::os::unix::fs::symlink(&a, &link).unwrap();
    open_routed(vec![a.clone()], Target::Auto, cx);
    open_routed(vec![link], Target::Auto, cx);
    assert_eq!(kinds(cx), vec![WindowKind::Folder(a)]);
}

#[gpui::test]
fn file_inside_b_opens_in_b(cx: &mut TestAppContext) {
    routed(&[], cx);
    let (a, b) = (project(cx), project(cx));
    open_routed(vec![a.clone()], Target::Auto, cx);
    open_routed(vec![b.clone()], Target::Auto, cx);
    // A is the most recently used.
    open_routed(vec![a.clone()], Target::Auto, cx);
    open_routed(vec![b.join("src").join("x.rs")], Target::Auto, cx);
    assert_eq!(
        kinds(cx),
        vec![WindowKind::Folder(b.clone()), WindowKind::Folder(a.clone())],
        "no new window; B is now the most recently used"
    );
    let in_b = active_tab(routed_workbench(&WindowKind::Folder(b), cx), cx);
    assert!(
        in_b.as_deref().is_some_and(|p| p.ends_with("src/x.rs")),
        "{in_b:?}"
    );
    let in_a = active_tab(routed_workbench(&WindowKind::Folder(a), cx), cx);
    assert_eq!(in_a, None, "A is unchanged");
}

#[gpui::test]
fn new_window_then_pick_folder(cx: &mut TestAppContext) {
    routed(&[], cx);
    let (a, b, c) = (project(cx), project(cx), project(cx));
    open_routed(vec![a.clone()], Target::Auto, cx);
    open_routed(vec![b.clone()], Target::Auto, cx);
    cx.update(|cx| router::open_welcome(Target::NewWindow, cx));
    cx.run_until_parked();
    let windows = routed_windows(cx);
    assert_eq!(windows.len(), 3);
    assert_eq!(
        windows[0].1,
        WindowKind::Welcome,
        "the new window is Welcome"
    );
    let welcome = windows[0].0;
    let view = cx
        .update(|cx| welcome.update(cx, |root, _, _| root.downcast::<WelcomeView>().ok()))
        .unwrap()
        .expect("the root is the Welcome view");
    cx.update(|cx| {
        view.update(cx, |_, cx| {
            cx.emit(WelcomeEvent::Open {
                path: c.clone(),
                new_window: false,
            })
        })
    });
    cx.run_until_parked();
    let windows = routed_windows(cx);
    assert_eq!(windows.len(), 3, "the Welcome window turned into C");
    assert_eq!(windows[0].0.window_id(), welcome.window_id());
    assert_eq!(windows[0].1, WindowKind::Folder(c));
}

#[gpui::test]
fn open_recent_cmd_enter_new_window(cx: &mut TestAppContext) {
    let other = {
        init(cx);
        project(cx)
    };
    routed(&[&other], cx);
    let a = project(cx);
    open_routed(vec![a.clone()], Target::Auto, cx);
    let handle = routed_workbench(&WindowKind::Folder(a.clone()), cx);
    let vcx = &mut VisualTestContext::from_window(handle.into(), cx);
    vcx.simulate_keystrokes("ctrl-r");
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.simulate_keystrokes("cmd-enter");
    vcx.run_until_parked();
    assert_eq!(
        kinds(cx),
        vec![WindowKind::Folder(other), WindowKind::Folder(a)],
        "⌘Enter opens a new window; this one stays"
    );
}

#[gpui::test]
fn replace_releases_old_workbench(cx: &mut TestAppContext) {
    routed(&[], cx);
    let (a, b) = (project(cx), project(cx));
    open_routed(vec![a.clone()], Target::Auto, cx);
    let handle = routed_workbench(&WindowKind::Folder(a), cx);
    let released = handle
        .update(cx, |_, _, cx| cx.entity().downgrade())
        .unwrap();
    handle
        .update(cx, |wb, window, cx| {
            wb.replace_with(Replacement::Path(b.clone()), window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    let windows = routed_windows(cx);
    assert_eq!(windows.len(), 1, "the same window");
    assert_eq!(windows[0].0.window_id(), handle.window_id());
    assert_eq!(windows[0].1, WindowKind::Folder(b));
    assert!(
        released.upgrade().is_none(),
        "the old workbench is released"
    );
}

#[gpui::test]
fn same_file_two_windows_reload(cx: &mut TestAppContext) {
    routed(&[], cx);
    let a = project(cx);
    let file = a.join("src").join("x.rs");
    open_routed(vec![a.clone()], Target::Auto, cx);
    open_routed(vec![file.clone()], Target::Auto, cx);
    open_routed(vec![file.clone()], Target::NewWindow, cx);
    assert_eq!(
        kinds(cx),
        vec![
            WindowKind::File(file.clone()),
            WindowKind::Folder(a.clone())
        ],
        "the file opens alone in a new window"
    );
    let in_a = routed_workbench(&WindowKind::Folder(a), cx);
    let in_b = routed_workbench(&WindowKind::File(file.clone()), cx);
    let view = |handle: WindowHandle<Workbench>, cx: &mut TestAppContext| {
        handle
            .read_with(cx, |wb, cx| {
                wb.editor_groups()
                    .read(cx)
                    .active_body()
                    .expect("the file is open")
                    .downcast::<CodeView>()
                    .expect("tab bodies are code views")
            })
            .unwrap()
    };
    let text = |view: &Entity<CodeView>, cx: &mut TestAppContext| {
        cx.update(|cx| {
            let s = view.read(cx).snapshot();
            s.text_for_range(0..s.len_bytes())
        })
    };
    draw(in_a, cx);
    draw(in_b, cx);
    let (view_a, view_b) = (view(in_a, cx), view(in_b, cx));
    // Save an edit in A.
    let vcx = &mut VisualTestContext::from_window(in_a.into(), cx);
    let focus = vcx.update(|_, cx| view_a.focus_handle(cx));
    vcx.update(|window, cx| window.focus(&focus, cx));
    vcx.simulate_input("y");
    vcx.run_until_parked();
    run(in_a, "workbench.action.files.save", None, cx);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "yfn x() {}\n");
    assert_eq!(text(&view_b, cx), "fn x() {}\n", "B has its own buffer");
    // B's watcher reports the change, as for any change on disk.
    in_b.update(cx, |wb, _, cx| {
        wb.on_open_file_changes(std::slice::from_ref(&file), cx)
    })
    .unwrap();
    draw(in_b, cx);
    assert_eq!(text(&view_b, cx), "yfn x() {}\n", "B reloads");
}

#[gpui::test]
fn status_message_stays_in_window(cx: &mut TestAppContext) {
    let (a, b) = open_two(cx, "{}", "{}");
    let (id_a, id_b) = (a.window_id(), b.window_id());
    cx.update(|cx| status_message::show(id_a, "hi", cx));
    draw(a, cx);
    draw(b, cx);
    let shown = |id, cx: &mut TestAppContext| {
        cx.update(|cx| status_message::current(id, cx).map(|m| m.to_string()))
    };
    assert_eq!(shown(id_a, cx).as_deref(), Some("hi"));
    assert_eq!(shown(id_b, cx), None, "B shows nothing");

    // B's own message replaces nothing in A; A's timer clears only A's.
    cx.update(|cx| status_message::show(id_b, "there", cx));
    cx.update(|cx| status_message::show(id_a, "again", cx));
    assert_eq!(shown(id_b, cx).as_deref(), Some("there"));
    cx.executor()
        .advance_clock(status_message::DURATION + Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(shown(id_a, cx), None);
    assert_eq!(shown(id_b, cx), None);
}

// Quitting, the last window, the Dock and the menu with no window (spec
// M8–M11).

/// Two routed folder windows, A with a dirty `a.rs` and B with a dirty
/// `b.rs`; B, opened last, is the most recently used. Quits are counted,
/// not done.
fn two_dirty_windows(
    cx: &mut TestAppContext,
) -> (WindowHandle<Workbench>, WindowHandle<Workbench>) {
    routed(&[], cx);
    cx.update(app_menu::install);
    cx.update(app::count_quits_for_test);
    let (a, b) = (project(cx), project(cx));
    std::fs::write(a.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(b.join("b.rs"), "fn b() {}\n").unwrap();
    open_routed(vec![a.clone()], Target::Auto, cx);
    open_routed(vec![b.clone()], Target::Auto, cx);
    let in_a = routed_workbench(&WindowKind::Folder(a), cx);
    let in_b = routed_workbench(&WindowKind::Folder(b), cx);
    edit_file(in_a, "a.rs", cx);
    edit_file(in_b, "b.rs", cx);
    (in_a, in_b)
}

/// Inserts `x` at the start of `path` (root-relative), opening it.
fn edit_file(handle: WindowHandle<Workbench>, path: &str, cx: &mut TestAppContext) {
    let view = open_file(handle, path, cx);
    let docs = handle
        .read_with(cx, |wb, _| wb.documents().clone())
        .unwrap();
    let edit = (0..0, Arc::<str>::from("x"));
    cx.update(|cx| {
        docs.update(cx, |d, cx| {
            d.apply_edits(
                Path::new(path),
                vec![edit],
                None,
                None,
                view.entity_id(),
                cx,
            )
        })
    })
    .expect("the file is loaded");
    draw(handle, cx);
}

fn quit(cx: &mut TestAppContext) {
    cx.update(|cx| cx.dispatch_action(&app_menu::Quit));
    cx.run_until_parked();
}

/// The pending prompt's message.
fn prompt(cx: &mut TestAppContext) -> String {
    cx.pending_prompt().expect("a prompt is up").0
}

#[gpui::test]
fn quit_cancel_in_second_window_keeps_all(cx: &mut TestAppContext) {
    two_dirty_windows(cx);
    quit(cx);
    assert!(prompt(cx).contains("b.rs"), "B, used last, asks first");
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert!(prompt(cx).contains("a.rs"), "then A");
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(cx.update(|cx| app::quits_for_test(cx)), 0, "no quit");
    assert_eq!(routed_windows(cx).len(), 2, "both windows stay");
}

#[gpui::test]
fn quit_dont_save_both_quits(cx: &mut TestAppContext) {
    two_dirty_windows(cx);
    quit(cx);
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(cx.update(|cx| app::quits_for_test(cx)), 1);
}

#[gpui::test]
fn quit_survives_window_closed_during_prompt(cx: &mut TestAppContext) {
    let (in_a, in_b) = two_dirty_windows(cx);
    // A is the most recently used: it asks first.
    cx.update(|cx| router::open_for_test(vec![root_of(in_a, cx)], Target::Auto, cx));
    cx.run_until_parked();
    quit(cx);
    assert!(prompt(cx).contains("a.rs"), "A asks first");
    in_b.update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt(), "B is gone and asks nothing");
    assert_eq!(cx.update(|cx| app::quits_for_test(cx)), 1);
}

#[gpui::test]
fn quit_asks_again_for_window_edited_during_prompt(cx: &mut TestAppContext) {
    let data = routed(&[], cx);
    cx.update(app_menu::install);
    cx.update(app::count_quits_for_test);
    let (a, b) = (project(cx), project(cx));
    std::fs::write(a.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(b.join("c.rs"), "fn c() {}\n").unwrap();
    open_routed(vec![a.clone()], Target::Auto, cx);
    open_routed(vec![b.clone()], Target::Auto, cx);
    let in_a = routed_workbench(&WindowKind::Folder(a), cx);
    let in_b = routed_workbench(&WindowKind::Folder(b), cx);
    // B, used last, is clean when its turn comes; A asks after it.
    edit_file(in_a, "a.rs", cx);
    quit(cx);
    assert!(prompt(cx).contains("a.rs"), "B is clean: A asks");
    // B is edited while A's prompt is up.
    edit_file(in_b, "c.rs", cx);
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| app::quits_for_test(cx)),
        0,
        "B was not asked"
    );
    assert!(prompt(cx).contains("c.rs"), "B asks now");
    // Only the quit's own write brings the list back.
    std::fs::remove_file(session_path(&data)).unwrap();
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(cx.update(|cx| app::quits_for_test(cx)), 1);
    let saved = session::load(&session_path(&data));
    assert_eq!(saved.windows.len(), 2, "the quit wrote both windows");
}

fn root_of(handle: WindowHandle<Workbench>, cx: &mut App) -> PathBuf {
    handle
        .read_with(cx, |wb, _| wb.root().to_path_buf())
        .unwrap()
}

/// The File menu's item `title`, and whether it is disabled.
fn file_item_disabled(title: &str, cx: &mut TestAppContext) -> bool {
    let menus = cx.update(|cx| cx.get_menus()).expect("menus are set");
    let file = menus
        .iter()
        .find(|m| m.name.as_ref() == "File")
        .expect("a File menu");
    file.items
        .iter()
        .find_map(|item| match item {
            OwnedMenuItem::Action { name, disabled, .. } if name == title => Some(*disabled),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {title} item"))
}

#[gpui::test]
fn close_last_window_keeps_app(cx: &mut TestAppContext) {
    routed(&[], cx);
    cx.update(|cx| cx.set_quit_mode(QuitMode::Explicit));
    cx.update(app_menu::install);
    let a = project(cx);
    open_routed(vec![a], Target::Auto, cx);
    assert!(!file_item_disabled("Save", cx));
    for (handle, _) in routed_windows(cx) {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
    }
    cx.run_until_parked();
    assert!(routed_windows(cx).is_empty());
    assert!(cx.update(|cx| cx.windows().is_empty()));
    assert!(!file_item_disabled("New Window", cx), "New Window runs");
    let open = if cfg!(target_os = "macos") {
        "Open…"
    } else {
        "Open Folder…"
    };
    assert!(!file_item_disabled(open, cx), "{open} runs");
    assert!(file_item_disabled("Save", cx), "Save needs a workbench");
    assert!(
        file_item_disabled("Close Folder", cx),
        "Close Folder needs a workbench"
    );
    let menus = cx.update(|cx| cx.get_menus()).expect("menus are set");
    let settings_json = menus.iter().flat_map(|m| m.items.iter()).find_map(|item| {
        let OwnedMenuItem::Submenu(menu) = item else {
            return None;
        };
        menu.items.iter().find_map(|item| match item {
            OwnedMenuItem::Action { name, disabled, .. } if name == "Open Settings (JSON)" => {
                Some(*disabled)
            }
            _ => None,
        })
    });
    assert_eq!(
        settings_json,
        Some(false),
        "user settings open without a window"
    );
}

#[gpui::test]
fn menu_open_folder_without_window(cx: &mut TestAppContext) {
    routed(&[], cx);
    cx.update(app_menu::install);
    let a = project(cx);
    assert!(cx.update(|cx| cx.windows().is_empty()));
    let id = if cfg!(target_os = "macos") {
        "workbench.action.files.openFileFolder"
    } else {
        "workbench.action.files.openFolder"
    };
    cx.update(|cx| cx.dispatch_action(&RunCommand { id: id.into() }));
    cx.run_until_parked();
    let picked = a.clone();
    cx.simulate_path_prompt_response(move |_| Some(vec![picked]));
    cx.run_until_parked();
    assert_eq!(kinds(cx), vec![WindowKind::Folder(a)], "a folder window");
}

#[gpui::test]
fn menu_open_recent_without_window(cx: &mut TestAppContext) {
    let a = {
        init(cx);
        project(cx)
    };
    routed(&[&a], cx);
    cx.update(app_menu::install);
    let entry = RecentEntry {
        path: a.clone(),
        kind: RecentKind::Folder,
    };
    cx.update(|cx| cx.dispatch_action(&OpenRecentEntry { entry }));
    cx.run_until_parked();
    assert_eq!(kinds(cx), vec![WindowKind::Folder(a)]);
}

#[gpui::test]
fn reopen_opens_welcome(cx: &mut TestAppContext) {
    routed(&[], cx);
    cx.update(app::on_reopen_for_test);
    cx.run_until_parked();
    assert_eq!(kinds(cx), vec![WindowKind::Welcome]);
}

// `windows.json` (spec §6.6): the router writes its workbench windows,
// most recently used first, and a start reopens them.

fn folder_window(path: &Path) -> SessionWindow {
    SessionWindow {
        kind: RecentKind::Folder,
        path: path.to_path_buf(),
    }
}

/// `windows.json` in `data` once the background writes are done.
fn saved_session(data: &Path, cx: &mut TestAppContext) -> Session {
    cx.run_until_parked();
    session::load(&session_path(data))
}

#[gpui::test]
fn session_follows_open_and_close(cx: &mut TestAppContext) {
    let data = routed(&[], cx);
    let (a, b) = (project(cx), project(cx));
    open_routed(vec![a.clone()], Target::Auto, cx);
    open_routed(vec![b.clone()], Target::Auto, cx);
    assert_eq!(
        saved_session(&data, cx).windows,
        vec![folder_window(&b), folder_window(&a)],
        "most recently used first"
    );
    open_routed(vec![a.clone()], Target::Auto, cx);
    assert_eq!(
        saved_session(&data, cx).windows,
        vec![folder_window(&a), folder_window(&b)],
        "A, focused again, comes first"
    );
    let in_b = routed_workbench(&WindowKind::Folder(b), cx);
    in_b.update(cx, |_, window, _| window.remove_window())
        .unwrap();
    assert_eq!(
        saved_session(&data, cx).windows,
        vec![folder_window(&a)],
        "a closed window leaves the list"
    );
    cx.update(|cx| router::open_welcome(Target::NewWindow, cx));
    assert_eq!(kinds(cx).len(), 2);
    assert_eq!(
        saved_session(&data, cx).windows,
        vec![folder_window(&a)],
        "Welcome is not written"
    );
}

#[gpui::test]
fn restore_opens_in_reverse_order(cx: &mut TestAppContext) {
    let data = routed(&[], cx);
    let (a, c) = (project(cx), project(cx));
    let gone = data.join("gone");
    let saved = Session {
        windows: vec![folder_window(&c), folder_window(&gone), folder_window(&a)],
    };
    session::save(&session_path(&data), &saved).unwrap();
    cx.update(|cx| {
        let windows = session::to_restore(
            &session::load(&session_path(&data)),
            lucerna_workbench::RestoreWindows::All,
            Path::exists,
        );
        router::restore(windows, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        kinds(cx),
        vec![WindowKind::Folder(c.clone()), WindowKind::Folder(a.clone())],
        "two windows; C, used last before, is the most recently used"
    );
    assert_eq!(
        saved_session(&data, cx).windows,
        vec![folder_window(&c), folder_window(&a)],
        "the missing folder is dropped from the list"
    );
}

#[gpui::test]
fn requests_wait_for_restore(cx: &mut TestAppContext) {
    let data = routed(&[], cx);
    let (a, b) = (project(cx), project(cx));
    cx.update(router::begin_restore);
    open_routed(vec![a.clone()], Target::Auto, cx);
    assert!(
        cx.update(|cx| cx.windows().is_empty()),
        "the request waits for the restore"
    );
    cx.update(|cx| router::restore(vec![folder_window(&b), folder_window(&a)], cx));
    cx.run_until_parked();
    assert_eq!(
        kinds(cx),
        vec![WindowKind::Folder(a.clone()), WindowKind::Folder(b.clone())],
        "one window each; A, asked for, in front"
    );
    assert_eq!(cx.update(|cx| cx.windows().len()), 2);
    assert_eq!(
        saved_session(&data, cx).windows,
        vec![folder_window(&a), folder_window(&b)]
    );
}

/// `luc`'s reply waits for the restore and carries the real result of
/// its open, also for a request that waited (spec M13, M14).
#[cfg(unix)]
#[gpui::test]
fn cli_reply_waits_for_restore(cx: &mut TestAppContext) {
    use futures::channel::oneshot;
    use lucerna_workbench::cli_socket::{CliReply, CliRequest};

    routed(&[], cx);
    let a = project(cx);
    // Reads the user configuration as `app::run` does; opens nothing.
    open_routed(Vec::new(), Target::Auto, cx);
    cx.update(router::begin_restore);
    let (ok_tx, mut ok_rx) = oneshot::channel();
    let (err_tx, mut err_rx) = oneshot::channel();
    cx.update(|cx| {
        router::answer_cli(CliRequest::new(a.clone(), Vec::new()), ok_tx, cx);
        let missing = CliRequest::new(a.clone(), vec![PathBuf::from("nope")]);
        router::answer_cli(missing, err_tx, cx);
    });
    assert_eq!(
        ok_rx.try_recv().unwrap(),
        None,
        "no reply before the restore"
    );
    cx.update(|cx| router::restore(Vec::new(), cx));
    cx.run_until_parked();
    assert_eq!(ok_rx.try_recv().unwrap(), Some(CliReply::ok()));
    let expected = format!("path not found: {}", a.join("nope").display());
    assert_eq!(err_rx.try_recv().unwrap(), Some(CliReply::error(expected)));
    assert_eq!(kinds(cx), vec![WindowKind::Folder(a)]);
}

#[cfg(unix)]
#[gpui::test]
fn cli_newer_version_rejected(cx: &mut TestAppContext) {
    use futures::channel::oneshot;
    use lucerna_workbench::cli_socket::{CliReply, CliRequest, PROTOCOL_VERSION};

    routed(&[], cx);
    let a = project(cx);
    open_routed(Vec::new(), Target::Auto, cx);
    let (tx, mut rx) = oneshot::channel();
    let mut request = CliRequest::new(a, Vec::new());
    request.version = PROTOCOL_VERSION + 1;
    cx.update(|cx| router::answer_cli(request, tx, cx));
    cx.run_until_parked();
    assert_eq!(
        rx.try_recv().unwrap(),
        Some(CliReply::error(
            "this Lucerna is older than luc; update Lucerna"
        ))
    );
    assert!(kinds(cx).is_empty(), "nothing opens");
}

/// `luc` in `/` with no path while the app runs: as a click on the Dock
/// icon, the Welcome screen when no window is open, else the most
/// recently used window comes forward and nothing opens (spec M9).
#[cfg(unix)]
#[gpui::test]
fn cli_activate_reopens(cx: &mut TestAppContext) {
    use futures::channel::oneshot;
    use lucerna_workbench::cli_socket::{CliReply, CliRequest};

    routed(&[], cx);
    open_routed(Vec::new(), Target::Auto, cx);
    let activate = |cx: &mut TestAppContext| {
        let (tx, mut rx) = oneshot::channel();
        cx.update(|cx| router::answer_cli(CliRequest::activate(PathBuf::from("/")), tx, cx));
        cx.run_until_parked();
        assert_eq!(rx.try_recv().unwrap(), Some(CliReply::ok()));
    };
    activate(cx);
    assert_eq!(kinds(cx), vec![WindowKind::Welcome], "Welcome, not `/`");
    let (a, b) = (project(cx), project(cx));
    open_routed(vec![a.clone()], Target::NewWindow, cx);
    open_routed(vec![b.clone()], Target::NewWindow, cx);
    let (front, _) = routed_windows(cx)[0];
    activate(cx);
    assert_eq!(cx.update(|cx| cx.windows().len()), 3, "nothing new opens");
    assert_eq!(
        routed_windows(cx)[0].0,
        front,
        "the last used stays in front"
    );
}

/// The detach path of `luc <file>`: the started app opens the file from
/// its launch, then `luc` sends the same path; rule 1 only focuses it, no
/// second window or tab.
#[cfg(unix)]
#[gpui::test]
fn cli_same_file_as_launch_opens_once(cx: &mut TestAppContext) {
    use futures::channel::oneshot;
    use lucerna_workbench::cli_socket::{CliReply, CliRequest};

    routed(&[], cx);
    let a = project(cx);
    let file = a.join("src").join("x.rs");
    open_routed(vec![file.clone()], Target::Auto, cx);
    let (tx, mut rx) = oneshot::channel();
    cx.update(|cx| router::answer_cli(CliRequest::new(a.clone(), vec![file.clone()]), tx, cx));
    cx.run_until_parked();
    assert_eq!(rx.try_recv().unwrap(), Some(CliReply::ok()));
    assert_eq!(kinds(cx), vec![WindowKind::File(file.clone())]);
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    let handle = routed_workbench(&WindowKind::File(file.clone()), cx);
    let tabs = handle
        .read_with(cx, |wb, cx| {
            let groups = wb.editor_groups().read(cx);
            groups
                .model()
                .groups()
                .iter()
                .map(|g| g.tabs.len())
                .sum::<usize>()
        })
        .unwrap();
    assert_eq!(tabs, 1, "one tab on the file");
    assert_eq!(active_tab(handle, cx), Some(file));
}

/// A window on `root` keeping its trust in `data`, started untrusted.
fn trust_window(root: &Path, data: &Path, cx: &mut TestAppContext) -> WindowHandle<Workbench> {
    let (root, data) = (root.to_path_buf(), data.to_path_buf());
    let handle = cx.add_window(move |window, cx| Workbench::with_data_dir(root, data, window, cx));
    settle(handle, cx);
    handle
}

fn is_trusted(handle: WindowHandle<Workbench>, cx: &mut TestAppContext) -> bool {
    let servers = language_servers(handle, cx);
    cx.update(|cx| servers.read(cx).is_trusted())
}

/// Opens the trust modal in the window and clicks `Trust and Continue`,
/// with the parent checkbox when `parent`.
fn trust_in(handle: WindowHandle<Workbench>, parent: bool, cx: &mut TestAppContext) {
    handle
        .update(cx, |wb, window, cx| wb.show_trust_modal(window, cx))
        .unwrap();
    draw(handle, cx);
    let vcx = &mut VisualTestContext::from_window(handle.into(), cx);
    let mut click = |selector: &'static str| {
        let bounds = vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        vcx.simulate_click(bounds.center(), gpui::Modifiers::none());
        vcx.run_until_parked();
    };
    if parent {
        click("trust-parent-checkbox");
    }
    click("trust-continue");
}

#[gpui::test]
fn trust_in_a_reaches_b(cx: &mut TestAppContext) {
    init(cx);
    let data = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    let t = base.path().join("t");
    let sub = t.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let a = trust_window(&t, data.path(), cx);
    let b = trust_window(&sub, data.path(), cx);
    assert!(!is_trusted(a, cx) && !is_trusted(b, cx));
    trust_in(a, true, cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    settle(b, cx);
    assert!(is_trusted(a, cx));
    assert!(is_trusted(b, cx), "B is under the parent A trusted");
}

#[gpui::test]
fn trust_other_folder_leaves_b(cx: &mut TestAppContext) {
    init(cx);
    let data = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    let (one, two) = (base.path().join("one"), base.path().join("two"));
    std::fs::create_dir_all(&one).unwrap();
    std::fs::create_dir_all(&two).unwrap();
    let a = trust_window(&one, data.path(), cx);
    let b = trust_window(&two, data.path(), cx);
    trust_in(a, false, cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    settle(b, cx);
    assert!(is_trusted(a, cx));
    assert!(!is_trusted(b, cx), "B's folder was not trusted");
}

#[gpui::test]
fn clear_trust_in_a_restricts_b(cx: &mut TestAppContext) {
    init(cx);
    let data = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    let t = base.path().join("t");
    let sub = t.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let a = trust_window(&t, data.path(), cx);
    let b = trust_window(&sub, data.path(), cx);
    trust_in(a, true, cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    settle(b, cx);
    assert!(is_trusted(b, cx));
    a.update(cx, |wb, _, cx| wb.clear_trust(cx)).unwrap();
    cx.executor().advance_clock(Duration::from_millis(200));
    settle(b, cx);
    assert!(!is_trusted(b, cx), "B is back in Restricted Mode");
}
