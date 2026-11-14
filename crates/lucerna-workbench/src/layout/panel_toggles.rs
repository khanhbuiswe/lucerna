//! The Explorer, Search, Extensions and Terminal buttons at the status
//! bar's left end. They replace the activity bar: the Explorer button shows
//! or hides the sidebar. The Search button opens (or focuses) the Search
//! tab. The Extensions button opens (or focuses) the Extensions tab, shows
//! the number of available updates and spins while an install runs. The
//! Terminal button runs ⌃` (`workbench.action.terminal.toggleTerminal`).

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, Context, IntoElement, MouseButton, ParentElement, Styled,
    Transformation, div, percentage, prelude::*, px, svg,
};

use crate::extensions::ExtensionService;
use crate::icons::{LOADER_CIRCLE, TERMINAL};
use crate::layout::color;
use crate::ui::{BLOCKS_ICON, FILES_ICON, GRID, SEARCH_ICON, icon_button, tooltip};
use crate::{AppState, Workbench};

/// `search_active`: the Search tab is the focused group's active tab;
/// `extensions_open`: the Extensions tab is open; `panel_visible`: the
/// panel shows (each button shows active).
pub(crate) fn render(
    sidebar_visible: bool,
    search_active: bool,
    extensions_open: bool,
    panel_visible: bool,
    badge: Option<String>,
    cx: &mut Context<Workbench>,
) -> impl IntoElement {
    let explorer = icon_button("panel-toggle-explorer", FILES_ICON, sidebar_visible, cx)
        .debug_selector(|| "panel-toggle-explorer".into())
        .on_click(cx.listener(|wb, _, window, cx| wb.explorer_toggle_clicked(window, cx)));
    let search = icon_button("panel-toggle-search", SEARCH_ICON, search_active, cx)
        .debug_selector(|| "panel-toggle-search".into())
        .on_click(cx.listener(|wb, _, window, cx| {
            wb.run_command("workbench.action.findInFiles", None, window, cx);
        }));
    let extensions = icon_button("panel-toggle-extensions", BLOCKS_ICON, extensions_open, cx)
        .debug_selector(|| "panel-toggle-extensions".into())
        .on_click(cx.listener(|wb, _, window, cx| {
            wb.run_command("workbench.view.extensions", None, window, cx);
        }));
    let terminal = icon_button("panel-toggle-terminal", TERMINAL, panel_visible, cx)
        .debug_selector(|| "panel-toggle-terminal".into())
        .tooltip(tooltip("Terminal (⌃`)"))
        // Keeps the focus where it is, so a click with the terminal
        // focused hides it, as ⌃` does.
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(cx.listener(|wb, _, window, cx| {
            wb.run_command("workbench.action.terminal.toggleTerminal", None, window, cx);
        }));
    // Marks the active state for tests, which cannot read colors.
    let terminal = div()
        .when(panel_visible, |d| {
            d.debug_selector(|| "panel-toggle-terminal-active".into())
        })
        .child(terminal);
    let ui = &cx.global::<AppState>().theme.ui;
    let (badge_bg, badge_fg, icon_muted) = (
        color(ui.element_selected),
        color(ui.text),
        color(ui.icon_muted),
    );
    let badge = badge.map(|count| {
        div()
            .debug_selector(|| "panel-toggle-extensions-badge".into())
            .absolute()
            .top(px(-GRID / 2.))
            .right(px(-GRID / 2.))
            .min_w(px(3. * GRID))
            .h(px(3. * GRID))
            .px(px(GRID / 2.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(badge_bg)
            .text_color(badge_fg)
            .text_size(px(9.))
            .child(count)
    });
    let installing = cx
        .try_global::<ExtensionService>()
        .is_some_and(ExtensionService::any_installing);
    let spinner = installing.then(|| {
        div()
            .debug_selector(|| "panel-toggle-extensions-spinner".into())
            .absolute()
            .bottom(px(-GRID / 2.))
            .right(px(-GRID / 2.))
            .child(
                svg()
                    .data(LOADER_CIRCLE)
                    .size(px(10.))
                    .text_color(icon_muted)
                    .with_animation(
                        "extensions-spinner",
                        Animation::new(Duration::from_secs(1)).repeat(),
                        |svg, delta| {
                            svg.with_transformation(Transformation::rotate(percentage(delta)))
                        },
                    ),
            )
    });
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap_0p5()
        .child(explorer)
        .child(search)
        .child(
            div()
                .relative()
                .child(extensions)
                .children(badge)
                .children(spinner),
        )
        .child(terminal)
}
