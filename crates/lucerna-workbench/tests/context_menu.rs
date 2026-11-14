//! The reusable context menu: keyboard and mouse handling, closing and
//! focus return, driven through GPUI's test platform.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Context, Entity, FocusHandle, Focusable, IntoElement, Modifiers, ParentElement, Render, Styled,
    TestAppContext, VisualTestContext, Window, div, point, prelude::*, px,
};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::AppState;
use lucerna_workbench::ui::{
    ContextMenu, DeployedMenu, MenuCommand, deploy_context_menu, render_deployed,
};

fn init(cx: &mut TestAppContext, user_keymap: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), user_keymap);
    assert!(error.is_none(), "keymap error: {error:?}");
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
}

type Closed = Rc<RefCell<Vec<Option<MenuCommand>>>>;

struct Host {
    focus: FocusHandle,
    /// Another focusable area, as the editor is beside the Explorer: a
    /// mouse down on it takes the focus.
    other: FocusHandle,
    deployed: Option<DeployedMenu>,
    closed: Closed,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("host")
            .size_full()
            .track_focus(&self.focus)
            .child(
                div()
                    .id("other")
                    .absolute()
                    .top(px(0.))
                    .left(px(400.))
                    .size(px(20.))
                    .track_focus(&self.other)
                    .on_mouse_down(gpui::MouseButton::Left, {
                        let other = self.other.clone();
                        move |_, window, cx| window.focus(&other, cx)
                    }),
            )
            .children(self.deployed.as_ref().map(render_deployed))
    }
}

/// A host with the menu `Copy Path`, separator, `New File…` (disabled)
/// deployed at (50, 50).
fn open(cx: &mut TestAppContext) -> (Entity<Host>, Closed, &mut VisualTestContext) {
    let closed = Closed::default();
    let host_closed = closed.clone();
    let (host, cx) = cx.add_window_view(move |window, cx| {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Host {
            focus,
            other: cx.focus_handle(),
            deployed: None,
            closed: host_closed,
        }
    });
    // Focus events only fire in an active window.
    cx.update(|window, _| window.activate_window());
    host.update_in(cx, |host, window, cx| {
        let owner = host.focus.clone();
        let menu = cx.new(|cx| {
            ContextMenu::new(owner, cx)
                .entry("Copy Path", "copyFilePath")
                .separator()
                .entry("New File…", "explorer.newFile")
                .disabled_when(true)
        });
        host.deployed = Some(deploy_context_menu(
            menu,
            point(px(50.), px(50.)),
            window,
            cx,
            |host, command, _, cx| {
                host.deployed = None;
                host.closed.borrow_mut().push(command);
                cx.notify();
            },
        ));
    });
    draw(cx);
    (host, closed, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn menu(host: &Entity<Host>, cx: &mut VisualTestContext) -> Entity<ContextMenu> {
    host.read_with(cx, |host, _| host.deployed.as_ref().unwrap().menu.clone())
}

fn host_focused(host: &Entity<Host>, cx: &mut VisualTestContext) -> bool {
    let focus = host.read_with(cx, |host, cx| host.focus_handle(cx));
    cx.update(|window, _| focus.is_focused(window))
}

impl Focusable for Host {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

#[gpui::test]
fn opens_with_nothing_selected_and_down_selects_first(cx: &mut TestAppContext) {
    init(cx, None);
    let (host, _, cx) = open(cx);
    let menu = menu(&host, cx);
    assert_eq!(menu.read_with(cx, |m, _| m.selected()), None);
    cx.simulate_keystrokes("down");
    assert_eq!(menu.read_with(cx, |m, _| m.selected()), Some(0));
    // The disabled entry is skipped: down wraps back to the first.
    cx.simulate_keystrokes("down");
    assert_eq!(menu.read_with(cx, |m, _| m.selected()), Some(0));
}

#[gpui::test]
fn enter_runs_selected_and_closes(cx: &mut TestAppContext) {
    init(cx, None);
    let (host, closed, cx) = open(cx);
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert_eq!(
        *closed.borrow(),
        vec![Some(MenuCommand {
            command: "copyFilePath",
            args: None
        })]
    );
    assert!(cx.debug_bounds("context-menu").is_none());
    assert!(host_focused(&host, cx));
}

#[gpui::test]
fn escape_closes_without_command(cx: &mut TestAppContext) {
    init(cx, None);
    let (host, closed, cx) = open(cx);
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert_eq!(*closed.borrow(), vec![None]);
    assert!(cx.debug_bounds("context-menu").is_none());
    assert!(host_focused(&host, cx));
}

#[gpui::test]
fn click_outside_closes(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, closed, cx) = open(cx);
    cx.simulate_click(point(px(5.), px(5.)), Modifiers::none());
    draw(cx);
    assert_eq!(*closed.borrow(), vec![None]);
    assert!(cx.debug_bounds("context-menu").is_none());
}

#[gpui::test]
fn disabled_entry_does_nothing_on_click(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, closed, cx) = open(cx);
    let position = cx
        .debug_bounds("context-menu-item-explorer.newFile")
        .unwrap()
        .center();
    cx.simulate_click(position, Modifiers::none());
    draw(cx);
    assert!(closed.borrow().is_empty());
    assert!(cx.debug_bounds("context-menu").is_some());
}

#[gpui::test]
fn clicking_an_entry_runs_it(cx: &mut TestAppContext) {
    init(cx, None);
    let (_, closed, cx) = open(cx);
    let position = cx
        .debug_bounds("context-menu-item-copyFilePath")
        .unwrap()
        .center();
    cx.simulate_click(position, Modifiers::none());
    draw(cx);
    assert_eq!(closed.borrow().len(), 1);
    assert!(cx.debug_bounds("context-menu").is_none());
}

#[gpui::test]
fn shows_keybinding_label(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"[{ "key": "alt+cmd+c", "command": "copyFilePath" }]"#),
    );
    let (host, _, cx) = open(cx);
    let label = cx.update(|_, cx| {
        cx.global::<AppState>()
            .keymap
            .label_for("copyFilePath")
            .unwrap()
    });
    assert!(!label.is_empty());
    let menu = host.read_with(cx, |host, _| host.deployed.as_ref().unwrap().menu.clone());
    let shown = menu.read_with(cx, |menu, cx| menu.shortcut_label(0, cx));
    assert_eq!(shown, Some(label));
    // The label is drawn inside the bound entry, and only there.
    let item = cx.debug_bounds("context-menu-item-copyFilePath").unwrap();
    let shortcut = cx
        .debug_bounds("context-menu-shortcut-copyFilePath")
        .expect("shortcut label drawn");
    assert!(item.contains(&shortcut.center()));
    assert!(
        cx.debug_bounds("context-menu-shortcut-explorer.newFile")
            .is_none()
    );
}

