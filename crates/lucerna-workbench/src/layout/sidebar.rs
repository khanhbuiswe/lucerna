//! The resizable sidebar: the Explorer's slot (it draws its own root row,
//! so there is no header), with a drag sash on its right edge.

use gpui::{
    AnyView, App, Context, FocusHandle, IntoElement, ParentElement, Pixels, Render, Styled, Window,
    div, prelude::*, px,
};
use lucerna_theme::UiColors;

use super::color;

const SASH_WIDTH: Pixels = px(4.);

/// Drag payload while the sash is being dragged.
pub(crate) struct SidebarSash;

/// GPUI requires a view to draw under the cursor while dragging; the sash
/// drag draws nothing.
pub(crate) struct SashGhost;

impl Render for SashGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

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
            div()
                .id("sidebar-sash")
                .absolute()
                .top_0()
                .right_0()
                .w(SASH_WIDTH)
                .h_full()
                .cursor_col_resize()
                // Over the scrollbar strip of the content on this edge: the
                // sash wins on its own few px.
                .occlude()
                .on_drag(SidebarSash, |_, _, _, cx: &mut App| cx.new(|_| SashGhost)),
        )
}
