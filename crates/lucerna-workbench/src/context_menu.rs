//! A reusable context menu: a list of command entries and separators
//! shown at a point, driven by the keyboard (up, down, home, end, enter,
//! escape) or the mouse. Confirming an entry emits [`MenuCommand`] for
//! the host to run; the menu itself never runs commands.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    Context, DismissEvent, Div, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render, SharedString, Stateful,
    Styled, Subscription, Window, anchored, deferred, div, prelude::*, px,
};
use serde_json::Value;

use crate::AppState;
use crate::layout::color;
use crate::ui::{GRID, UI_FONT_SIZE_SMALL, UI_FONT_SIZE_STATUS, popover};

/// Narrowest the menu gets.
const MENU_MIN_WIDTH: f32 = 200.0;
/// How far from the window edges the menu stays.
const WINDOW_MARGIN: f32 = 8.0;

/// One line of a context menu.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuItem {
    Entry {
        label: SharedString,
        command: &'static str,
        args: Option<Value>,
        disabled: bool,
    },
    Separator,
}

impl MenuItem {
    fn is_enabled_entry(&self) -> bool {
        matches!(
            self,
            MenuItem::Entry {
                disabled: false,
                ..
            }
        )
    }
}

/// Emitted when an entry is confirmed, right after [`DismissEvent`].
#[derive(Debug, Clone, PartialEq)]
pub struct MenuCommand {
    pub command: &'static str,
    pub args: Option<Value>,
}

/// Drops separators at either end and repeated ones.
fn normalize(items: Vec<MenuItem>) -> Vec<MenuItem> {
    let mut out: Vec<MenuItem> = Vec::with_capacity(items.len());
    for item in items {
        if item == MenuItem::Separator && matches!(out.last(), None | Some(MenuItem::Separator)) {
            continue;
        }
        out.push(item);
    }
    if out.last() == Some(&MenuItem::Separator) {
        out.pop();
    }
    out
}

/// The next enabled entry after `from` (or the first / last one when
/// `from` is `None`), wrapping around; `None` when no entry is enabled.
fn next_enabled(items: &[MenuItem], from: Option<usize>, forward: bool) -> Option<usize> {
    let n = items.len();
    if n == 0 {
        return None;
    }
    let start = match (from, forward) {
        (None, true) => 0,
        (None, false) => n - 1,
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
    };
    (0..n)
        .map(|k| {
            if forward {
                (start + k) % n
            } else {
                (start + n - k) % n
            }
        })
        .find(|&ix| items[ix].is_enabled_entry())
}

pub struct ContextMenu {
    owner: FocusHandle,
    focus: FocusHandle,
    /// What the builder was given, as given.
    raw: Vec<MenuItem>,
    /// `raw` normalized.
    items: Vec<MenuItem>,
    selected: Option<usize>,
}

impl gpui::prelude::FluentBuilder for ContextMenu {}

impl ContextMenu {
    /// A menu that gives focus back to `owner` when it closes.
    pub fn new(owner: FocusHandle, cx: &mut Context<Self>) -> Self {
        Self {
            owner,
            focus: cx.focus_handle(),
            raw: Vec::new(),
            items: Vec::new(),
            selected: None,
        }
    }

    pub fn entry(self, label: impl Into<SharedString>, command: &'static str) -> Self {
        self.push_entry(label.into(), command, None)
    }

    pub fn entry_with_args(
        self,
        label: impl Into<SharedString>,
        command: &'static str,
        args: Value,
    ) -> Self {
        self.push_entry(label.into(), command, Some(args))
    }

    fn push_entry(
        mut self,
        label: SharedString,
        command: &'static str,
        args: Option<Value>,
    ) -> Self {
        self.raw.push(MenuItem::Entry {
            label,
            command,
            args,
            disabled: false,
        });
        self.items = normalize(self.raw.clone());
        self
    }

    /// Disables the entry added last when `cond` holds.
    pub fn disabled_when(mut self, cond: bool) -> Self {
        if let Some(MenuItem::Entry { disabled, .. }) = self.raw.last_mut() {
            *disabled = cond;
        }
        self.items = normalize(self.raw.clone());
        self
    }

    pub fn separator(mut self) -> Self {
        self.raw.push(MenuItem::Separator);
        self.items = normalize(self.raw.clone());
        self
    }

