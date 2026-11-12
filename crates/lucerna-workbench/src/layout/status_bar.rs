//! The 26 px status bar: panel toggles, left items and the status
//! message, the pending key chord, then right items.

use gpui::{AnyView, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, px};
use lucerna_theme::UiColors;

use super::color;
use crate::ui::{GRID, STATUS_BAR_HEIGHT, UI_FONT_SIZE_STATUS};

pub(crate) fn render(
    toggles: impl IntoElement,
    left: &[AnyView],
    message: Option<SharedString>,
    pending_chord: Option<String>,
    right: &[AnyView],
    ui: &UiColors,
) -> impl IntoElement {
    let pending = pending_chord.map(|keys| {
        div()
            .id("pending-chord")
            .debug_selector(|| "pending-chord".into())
            .child(format!(
                "({keys}) was pressed. Waiting for second key of chord..."
            ))
    });
    div()
        .debug_selector(|| "status-bar".into())
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(3. * GRID))
        .px(px(GRID))
        .w_full()
        .h(px(STATUS_BAR_HEIGHT))
        .bg(color(ui.status_bar_background))
        .border_t_1()
        .border_color(color(ui.border))
        .text_size(px(UI_FONT_SIZE_STATUS))
        .text_color(color(ui.text_muted))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2. * GRID))
                .child(toggles)
                .children(left.iter().cloned())
                .children(message.map(|message| {
                    div()
                        .debug_selector(|| "status-message".into())
                        .whitespace_nowrap()
                        .child(message)
                })),
        )
        .children(pending)
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4. * GRID))
                .pr(px(GRID))
                .children(right.iter().cloned()),
        )
}
