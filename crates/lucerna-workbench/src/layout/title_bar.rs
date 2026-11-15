//! The 34 px integrated title bar: macOS traffic lights inset into the
//! window, then the root folder's name, its git branch (a button that
//! switches branches) and, in Restricted Mode, the "Restricted Mode"
//! button. Dragging it moves the window and a double-click zooms it. A
//! plain variant shows only a title, for the Welcome screen.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    Bounds, Context, Div, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement,
    Pixels, Stateful, Styled, TitlebarOptions, WindowBounds, WindowOptions, canvas, div, point,
    prelude::*, px, svg,
};
use lucerna_theme::UiColors;

use super::color;
use crate::Workbench;
use crate::icons;
use crate::ui::{GRID, TITLE_BAR_HEIGHT, UI_FONT_SIZE_SMALL};

/// Room left of the root name for the traffic lights.
const TRAFFIC_LIGHTS_INSET: Pixels = px(80.);

/// Options for the workbench window: a transparent title bar whose traffic
/// lights sit inside the window at (12, 11). The workbench moves the window
/// itself (see [`render`]), so AppKit does not own titlebar dragging.
pub fn window_options(bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            appears_transparent: true,
            traffic_light_position: Some(point(px(12.), px(11.))),
            ..Default::default()
        }),
        app_owns_titlebar_drag: true,
        ..Default::default()
    }
}

/// The bar both title bars share: 34 px, the traffic lights' inset, the
/// title bar colors, and a double-click that zooms the window. The caller
/// adds the drag (see [`render`]).
fn bar(ui: &UiColors) -> Stateful<Div> {
    div()
        .id("title-bar")
        .debug_selector(|| "title-bar".into())
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .w_full()
        .h(px(TITLE_BAR_HEIGHT))
        .gap(px(2. * GRID))
        .pl(TRAFFIC_LIGHTS_INSET)
        .bg(color(ui.title_bar_background))
        .border_b_1()
        .border_color(color(ui.border_variant))
        .on_click(|ev, window, _| {
            if ev.click_count() == 2 {
                window.zoom_window();
            }
        })
}

/// A title bar with only `title`, centered, for a window without a
/// workbench (the Welcome screen). `pressed` is the owner's "a press began
/// on the bar" flag, which the drag needs between events.
pub(crate) fn render_plain(
    title: &str,
    pressed: Rc<Cell<bool>>,
    ui: &UiColors,
) -> impl IntoElement + use<> {
    let down = pressed.clone();
    let up = pressed.clone();
    bar(ui)
        .justify_center()
        // Balances the inset so the title sits at the window's center.
        .pr(TRAFFIC_LIGHTS_INSET)
        .on_mouse_down(MouseButton::Left, move |_, _, _| down.set(true))
        .on_mouse_up(MouseButton::Left, move |_, _, _| up.set(false))
        .on_mouse_move(move |ev: &MouseMoveEvent, window, _| {
            if pressed.take() && ev.pressed_button == Some(MouseButton::Left) {
                window.start_window_move();
            }
        })
        .child(
            div()
                .debug_selector(|| "title-bar-title".into())
                .min_w_0()
                .truncate()
                .text_color(color(ui.text))
                .child(title.to_string()),
        )
}

/// What the title bar shows of the workspace.
pub(crate) struct TitleBarState {
    pub root_name: String,
    pub branch: Option<String>,
    /// A branch switch is running.
    pub switching: bool,
    /// Where the branch button was laid out, for the picker it opens.
    pub branch_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub restricted: bool,
}

