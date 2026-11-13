//! The Extensions page in an editor tab (spec §5.1–§5.3): opening and
//! focusing the single tab, its filter buttons and cards, installs from a
//! fake registry with their progress, failures and theme toast, the update
//! badge, the saved tab, the install Quick Open and Clear Build Cache.

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
use lucerna_project::QUICK_OPEN_LIMIT;
use lucerna_settings::parse_when;
use lucerna_workbench::extensions::panel_model::RowState;
use lucerna_workbench::extensions::{ExtensionService, FILTER_LABELS, MAX_DESCRIBES};
use lucerna_workbench::{
    AppState, ConfigPaths, GroupState, GroupsState, OpenMode, SidebarView, TabItem, Workbench,
    WorkspaceState, save_state, state_path, watch_config,
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
/// whose registry is fake.
struct Env {
    project: TempDir,
    user: TempDir,
    tmp: TempDir,
    registry: PathBuf,
}

impl Env {
    /// The registry is not fetched yet.
    fn unfetched(entries: &[FakeEntry]) -> Env {
        let env = Env {
            project: TempDir::new().unwrap(),
            user: TempDir::new().unwrap(),
            tmp: TempDir::new().unwrap(),
            registry: fake_registry(entries),
        };
        fake_wasi_sdk(&env.tmp.path().join("sdk"), &wasm_fixture());
        env
    }

    /// The registry is already fetched.
    fn new(entries: &[FakeEntry]) -> Env {
        let env = Env::unfetched(entries);
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
            .allow_file_urls()
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

fn holds(wb: &Entity<Workbench>, when: &str, cx: &mut VisualTestContext) -> bool {
    let ctx = cx.update(|window, cx| wb.read(cx).context_keys(window, cx));
    parse_when(when).unwrap().eval(&ctx)
}

fn row_state(wb: &Entity<Workbench>, id: &str, cx: &mut VisualTestContext) -> Option<RowState> {
    cx.update(|_, cx| {
        let page = wb.read(cx).extensions_page().clone();
        page.read(cx).rows().get(id).map(|row| row.state.clone())
    })
}

fn installed_ids(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        let page = wb.read(cx).extensions_page().clone();
        page.read(cx)
            .rows()
            .installed
            .iter()
            .map(|row| row.id.clone())
            .collect()
    })
}

fn query(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        let page = wb.read(cx).extensions_page().clone();
        page.read(cx).query_input().read(cx).text().to_string()
    })
}

/// `(group, tab index)` of every Extensions tab.
fn extension_tabs(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<(usize, usize)> {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx).model().clone();
        groups
            .groups()
            .iter()
            .enumerate()
            .flat_map(|(g, group)| {
                group
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.item == TabItem::Extensions)
                    .map(move |(i, _)| (g, i))
            })
            .collect()
    })
}

/// The active group and whether its active tab is the Extensions tab.
fn active(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> (usize, bool) {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx).model().clone();
        let extensions = groups
            .active_tab()
            .is_some_and(|t| t.item == TabItem::Extensions);
        (groups.active_group(), extensions)
    })
}

fn open_extensions(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    wb.update_in(cx, |wb, window, cx| wb.open_extensions(window, cx));
    settle(cx);
}

#[gpui::test]
fn cmd_shift_x_opens_single_tab_and_focuses_search(cx: &mut TestAppContext) {
    let env = Env::basic();
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    settle(cx);
    assert!(extension_tabs(&wb, cx).is_empty());

    cx.simulate_keystrokes("cmd-shift-x");
    settle(cx);
    cx.simulate_keystrokes("cmd-shift-x");
    settle(cx);
    assert_eq!(extension_tabs(&wb, cx), [(0, 0)], "one Extensions tab");
    assert_eq!(active(&wb, cx), (0, true));
    assert!(shown("extensions-page", cx));
    let tab = cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .active_tab()
            .cloned()
            .unwrap()
    });
    assert!(tab.pinned, "never a preview");
    assert_eq!(tab.title(), "Extensions");
    assert!(shown("tab-icon-0-0", cx), "the tab shows the blocks icon");
    // The sidebar is not involved.
    assert_eq!(
        wb.read_with(cx, |wb, _| wb.active_sidebar()),
        SidebarView::Explorer
    );

    cx.simulate_input("abc");
    settle(cx);
    assert_eq!(query(&wb, cx), "abc", "typing goes into the search");

    // Cmd+W closes it.
    cx.simulate_keystrokes("cmd-w");
    settle(cx);
    assert!(extension_tabs(&wb, cx).is_empty());
    assert!(!shown("extensions-page", cx));
}

