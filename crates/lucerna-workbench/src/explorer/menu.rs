//! The Explorer's context menu: which entries a row of each kind shows.

use gpui::prelude::FluentBuilder as _;
use gpui::{Context, FocusHandle};
use serde_json::{Value, json};

use super::model::SelectedKind;
use crate::ui::ContextMenu;

/// What the OS file manager is called, as VS Code labels "Reveal".
const REVEAL_LABEL: &str = if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else if cfg!(target_os = "windows") {
    "Reveal in File Explorer"
} else {
    "Open Containing Folder"
};

/// The palette title of `revealFileInOS`: [`REVEAL_LABEL`] under "File".
pub(super) const REVEAL_TITLE: &str = if cfg!(target_os = "macos") {
    "File: Reveal in Finder"
} else if cfg!(target_os = "windows") {
    "File: Reveal in File Explorer"
} else {
    "File: Open Containing Folder"
};

/// The menu for a resource of `kind`, in the order of the spec's table.
/// Entries that do not apply are left out. `single_file` marks the lone-file
/// mode, which never shows the entries that edit the tree. `path` is the
/// root-relative row the menu is for (empty for the root): every entry
/// carries it as `args.path`, so a command never falls back to whatever is
/// selected when it runs (the row may vanish while the menu is open).
pub(super) fn build_menu(
    kind: SelectedKind,
    single_file: bool,
    path: String,
    owner: FocusHandle,
    cx: &mut Context<ContextMenu>,
) -> ContextMenu {
    let is_file = kind == SelectedKind::File;
    let args: Value = json!({ "path": path });
    let entry = |m: ContextMenu, label: &'static str, command: &'static str| {
        m.entry_with_args(label, command, args.clone())
    };
    let menu = ContextMenu::new(owner, cx)
        .when(!is_file && !single_file, |m| {
            let m = entry(m, "New File…", "explorer.newFile");
            entry(m, "New Folder…", "explorer.newFolder")
        })
        .separator()
        .when(is_file, |m| {
            entry(m, "Open to the Side", "explorer.openToSide")
        });
    let menu = entry(menu, REVEAL_LABEL, "revealFileInOS");
    // `terminal.explorerKind` (VS Code): "integrated" (default),
    // "external" or "both".
    let kind_setting = cx
        .global::<crate::AppState>()
        .settings
        .get::<String>("terminal.explorerKind", None);
    let (integrated, external) = match kind_setting.as_deref() {
        Some("external") => (false, true),
        Some("both") => (true, true),
        _ => (true, false),
    };
    let menu = menu
        .when(integrated, |m| {
            entry(m, "Open in Integrated Terminal", "openInIntegratedTerminal")
        })
        .when(external, |m| {
            entry(m, "Open in External Terminal", "openInTerminal")
        })
        .separator()
        .when(!is_file, |m| {
            entry(m, "Find in Folder…", "filesExplorer.findInFolder")
        })
        .separator();
    let menu = entry(menu, "Copy Path", "copyFilePath");
    entry(menu, "Copy Relative Path", "copyRelativeFilePath")
        .separator()
        .when(kind != SelectedKind::Root && !single_file, |m| {
            let m = entry(m, "Rename…", "renameFile");
            entry(m, "Delete", "moveFileToTrash")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_title_is_the_label_under_file() {
        assert_eq!(REVEAL_TITLE, format!("File: {REVEAL_LABEL}"));
    }
}