#[gpui::test]
fn hover_moves_selection(cx: &mut TestAppContext) {
    init(cx, None);
    let (host, _, cx) = open(cx);
    let menu = menu(&host, cx);
    let position = cx
        .debug_bounds("context-menu-item-copyFilePath")
        .unwrap()
        .center();
    cx.simulate_mouse_move(position, None, Modifiers::none());
    draw(cx);
    assert_eq!(menu.read_with(cx, |m, _| m.selected()), Some(0));
}

fn other_focused(host: &Entity<Host>, cx: &mut VisualTestContext) -> bool {
    let other = host.read_with(cx, |host, _| host.other.clone());
    cx.update(|window, _| other.is_focused(window))
}

#[gpui::test]
fn click_into_another_view_keeps_its_focus(cx: &mut TestAppContext) {
    init(cx, None);
    let (host, closed, cx) = open(cx);
    cx.simulate_click(point(px(410.), px(10.)), Modifiers::none());
    draw(cx);
    assert_eq!(*closed.borrow(), vec![None]);
    assert!(cx.debug_bounds("context-menu").is_none());
    assert!(other_focused(&host, cx), "the clicked view keeps the focus");
    assert!(!host_focused(&host, cx));
}

#[gpui::test]
fn closes_when_focus_moves_elsewhere(cx: &mut TestAppContext) {
    init(cx, None);
    let (host, closed, cx) = open(cx);
    let other = host.read_with(cx, |host, _| host.other.clone());
    cx.update(|window, cx| window.focus(&other, cx));
    draw(cx);
    draw(cx);
    assert_eq!(*closed.borrow(), vec![None]);
    assert!(cx.debug_bounds("context-menu").is_none());
    assert!(other_focused(&host, cx));
}