#[gpui::test]
fn status_bar_button_focuses_existing_tab_in_other_group(cx: &mut TestAppContext) {
    let env = Env::basic();
    std::fs::write(env.project.path().join("a.txt"), "a\n").unwrap();
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    settle(cx);
    // The third toggle, after Explorer and Search.
    let left = |name, cx: &mut VisualTestContext| cx.debug_bounds(name).unwrap().left();
    assert!(left("panel-toggle-search", cx) < left("panel-toggle-extensions", cx));

    click("panel-toggle-extensions", cx);
    settle(cx);
    assert_eq!(extension_tabs(&wb, cx), [(0, 0)]);
    assert!(wb.read_with(cx, |wb, _| wb.sidebar_visible()), "unchanged");

    // A file in a second group takes focus.
    let file = env.project.path().join("a.txt");
    wb.update_in(cx, |wb, window, cx| {
        wb.open_path(file.into(), OpenMode::ToSide, cx);
        wb.focus_editor_area(window, cx);
    });
    settle(cx);
    assert_eq!(active(&wb, cx), (1, false));
    assert!(!holds(&wb, "extensionsViewletFocus", cx));

    click("panel-toggle-extensions", cx);
    settle(cx);
    assert_eq!(extension_tabs(&wb, cx), [(0, 0)], "still one tab");
    assert_eq!(active(&wb, cx), (0, true));
    assert!(holds(&wb, "extensionsViewletFocus", cx));
    cx.simulate_input("x");
    settle(cx);
    assert_eq!(query(&wb, cx), "x", "the search has focus");
}