pub(crate) fn render(
    state: TitleBarState,
    ui: &UiColors,
    cx: &mut Context<Workbench>,
) -> impl IntoElement {
    let TitleBarState {
        root_name,
        branch,
        switching,
        branch_bounds,
        restricted,
    } = state;
    bar(ui)
        // Start the native move on the first drag motion, not on mouse
        // down, so a double-click still reaches `on_click`.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|wb, _, _, _| wb.title_bar_pressed = true),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|wb, _, _, _| wb.title_bar_pressed = false),
        )
        .on_mouse_move(cx.listener(|wb, ev: &MouseMoveEvent, window, _| {
            // The press may have been released outside the bar, where the
            // mouse-up above never fires: only move while the button is held.
            let pressed = std::mem::take(&mut wb.title_bar_pressed);
            if pressed && ev.pressed_button == Some(MouseButton::Left) {
                window.start_window_move();
            }
        }))
        .child(
            div()
                .debug_selector(|| "title-bar-root".into())
                .text_color(color(ui.text))
                .child(root_name),
        )
        .children(branch.map(|branch| branch_button(branch, switching, branch_bounds, ui, cx)))
        .when(restricted, |bar| bar.child(restricted_mode_button(ui, cx)))
}

/// The branch icon and name: a click opens the branch picker under it.
/// While a switch runs it is dimmed, says "Switching…" and ignores
/// clicks. Presses on it neither move nor zoom the window. Its bounds go
/// to `bounds` as it is laid out.
fn branch_button(
    branch: String,
    switching: bool,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    ui: &UiColors,
    cx: &mut Context<Workbench>,
) -> impl IntoElement {
    let hover = color(ui.ghost_element_hover);
    let text = if switching {
        ui.text_disabled
    } else {
        ui.text_muted
    };
    div()
        .id("title-bar-branch-button")
        .debug_selector(|| "title-bar-branch-button".into())
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(GRID))
        .min_w_0()
        .h(px(6. * GRID))
        .px(px(1.5 * GRID))
        .rounded(px(GRID))
        .when(!switching, |button| {
            button.cursor_pointer().hover(move |s| s.bg(hover))
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|wb, _: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                wb.branch_button_pressed();
            }),
        )
        .on_click(cx.listener(|wb, _, window, cx| {
            cx.stop_propagation();
            wb.branch_button_clicked(window, cx);
        }))
        .child(
            div()
                .debug_selector(|| "title-bar-branch-icon".into())
                .flex_none()
                .size(px(UI_FONT_SIZE_SMALL))
                .child(
                    svg()
                        .data(icons::GIT_BRANCH)
                        .size(px(UI_FONT_SIZE_SMALL))
                        .text_color(color(if switching {
                            ui.text_disabled
                        } else {
                            ui.icon_muted
                        })),
                ),
        )
        .child(
            div()
                .debug_selector(|| "title-bar-branch".into())
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(color(text))
                .child(branch),
        )
        .when(switching, |button| {
            button.child(
                div()
                    .debug_selector(|| "title-bar-branch-switching".into())
                    .flex_none()
                    .text_color(color(text))
                    .child("Switching…"),
            )
        })
        .child(
            canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {})
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        )
}

/// A warning icon and "Restricted Mode": opens the trust modal. Presses on
/// it neither move nor zoom the window.
fn restricted_mode_button(ui: &UiColors, cx: &mut Context<Workbench>) -> impl IntoElement {
    let hover = color(ui.ghost_element_hover);
    div()
        .id("restricted-mode")
        .debug_selector(|| "restricted-mode".into())
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(GRID))
        .h(px(6. * GRID))
        .px(px(1.5 * GRID))
        .rounded(px(GRID))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .text_size(px(UI_FONT_SIZE_SMALL))
        .text_color(color(ui.text))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|wb, _: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                wb.restricted_button_pressed();
            }),
        )
        .on_click(cx.listener(|wb, _, window, cx| {
            cx.stop_propagation();
            wb.restricted_button_clicked(window, cx);
        }))
        .child(
            svg()
                .data(icons::TRIANGLE_ALERT)
                .flex_none()
                .size(px(UI_FONT_SIZE_SMALL))
                .text_color(color(ui.warning)),
        )
        .child("Restricted Mode")
}
