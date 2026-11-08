//! Layered JSONC settings for Lucerna.

mod error;
mod keymap;
mod store;
mod when;

pub use error::SettingsError;
pub use keymap::{KeyBinding, Keymap, Keystroke, Resolution, parse_keys};
pub use store::{Layer, SettingsStore};
pub use when::{Context, ContextValue, WhenExpr, parse_when};
