//! The shell's fixed regions. Each module renders one region from state the
//! [`Workbench`](crate::Workbench) owns.

pub(crate) mod activity_bar;
pub(crate) mod sidebar;
pub(crate) mod status_bar;

use gpui::{Pixels, px};

pub(crate) const ACTIVITY_BAR_WIDTH: Pixels = px(48.);
pub(crate) const SIDEBAR_DEFAULT_WIDTH: Pixels = px(260.);
pub(crate) const SIDEBAR_MIN_WIDTH: Pixels = px(160.);
pub(crate) const SIDEBAR_MAX_WIDTH: Pixels = px(600.);
pub(crate) const STATUS_BAR_HEIGHT: Pixels = px(22.);

/// Which end of the status bar an item sits at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusBarSide {
    Left,
    Right,
}

/// The single conversion from theme colors to GPUI colors.
pub(crate) fn color(c: lucerna_theme::Rgba) -> gpui::Rgba {
    gpui::Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}
