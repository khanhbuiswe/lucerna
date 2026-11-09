//! Zed-style chrome: title bar, status-bar panel toggles, flat tabs, the
//! empty editor hint and the shared popover style.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    AbsoluteLength, Bounds, Entity, Fill, Length, Modifiers, MouseButton, Styled, TestAppContext,
    VisualTestContext, point, px, size,
};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, OpenMode, SidebarView, Workbench, ui, window_options};

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
    });
}

fn open(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut VisualTestContext) {
    cx.add_window_view(|window, cx| Workbench::new(PathBuf::from("/tmp/project"), window, cx))
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn bounds(name: &'static str, cx: &mut VisualTestContext) -> Bounds<gpui::Pixels> {
    cx.debug_bounds(name)
        .unwrap_or_else(|| panic!("no element {name}"))
}

fn open_path(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new(path)), OpenMode::Pinned, cx)
    });
}

#[gpui::test]
fn no_activity_bar(cx: &mut TestAppContext) {
    init(cx);
    let (_wb, cx) = open(cx);
    draw(cx);
    assert!(cx.debug_bounds("activity-bar").is_none());

    // Top to bottom: title bar, body, status bar.
    let title = bounds("title-bar", cx);
    let status = bounds("status-bar", cx);
    let sidebar = bounds("sidebar", cx);
    assert_eq!(title.top(), px(0.));
    assert_eq!(title.size.height, px(ui::TITLE_BAR_HEIGHT));
    assert_eq!(status.size.height, px(ui::STATUS_BAR_HEIGHT));
    // The sidebar starts at the window's left edge, under the title bar.
    assert_eq!(sidebar.left(), px(0.));
    assert_eq!(sidebar.top(), title.bottom());
    assert_eq!(sidebar.bottom(), status.top());

    // The panel toggles sit at the status bar's left end, Explorer first.
    let explorer = bounds("panel-toggle-explorer", cx);
    let search = bounds("panel-toggle-search", cx);
    assert!(status.contains(&explorer.center()) && status.contains(&search.center()));
    assert!(explorer.left() < search.left());
    assert_eq!(explorer.size, size(px(24.), px(20.)));
}

#[gpui::test]
fn status_toggle_switches_panel(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    let state = |cx: &mut VisualTestContext| {
        wb.read_with(cx, |wb, _| (wb.sidebar_visible(), wb.active_sidebar()))
    };
    let click = |name: &'static str, cx: &mut VisualTestContext| {
        draw(cx);
        let b = bounds(name, cx);
        cx.simulate_click(b.center(), Modifiers::none());
    };

    click("panel-toggle-search", cx);
    assert_eq!(state(cx), (true, SidebarView::Search));
    // Clicking the active panel's toggle hides the sidebar.
    click("panel-toggle-search", cx);
    assert!(!state(cx).0);
    click("panel-toggle-explorer", cx);
    assert_eq!(state(cx), (true, SidebarView::Explorer));
    // The keybinding still switches panels.
    cx.simulate_keystrokes("cmd-b");
    assert!(!state(cx).0);
    cx.simulate_keystrokes("cmd-shift-e");
    assert_eq!(state(cx), (true, SidebarView::Explorer));
}

#[gpui::test]
fn title_bar_shows_root_name(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    assert_eq!(wb.read_with(cx, |wb, _| wb.root_name()), "project");
    draw(cx);
    let title = bounds("title-bar", cx);
    let name = bounds("title-bar-root", cx);
    // Clear of the traffic lights.
    assert_eq!(name.left(), px(80.));
    assert!(title.contains(&name.center()));
    // The Explorer's root row is a single 24 px row.
    let root = bounds("explorer-root", cx);
    assert_eq!(root.size.height, px(ui::TREE_ROW_HEIGHT));

    let opts = window_options(Bounds::new(point(px(0.), px(0.)), size(px(800.), px(600.))));
    let titlebar = opts.titlebar.expect("custom titlebar");
    assert!(titlebar.appears_transparent);
    assert_eq!(
        titlebar.traffic_light_position,
        Some(point(px(12.), px(11.)))
    );
}

#[gpui::test]
fn tab_close_button_only_on_hover_or_active(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    open_path(&wb, "a.rs", cx);
    open_path(&wb, "b.rs", cx);
    draw(cx);
    let tab_a = bounds("tab-0-0", cx);
    assert_eq!(tab_a.size.height, px(ui::TAB_HEIGHT));
    // b.rs is active: its close button shows; a.rs's does not.
    assert!(cx.debug_bounds("tab-close-0-1").is_some());
    assert!(cx.debug_bounds("tab-close-0-0").is_none());

    cx.simulate_mouse_move(tab_a.center(), None, Modifiers::none());
    draw(cx);
    assert!(cx.debug_bounds("tab-close-0-0").is_some());
    // Showing the button does not resize the tab.
    assert_eq!(bounds("tab-0-0", cx), tab_a);

    let editor = bounds("editor-groups", cx);
    cx.simulate_mouse_move(editor.center(), None, Modifiers::none());
    draw(cx);
    assert!(cx.debug_bounds("tab-close-0-0").is_none());
    assert!(cx.debug_bounds("tab-close-0-1").is_some());
}

#[gpui::test]
fn empty_editor_area_shows_hint(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    draw(cx);
    assert!(cx.debug_bounds("editor-empty-hint").is_some());
    open_path(&wb, "a.rs", cx);
    draw(cx);
    assert!(cx.debug_bounds("editor-empty-hint").is_none());
    cx.simulate_keystrokes("cmd-w");
    draw(cx);
    assert!(cx.debug_bounds("editor-empty-hint").is_some());
}

