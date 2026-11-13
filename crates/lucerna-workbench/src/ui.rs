//! Shared chrome metrics, icons and styled building blocks. Every view that
//! draws chrome takes its sizes and popover/icon-button styling from here.

use gpui::{
    AnyElement, AnyView, App, Axis, ClickEvent, ClipboardItem, Context, Div, ElementId, Entity,
    Hsla, IntoElement, MouseButton, ParentElement, Pixels, Render, SharedString, Stateful, Styled,
    Window, div, img, prelude::*, px, relative, svg,
};
use lucerna_theme::{IconSource, Rgba};

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

/// A chevron pointing up.
pub const CHEVRON_UP_ICON: &[u8] = icons::CHEVRON_UP;

/// A chevron pointing left: previous match.
pub const CHEVRON_LEFT_ICON: &[u8] = icons::CHEVRON_LEFT;

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

/// A section title for a page or list: `title` in capitals (caption size,
/// editor font, `text.muted`), then a 1 px `border.variant` rule filling the
/// rest of the row.
pub fn section_header(title: &str, cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .flex()
        .items_center()
        .gap(px(2. * GRID))
        .w_full()
        .child(
            div()
                .flex_none()
                .font_family(crate::fonts::editor_font_family(None, cx))
                .text_size(px(UI_FONT_SIZE_CAPTION))
                .text_color(color(ui.text_muted))
                .child(title.to_uppercase()),
        )
        .child(div().flex_1().h(px(1.)).bg(color(ui.border_variant)))
}

/// A full-width button row 7 grid units tall: a 14 px `icon` in
/// `icon.muted`, then `label` (cut off when long), then `keys` at the right
/// when given. `focused` takes the `element.selected` background; otherwise
/// the row shows `ghost_element.hover` on hover. It only builds the row:
/// add `on_click`, and any further children (a description, a remove
/// button) after the label. A row that adds its own children passes
/// `keys: None` and draws its shortcut itself, so the order stays name,
/// description, shortcut, button.
pub fn list_button(
    id: impl Into<ElementId>,
    icon: &'static [u8],
    label: SharedString,
    keys: Option<String>,
    focused: bool,
    cx: &App,
) -> Stateful<Div> {
    let ui = &cx.global::<AppState>().theme.ui;
    let hover = color(ui.ghost_element_hover);
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2. * GRID))
        .w_full()
        .h(px(7. * GRID))
        .px(px(2. * GRID))
        .rounded(px(GRID))
        .overflow_hidden()
        .cursor_pointer()
        .text_color(color(ui.text))
        .when(focused, |row| row.bg(color(ui.element_selected)))
        .when(!focused, |row| row.hover(move |s| s.bg(hover)))
        .child(
            svg()
                .data(icon)
                .size(px(FILE_ICON_SIZE))
                .flex_none()
                .text_color(color(ui.icon_muted)),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(UI_FONT_SIZE))
                .child(label),
        )
        .when_some(keys, |row, keys| {
            row.child(
                div()
                    .flex_none()
                    .ml_auto()
                    .text_size(px(UI_FONT_SIZE_STATUS))
                    .text_color(color(ui.text_muted))
                    .child(keys),
            )
        })
}

/// A tooltip showing `text`, for an element's `tooltip`: small text on
/// `elevated_surface.background`, bordered like a [`popover`].
pub fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text = text.into();
    move |_, cx| {
        let text = text.clone();
        cx.new(|_| Tooltip { text }).into()
    }
}

struct Tooltip {
    text: SharedString,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        tooltip_box(cx).child(self.text.clone())
    }
}

/// The box a [`tooltip`] draws, for tooltips placed by hand (the
/// terminal's link tooltip): add the text as children.
pub fn tooltip_box(cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .px(px(2. * GRID))
        .py(px(GRID))
        .rounded(px(GRID))
        .border_1()
        .border_color(color(ui.border))
        .bg(color(ui.elevated_surface_background))
        .text_size(px(UI_FONT_SIZE_STATUS))
        .text_color(color(ui.text))
        .shadow_md()
}

/// A small bordered text button, 24 px tall; its id and debug selector are
/// `id`. Clicking it leaves focus where it was. Add `on_click`.
pub fn text_button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Stateful<Div> {
    styled_text_button(id.into(), label.into(), false, cx)
}

/// A [`text_button`] for the main action of a dialog: on
/// `element.selected` (`element.hover` on hover), with the stronger
/// `border` color.
pub fn primary_text_button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Stateful<Div> {
    styled_text_button(id.into(), label.into(), true, cx)
}

