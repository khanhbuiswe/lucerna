//! Layered JSONC settings for Lucerna.

mod error;
mod store;

pub use error::SettingsError;
pub use store::{Layer, SettingsStore};
