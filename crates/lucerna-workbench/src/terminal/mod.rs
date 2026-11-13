//! The integrated terminal: the panel's tabs and, per tab, the view of
//! one `lucerna_terminal::Terminal`.

pub(crate) mod panel;
pub(crate) mod view;

pub use panel::{PanelEvent, TabState, TerminalPanel, TerminalTab, tab_title};
pub use view::TerminalView;
