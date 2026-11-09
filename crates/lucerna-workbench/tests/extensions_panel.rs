//! The Extensions panel (spec §5.1–§5.3): its status bar button, the
//! view commands, installs from a fake registry with their progress,
//! failures and theme toast, the update badge and the saved sidebar view.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use lucerna_extension::process::CancelToken;
use lucerna_extension::store::Store;
use lucerna_extension::test_support::{
    FakeEntry, FakeValidator, add_commit, fake_registry, fake_repo, fake_wasi_sdk, file_url,
    registry_update,
};
use lucerna_settings::parse_when;
use lucerna_workbench::extensions::ExtensionService;
use lucerna_workbench::extensions::panel_model::RowState;
use lucerna_workbench::{
    AppState, ConfigPaths, GroupState, GroupsState, SidebarView, Workbench, WorkspaceState,
    load_state, save_state, state_path, watch_config,
};
use tempfile::TempDir;

const THEME: &str = r#"{
  "name": "X", "author": "t",
  "themes": [{ "name": "X Dark", "appearance": "dark", "style": {} }]
}"#;

fn wasm_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../lucerna-syntax/tests/fixtures/wasm/json.wasm")
}

fn manifest(id: &str, version: &str) -> String {
    format!(
        "id = \"{id}\"\nname = \"Ext {id}\"\ndescription = \"About {id}\"\nversion = \"{version}\"\nschema_version = 1\n"
    )
}

/// A repository holding a theme extension `id` at `version`.
fn theme_repo(id: &str, version: &str) -> PathBuf {
    let manifest = manifest(id, version);
    fake_repo(&[("extension.toml", &manifest), ("themes/x.json", THEME)]).0
}

/// A project, a user config directory, and a store (with a fake wasi-sdk)
/// whose registry is fake and already fetched.
struct Env {
    project: TempDir,
    user: TempDir,
    tmp: TempDir,
    registry: PathBuf,
}

impl Env {
    fn new(entries: &[FakeEntry]) -> Env {
        let env = Env {
            project: TempDir::new().unwrap(),
            user: TempDir::new().unwrap(),
            tmp: TempDir::new().unwrap(),
            registry: fake_registry(entries),
        };
        fake_wasi_sdk(&env.tmp.path().join("sdk"), &wasm_fixture());
        env.store()
            .refresh_registry(&CancelToken::default())
            .unwrap();
        env
    }

    /// A registry with one theme extension, `x` (a registry cannot be
    /// empty).
    fn basic() -> Env {
        let repo = theme_repo("x", "0.1.0");
        Env::new(&[FakeEntry::new("x", "0.1.0", &repo)])
    }

    fn root(&self) -> PathBuf {
        self.tmp.path().join("extensions")
    }

    fn store(&self) -> Store {
        Store::open(self.root(), Arc::new(FakeValidator))
            .with_remote(file_url(&self.registry))
            .with_wasi_sdk(self.tmp.path().join("sdk"))
    }

    fn data_dir(&self) -> PathBuf {
        self.tmp.path().join("data")
    }
}

fn init(env: &Env, cx: &mut TestAppContext) {
    let store = env.store();
    cx.update(|cx| lucerna_workbench::app::set_app_state_with_store(store, cx));
    let paths = ConfigPaths::with_user_dir(env.user.path(), env.project.path());
    cx.update(|cx| watch_config(paths, cx));
}

fn workbench<'a>(
    env: &Env,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let (root, data) = (env.project.path().to_path_buf(), env.data_dir());
    cx.add_window_view(|window, cx| Workbench::with_data_dir(root, data, window, cx))
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        draw(cx);
        cx.run_until_parked();
    }
    draw(cx);
}

fn click(name: &'static str, cx: &mut VisualTestContext) {
    draw(cx);
    let bounds = cx
        .debug_bounds(name)
        .unwrap_or_else(|| panic!("no element {name}"));
    cx.simulate_click(bounds.center(), Modifiers::none());
}

fn shown(name: &'static str, cx: &mut VisualTestContext) -> bool {
    draw(cx);
    cx.debug_bounds(name).is_some()
}

fn sidebar(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> (bool, SidebarView) {
    wb.read_with(cx, |wb, _| (wb.sidebar_visible(), wb.active_sidebar()))
}

fn holds(wb: &Entity<Workbench>, when: &str, cx: &mut VisualTestContext) -> bool {
    let ctx = cx.update(|window, cx| wb.read(cx).context_keys(window, cx));
    parse_when(when).unwrap().eval(&ctx)
}

fn row_state(wb: &Entity<Workbench>, id: &str, cx: &mut VisualTestContext) -> Option<RowState> {
    cx.update(|_, cx| {
        let panel = wb.read(cx).extensions_panel().clone();
        panel.read(cx).rows().get(id).map(|row| row.state.clone())
    })
}

fn installed_ids(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        let panel = wb.read(cx).extensions_panel().clone();
        panel
            .read(cx)
            .rows()
            .installed
            .iter()
            .map(|row| row.id.clone())
            .collect()
    })
}

