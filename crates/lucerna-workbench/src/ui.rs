//! Shared chrome metrics, icons and styled building blocks. Every view that
//! draws chrome takes its sizes and popover/icon-button styling from here.

use gpui::{App, Div, ElementId, ParentElement, Stateful, Styled, div, prelude::*, px, svg};

use crate::AppState;
use crate::layout::color;

/// Spacing unit: paddings and gaps are multiples of it.
pub const GRID: f32 = 4.0;
pub const TITLE_BAR_HEIGHT: f32 = 34.0;
pub const STATUS_BAR_HEIGHT: f32 = 26.0;
pub const TAB_HEIGHT: f32 = 32.0;
/// Height of a file tree row, and of the Explorer header.
pub const TREE_ROW_HEIGHT: f32 = 24.0;
pub const POPOVER_MAX_WIDTH: f32 = 600.0;
pub const POPOVER_RADIUS: f32 = 8.0;
/// UI text size; the UI uses the system font, the editor its own.
pub const UI_FONT_SIZE: f32 = 13.0;
/// The system UI font in GPUI.
pub const UI_FONT_FAMILY: &str = ".SystemUIFont";

/// Two overlapping pages.
pub const FILES_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="black" stroke-width="1.25" stroke-linejoin="round"><path d="M5.5 4V1.5H10L13.5 5v7h-3"/><path d="M2.5 4.5H7L10.5 8v6.5h-8Z"/></svg>"#;

/// A magnifying glass.
pub const SEARCH_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="black" stroke-width="1.25" stroke-linecap="round"><circle cx="6.75" cy="6.75" r="4.5"/><path d="M10.25 10.25 14 14"/></svg>"#;

/// A thin ×, for closing tabs.
pub const CLOSE_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round"><path d="M4 4l8 8M12 4l-8 8"/></svg>"#;

/// The floating container for Quick Open, the theme picker and the find
/// widget: up to [`POPOVER_MAX_WIDTH`] wide, rounded, bordered, shadowed,
/// on `elevated_surface.background`. Place it with [`popover_overlay`].
pub fn popover(cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .flex()
        .flex_col()
        .w_full()
        .max_w(px(POPOVER_MAX_WIDTH))
        .overflow_hidden()
        .rounded(px(POPOVER_RADIUS))
        .border_1()
        .border_color(color(ui.border))
        .bg(color(ui.elevated_surface_background))
        .text_color(color(ui.text))
        .shadow_lg()
}

/// Covers its parent and holds `popover` at the top centre, 24 px below
/// the title bar. Give it a `relative` parent the size of the window.
pub fn popover_overlay(popover: impl IntoElement) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .pt(px(TITLE_BAR_HEIGHT + 6. * GRID))
        .px(px(4. * GRID))
        .child(popover)
}

/// A 24×20 icon button: `active` gets the `element.selected` background
/// and `icon` color, otherwise `icon.muted`; hover uses
/// `ghost_element.hover`. `svg_data` is inline SVG such as [`FILES_ICON`];
/// add `on_click` to the result.
pub fn icon_button(
    id: impl Into<ElementId>,
    svg_data: &'static [u8],
    active: bool,
    cx: &App,
) -> Stateful<Div> {
    let ui = &cx.global::<AppState>().theme.ui;
    let hover = color(ui.ghost_element_hover);
    div()
        .id(id)
        .flex()
        .flex_none()
        .justify_center()
        .items_center()
        .w(px(6. * GRID))
        .h(px(5. * GRID))
        .rounded(px(GRID))
        .cursor_pointer()
        .when(active, |d| d.bg(color(ui.element_selected)))
        .when(!active, |d| d.hover(move |s| s.bg(hover)))
        .child(
            svg()
                .data(svg_data)
                .size(px(14.))
                .text_color(color(if active { ui.icon } else { ui.icon_muted })),
        )
}
