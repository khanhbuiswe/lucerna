//! The panel under the editor area, or right of it, as in Zed's bottom and
//! right docks: a tab bar over a body, a sash on the edge facing the
//! editor, maximize and hide. The terminal is its only content.

use gpui::{
    Axis, Context, Div, Entity, ParentElement, Pixels, Stateful, Styled, div, prelude::*, px,
};
use serde::{Deserialize, Serialize};

use super::color;
use crate::icons;
use crate::terminal::TerminalPanel;
use crate::terminal::panel::tab_elements;
use crate::ui::{GRID, TAB_HEIGHT, bar_icon_button, sash, tab_bar, tab_bar_filler, tooltip};
use crate::{AppState, CommandRegistry, Workbench};

/// Height of the bottom panel and width of the right one, until resized.
pub(crate) const DEFAULT_BOTTOM_HEIGHT: f32 = 320.;
pub(crate) const DEFAULT_RIGHT_WIDTH: f32 = 640.;
/// The smallest panel: the tab bar and about four terminal rows tall, or
/// about twenty columns wide.
pub(crate) const MIN_HEIGHT: f32 = TAB_HEIGHT + 4. * 20.;
pub(crate) const MIN_WIDTH: f32 = 20. * 10.;
/// Resizing the panel never leaves the editor area smaller than this.
pub(crate) const EDITOR_MIN_SIZE: f32 = 120.;
/// The sash straddles the border facing the editor.
const SASH_THICKNESS: f32 = 6.;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum PanelPosition {
    #[default]
    Bottom,
    Right,
}

impl PanelPosition {
    /// `workbench.panel.defaultLocation`: `"bottom"` (the default) or
    /// `"right"`.
    pub(crate) fn from_setting(settings: lucerna_settings::SettingsView<'_>) -> Self {
        let value = settings.get::<String>("workbench.panel.defaultLocation", None);
        match value.as_deref() {
            Some("right") => PanelPosition::Right,
            _ => PanelPosition::Bottom,
        }
    }
}

/// Where the panel is, how big in each position, and whether it shows;
/// kept in the workspace state. A field missing from the file takes its
/// default.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct PanelState {
    pub visible: bool,
    pub position: PanelPosition,
    pub bottom_height: f32,
    pub right_width: f32,
    pub maximized: bool,
}

impl Default for PanelState {
    fn default() -> Self {
        PanelState {
            visible: false,
            position: PanelPosition::Bottom,
            bottom_height: DEFAULT_BOTTOM_HEIGHT,
            right_width: DEFAULT_RIGHT_WIDTH,
            maximized: false,
        }
    }
}

impl PanelState {
    /// Sizes from a state file made usable: a size that is not a number
    /// goes back to the default, a small one up to the minimum.
    pub(crate) fn sanitized(mut self) -> Self {
        let fix = |v: f32, default: f32, min: f32| {
            if v.is_finite() { v.max(min) } else { default }
        };
        self.bottom_height = fix(self.bottom_height, DEFAULT_BOTTOM_HEIGHT, MIN_HEIGHT);
        self.right_width = fix(self.right_width, DEFAULT_RIGHT_WIDTH, MIN_WIDTH);
        self
    }
}

/// `size` kept between the position's minimum and what leaves the editor
/// area [`EDITOR_MIN_SIZE`] of `available` (the editor area plus panel).
/// The editor's share wins when both cannot hold.
pub(crate) fn clamp_size(position: PanelPosition, size: Pixels, available: Pixels) -> f32 {
    let min = match position {
        PanelPosition::Bottom => MIN_HEIGHT,
        PanelPosition::Right => MIN_WIDTH,
    };
    let max = f32::from(available) - EDITOR_MIN_SIZE;
    f32::from(size).max(min).min(max).max(0.)
}

