//! Which settings belong to the app rather than to a window.

/// Settings read from the Default and User layers only, as VS Code's
/// `scope: application`: a value in a folder's `.lucerna/settings.json`
/// is ignored. Theme, icon theme, startup editor, `files.hotExit`,
/// `window.*` and `extensions.*`.
pub fn is_app_scope(key: &str) -> bool {
    matches!(
        key,
        "workbench.colorTheme"
            | "workbench.preferredDarkColorTheme"
            | "workbench.preferredLightColorTheme"
            | "workbench.iconTheme"
            | "workbench.startupEditor"
            | "files.hotExit"
    ) || key.starts_with("window.")
        || key.starts_with("extensions.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_scope_keys() {
        for key in [
            "workbench.colorTheme",
            "workbench.preferredDarkColorTheme",
            "workbench.preferredLightColorTheme",
            "workbench.iconTheme",
            "workbench.startupEditor",
            "files.hotExit",
            "window.restoreWindows",
            "window.autoDetectColorScheme",
            "extensions.autoCheckUpdates",
        ] {
            assert!(is_app_scope(key), "{key}");
        }
        for key in [
            "editor.tabSize",
            "files.associations",
            "workbench.editor.showIcons",
            "window",
            "windowless.x",
        ] {
            assert!(!is_app_scope(key), "{key}");
        }
    }
}
