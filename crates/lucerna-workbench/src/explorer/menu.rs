//! The Explorer's context menu: which entries a row of each kind shows.

use gpui::prelude::FluentBuilder as _;
use gpui::{Context, FocusHandle};

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

/// The menu for a resource of `kind`, in the order of the spec's table.
/// Entries that do not apply are left out. `single_file` marks the lone-file
/// mode, which never shows the entries that edit the tree.
pub(super) fn build_menu(
    kind: SelectedKind,
    single_file: bool,
    owner: FocusHandle,
    cx: &mut Context<ContextMenu>,
) -> ContextMenu {
    let is_file = kind == SelectedKind::File;
    ContextMenu::new(owner, cx)
        .when(!is_file && !single_file, |m| {
            m.entry("New File…", "explorer.newFile")
                .entry("New Folder…", "explorer.newFolder")
        })
        .separator()
        .when(is_file, |m| {
            m.entry("Open to the Side", "explorer.openToSide")
        })
        .entry(REVEAL_LABEL, "revealFileInOS")
        .entry("Open in Terminal", "openInTerminal")
        .separator()
        .when(!is_file, |m| {
            m.entry("Find in Folder…", "filesExplorer.findInFolder")
        })
        .separator()
        .entry("Copy Path", "copyFilePath")
        .entry("Copy Relative Path", "copyRelativeFilePath")
}
