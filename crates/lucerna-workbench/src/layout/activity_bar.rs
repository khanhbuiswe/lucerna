//! The 48 px strip of view icons on the far left.

use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::*, px, svg};
use lucerna_theme::UiColors;

use super::{ACTIVITY_BAR_WIDTH, color};
use crate::{SidebarView, Workbench};

/// Two overlapping pages.
const EXPLORER_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill-rule="evenodd" d="M8 2h8l5 5v11a1 1 0 0 1-1 1h-3v3a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h3V3a1 1 0 0 1 1-1Zm1 2v13h10V8h-4V4H9ZM5 8v13h10v-2H8a1 1 0 0 1-1-1V8H5Z"/></svg>"#;

/// A magnifying glass.
const SEARCH_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill-rule="evenodd" d="M10 3a7 7 0 0 1 5.6 11.2l5.1 5.1-1.4 1.4-5.1-5.1A7 7 0 1 1 10 3Zm0 2a5 5 0 1 0 0 10 5 5 0 0 0 0-10Z"/></svg>"#;

pub(crate) fn render(
    active: SidebarView,
    sidebar_visible: bool,
    ui: &UiColors,
    cx: &mut Context<Workbench>,
) -> impl IntoElement {
    let item = |view: SidebarView, icon: &'static [u8], cx: &mut Context<Workbench>| {
        let selected = sidebar_visible && view == active;
        let id = match view {
            SidebarView::Explorer => "activity-explorer",
            SidebarView::Search => "activity-search",
        };
        let fg = if selected { ui.text } else { ui.text_muted };
        div()
            .id(id)
            .flex()
            .justify_center()
            .items_center()
            .w_full()
            .h(px(48.))
            .border_l_2()
            .border_color(color(if selected { ui.text } else { ui.background }))
            .cursor_pointer()
            .on_click(cx.listener(move |wb, _, window, cx| wb.activity_clicked(view, window, cx)))
            .child(svg().data(icon).size(px(24.)).text_color(color(fg)))
    };
    div()
        .flex()
        .flex_col()
        .flex_none()
        .w(ACTIVITY_BAR_WIDTH)
        .h_full()
        .bg(color(ui.background))
        .border_r_1()
        .border_color(color(ui.border))
        .child(item(SidebarView::Explorer, EXPLORER_ICON, cx))
        .child(item(SidebarView::Search, SEARCH_ICON, cx))
}
