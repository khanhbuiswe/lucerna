//! Shared chrome metrics, icons and styled building blocks. Every view that
//! draws chrome takes its sizes and popover/icon-button styling from here.

use gpui::{
    AnyElement, App, Div, ElementId, Entity, Hsla, MouseButton, ParentElement, Pixels, Stateful,
    Styled, div, img, prelude::*, px, svg,
};
use lucerna_theme::IconSource;

use crate::AppState;
use crate::icons;
use crate::layout::color;
use crate::text_input::TextInput;

/// Spacing unit: paddings and gaps are multiples of it.
pub const GRID: f32 = 4.0;
pub const TITLE_BAR_HEIGHT: f32 = 34.0;
pub const STATUS_BAR_HEIGHT: f32 = 26.0;
pub const TAB_HEIGHT: f32 = 32.0;
/// Horizontal padding inside a tab.
pub const TAB_PADDING_X: f32 = 2.5 * GRID;
/// The bar under each tab bar showing the active file's path.
pub const PATH_BAR_HEIGHT: f32 = TAB_HEIGHT - 2. * GRID;
/// Height of a file tree row, and of the Explorer root row.
pub const TREE_ROW_HEIGHT: f32 = 24.0;
/// Width of the thin overlay scrollbar thumb.
pub const SCROLLBAR_WIDTH: f32 = 6.0;
/// The scrollbar thumb never gets shorter than this.
pub const MIN_THUMB_HEIGHT: f32 = 6. * GRID;
pub const POPOVER_MAX_WIDTH: f32 = 600.0;
pub const POPOVER_RADIUS: f32 = 8.0;
/// UI text size; the UI uses IBM Plex Sans, the editor its own font.
pub const UI_FONT_SIZE: f32 = 14.0;
/// Smaller UI text: tab titles, the path bar and small headers.
pub const UI_FONT_SIZE_SMALL: f32 = 13.0;
/// Status bar text: one step below [`UI_FONT_SIZE_SMALL`], as in Zed.
pub const UI_FONT_SIZE_STATUS: f32 = 12.0;
/// Captions smaller than [`UI_FONT_SIZE_SMALL`]: the search panel's field
/// labels, folder hints and match counts, and toggle labels like `Aa`.
pub const UI_FONT_SIZE_CAPTION: f32 = 11.0;
/// The bundled UI font (see [`crate::fonts`]).
pub const UI_FONT_FAMILY: &str = crate::fonts::UI_FONT_FAMILY;

/// Two overlapping pages: the Explorer toggle.
pub const FILES_ICON: &[u8] = icons::FILES;

/// A magnifying glass.
pub const SEARCH_ICON: &[u8] = icons::SEARCH;

/// Four blocks, one apart: the Extensions toggle.
pub const BLOCKS_ICON: &[u8] = icons::BLOCKS;

/// A thin ×, for closing tabs.
pub const CLOSE_ICON: &[u8] = icons::X;

/// A chevron pointing right: a collapsed directory.
pub const CHEVRON_RIGHT_ICON: &[u8] = icons::CHEVRON_RIGHT;

/// A chevron pointing down: an expanded directory.
pub const CHEVRON_DOWN_ICON: &[u8] = icons::CHEVRON_DOWN;

/// A chevron pointing up: previous match.
pub const CHEVRON_UP_ICON: &[u8] = icons::CHEVRON_UP;

/// A single page with a folded corner.
pub const FILE_ICON: &[u8] = icons::FILE;

/// Size of a file or folder icon in a tree row.
pub const FILE_ICON_SIZE: f32 = 14.0;

/// An icon from an icon theme at [`FILE_ICON_SIZE`]; see
/// [`file_icon_sized`].
pub fn file_icon(source: &IconSource, color: Hsla) -> AnyElement {
    file_icon_sized(source, px(FILE_ICON_SIZE), color)
}

/// An icon from an icon theme, `size` square. A bundled Lucide icon is an
/// `svg` filled with `color`; an extension's icon is an `img` of its file,
/// which keeps the colors it was drawn with. GPUI's `img` loads SVG files
/// itself (rasterised at 1x by its SVG renderer, then scaled to `size`).
pub fn file_icon_sized(source: &IconSource, size: Pixels, color: Hsla) -> AnyElement {
    match source {
        IconSource::Builtin(name) => match icons::lucide_svg(name) {
            Some(data) => svg()
                .data(data)
                .size(size)
                .text_color(color)
                .into_any_element(),
            None => div().size(size).into_any_element(),
        },
        IconSource::File(path) => img(path.clone()).size(size).into_any_element(),
    }
}

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

/// A bordered box holding `input`; add any controls after it.
pub(crate) fn input_box(input: &Entity<TextInput>, cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .flex()
        .items_center()
        .gap(px(GRID / 2.))
        .h(px(7. * GRID))
        .pl(px(2. * GRID))
        .pr(px(GRID))
        .rounded(px(GRID))
        .border_1()
        .border_color(color(ui.border_variant))
        .bg(color(ui.editor_background))
        .child(div().flex_1().min_w_0().child(input.clone()))
}

/// A 24×20 text toggle such as `Aa`, `ab` or `.*`, styled like
/// [`icon_button`]: `element.selected` while `active`. Clicking it leaves
/// focus where it was (in the input it belongs to). Its debug selector is
/// `id`; add `on_click` to the result.
pub fn toggle_button(
    id: &'static str,
    label: &'static str,
    active: bool,
    cx: &App,
) -> Stateful<Div> {
    let ui = &cx.global::<AppState>().theme.ui;
    let hover = color(ui.ghost_element_hover);
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .flex_none()
        .justify_center()
        .items_center()
        .w(px(6. * GRID))
        .h(px(5. * GRID))
        .rounded(px(GRID))
        .cursor_pointer()
        // Keeps a focusable ancestor (the sidebar) from taking focus.
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .text_size(px(UI_FONT_SIZE_CAPTION))
        .when(active, |d| {
            d.bg(color(ui.element_selected)).text_color(color(ui.icon))
        })
        .when(!active, |d| {
            d.text_color(color(ui.icon_muted))
                .hover(move |s| s.bg(hover))
        })
        .child(label)
}

/// Top and height of a scrollbar thumb inside a `viewport` tall track
/// scrolled `scroll_top` into `content` (all in the same unit), or `None`
/// when nothing overflows. The thumb is at least [`MIN_THUMB_HEIGHT`].
pub(crate) fn thumb_geometry(viewport: f32, content: f32, scroll_top: f32) -> Option<(f32, f32)> {
    if content <= viewport || viewport <= 0. {
        return None;
    }
    let height = (viewport * viewport / content)
        .max(MIN_THUMB_HEIGHT)
        .min(viewport);
    let max_scroll = content - viewport;
    let top = (viewport - height) * (scroll_top / max_scroll).clamp(0., 1.);
    Some((top, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_thumb_without_overflow() {
        assert_eq!(thumb_geometry(100., 100., 0.), None);
        assert_eq!(thumb_geometry(100., 40., 0.), None);
    }

    #[test]
    fn thumb_tracks_scroll_and_keeps_a_minimum() {
        // Half the content visible: half-height thumb.
        assert_eq!(thumb_geometry(200., 400., 0.), Some((0., 100.)));
        assert_eq!(thumb_geometry(200., 400., 200.), Some((100., 100.)));
        let (top, height) = thumb_geometry(100., 100_000., 99_900.).unwrap();
        assert_eq!(height, MIN_THUMB_HEIGHT);
        assert_eq!(top + height, 100.);
    }
}