#[gpui::test]
fn filter_buttons_set_query_tokens(cx: &mut TestAppContext) {
    assert_eq!(
        FILTER_LABELS,
        [
            "All",
            "Installed",
            "Updates",
            "Themes",
            "Icon Themes",
            "Languages"
        ]
    );
    let env = Env::basic();
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    open_extensions(&wb, cx);

    click("extensions-filter-installed", cx);
    assert_eq!(query(&wb, cx), "@installed");
    click("extensions-filter-themes", cx);
    assert_eq!(query(&wb, cx), "@themes", "replaces the other filter");
    click("extensions-filter-icon-themes", cx);
    assert_eq!(query(&wb, cx), "@icons");
    click("extensions-filter-languages", cx);
    assert_eq!(query(&wb, cx), "@languages");
    click("extensions-filter-updates", cx);
    assert_eq!(query(&wb, cx), "@updates");
    click("extensions-filter-all", cx);
    assert_eq!(query(&wb, cx), "");

    // Free text stays; a typed token still filters.
    cx.simulate_input("foo");
    click("extensions-filter-installed", cx);
    assert_eq!(query(&wb, cx), "@installed foo");
    settle(cx);
    assert_eq!(row_state(&wb, "x", cx), None, "@installed hides x");
    click("extensions-filter-all", cx);
    assert_eq!(query(&wb, cx), "foo");
    for _ in 0..3 {
        cx.simulate_keystrokes("backspace");
    }
    cx.simulate_input("@installed");
    settle(cx);
    assert_eq!(row_state(&wb, "x", cx), None);
    assert!(shown("extensions-filter-installed-active", cx));
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
    // The card's description was fetched when it was laid out.
    let description = cx.update(|_, cx| {
        let page = wb.read(cx).extensions_page().clone();
        page.read(cx).rows().get("x").unwrap().description.clone()
    });
    assert_eq!(description.as_deref(), Some("About x"));
    assert!(shown("extension-card-x", cx));

    // Every state the row goes through.
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _observe = {
        let (seen, wb) = (seen.clone(), wb.clone());
        cx.update(|_, cx| {
            cx.observe_global::<ExtensionService>(move |cx| {
                let page = wb.read(cx).extensions_page().clone();
                if let Some(row) = page.read(cx).rows().get("x") {
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
        "the card showed the install running: {:?}",
        seen.borrow()
    );
    assert_eq!(installed_ids(&wb, cx), ["x"]);
    assert_eq!(row_state(&wb, "x", cx), Some(RowState::Installed));
    assert!(env.root().join("installed/x/themes/x.json").is_file());
    assert!(shown("extension-uninstall-x", cx));
    assert!(!shown("extension-install-x", cx));
}

#[gpui::test]
fn install_during_describe_is_queued(cx: &mut TestAppContext) {
    let repo = theme_repo("x", "0.1.0");
    let env = Env::new(&[FakeEntry::new("x", "0.1.0", &repo)]);
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    open_extensions(&wb, cx);

    // Both start before any background work runs.
    cx.update(|_, cx| {
        ExtensionService::describe("x".into(), cx);
        ExtensionService::install("x".into(), cx);
    });
    assert!(cx.update(|_, cx| ExtensionService::global(cx).is_installing("x")));
    assert!(
        matches!(row_state(&wb, "x", cx), Some(RowState::Installing(_))),
        "the card shows the install at once"
    );
    assert!(shown("extension-progress-x", cx));
    assert!(shown("extension-cancel-x", cx));

    settle(cx);
    assert_eq!(installed_ids(&wb, cx), ["x"], "the queued install ran");
    assert_eq!(row_state(&wb, "x", cx), Some(RowState::Installed));
    assert_eq!(
        cx.update(|_, cx| ExtensionService::global(cx)
            .last_error("x")
            .map(String::from)),
        None
    );
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

    // "Show Log" sits under "Install", flush with its right edge.
    draw(cx);
    let install = cx.debug_bounds("extension-install-x").unwrap();
    let show_log = cx.debug_bounds("extension-show-log-x").unwrap();
    assert!(
        (install.right() - show_log.right()).abs() < gpui::px(1.),
        "Show Log right edge {:?} should match Install right edge {:?}",
        show_log.right(),
        install.right()
    );
    assert!(show_log.top() >= install.bottom(), "below Install");

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
    assert_eq!(
        tab.path().map(|p| p.to_path_buf()),
        Some(env.root().join("logs/x.log"))
    );
    assert!(!tab.pinned, "the log opens in a preview tab");
    assert_eq!(extension_tabs(&wb, cx), [(0, 0)], "the page stays open");
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
fn extensions_tab_restored_from_workspace_state(cx: &mut TestAppContext) {
    let env = Env::basic();
    for name in ["a.txt", "b.txt"] {
        std::fs::write(env.project.path().join(name), "text\n").unwrap();
    }
    init(&env, cx);
    let (a, b) = (
        env.project.path().join("a.txt"),
        env.project.path().join("b.txt"),
    );
    {
        let (wb, cx) = workbench(&env, cx);
        wb.update_in(cx, |wb, window, cx| {
            wb.open_path(a.clone().into(), OpenMode::Pinned, cx);
            wb.open_path(b.clone().into(), OpenMode::ToSide, cx);
            wb.open_extensions(window, cx);
        });
        settle(cx);
        // Group 1 holds b.txt then the page, which is active.
        assert_eq!(extension_tabs(&wb, cx), [(1, 1)]);
        wb.update(cx, |wb, cx| wb.save_workspace_state(cx));
    }

    let (wb, cx) = workbench(&env, cx);
    settle(cx);
    assert_eq!(extension_tabs(&wb, cx), [(1, 1)], "same group and position");
    assert_eq!(active(&wb, cx), (1, true));
    assert!(shown("extensions-page", cx));
    let paths: Vec<Vec<Option<PathBuf>>> = cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .groups()
            .iter()
            .map(|g| {
                g.tabs
                    .iter()
                    .map(|t| t.path().map(|p| p.to_path_buf()))
                    .collect()
            })
            .collect()
    });
    assert_eq!(paths, [vec![Some(a)], vec![Some(b), None]]);
}

#[gpui::test]
fn old_sidebar_state_extensions_restores_explorer(cx: &mut TestAppContext) {
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
            search: Default::default(),
            panel: Default::default(),
        },
    )
    .unwrap();
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    settle(cx);
    assert_eq!(
        wb.read_with(cx, |wb, _| (wb.sidebar_visible(), wb.active_sidebar())),
        (true, SidebarView::Explorer)
    );
    assert!(extension_tabs(&wb, cx).is_empty());
}

#[gpui::test]
fn extensions_viewlet_focus_when_tab_focused(cx: &mut TestAppContext) {
    let env = Env::basic();
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    settle(cx);
    assert!(!holds(&wb, "extensionsViewletFocus", cx));

    cx.simulate_keystrokes("cmd-shift-x");
    settle(cx);
    assert!(holds(&wb, "extensionsViewletFocus && textInputFocus", cx));
    assert!(!holds(&wb, "sideBarFocus", cx));

    // Focus moves to the sidebar: the tab stays open but loses focus.
    let ran = wb.update_in(cx, |wb, window, cx| {
        wb.run_command("workbench.view.explorer", None, window, cx)
    });
    assert!(ran);
    settle(cx);
    assert_eq!(extension_tabs(&wb, cx), [(0, 0)]);
    assert!(!holds(&wb, "extensionsViewletFocus", cx));

    let ran = wb.update_in(cx, |wb, window, cx| {
        wb.run_command("workbench.view.extensions", None, window, cx)
    });
    assert!(ran);
    settle(cx);
    assert!(holds(&wb, "extensionsViewletFocus", cx));
}

#[gpui::test]
fn install_quick_open_is_capped_and_shows_loading(cx: &mut TestAppContext) {
    let repo = theme_repo("x", "0.1.0");
    let ids: Vec<String> = (0..QUICK_OPEN_LIMIT + 5)
        .map(|i| format!("ext{i:03}"))
        .collect();
    let entries: Vec<FakeEntry> = ids
        .iter()
        .map(|id| FakeEntry::new(id, "0.1.0", &repo))
        .collect();
    let env = Env::unfetched(&entries);
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    // Before `registry.json` exists or was read.
    wb.update_in(cx, |wb, window, cx| {
        wb.run_command(
            "workbench.extensions.action.installExtension",
            None,
            window,
            cx,
        )
    });
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
    assert_eq!(rows(cx), ["Loading extension list…"]);

    // The command fetched the registry; the list fills in.
    settle(cx);
    let shown_rows = rows(cx);
    assert_eq!(shown_rows.len(), QUICK_OPEN_LIMIT, "capped");
    assert_eq!(shown_rows[0], "ext000");
    cx.simulate_input("ext054");
    settle(cx);
    assert_eq!(rows(cx), ["ext054"], "all ids are searched");
    cx.simulate_keystrokes("enter");
    settle(cx);
    assert!(wb.read_with(cx, |wb, _| wb.quick_open().is_none()));
    assert!(env.root().join("installed/ext054").is_dir());
}

#[gpui::test]
fn clear_build_cache_refused_while_installing(cx: &mut TestAppContext) {
    let (x, y) = (theme_repo("x", "0.1.0"), theme_repo("y", "0.1.0"));
    let env = Env::new(&[
        FakeEntry::new("x", "0.1.0", &x),
        FakeEntry::new("y", "0.1.0", &y),
    ]);
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    settle(cx);
    cx.update(|_, cx| ExtensionService::install("x".into(), cx));
    settle(cx);
    wb.update(cx, |wb, cx| wb.dismiss_toast(cx));
    assert!(env.root().join("work/x").is_dir());

    let clear = |wb: &Entity<Workbench>, cx: &mut VisualTestContext| {
        wb.update_in(cx, |wb, window, cx| {
            wb.run_command("lucerna.extensions.clearBuildCache", None, window, cx)
        })
    };
    cx.update(|_, cx| ExtensionService::install("y".into(), cx));
    assert!(clear(&wb, cx));
    let message = wb.read_with(cx, |wb, _| wb.toast().map(|t| t.message().to_string()));
    assert_eq!(
        message.as_deref(),
        Some("Can't clear the build cache while an extension is installing.")
    );
    assert!(shown("toast", cx));
    settle(cx);
    assert!(env.root().join("work/x").is_dir(), "nothing was cleared");
    assert!(env.root().join("installed/y").is_dir());

    wb.update(cx, |wb, cx| wb.dismiss_toast(cx));
    assert!(clear(&wb, cx));
    settle(cx);
    assert!(!env.root().join("work").exists(), "cleared when idle");
}

#[gpui::test]
fn failed_describe_is_retried_after_a_minute(cx: &mut TestAppContext) {
    // The repository has no extension.toml, so describing it fails.
    let repo = fake_repo(&[("README.md", "nothing\n")]).0;
    let env = Env::new(&[FakeEntry::new("x", "0.1.0", &repo)]);
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    let describing = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| ExtensionService::global(cx).is_describing("x"))
    };
    wb.update_in(cx, |wb, window, cx| wb.open_extensions(window, cx));
    draw(cx);
    assert!(describing(cx), "asked when the card was laid out");
    settle(cx);
    assert!(!describing(cx));
    assert!(shown("extension-card-x", cx));
    let description = cx.update(|_, cx| {
        let page = wb.read(cx).extensions_page().clone();
        page.read(cx).rows().get("x").unwrap().description.clone()
    });
    assert_eq!(description, None);
    draw(cx);
    assert!(!describing(cx), "not again within a minute");

    cx.executor()
        .advance_clock(std::time::Duration::from_secs(61));
    draw(cx);
    assert!(describing(cx), "asked again after a minute");
    settle(cx);
}