#[gpui::test]
fn status_bar_button_toggles_panel(cx: &mut TestAppContext) {
    let env = Env::basic();
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    settle(cx);
    // The third toggle, after Explorer and Search.
    let left = |name, cx: &mut VisualTestContext| cx.debug_bounds(name).unwrap().left();
    assert!(left("panel-toggle-search", cx) < left("panel-toggle-extensions", cx));

    click("panel-toggle-extensions", cx);
    assert_eq!(sidebar(&wb, cx), (true, SidebarView::Extensions));
    assert!(shown("extensions-panel", cx));
    assert!(holds(&wb, "sideBarFocus && extensionsViewletFocus", cx));
    assert!(!holds(
        &wb,
        "explorerViewletFocus || searchViewletFocus",
        cx
    ));

    click("panel-toggle-extensions", cx);
    assert!(
        !sidebar(&wb, cx).0,
        "the active panel's button hides the sidebar"
    );
    assert!(!holds(&wb, "extensionsViewletFocus", cx));
}

#[gpui::test]
fn cmd_shift_x_opens_panel(cx: &mut TestAppContext) {
    let env = Env::basic();
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    settle(cx);
    cx.simulate_keystrokes("cmd-shift-x");
    assert_eq!(sidebar(&wb, cx), (true, SidebarView::Extensions));
    assert!(holds(&wb, "extensionsViewletFocus", cx));
    let ran = wb.update_in(cx, |wb, window, cx| {
        wb.run_command("workbench.view.explorer", None, window, cx)
    });
    assert!(ran);
    assert_eq!(sidebar(&wb, cx).1, SidebarView::Explorer);
    let ran = wb.update_in(cx, |wb, window, cx| {
        wb.run_command("workbench.view.extensions", None, window, cx)
    });
    assert!(ran);
    assert_eq!(sidebar(&wb, cx), (true, SidebarView::Extensions));
}

#[gpui::test]
fn install_button_runs_and_shows_progress(cx: &mut TestAppContext) {
    let repo = theme_repo("x", "0.1.0");
    let env = Env::new(&[FakeEntry::new("x", "0.1.0", &repo)]);
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    cx.simulate_keystrokes("cmd-shift-x");
    settle(cx);
    assert_eq!(row_state(&wb, "x", cx), Some(RowState::NotInstalled));
    assert!(installed_ids(&wb, cx).is_empty());

    // Every state the row goes through.
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _observe = {
        let (seen, wb) = (seen.clone(), wb.clone());
        cx.update(|_, cx| {
            cx.observe_global::<ExtensionService>(move |cx| {
                let panel = wb.read(cx).extensions_panel().clone();
                if let Some(row) = panel.read(cx).rows().get("x") {
                    seen.borrow_mut().push(row.state.clone());
                }
            })
        })
    };
    click("extension-install-x", cx);
    settle(cx);

    assert!(
        seen.borrow()
            .iter()
            .any(|s| matches!(s, RowState::Installing(_))),
        "the row showed the install running: {:?}",
        seen.borrow()
    );
    assert_eq!(installed_ids(&wb, cx), ["x"]);
    assert_eq!(row_state(&wb, "x", cx), Some(RowState::Installed));
    assert!(env.root().join("installed/x/themes/x.json").is_file());
    assert!(shown("extension-uninstall-x", cx));
    assert!(!shown("extension-install-x", cx));
}

#[gpui::test]
fn failed_install_shows_error_and_log_button(cx: &mut TestAppContext) {
    // The registry says 0.2.0 but the manifest says 0.1.0.
    let repo = theme_repo("x", "0.1.0");
    let env = Env::new(&[FakeEntry::new("x", "0.2.0", &repo)]);
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    cx.simulate_keystrokes("cmd-shift-x");
    settle(cx);

    click("extension-install-x", cx);
    settle(cx);
    match row_state(&wb, "x", cx) {
        Some(RowState::Failed(error)) => assert!(error.contains("version mismatch"), "{error}"),
        other => panic!("expected a failure, got {other:?}"),
    }
    assert!(shown("extension-error-x", cx));
    assert!(shown("extension-install-x", cx), "it can be tried again");

    click("extension-show-log-x", cx);
    settle(cx);
    let tab = cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .active_tab()
            .cloned()
            .expect("the log is open")
    });
    assert_eq!(&*tab.path, env.root().join("logs/x.log"));
    assert!(!tab.pinned, "the log opens in a preview tab");
}

