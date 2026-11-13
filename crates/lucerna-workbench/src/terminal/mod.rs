//! The integrated terminal: the panel's tabs and, per tab, the view of
//! one `lucerna_terminal::Terminal`.

pub(crate) mod commands;
pub(crate) mod element;
pub(crate) mod panel;
pub(crate) mod settings;
pub(crate) mod view;

pub use commands::{DEFAULT_COMMANDS_TO_SKIP_SHELL, skips_shell};
pub use panel::{PanelEvent, TabState, TerminalPanel, TerminalTab, tab_title};
pub use settings::{TerminalFont, terminal_font};
pub use view::{TerminalSpawner, TerminalView, TerminalViewEvent, set_default_terminal_spawner};