/// A registry of `n` uninstalled extensions `e00`, `e01`, … sharing one
/// repository.
fn many(n: usize) -> Env {
    let repo = theme_repo("x", "0.1.0");
    let entries: Vec<FakeEntry> = (0..n)
        .map(|i| FakeEntry::new(&format!("e{i:02}"), "0.1.0", &repo))
        .collect();
    Env::new(&entries)
}

/// The ids of the cards whose list items (card and spacing) are drawn
/// inside the list's bounds, top first.
fn cards_on_screen(n: usize, cx: &mut VisualTestContext) -> Vec<String> {
    let list = cx
        .debug_bounds("extensions-list")
        .expect("the list is drawn");
    (0..n)
        .map(|i| format!("e{i:02}"))
        .filter(|id| {
            let selector: &'static str = Box::leak(format!("extension-item-{id}").into_boxed_str());
            cx.debug_bounds(selector)
                .is_some_and(|b| b.top() < list.bottom() && b.bottom() > list.top())
        })
        .collect()
}

/// `(running, queued)` describes.
fn describes(cx: &mut VisualTestContext, n: usize) -> (Vec<String>, Vec<String>) {
    cx.update(|_, cx| {
        let service = ExtensionService::global(cx);
        let running = (0..n)
            .map(|i| format!("e{i:02}"))
            .filter(|id| service.is_describing(id))
            .collect();
        let queued = service.queued_describes().map(String::from).collect();
        (running, queued)
    })
}

