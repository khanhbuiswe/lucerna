//! Lucerna's integrated terminal core: a PTY, the VT parser and a snapshot of
//! the visible screen. No gpui and no alacritty type in the public API.

mod color;
mod events;
mod input;
mod mouse;
mod options;
mod runs;
mod snapshot;
mod terminal;

pub use color::{Rgba8, TerminalPalette, contrast_ratio, ensure_contrast, resolve_color};
pub use events::{Batch, MAX_BATCH_EVENTS, TerminalEvent};
pub use input::{KeyInput, key_to_bytes};
pub use mouse::{MouseAction, MouseButtonKind, alternate_scroll, mouse_report};
pub use options::{MAX_SCROLLBACK, TerminalOptions, build_env, resolve_shell};
pub use runs::{BgRect, BlockRect, LineLayout, RunStyle, TextRun, Underline, layout_line};
pub use snapshot::{
    Cell, CellFlags, CursorShape, CursorState, GridPoint, Modes, NamedColor, Scroll,
    SelectionRange, Snapshot, TermColor,
};
pub use terminal::Terminal;