/// Drag payload while the panel's sash is dragged.
pub(crate) struct PanelSash;

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.togglePanel",
        "View: Toggle Panel Visibility",
        |wb, _, window, cx| wb.toggle_panel(window, cx),
    );
    commands.register(
        "workbench.action.toggleMaximizedPanel",
        "View: Toggle Maximized Panel",
        |wb, _, window, cx| wb.toggle_maximized_panel(window, cx),
    );
    commands.register(
        "workbench.action.positionPanelBottom",
        "View: Move Panel To Bottom",
        |wb, _, _, cx| wb.set_panel_position(PanelPosition::Bottom, cx),
    );
    commands.register(
        "workbench.action.positionPanelRight",
        "View: Move Panel Right",
        |wb, _, _, cx| wb.set_panel_position(PanelPosition::Right, cx),
    );
}

/// The panel (debug selector `terminal-panel`): sized by `state` unless
/// maximized, when it takes the editor area's place. Its focus is the
/// terminal panel's, outside the editor area's.
pub(crate) fn render(
    state: &PanelState,
    panel: &Entity<TerminalPanel>,
    cx: &mut Context<Workbench>,
) -> Stateful<Div> {
    let ui = &cx.global::<AppState>().theme.ui;
    let (background, border, bar_background) = (
        color(ui.panel_background),
        color(ui.border),
        color(ui.tab_bar_background),
    );
    let border_variant = color(ui.border_variant);
    let bottom = state.position == PanelPosition::Bottom;
    let maximized = state.maximized;
    let focus = panel.read(cx).focus_handle().clone();

    let sash = (!maximized).then(|| {
        let offset = px(-SASH_THICKNESS / 2.);
        if bottom {
            sash(
                "terminal-panel-sash",
                Axis::Vertical,
                px(SASH_THICKNESS),
                PanelSash,
            )
            .top(offset)
            .left_0()
        } else {
            sash(
                "terminal-panel-sash",
                Axis::Horizontal,
                px(SASH_THICKNESS),
                PanelSash,
            )
            .left(offset)
            .top_0()
        }
    });
    let maximize_icon = if maximized {
        icons::MINIMIZE_2
    } else {
        icons::MAXIMIZE_2
    };
    let buttons = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(GRID / 2.))
        .px(px(GRID))
        .h_full()
        .border_b_1()
        .border_color(border_variant)
        .child(
            bar_icon_button("terminal-new", icons::PLUS, false, cx)
                .tooltip(tooltip("New Terminal"))
                .on_click(cx.listener(|wb, _, window, cx| wb.new_terminal_tab(window, cx))),
        )
        .child(
            bar_icon_button("terminal-maximize", maximize_icon, false, cx)
                .tooltip(tooltip(if maximized {
                    "Restore Panel Size"
                } else {
                    "Maximize Panel Size"
                }))
                .on_click(cx.listener(|wb, _, window, cx| {
                    wb.run_command("workbench.action.toggleMaximizedPanel", None, window, cx);
                })),
        )
        .child(
            bar_icon_button("terminal-hide", icons::X, false, cx)
                .tooltip(tooltip("Hide Panel"))
                .on_click(cx.listener(|wb, _, window, cx| wb.hide_panel(window, cx))),
        );
    let bar = div()
        .debug_selector(|| "terminal-tab-bar".into())
        .flex()
        .flex_row()
        .flex_none()
        .w_full()
        .h(px(TAB_HEIGHT))
        .bg(bar_background)
        .child(
            tab_bar("terminal-tabs", cx)
                .flex_1()
                .min_w_0()
                .children(tab_elements(panel, cx))
                .child(tab_bar_filler(cx)),
        )
        .child(buttons);

    let frame = div()
        .id("terminal-panel")
        .debug_selector(|| "terminal-panel".into())
        .track_focus(&focus)
        .relative()
        .flex()
        .flex_col()
        .bg(background)
        .border_color(border);
    let frame = match (maximized, bottom) {
        (true, _) => frame.flex_1().min_w_0().min_h_0(),
        (false, true) => frame
            .flex_grow_0()
            .flex_shrink_1()
            .w_full()
            .h(px(state.bottom_height))
            .border_t_1(),
        (false, false) => frame
            .flex_grow_0()
            .flex_shrink_1()
            .h_full()
            .w(px(state.right_width))
            .border_l_1(),
    };
    frame
        .child(bar)
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(panel.clone()),
        )
        .children(sash)
}
