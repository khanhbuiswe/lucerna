//! Layered JSONC settings for Lucerna.

mod error;
mod keymap;
mod scope;
mod store;
mod when;

pub use error::SettingsError;
pub use keymap::{KeyBinding, Keymap, Keystroke, Resolution, parse_keys};
pub use scope::is_app_scope;
pub use store::{Layer, SettingsStore, SettingsView, WorkspaceLayer};
pub use when::{Context, ContextValue, WhenExpr, parse_when};