    /// The lines as drawn: no separator first, last or doubled.
    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    /// The keybinding label shown for the entry at `ix`, if its command
    /// is bound.
    pub fn shortcut_label(&self, ix: usize, cx: &gpui::App) -> Option<String> {
        match self.items.get(ix)? {
            MenuItem::Entry { command, .. } => cx.global::<AppState>().keymap.label_for(command),
            MenuItem::Separator => None,
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn select_next(&mut self, cx: &mut Context<Self>) {
        self.selected = next_enabled(&self.items, self.selected, true);
        cx.notify();
    }

    pub fn select_prev(&mut self, cx: &mut Context<Self>) {
        self.selected = next_enabled(&self.items, self.selected, false);
        cx.notify();
    }

    pub fn select_first(&mut self, cx: &mut Context<Self>) {
        self.selected = next_enabled(&self.items, None, true);
        cx.notify();
    }

    pub fn select_last(&mut self, cx: &mut Context<Self>) {
        self.selected = next_enabled(&self.items, None, false);
        cx.notify();
    }

    fn confirm(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(MenuItem::Entry {
            command,
            args,
            disabled: false,
            ..
        }) = self.items.get(ix)
        else {
            return;
        };
        let command = MenuCommand {
            command,
            args: args.clone(),
        };
        cx.emit(DismissEvent);
        cx.emit(command);
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.shift || m.platform {
            return;
        }
        match ev.keystroke.key.as_str() {
            "up" => self.select_prev(cx),
            "down" => self.select_next(cx),
            "home" => self.select_first(cx),
            "end" => self.select_last(cx),
            "enter" => match self.selected {
                Some(ix) => self.confirm(ix, cx),
                None => return,
            },
            "escape" => self.dismiss(cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl EventEmitter<DismissEvent> for ContextMenu {}
impl EventEmitter<MenuCommand> for ContextMenu {}

impl Focusable for ContextMenu {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ContextMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<AppState>().theme.ui.clone();
        let max_height = window.viewport_size().height * 0.75;
        let rows: Vec<_> = self
            .items
            .iter()
            .enumerate()
            .map(|(ix, item)| match item {
                MenuItem::Separator => div()
                    .debug_selector(move || format!("context-menu-separator-{ix}"))
                    .h(px(1.))
                    .my(px(GRID))
                    .bg(color(ui.border_variant))
                    .into_any_element(),
                MenuItem::Entry {
                    label,
                    command,
                    disabled,
                    ..
                } => {
                    let shortcut = self.shortcut_label(ix, cx);
                    let selector = format!("context-menu-item-{command}");
                    let row: Stateful<Div> = div()
                        .id(("context-menu-item", ix))
                        .debug_selector(move || selector)
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(4. * GRID))
                        .h(px(6. * GRID))
                        .px(px(2. * GRID))
                        .rounded(px(GRID))
                        .text_size(px(UI_FONT_SIZE_SMALL))
                        .child(div().child(label.clone()))
                        .children(shortcut.map(|label| {
                            let selector = format!("context-menu-shortcut-{command}");
                            div()
                                .debug_selector(move || selector)
                                .text_size(px(UI_FONT_SIZE_STATUS))
                                .text_color(color(ui.text_muted))
                                .child(label)
                        }));
                    if *disabled {
                        row.text_color(color(ui.text_disabled)).into_any_element()
                    } else {
                        row.text_color(color(ui.text))
                            .cursor_pointer()
                            .when(self.selected == Some(ix), |row| {
                                row.bg(color(ui.ghost_element_hover))
                            })
                            .on_hover(cx.listener(move |menu, hovered: &bool, _, cx| {
                                if *hovered && menu.selected != Some(ix) {
                                    menu.selected = Some(ix);
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |menu, _, _, cx| menu.confirm(ix, cx)))
                            .into_any_element()
                    }
                }
            })
            .collect();
        popover(cx)
            .id("context-menu")
            .debug_selector(|| "context-menu".into())
            .track_focus(&self.focus)
            .max_w(window.viewport_size().width)
            .w_auto()
            .min_w(px(MENU_MIN_WIDTH))
            .max_h(max_height)
            .p(px(GRID))
            .occlude()
            .overflow_y_scroll()
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(cx.listener(|menu, _: &MouseDownEvent, _, cx| menu.dismiss(cx)))
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .children(rows)
    }
}

/// A menu being shown: keep it in the view that opened it and draw it
/// with [`render_deployed`]; drop it to close the menu.
pub struct DeployedMenu {
    pub menu: gpui::Entity<ContextMenu>,
    pub position: Point<Pixels>,
    _subs: Vec<Subscription>,
}

/// Focuses `menu` and watches it. The menu closes when it is dismissed,
/// an entry is confirmed or the focus leaves it; `on_close` then gets the
/// confirmed command, or `None`, once. Focus returns to the owner only if
/// the menu (or nothing) still holds it, so a click into another view
/// keeps that view focused.
pub fn deploy_context_menu<V: 'static>(
    menu: gpui::Entity<ContextMenu>,
    position: Point<Pixels>,
    window: &mut Window,
    cx: &mut Context<V>,
    on_close: impl Fn(&mut V, Option<MenuCommand>, &mut Window, &mut Context<V>) + 'static,
) -> DeployedMenu {
    let owner = menu.read(cx).owner.clone();
    let focus = menu.read(cx).focus.clone();
    window.focus(&focus, cx);
    let on_close = Rc::new(on_close);
    let confirmed: Rc<RefCell<Option<MenuCommand>>> = Rc::default();
    let closed = Rc::new(Cell::new(false));
    let on_command = {
        let confirmed = confirmed.clone();
        cx.subscribe_in(&menu, window, move |_, _, event: &MenuCommand, _, _| {
            *confirmed.borrow_mut() = Some(event.clone());
        })
    };
    // The command arrives after the dismissal, so close one step later.
    let on_dismiss = {
        let focus = focus.clone();
        cx.subscribe_in(&menu, window, move |_, _, _: &DismissEvent, window, cx| {
            if closed.replace(true) {
                return;
            }
            let (confirmed, owner, focus) = (confirmed.clone(), owner.clone(), focus.clone());
            let on_close = on_close.clone();
            cx.defer_in(window, move |view, window, cx| {
                if focus.contains_focused(window, cx) || window.focused(cx).is_none() {
                    window.focus(&owner, cx);
                }
                let command = confirmed.borrow_mut().take();
                on_close(view, command, window, cx);
            });
        })
    };
    // Focus moved elsewhere (another view, a palette): close.
    let on_blur = {
        let menu = menu.clone();
        cx.on_blur(&focus, window, move |_, _, cx| {
            menu.update(cx, |menu, cx| menu.dismiss(cx));
        })
    };
    DeployedMenu {
        menu,
        position,
        _subs: vec![on_command, on_dismiss, on_blur],
    }
}

/// Draws a deployed menu at its position, kept 8 px inside the window.
pub fn render_deployed(d: &DeployedMenu) -> impl IntoElement {
    deferred(
        anchored()
            .position(d.position)
            .snap_to_window_with_margin(px(WINDOW_MARGIN))
            .child(d.menu.clone()),
    )
    .with_priority(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &'static str, disabled: bool) -> MenuItem {
        MenuItem::Entry {
            label: name.into(),
            command: name,
            args: None,
            disabled,
        }
    }

    #[test]
    fn normalize_drops_leading_trailing_and_double_separators() {
        let sep = || MenuItem::Separator;
        let a = || entry("a", false);
        let b = || entry("b", false);
        assert_eq!(
            normalize(vec![sep(), a(), sep(), sep(), b(), sep()]),
            vec![a(), sep(), b()]
        );
        assert_eq!(normalize(vec![sep()]), vec![]);
    }

    #[test]
    fn next_skips_separators_and_disabled() {
        let items = vec![
            entry("a", false),
            MenuItem::Separator,
            entry("b", true),
            entry("c", false),
        ];
        assert_eq!(next_enabled(&items, None, true), Some(0));
        assert_eq!(next_enabled(&items, None, false), Some(3));
        assert_eq!(next_enabled(&items, Some(0), true), Some(3));
        assert_eq!(next_enabled(&items, Some(3), true), Some(0));
        assert_eq!(next_enabled(&items, Some(3), false), Some(0));
        let all_disabled = vec![entry("a", true), MenuItem::Separator, entry("b", true)];
        assert_eq!(next_enabled(&all_disabled, None, true), None);
        assert_eq!(next_enabled(&all_disabled, Some(0), false), None);
    }
}
