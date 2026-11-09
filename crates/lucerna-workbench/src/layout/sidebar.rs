//! The resizable sidebar: the active view's slot (under a one-row header
//! for Search; the Explorer draws its own root row),
//! with a drag sash on its right edge.

use gpui::{
    AnyView, App, Context, FocusHandle, IntoElement, ParentElement, Pixels, Render, Styled, Window,
    div, prelude::*, px,
};
use lucerna_theme::UiColors;

use super::color;
use crate::SidebarView;
use crate::ui::{GRID, TREE_ROW_HEIGHT, UI_FONT_SIZE_SMALL};

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
    active: SidebarView,
    slot: Option<AnyView>,
    focus: &FocusHandle,
    ui: &UiColors,
) -> impl IntoElement {
    // The Explorer draws its own root row in place of a header.
    let title = match active {
        SidebarView::Explorer => None,
        SidebarView::Search => Some("SEARCH"),
    };
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
        .children(title.map(|title| {
            // One light row, no heavy panel header.
            div()
                .debug_selector(|| "sidebar-header".into())
                .flex()
                .flex_none()
                .items_center()
                .h(px(TREE_ROW_HEIGHT))
                .px(px(3. * GRID))
                .text_size(px(UI_FONT_SIZE_SMALL))
                .text_color(color(ui.text_muted))
                .overflow_hidden()
                .child(title)
        }))
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
                .on_drag(SidebarSash, |_, _, _, cx: &mut App| cx.new(|_| SashGhost)),
        )
}