fn styled_text_button(
    id: SharedString,
    label: SharedString,
    primary: bool,
    cx: &App,
) -> Stateful<Div> {
    let ui = &cx.global::<AppState>().theme.ui;
    let (background, hover, border) = if primary {
        (ui.element_selected, ui.element_hover, ui.border)
    } else {
        (
            ui.editor_background,
            ui.ghost_element_hover,
            ui.border_variant,
        )
    };
    let hover = color(hover);
    let selector = id.clone();
    div()
        .id(id)
        .debug_selector(move || selector.to_string())
        .flex()
        .flex_none()
        .items_center()
        .h(px(6. * GRID))
        .px(px(2. * GRID))
        .rounded(px(GRID))
        .border_1()
        .border_color(color(border))
        .bg(color(background))
        .text_size(px(UI_FONT_SIZE_SMALL))
        .text_color(color(ui.text))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .hover(move |s| s.bg(hover))
        .child(label)
}

/// A command to copy, such as a language server's install command: the
/// command in `code_font` (caption size, `text.muted`, cut off with an
/// ellipsis; debug selector `text_selector`) and a "Copy" [`text_button`]
/// (id `copy_id`) that puts it on the clipboard. Size and pad the row
/// where it is placed.
pub fn copy_command_row(
    command: String,
    text_selector: String,
    copy_id: String,
    code_font: SharedString,
    cx: &App,
) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    let copied = command.clone();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2. * GRID))
        .child(
            div()
                .debug_selector(move || text_selector)
                .flex_auto()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_family(code_font)
                .text_size(px(UI_FONT_SIZE_CAPTION))
                .text_color(color(ui.text_muted))
                .child(command),
        )
        .child(
            text_button(copy_id, "Copy", cx)
                .tooltip(tooltip("Copy the install command"))
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                }),
        )
}

/// A warning line, such as an invalid `lsp` setting entry: a
/// `warning`-colored triangle and the text, wrapping. Its debug selector
/// is `selector`.
pub fn warning_row(selector: String, text: impl Into<SharedString>, cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .debug_selector(move || selector)
        .flex()
        .flex_row()
        .items_start()
        .gap(px(2. * GRID))
        .child(
            svg()
                .data(icons::TRIANGLE_ALERT)
                .flex_none()
                .mt(px(1.))
                .size(px(UI_FONT_SIZE_SMALL))
                .text_color(color(ui.warning)),
        )
        .child(div().flex_1().min_w_0().child(text.into()))
}

/// A card heading a group of a results list (a file of search results, a
/// language of the Health tab): a row with a 1px `border` on
/// `editor.subheader.background`, small text.
pub fn subheader_card(cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .flex()
        .items_center()
        .rounded_sm()
        .border_1()
        .border_color(color(ui.border))
        .bg(color(ui.editor_subheader_background))
        .text_size(px(UI_FONT_SIZE_SMALL))
}

/// A 14 px checkbox: a bordered box, filled with `element.selected` and
/// holding a check mark while `checked`. Put it in a clickable row with
/// its label.
pub fn checkbox(checked: bool, cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .flex()
        .flex_none()
        .justify_center()
        .items_center()
        .size(px(14.))
        .rounded(px(3.))
        .border_1()
        .border_color(color(ui.border))
        .bg(color(if checked {
            ui.element_selected
        } else {
            ui.editor_background
        }))
        .when(checked, |d| {
            d.child(
                svg()
                    .data(icons::CHECK)
                    .size(px(10.))
                    .text_color(color(ui.icon)),
            )
        })
}

/// Height of a search bar row: the find bar and the Search tab's toolbar.
pub const SEARCH_BAR_HEIGHT: f32 = 10. * GRID;
/// Height of the bordered input box in a search bar row.
pub const SEARCH_INPUT_HEIGHT: f32 = 7. * GRID;
/// The query box never gets narrower than this (Zed's `min_w_32`).
pub const SEARCH_QUERY_MIN_WIDTH: f32 = 32. * GRID;
/// The share of a search bar row the query box asks for, as in Zed.
pub const SEARCH_QUERY_BASIS: f32 = 0.667;

/// A bordered box holding `input`; add any controls after it.
pub(crate) fn input_box(input: &Entity<TextInput>, cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .flex()
        .items_center()
        .gap(px(GRID / 2.))
        .h(px(SEARCH_INPUT_HEIGHT))
        .pl(px(2. * GRID))
        .pr(px(GRID))
        .rounded(px(GRID))
        .border_1()
        .border_color(color(ui.border_variant))
        .bg(color(ui.editor_background))
        .child(div().flex_1().min_w_0().child(input.clone()))
}

