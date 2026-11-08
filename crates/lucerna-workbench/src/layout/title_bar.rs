//! The 34 px integrated title bar: macOS traffic lights inset into the
//! window, then the root folder's name. Dragging it moves the window and a
//! double-click zooms it.

use gpui::{
    Bounds, Context, IntoElement, MouseButton, MouseMoveEvent, ParentElement, Pixels, Styled,
    TitlebarOptions, WindowBounds, WindowOptions, div, point, prelude::*, px,
};
use lucerna_theme::UiColors;

use super::color;
use crate::Workbench;
use crate::ui::TITLE_BAR_HEIGHT;

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

pub(crate) fn render(
    root_name: String,
    ui: &UiColors,
    cx: &mut Context<Workbench>,
) -> impl IntoElement {
    div()
        .id("title-bar")
        .debug_selector(|| "title-bar".into())
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .w_full()
        .h(px(TITLE_BAR_HEIGHT))
        .pl(TRAFFIC_LIGHTS_INSET)
        .bg(color(ui.title_bar_background))
        .border_b_1()
        .border_color(color(ui.border_variant))
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
        .on_click(|ev, window, _| {
            if ev.click_count() == 2 {
                window.zoom_window();
            }
        })
        .child(
            div()
                .debug_selector(|| "title-bar-root".into())
                .text_color(color(ui.text))
                .child(root_name),
        )
}
