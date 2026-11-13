//! The resizable sidebar: the Explorer's slot (it draws its own root row,
//! so there is no header), with a drag sash on its right edge.

use gpui::{
    AnyView, Axis, FocusHandle, IntoElement, ParentElement, Pixels, Styled, div, prelude::*, px,
};
use lucerna_theme::UiColors;

use super::color;
use crate::ui::{SASH_SIZE, sash};

/// Drag payload while the sash is being dragged.
pub(crate) struct SidebarSash;

pub(crate) fn render(
    width: Pixels,
    slot: Option<AnyView>,
    focus: &FocusHandle,
    ui: &UiColors,
) -> impl IntoElement {
    div()
        .id("sidebar")
        .debug_selector(|| "sidebar".into())
        .track_focus(focus)
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .w(width)
        .h_full()
        .bg(color(ui.panel_background))
        .border_r_1()
        .border_color(color(ui.border))
        .child(div().flex_1().min_h_0().overflow_hidden().children(slot))
        .child(
            sash("sidebar-sash", Axis::Horizontal, px(SASH_SIZE), SidebarSash)
                .top_0()
                .right_0(),
        )
}
