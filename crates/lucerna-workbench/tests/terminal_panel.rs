//! The bottom / right panel that hosts terminals: layout in both
//! positions, the sash, maximize, saved state, the status bar button and
//! the tab bar.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    BorrowAppContext, Bounds, Entity, Modifiers, MouseButton, Pixels, TestAppContext,
    VisualTestContext, point, px,
};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{
    AppState, PanelPosition, PanelState, Workbench, WorkspaceState, load_state, save_state, ui,
};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext) {
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

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn bounds(name: &'static str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds(name)
        .unwrap_or_else(|| panic!("no element {name}"))
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(wb.run_command(command, None, window, cx), "{command}");
        })
    });
    draw(cx);
}

fn panel(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> PanelState {
    wb.read_with(cx, |wb, _| wb.panel_state().clone())
}

fn tab_ids(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<u64> {
    cx.update(|_, cx| {
        let panel = wb.read(cx).terminal_panel().read(cx);
        panel.tabs().iter().map(|t| t.id).collect()
    })
}

fn add_tab(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> u64 {
    let id = cx.update(|_, cx| {
        let panel = wb.read(cx).terminal_panel().clone();
        panel.update(cx, |p, cx| p.add_tab(None, cx))
    });
    draw(cx);
    id
}

#[gpui::test]
fn toggle_panel_shows_and_hides(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    draw(cx);
    assert!(cx.debug_bounds("terminal-panel").is_none());
    assert!(!panel(&wb, cx).visible);

    run(&wb, "workbench.action.togglePanel", cx);
    assert!(cx.debug_bounds("terminal-panel").is_some());
    assert!(panel(&wb, cx).visible);
    // Opening with no terminal creates one.
    assert_eq!(tab_ids(&wb, cx).len(), 1);
    assert!(cx.debug_bounds("terminal-tab-0").is_some());

    run(&wb, "workbench.action.togglePanel", cx);
    assert!(cx.debug_bounds("terminal-panel").is_none());
    // Hiding keeps the terminals.
    assert_eq!(tab_ids(&wb, cx).len(), 1);
    run(&wb, "workbench.action.togglePanel", cx);
    assert_eq!(tab_ids(&wb, cx).len(), 1);
}

#[gpui::test]
fn panel_focus_is_outside_the_editor_area(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    run(&wb, "workbench.action.togglePanel", cx);
    let (in_panel, in_editor) = cx.update(|window, cx| {
        let wb = wb.read(cx);
        let keys = wb.context_keys(window, cx);
        let panel_focus = wb.terminal_panel().read(cx).focus_handle().clone();
        (
            panel_focus.contains_focused(window, cx),
            lucerna_settings::parse_when("editorFocus")
                .unwrap()
                .eval(&keys),
        )
    });
    assert!(in_panel, "showing the panel focuses it");
    assert!(!in_editor, "the panel is not part of the editor area");
    // Hiding it gives the focus back to the editor area.
    run(&wb, "workbench.action.togglePanel", cx);
    let in_panel = cx.update(|window, cx| {
        let panel_focus = wb.read(cx).terminal_panel().read(cx).focus_handle().clone();
        panel_focus.contains_focused(window, cx)
    });
    assert!(!in_panel);
}

#[gpui::test]
fn bottom_panel_spans_editor_area_only(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    draw(cx);
    let area_before = bounds("editor-area", cx);
    run(&wb, "workbench.action.togglePanel", cx);
    assert_eq!(panel(&wb, cx).position, PanelPosition::Bottom);
    let sidebar = bounds("sidebar", cx);
    let p = bounds("terminal-panel", cx);
    let area = bounds("editor-area", cx);
    let status = bounds("status-bar", cx);
    assert_eq!(p.left(), area.left());
    assert_eq!(p.left(), sidebar.right());
    assert_eq!(p.right(), area_before.right());
    assert_eq!(p.size.height, px(320.));
    assert_eq!(p.bottom(), status.top());
    assert_eq!(area.bottom(), p.top());
    assert_eq!(area.top(), area_before.top());
}

#[gpui::test]
fn right_panel_full_height(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    run(&wb, "workbench.action.togglePanel", cx);
    run(&wb, "workbench.action.positionPanelRight", cx);
    assert_eq!(panel(&wb, cx).position, PanelPosition::Right);
    let p = bounds("terminal-panel", cx);
    let title = bounds("title-bar", cx);
    let status = bounds("status-bar", cx);
    let area = bounds("editor-area", cx);
    assert_eq!(p.top(), title.bottom());
    assert_eq!(p.bottom(), status.top());
    assert_eq!(p.size.width, px(640.));
    assert_eq!(p.right(), status.right());
    assert_eq!(area.right(), p.left());
    assert_eq!(area.bottom(), status.top());

    run(&wb, "workbench.action.positionPanelBottom", cx);
    assert_eq!(bounds("terminal-panel", cx).size.height, px(320.));
}

#[gpui::test]
fn sash_drag_resizes_and_clamps(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    run(&wb, "workbench.action.togglePanel", cx);
    let sash = bounds("terminal-panel-sash", cx).center();
    let p = bounds("terminal-panel", cx);
    let none = Modifiers::none();
    cx.simulate_mouse_down(sash, MouseButton::Left, none);
    cx.simulate_mouse_move(sash - point(px(0.), px(10.)), MouseButton::Left, none);
    let to = point(sash.x, p.bottom() - px(400.));
    cx.simulate_mouse_move(to, MouseButton::Left, none);
    draw(cx);
    assert_eq!(panel(&wb, cx).bottom_height, 400.);
    assert_eq!(bounds("terminal-panel", cx).size.height, px(400.));

    // Up 2000 px: the editor area keeps at least 120 px.
    let far = point(sash.x, sash.y - px(2000.));
    cx.simulate_mouse_move(far, MouseButton::Left, none);
    cx.simulate_mouse_up(far, MouseButton::Left, none);
    draw(cx);
    assert!(bounds("editor-area", cx).size.height >= px(120.));
    assert_eq!(
        bounds("editor-area", cx).bottom(),
        bounds("terminal-panel", cx).top()
    );

    // Down past the bottom: the panel keeps its minimum.
    let sash = bounds("terminal-panel-sash", cx).center();
    cx.simulate_mouse_down(sash, MouseButton::Left, none);
    cx.simulate_mouse_move(sash + point(px(0.), px(10.)), MouseButton::Left, none);
    let low = point(sash.x, bounds("status-bar", cx).bottom());
    cx.simulate_mouse_move(low, MouseButton::Left, none);
    cx.simulate_mouse_up(low, MouseButton::Left, none);
    draw(cx);
    let height = bounds("terminal-panel", cx).size.height;
    assert!(height >= px(ui::TAB_HEIGHT + 4. * 16.), "{height:?}");
}

#[gpui::test]
fn right_sash_drag_resizes(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    run(&wb, "workbench.action.togglePanel", cx);
    run(&wb, "workbench.action.positionPanelRight", cx);
    let sash = bounds("terminal-panel-sash", cx).center();
    let p = bounds("terminal-panel", cx);
    let none = Modifiers::none();
    cx.simulate_mouse_down(sash, MouseButton::Left, none);
    cx.simulate_mouse_move(sash + point(px(10.), px(0.)), MouseButton::Left, none);
    let to = point(p.right() - px(500.), sash.y);
    cx.simulate_mouse_move(to, MouseButton::Left, none);
    cx.simulate_mouse_up(to, MouseButton::Left, none);
    draw(cx);
    assert_eq!(panel(&wb, cx).right_width, 500.);
    // The bottom height is kept separately.
    assert_eq!(panel(&wb, cx).bottom_height, 320.);
}

#[gpui::test]
fn maximize_fills_editor_area_and_restores(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    draw(cx);
    let area = bounds("editor-area", cx);
    for position in ["Bottom", "Right"] {
        run(
            &wb,
            &format!("workbench.action.positionPanel{position}"),
            cx,
        );
        if !panel(&wb, cx).visible {
            run(&wb, "workbench.action.togglePanel", cx);
        }
        let normal = bounds("terminal-panel", cx);
        run(&wb, "workbench.action.toggleMaximizedPanel", cx);
        assert!(panel(&wb, cx).maximized);
        assert_eq!(bounds("terminal-panel", cx), area, "{position}");
        assert!(cx.debug_bounds("terminal-panel-sash").is_none());
        run(&wb, "workbench.action.toggleMaximizedPanel", cx);
        assert!(!panel(&wb, cx).maximized);
        assert_eq!(bounds("terminal-panel", cx), normal, "{position}");
    }
    // The ⤢ button does the same.
    let b = bounds("terminal-maximize", cx);
    cx.simulate_click(b.center(), Modifiers::none());
    draw(cx);
    assert!(panel(&wb, cx).maximized);
    // The × button hides the panel and keeps the terminal.
    let b = bounds("terminal-hide", cx);
    cx.simulate_click(b.center(), Modifiers::none());
    draw(cx);
    assert!(!panel(&wb, cx).visible);
    assert_eq!(tab_ids(&wb, cx).len(), 1);
}

#[gpui::test]
fn maximize_shows_a_hidden_panel(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    run(&wb, "workbench.action.toggleMaximizedPanel", cx);
    let state = panel(&wb, cx);
    assert!(state.visible && state.maximized);
    assert_eq!(tab_ids(&wb, cx).len(), 1);
}

struct Project {
    root: TempDir,
    data: TempDir,
}

impl Project {
    fn new() -> Self {
        Project {
            root: TempDir::new().unwrap(),
            data: TempDir::new().unwrap(),
        }
    }

    fn state_file(&self) -> PathBuf {
        lucerna_workbench::state_path(self.data.path(), self.root.path())
    }

    fn open<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        let (root, data) = (
            self.root.path().to_path_buf(),
            self.data.path().to_path_buf(),
        );
        cx.add_window_view(|window, cx| Workbench::with_data_dir(root, data, window, cx))
    }
}

#[gpui::test]
fn state_round_trips(cx: &mut TestAppContext) {
    init(cx);
    let project = Project::new();
    {
        let (wb, cx) = project.open(cx);
        run(&wb, "workbench.action.togglePanel", cx);
        run(&wb, "workbench.action.positionPanelRight", cx);
        wb.update(cx, |wb, cx| wb.save_workspace_state(cx));
    }
    let saved = load_state(&project.state_file()).expect("saved").panel;
    assert_eq!(
        saved,
        PanelState {
            visible: true,
            position: PanelPosition::Right,
            bottom_height: 320.,
            right_width: 640.,
            maximized: false,
        }
    );

    // Restored: shown on the right, with a fresh terminal.
    let mut state = load_state(&project.state_file()).unwrap();
    state.panel.right_width = 500.;
    save_state(&project.state_file(), &state).unwrap();
    let (wb, cx) = project.open(cx);
    draw(cx);
    assert_eq!(panel(&wb, cx).right_width, 500.);
    assert_eq!(bounds("terminal-panel", cx).size.width, px(500.));
    assert_eq!(tab_ids(&wb, cx).len(), 1);
}

#[test]
fn old_state_without_panel_loads_default() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.json");
    let state: WorkspaceState = serde_json::from_value(serde_json::json!({
        "version": 1,
        "sidebar_visible": true,
        "sidebar_width": 260.0,
        "sidebar_view": "explorer",
        "groups": { "groups": [], "active_group": 0 },
        "scroll": {},
    }))
    .expect("an old file parses");
    save_state(&path, &state).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(value.as_object_mut().unwrap().remove("panel").is_some());
    fs::write(&path, value.to_string()).unwrap();
    let loaded = load_state(Path::new(&path)).expect("still loads");
    assert_eq!(loaded.panel, PanelState::default());
    assert_eq!(
        PanelState::default(),
        PanelState {
            visible: false,
            position: PanelPosition::Bottom,
            bottom_height: 320.,
            right_width: 640.,
            maximized: false,
        }
    );
}

#[gpui::test]
fn default_location_setting_places_a_new_panel(cx: &mut TestAppContext) {
    init(cx);
    cx.update(|cx| {
        cx.update_global::<AppState, _>(|state, _| {
            state
                .settings
                .set_layer(
                    Layer::User,
                    r#"{ "workbench.panel.defaultLocation": "right" }"#,
                )
                .unwrap();
        });
    });
    let (wb, cx) = open(cx);
    assert_eq!(panel(&wb, cx).position, PanelPosition::Right);
}

#[gpui::test]
fn status_button_toggles_panel(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    draw(cx);
    // After Extensions, in the status bar.
    let button = bounds("panel-toggle-terminal", cx);
    let extensions = bounds("panel-toggle-extensions", cx);
    assert!(bounds("status-bar", cx).contains(&button.center()));
    assert!(button.left() > extensions.left());
    assert!(cx.debug_bounds("panel-toggle-terminal-active").is_none());

    cx.simulate_click(button.center(), Modifiers::none());
    draw(cx);
    assert!(panel(&wb, cx).visible);
    assert!(cx.debug_bounds("panel-toggle-terminal-active").is_some());

    let button = bounds("panel-toggle-terminal", cx);
    cx.simulate_click(button.center(), Modifiers::none());
    draw(cx);
    assert!(!panel(&wb, cx).visible);
    assert!(cx.debug_bounds("panel-toggle-terminal-active").is_none());
}

#[gpui::test]
fn tab_bar_matches_editor_tab_height(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    run(&wb, "workbench.action.togglePanel", cx);
    assert_eq!(
        bounds("terminal-tab-bar", cx).size.height,
        px(ui::TAB_HEIGHT)
    );
    assert_eq!(bounds("terminal-tab-0", cx).size.height, px(ui::TAB_HEIGHT));
    // The active tab shows its close button; the bar its three buttons.
    assert!(cx.debug_bounds("terminal-tab-close-0").is_some());
    for name in ["terminal-new", "terminal-maximize", "terminal-hide"] {
        assert!(bounds("terminal-tab-bar", cx).contains(&bounds(name, cx).center()));
    }
    // ＋ adds a terminal and makes it active.
    let b = bounds("terminal-new", cx);
    cx.simulate_click(b.center(), Modifiers::none());
    draw(cx);
    assert_eq!(tab_ids(&wb, cx).len(), 2);
    assert_eq!(
        wb.read_with(cx, |wb, cx| wb.terminal_panel().read(cx).active()),
        Some(1)
    );
    // Clicking a tab activates it.
    let b = bounds("terminal-tab-0", cx);
    cx.simulate_click(b.center(), Modifiers::none());
    assert_eq!(
        wb.read_with(cx, |wb, cx| wb.terminal_panel().read(cx).active()),
        Some(0)
    );
}

#[gpui::test]
fn closing_last_tab_hides_panel(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    run(&wb, "workbench.action.togglePanel", cx);
    add_tab(&wb, cx);
    // Closing one of two keeps the panel.
    let b = bounds("terminal-tab-close-1", cx);
    cx.simulate_click(b.center(), Modifiers::none());
    draw(cx);
    assert_eq!(tab_ids(&wb, cx).len(), 1);
    assert!(panel(&wb, cx).visible);
    // Closing the last hides it.
    let b = bounds("terminal-tab-close-0", cx);
    cx.simulate_click(b.center(), Modifiers::none());
    draw(cx);
    assert!(tab_ids(&wb, cx).is_empty());
    assert!(!panel(&wb, cx).visible);
    assert!(cx.debug_bounds("terminal-panel").is_none());
    // Showing it again starts a new terminal.
    run(&wb, "workbench.action.togglePanel", cx);
    assert_eq!(tab_ids(&wb, cx).len(), 1);
}

#[gpui::test]
fn closing_last_tab_keeps_panel_when_setting_off(cx: &mut TestAppContext) {
    init(cx);
    cx.update(|cx| {
        cx.update_global::<AppState, _>(|state, _| {
            state
                .settings
                .set_layer(
                    Layer::User,
                    r#"{ "terminal.integrated.hideOnLastClosed": false }"#,
                )
                .unwrap();
        });
    });
    let (wb, cx) = open(cx);
    run(&wb, "workbench.action.togglePanel", cx);
    let b = bounds("terminal-tab-close-0", cx);
    cx.simulate_click(b.center(), Modifiers::none());
    draw(cx);
    assert!(tab_ids(&wb, cx).is_empty());
    assert!(panel(&wb, cx).visible);
}

#[gpui::test]
fn drag_reorders_tabs(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    run(&wb, "workbench.action.togglePanel", cx);
    add_tab(&wb, cx);
    add_tab(&wb, cx);
    let ids = tab_ids(&wb, cx);
    let from = bounds("terminal-tab-0", cx).center();
    let to = bounds("terminal-tab-2", cx).center();
    let none = Modifiers::none();
    cx.simulate_mouse_down(from, MouseButton::Left, none);
    cx.simulate_mouse_move(from + point(px(10.), px(0.)), MouseButton::Left, none);
    cx.simulate_mouse_move(to, MouseButton::Left, none);
    cx.simulate_mouse_up(to, MouseButton::Left, none);
    draw(cx);
    assert_eq!(tab_ids(&wb, cx), vec![ids[1], ids[2], ids[0]]);
}

#[gpui::test]
fn middle_click_closes_tab(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    run(&wb, "workbench.action.togglePanel", cx);
    add_tab(&wb, cx);
    let ids = tab_ids(&wb, cx);
    let b = bounds("terminal-tab-0", cx).center();
    cx.simulate_mouse_down(b, MouseButton::Middle, Modifiers::none());
    cx.simulate_mouse_up(b, MouseButton::Middle, Modifiers::none());
    draw(cx);
    assert_eq!(tab_ids(&wb, cx), vec![ids[1]]);
}

#[test]
fn long_tab_titles_are_cut() {
    use lucerna_workbench::terminal_tab_title;
    assert_eq!(terminal_tab_title("zsh"), "zsh");
    let long = "a".repeat(30);
    let cut = terminal_tab_title(&long);
    assert_eq!(cut.chars().count(), 24);
    assert!(cut.ends_with('…'));
    assert_eq!(terminal_tab_title(&"đ".repeat(24)), "đ".repeat(24));
}
