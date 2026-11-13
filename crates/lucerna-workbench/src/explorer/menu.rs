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

/// The menu for a resource of `kind`, in the order of the spec's table.
/// Entries that do not apply are left out. `single_file` marks the lone-file
/// mode, which never shows the entries that edit the tree. `path` is the
/// root-relative row the menu is for (`None` for the root): every entry
/// carries it as `args.path`, so a command never falls back to whatever is
/// selected when it runs (the row may vanish while the menu is open).
pub(super) fn build_menu(
    kind: SelectedKind,
    single_file: bool,
    path: Option<String>,
    owner: FocusHandle,
    cx: &mut Context<ContextMenu>,
) -> ContextMenu {
    let is_file = kind == SelectedKind::File;
    let args: Option<Value> = path.map(|path| json!({ "path": path }));
    let entry = |m: ContextMenu, label: &'static str, command: &'static str| match &args {
        Some(args) => m.entry_with_args(label, command, args.clone()),
        None => m.entry(label, command),
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
    let menu = entry(menu, "Open in Terminal", "openInTerminal")
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