/// The search toolbar: a full-width column on `editor.background` with a
/// 1px `border.variant` line under it. Holds [`search_bar_row`]s, as the
/// find bar and the Search tab's toolbar do.
pub fn search_toolbar(cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .flex()
        .flex_none()
        .flex_col()
        .w_full()
        .bg(color(ui.editor_background))
        .border_b_1()
        .border_color(color(ui.border_variant))
}

/// One row of a [`search_toolbar`]: [`SEARCH_BAR_HEIGHT`] tall, its
/// groups a [`GRID`] apart, 2×[`GRID`] in from the sides.
pub fn search_bar_row() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(GRID))
        .h(px(SEARCH_BAR_HEIGHT))
        .px(px(2. * GRID))
        .w_full()
}

/// The query box of a search bar: an [`input_box`] asking for about two
/// thirds of the row (shrinking down to [`SEARCH_QUERY_MIN_WIDTH`]), its
/// border in the theme's `error` color while `invalid` (no match or a bad
/// regex). Add the `Aa` `ab` `.*` toggles as children.
pub(crate) fn search_query_box(input: &Entity<TextInput>, invalid: bool, cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    search_query_column(input_box(input, cx)).when(invalid, |d| d.border_color(color(ui.error)))
}

/// Sizes `el` like the query box of a search bar row: about two thirds
/// of the row, never narrower than [`SEARCH_QUERY_MIN_WIDTH`]. Rows under
/// the query (the Search tab's include / exclude) use it to line up.
pub fn search_query_column<E: Styled>(el: E) -> E {
    el.flex_grow_0()
        .flex_shrink_1()
        .flex_basis(relative(SEARCH_QUERY_BASIS))
        .min_w(px(SEARCH_QUERY_MIN_WIDTH))
}

/// An [`icon_button`] for a bar: its debug selector is `id`, and pressing
/// it leaves focus where it was (in the query input). Add `on_click`.
pub fn bar_icon_button(
    id: &'static str,
    svg_data: &'static [u8],
    active: bool,
    cx: &App,
) -> Stateful<Div> {
    icon_button(id, svg_data, active, cx)
        .debug_selector(move || id.into())
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
}

/// A 1px `border.variant` line between groups of a search bar row.
pub fn bar_separator(cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .flex_none()
        .w(px(1.))
        .h(px(5. * GRID))
        .bg(color(ui.border_variant))
}

pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// The match navigation of a search bar: `‹` (`prev_id`) and `›`
/// (`next_id`) icon buttons, then `label` such as `"3/19"` or `"0/0"`,
/// small and muted. Clicking a button leaves focus where it was.
pub fn match_nav(
    (prev_id, on_prev): (&'static str, ClickHandler),
    (next_id, on_next): (&'static str, ClickHandler),
    label: String,
    cx: &App,
) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    let button = |id: &'static str, icon, handler: ClickHandler| {
        bar_icon_button(id, icon, false, cx).on_click(handler)
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(GRID / 2.))
        .child(button(prev_id, CHEVRON_LEFT_ICON, on_prev))
        .child(button(next_id, CHEVRON_RIGHT_ICON, on_next))
        .child(
            div()
                .flex_none()
                .min_w(px(10. * GRID))
                .pl(px(GRID))
                .whitespace_nowrap()
                .text_size(px(UI_FONT_SIZE_SMALL))
                .text_color(color(ui.text_muted))
                .child(label),
        )
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

/// Thickness of the sidebar's and the editor groups' sashes.
pub const SASH_SIZE: f32 = 4.0;
/// Square slot that holds a tab's close button, kept even while the
/// button is hidden so tabs do not change width on hover.
pub const TAB_CLOSE_SLOT: f32 = 16.0;
/// The unsaved (or conflict) dot shown in a tab's close slot.
pub const TAB_DOT_SIZE: f32 = 6.0;

/// GPUI requires a view to draw under the cursor while dragging; a sash
/// drag draws nothing.
pub(crate) struct SashGhost;

impl Render for SashGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// A draggable divider `thickness` thick, running the full length of its
/// parent across `axis`: with [`Axis::Horizontal`] it moves left and right
/// (a vertical strip, `col-resize` cursor), with [`Axis::Vertical`] up and
/// down (a horizontal strip, `row-resize` cursor). Dragging it carries
/// `payload`; the container that resizes handles `on_drag_move` for that
/// type. Its debug selector is its id. Place it with `absolute` offsets.
pub fn sash<T: 'static>(
    id: &'static str,
    axis: Axis,
    thickness: Pixels,
    payload: T,
) -> Stateful<Div> {
    let sash = div().id(id).debug_selector(move || id.into()).absolute();
    let sash = match axis {
        Axis::Horizontal => sash.w(thickness).h_full().cursor_col_resize(),
        Axis::Vertical => sash.h(thickness).w_full().cursor_row_resize(),
    };
    sash.on_drag(payload, |_, _, _, cx: &mut App| cx.new(|_| SashGhost))
}

