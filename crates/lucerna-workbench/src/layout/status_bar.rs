//! The 22 px status bar: left items, the pending key chord, right items.

use gpui::{AnyView, IntoElement, ParentElement, Styled, div, prelude::*};
use lucerna_theme::UiColors;

use super::{STATUS_BAR_HEIGHT, color};

pub(crate) fn render(
    left: &[AnyView],
    pending_chord: Option<String>,
    right: &[AnyView],
    ui: &UiColors,
) -> impl IntoElement {
    let pending = pending_chord.map(|keys| {
        div().id("pending-chord").child(format!(
            "({keys}) was pressed. Waiting for second key of chord..."
        ))
    });
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap_3()
        .px_2()
        .w_full()
        .h(STATUS_BAR_HEIGHT)
        .bg(color(ui.status_bar_background))
        .border_t_1()
        .border_color(color(ui.border))
        .text_xs()
        .text_color(color(ui.text_muted))
        .children(left.iter().cloned())
        .children(pending)
        .child(div().flex_1())
        .children(right.iter().cloned())
}
