//! Extension support shared by the app and the `luc ext` CLI.

mod commands;
mod page;
pub mod panel_model;
pub(crate) mod service;
mod toast;
mod validator;

use std::path::PathBuf;

pub(crate) use commands::{installable_ids, register_commands};
pub use page::{ExtensionsPage, FILTER_LABELS};
pub use service::{
    DESCRIBE_RETRY, ExtensionService, MAX_DESCRIBES, Registries, STARTUP_REFRESH_DELAY,
    extension_languages, rebuild_registries,
};
pub(crate) use toast::render as toast_element;
pub use toast::{TOAST_DURATION, Toast};
pub use validator::WasmGrammarValidator;

/// Environment variable that overrides the extensions directory (tests).
pub const EXTENSIONS_DIR_ENV: &str = "LUCERNA_EXTENSIONS_DIR";

/// `~/Library/Application Support/Lucerna/extensions`, or the value of
/// `LUCERNA_EXTENSIONS_DIR` when set and non-empty.
pub fn extensions_root() -> PathBuf {
    extensions_root_with(std::env::var_os(EXTENSIONS_DIR_ENV))
}

fn extensions_root_with(override_dir: Option<std::ffi::OsString>) -> PathBuf {
    if let Some(dir) = override_dir.filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    crate::default_data_dir()
        .unwrap_or_else(|| PathBuf::from(".lucerna"))
        .join("extensions")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins() {
        assert_eq!(
            extensions_root_with(Some("/tmp/x".into())),
            PathBuf::from("/tmp/x")
        );
    }

    #[test]
    fn default_ends_in_extensions() {
        let p = extensions_root_with(None);
        assert!(p.ends_with("extensions"), "{}", p.display());
    }

    #[test]
    fn empty_override_is_ignored() {
        let p = extensions_root_with(Some("".into()));
        assert!(p.ends_with("extensions"));
    }
}