/// The tab bar under which [`tab`]s sit: a row [`TAB_HEIGHT`] tall on
/// `tab_bar.background`, scrolling sideways. Add the tabs, then a
/// [`tab_bar_filler`].
pub fn tab_bar(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .id(id)
        .flex()
        .flex_row()
        .flex_none()
        .h(px(TAB_HEIGHT))
        .overflow_x_scroll()
        .bg(color(ui.tab_bar_background))
}

/// The rest of a [`tab_bar`] after its tabs: fills the row and carries the
/// bar's bottom border, which the active tab breaks.
pub fn tab_bar_filler(cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .flex_1()
        .h_full()
        .border_b_1()
        .border_color(color(ui.border_variant))
}

/// The close button of a [`tab`].
pub struct TabClose {
    pub id: ElementId,
    pub selector: String,
    pub on_click: ClickHandler,
}

/// What a [`tab`] shows.
pub struct TabProps {
    pub id: ElementId,
    /// The tab's debug selector.
    pub selector: String,
    /// The active tab takes `editor.background` and opens onto the body
    /// below; the others sit on `tab.inactive_background` over the bar's
    /// bottom border.
    pub active: bool,
    pub title: SharedString,
    pub title_color: Rgba,
    /// Drawn before the title.
    pub icon: Option<AnyElement>,
    /// Italic title: a preview tab.
    pub italic: bool,
    /// Struck-through title, with this debug selector: a deleted file.
    pub strike: Option<String>,
    /// The close button, when shown (on hover, or on the active tab).
    pub close: Option<TabClose>,
    /// A dot of this color (and debug selector) in the close button's slot
    /// while the button is hidden: unsaved changes.
    pub dot: Option<(Rgba, String)>,
}

/// A flat tab, as in Zed: icon, title, then a fixed square slot holding
/// the close button or a dot, so the tab keeps its width on hover. Add
/// `on_click`, `on_hover` and middle-click handling to the result.
pub fn tab(props: TabProps, cx: &App) -> Stateful<Div> {
    let ui = &cx.global::<AppState>().theme.ui;
    let hover = color(ui.ghost_element_hover);
    let TabProps {
        id,
        selector,
        active,
        title,
        title_color,
        icon,
        italic,
        strike,
        close,
        dot,
    } = props;
    let close = close.map(|close| {
        let TabClose {
            id,
            selector,
            on_click,
        } = close;
        div()
            .id(id)
            .debug_selector(move || selector)
            .flex()
            .justify_center()
            .items_center()
            .size_full()
            .rounded(px(GRID))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(move |ev, window, cx| {
                cx.stop_propagation();
                on_click(ev, window, cx);
            })
            .child(
                svg()
                    .data(CLOSE_ICON)
                    .size(px(10.))
                    .text_color(color(ui.icon_muted)),
            )
    });
    let dot = dot.map(|(fill, selector)| {
        div()
            .debug_selector(move || selector)
            .size(px(TAB_DOT_SIZE))
            .rounded_full()
            .bg(color(fill))
    });
    div()
        .id(id)
        .debug_selector(move || selector)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(1.5 * GRID))
        .px(px(TAB_PADDING_X))
        .h_full()
        .border_r_1()
        .border_color(color(ui.border_variant))
        // The bar's bottom border runs under every tab but the active one,
        // which opens onto the body below.
        .when(!active, |d| d.border_b_1())
        .bg(color(if active {
            ui.editor_background
        } else {
            ui.tab_inactive_background
        }))
        .when(!active, |d| d.hover(move |s| s.bg(hover)))
        .text_color(color(title_color))
        .text_size(px(UI_FONT_SIZE_SMALL))
        .cursor_pointer()
        .children(icon)
        .child(
            div()
                .when(italic, |d| d.italic())
                .when_some(strike, |d, selector| {
                    d.line_through().debug_selector(move || selector)
                })
                .child(title),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .justify_center()
                .items_center()
                .size(px(TAB_CLOSE_SLOT))
                .children(close)
                .children(dot),
        )
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