#[gpui::test]
fn popover_helper_has_radius_border_and_max_width(cx: &mut TestAppContext) {
    init(cx);
    let (_wb, cx) = open(cx);
    cx.update(|_, cx| {
        let ui_colors = cx.global::<AppState>().theme.ui.clone();
        let mut popover = ui::popover(cx);
        let style = popover.style();
        let radius = Some(AbsoluteLength::from(px(ui::POPOVER_RADIUS)));
        assert_eq!(style.corner_radii.top_left, radius);
        assert_eq!(style.corner_radii.bottom_right, radius);
        let border = Some(AbsoluteLength::from(px(1.)));
        assert_eq!(style.border_widths.top, border);
        assert_eq!(style.border_widths.left, border);
        assert_eq!(
            style.max_size.width,
            Some(Length::from(px(ui::POPOVER_MAX_WIDTH)))
        );
        let elevated = gpui::Hsla::from(gpui::Rgba {
            r: ui_colors.elevated_surface_background.r,
            g: ui_colors.elevated_surface_background.g,
            b: ui_colors.elevated_surface_background.b,
            a: ui_colors.elevated_surface_background.a,
        });
        assert_eq!(style.background, Some(Fill::from(elevated)));
        assert!(style.box_shadow.as_ref().is_some_and(|s| !s.is_empty()));
    });

    // Drawn with wide content, the popover stops at its maximum width.
    let (_view, cx) = cx.add_window_view(|_, _| PopoverHost);
    draw(cx);
    let b = bounds("popover-under-test", cx);
    assert_eq!(b.size.width, px(ui::POPOVER_MAX_WIDTH));
}

struct PopoverHost;

impl gpui::Render for PopoverHost {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement, div, prelude::*};
        div().size_full().child(
            ui::popover(cx)
                .debug_selector(|| "popover-under-test".into())
                .child(div().w(px(2000.)).h(px(20.))),
        )
    }
}

#[gpui::test]
fn icon_button_colors_follow_active(cx: &mut TestAppContext) {
    init(cx);
    let (_wb, cx) = open(cx);
    cx.update(|_, cx| {
        let mut on = ui::icon_button("on", ui::SEARCH_ICON, true, cx);
        let mut off = ui::icon_button("off", ui::SEARCH_ICON, false, cx);
        assert!(on.style().background.is_some());
        assert!(off.style().background.is_none());
        assert_eq!(on.style().size.width, Some(px(24.).into()));
        assert_eq!(on.style().size.height, Some(px(20.).into()));
    });
}

fn activate_first(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| {
            let first = m.groups()[0].tabs[0].id;
            m.activate(first)
        })
    });
}

#[gpui::test]
fn closed_hovered_tab_leaves_no_stale_close_button(cx: &mut TestAppContext) {
    init(cx);
    let (wb, cx) = open(cx);
    for path in ["a.rs", "b.rs", "c.rs"] {
        open_path(&wb, path, cx);
    }
    activate_first(&wb, cx);
    draw(cx);
    // Hover the last tab and close it with its ×.
    let c = bounds("tab-0-2", cx);
    cx.simulate_mouse_move(c.center(), None, Modifiers::none());
    draw(cx);
    let close = bounds("tab-close-0-2", cx);
    cx.simulate_click(close.center(), Modifiers::none());
    // Move away from the tab bar, then open a file into the freed slot.
    let editor = bounds("editor-groups", cx);
    cx.simulate_mouse_move(editor.center(), None, Modifiers::none());
    open_path(&wb, "d.rs", cx);
    activate_first(&wb, cx);
    draw(cx);
    assert!(cx.debug_bounds("tab-close-0-0").is_some(), "active tab");
    for name in ["tab-close-0-1", "tab-close-0-2"] {
        assert!(
            cx.debug_bounds(name).is_none(),
            "{name} shown without hover"
        );
    }
}

#[gpui::test]
fn title_bar_release_outside_does_not_move_window_later(cx: &mut TestAppContext) {
    init(cx);
    let (_wb, cx) = open(cx);
    draw(cx);
    let title = bounds("title-bar", cx);
    let editor = bounds("editor-groups", cx);
    let spot = point(title.right() - px(40.), title.center().y);
    // Press on the title bar, release over the editor.
    cx.simulate_mouse_down(spot, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(editor.center(), MouseButton::Left, Modifiers::none());
    // Hovering the title bar with no button held must not start a window
    // move (the test platform panics if one starts).
    cx.simulate_mouse_move(spot, None, Modifiers::none());
    cx.simulate_mouse_move(spot + point(px(5.), px(0.)), None, Modifiers::none());
}

#[test]
fn ui_font_constants() {
    assert_eq!(ui::UI_FONT_FAMILY, "IBM Plex Sans");
    assert_eq!(ui::UI_FONT_SIZE, 14.0);
    assert_eq!(ui::UI_FONT_SIZE_SMALL, 13.0);
}

/// GPUI draws an SVG as an alpha mask filled with `text_color`, so each
/// vendored Lucide icon (stroked with `currentColor`) must rasterize to
/// visible pixels.
#[gpui::test]
fn lucide_icons_rasterize_with_visible_strokes(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let renderer = cx.svg_renderer();
        for (name, bytes) in lucerna_workbench::icons::ALL {
            let image = renderer
                .render_single_frame(bytes, 1.0)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let pixels = image.as_bytes(0).unwrap();
            let opaque = pixels.chunks(4).filter(|px| px[3] > 0x80).count();
            assert!(opaque > 10, "{name} has {opaque} visible pixels");
        }
    });
}
