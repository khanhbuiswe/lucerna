//! The Explorer and Search buttons at the status bar's left end. They
//! replace the activity bar: the active panel's button hides the sidebar,
//! the other one switches to its panel.

use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::*};

use crate::ui::{FILES_ICON, SEARCH_ICON, icon_button};
use crate::{SidebarView, Workbench};

pub(crate) fn render(
    active: SidebarView,
    sidebar_visible: bool,
    cx: &mut Context<Workbench>,
) -> impl IntoElement {
    let toggle = |view: SidebarView, cx: &mut Context<Workbench>| {
        let (id, icon) = match view {
            SidebarView::Explorer => ("panel-toggle-explorer", FILES_ICON),
            SidebarView::Search => ("panel-toggle-search", SEARCH_ICON),
        };
        icon_button(id, icon, sidebar_visible && view == active, cx)
            .debug_selector(move || id.into())
            .on_click(
                cx.listener(move |wb, _, window, cx| wb.panel_toggle_clicked(view, window, cx)),
            )
    };
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap_0p5()
        .child(toggle(SidebarView::Explorer, cx))
        .child(toggle(SidebarView::Search, cx))
}