/// Opens the page in a 1000×600 window without letting background work
/// run, and draws it.
fn open_small<'a>(
    env: &Env,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    init(env, cx);
    let (wb, cx) = workbench(env, cx);
    cx.simulate_resize(gpui::size(gpui::px(1000.), gpui::px(600.)));
    settle(cx);
    wb.update_in(cx, |wb, window, cx| wb.open_extensions(window, cx));
    draw(cx);
    (wb, cx)
}

#[gpui::test]
fn describes_are_capped_to_cards_on_screen(cx: &mut TestAppContext) {
    const N: usize = 40;
    let env = many(N);
    let (_wb, cx) = open_small(&env, cx);
    let on_screen = cards_on_screen(N, cx);
    assert!(
        on_screen.len() > MAX_DESCRIBES && on_screen.len() < N,
        "{on_screen:?}"
    );
    let (running, queued) = describes(cx, N);
    assert_eq!(
        running.len(),
        MAX_DESCRIBES,
        "at most {MAX_DESCRIBES} at once"
    );
    let wanted: Vec<String> = running.iter().chain(&queued).cloned().collect();
    assert_eq!(wanted, on_screen, "the cards on screen, top first");
    // The card just below the screen is laid out as overdraw, not asked for.
    let below = format!("e{:02}", on_screen.len());
    assert!(!wanted.contains(&below), "{below} is off screen");

    settle(cx);
    let (running, queued) = describes(cx, N);
    assert!(running.is_empty() && queued.is_empty());
    let described = cx.update(|_, cx| {
        let registry = ExtensionService::global(cx).registry().cloned().unwrap();
        registry
            .entries
            .iter()
            .filter(|e| e.description.is_some())
            .map(|e| e.id.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(described, on_screen, "every card on screen was described");
}

#[gpui::test]
fn describe_requests_for_cards_scrolled_away_are_dropped(cx: &mut TestAppContext) {
    const N: usize = 40;
    let env = many(N);
    let (_wb, cx) = open_small(&env, cx);
    let (running, queued) = describes(cx, N);
    assert_eq!(running.len(), MAX_DESCRIBES);
    assert!(!queued.is_empty());

    // Scroll to the bottom without letting the describes finish.
    let list = cx.debug_bounds("extensions-list").unwrap();
    // The list only scrolls as far as it has measured, so in steps.
    for _ in 0..40 {
        cx.update(|window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::Scroll(gpui::ScrollEvent {
                    position: list.center(),
                    delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-400.))),
                    ..Default::default()
                }),
                cx,
            );
        });
        draw(cx);
    }
    let on_screen = cards_on_screen(N, cx);
    assert!(
        on_screen.contains(&format!("e{:02}", N - 1)),
        "{on_screen:?}"
    );
    let (still_running, now_queued) = describes(cx, N);
    assert_eq!(still_running, running, "running describes go on");
    assert!(
        queued.iter().all(|id| !now_queued.contains(id)),
        "requests for cards that scrolled away are dropped: {now_queued:?}"
    );
    assert_eq!(
        now_queued, on_screen,
        "the cards now on screen wait instead"
    );
    settle(cx);
}

