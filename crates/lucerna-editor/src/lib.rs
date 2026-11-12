//! Editing core: selections, edit plans and the undo-stop rule. Pure logic
//! over `lucerna-text`; no UI dependencies.

pub mod clipboard;
pub mod commands;
mod motion;
mod plan;
mod selection;
#[cfg(any(test, feature = "test-util"))]
pub mod test_util;
mod undo_stops;
pub mod words;

pub use motion::{Layout, Motion, PlainLayout, move_selections};
pub use plan::{AutoClosing, AutoClosingEdit, AutoIndent, AutoSurround, Ctx, EditConfig, EditPlan};
pub use selection::{Goal, Selection, SelectionSet};
pub use undo_stops::{EditKind, should_stop_between, typing_kind};
pub mod autoclose;
mod indent;
pub mod indentation;