#[gpui::test]
fn theme_install_offers_set_theme_toast(cx: &mut TestAppContext) {
    let repo = theme_repo("x", "0.1.0");
    let env = Env::new(&[FakeEntry::new("x", "0.1.0", &repo)]);
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    cx.simulate_keystrokes("cmd-shift-x");
    settle(cx);
    assert!(!shown("toast", cx));

    click("extension-install-x", cx);
    settle(cx);
    let message = wb.read_with(cx, |wb, _| wb.toast().map(|t| t.message().to_string()));
    assert_eq!(message.as_deref(), Some("Use theme “X Dark”?"));
    assert!(shown("toast", cx));

    click("toast-action", cx);
    settle(cx);
    let settings = std::fs::read_to_string(env.user.path().join("settings.json")).unwrap();
    assert!(
        settings.contains(r#""workbench.colorTheme": "X Dark""#),
        "{settings}"
    );
    assert_eq!(
        cx.update(|_, cx| cx.global::<AppState>().theme.name.clone()),
        "X Dark"
    );
    assert!(!shown("toast", cx), "the toast closes");
}

#[gpui::test]
fn update_badge_count(cx: &mut TestAppContext) {
    let repo = theme_repo("x", "0.1.0");
    let env = Env::new(&[FakeEntry::new("x", "0.1.0", &repo)]);
    env.store()
        .install("x", &|_| {}, &CancelToken::default())
        .unwrap();

    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    settle(cx);
    assert!(!shown("panel-toggle-extensions-badge", cx));

    // A new version in the registry.
    add_commit(&repo, &[("extension.toml", &manifest("x", "0.2.0"))]);
    registry_update(&env.registry, "x", "0.2.0");
    cx.update(|_, cx| ExtensionService::refresh_registry(cx));
    settle(cx);

    assert_eq!(
        cx.update(|_, cx| ExtensionService::global(cx).updates().to_vec()),
        ["x"]
    );
    assert!(shown("panel-toggle-extensions-badge", cx));
    assert_eq!(
        wb.read_with(cx, |wb, cx| wb.extension_badge(cx)),
        Some("1".to_string())
    );
    cx.simulate_keystrokes("cmd-shift-x");
    settle(cx);
    assert_eq!(row_state(&wb, "x", cx), Some(RowState::UpdateAvailable));
    assert!(shown("extension-update-x", cx));
}

#[gpui::test]
fn panel_state_restored_from_workspace_state(cx: &mut TestAppContext) {
    let env = Env::basic();
    let path = state_path(&env.data_dir(), env.project.path());
    save_state(
        &path,
        &WorkspaceState {
            version: 1,
            sidebar_visible: true,
            sidebar_width: 300.,
            sidebar_view: "extensions".into(),
            groups: GroupsState {
                groups: vec![GroupState {
                    tabs: Vec::new(),
                    active: None,
                }],
                active_group: 0,
            },
            scroll: Default::default(),
            group_widths: Vec::new(),
        },
    )
    .unwrap();
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    assert_eq!(sidebar(&wb, cx), (true, SidebarView::Extensions));

    cx.simulate_keystrokes("cmd-shift-e");
    cx.simulate_keystrokes("cmd-shift-x");
    wb.update(cx, |wb, cx| wb.save_workspace_state(cx));
    assert_eq!(load_state(&path).unwrap().sidebar_view, "extensions");
}

#[gpui::test]
fn install_command_lists_registry_ids(cx: &mut TestAppContext) {
    let (a, b) = (theme_repo("alpha", "0.1.0"), theme_repo("beta", "0.1.0"));
    let env = Env::new(&[
        FakeEntry::new("alpha", "0.1.0", &a),
        FakeEntry::new("beta", "0.1.0", &b),
    ]);
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    settle(cx);
    wb.update_in(cx, |wb, window, cx| {
        wb.run_command(
            "workbench.extensions.action.installExtension",
            None,
            window,
            cx,
        )
    });
    settle(cx);
    let rows = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let qo = wb
                .read(cx)
                .quick_open()
                .expect("Quick Open is shown")
                .clone();
            qo.read(cx).row_texts()
        })
    };
    assert_eq!(rows(cx), ["alpha", "beta"]);
    cx.simulate_input("bet");
    settle(cx);
    assert_eq!(rows(cx), ["beta"]);
    cx.simulate_keystrokes("enter");
    settle(cx);
    assert!(wb.read_with(cx, |wb, _| wb.quick_open().is_none()));
    assert!(env.root().join("installed/beta").is_dir());
}

#[gpui::test]
fn registry_failure_offers_retry(cx: &mut TestAppContext) {
    let env = Env::basic();
    // A store whose registry was never fetched and cannot be.
    let store = Store::open(env.tmp.path().join("other"), Arc::new(FakeValidator))
        .with_remote(file_url(&env.tmp.path().join("missing")));
    cx.update(|cx| lucerna_workbench::app::set_app_state_with_store(store, cx));
    let (wb, cx) = workbench(&env, cx);
    cx.simulate_keystrokes("cmd-shift-x");
    settle(cx);
    assert!(shown("extensions-registry-error", cx));
    assert!(wb.read_with(cx, |_, cx| {
        ExtensionService::global(cx).registry_error().is_some()
    }));
    click("extensions-retry", cx);
    settle(cx);
    assert!(shown("extensions-registry-error", cx), "it fails again");
}