#[gpui::test]
fn cancelling_install_queued_behind_describe_keeps_the_describe(cx: &mut TestAppContext) {
    let repo = theme_repo("x", "0.1.0");
    let env = Env::new(&[FakeEntry::new("x", "0.1.0", &repo)]);
    init(&env, cx);
    let (wb, cx) = workbench(&env, cx);
    settle(cx);
    cx.update(|_, cx| {
        ExtensionService::describe("x".into(), cx);
        ExtensionService::install("x".into(), cx);
        ExtensionService::cancel("x".into(), cx);
    });
    let (describing, installing) = cx.update(|_, cx| {
        let s = ExtensionService::global(cx);
        (s.is_describing("x"), s.is_installing("x"))
    });
    assert!(describing && !installing);
    settle(cx);
    assert!(!env.root().join("installed/x").exists(), "not installed");
    let description = cx.update(|_, cx| {
        let registry = ExtensionService::global(cx).registry().cloned().unwrap();
        registry.entries[0].description.clone()
    });
    assert_eq!(
        description.as_deref(),
        Some("About x"),
        "the describe finished"
    );
    assert!(wb.read_with(cx, |wb, _| wb.toast().is_none()));
}

#[gpui::test]
fn registry_failure_offers_retry_and_keeps_installed_cards(cx: &mut TestAppContext) {
    let env = Env::basic();
    env.store()
        .install("x", &|_| {}, &CancelToken::default())
        .unwrap();
    // No registry (neither registry.json nor the clone, whose origin would
    // still work), and a remote that cannot be fetched.
    std::fs::remove_file(env.root().join("registry.json")).unwrap();
    std::fs::remove_dir_all(env.root().join("registry")).unwrap();
    let store = Store::open(env.root(), Arc::new(FakeValidator))
        .allow_file_urls()
        .with_remote(file_url(&env.tmp.path().join("missing")));
    cx.update(|cx| lucerna_workbench::app::set_app_state_with_store(store, cx));
    let (wb, cx) = workbench(&env, cx);
    cx.simulate_keystrokes("cmd-shift-x");
    settle(cx);
    assert!(shown("extensions-registry-error", cx));
    assert!(wb.read_with(cx, |_, cx| {
        ExtensionService::global(cx).registry_error().is_some()
    }));
    assert_eq!(installed_ids(&wb, cx), ["x"]);
    assert!(shown("extension-card-x", cx), "installed cards still show");
    click("extensions-retry", cx);
    settle(cx);
    assert!(shown("extensions-registry-error", cx), "it fails again");
    assert!(shown("extension-card-x", cx));
}
