//! Explorer commands: New File, New Folder, Open to the Side, Reveal in
//! Finder, Open in Terminal, Find in Folder, Copy Path and Copy Relative
//! Path. Each acts on `args.path` when given, else on the Explorer's
//! selection (New File and New Folder always on the selection). None runs
//! while a name is being typed in the tree.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, ClipboardItem};
use lucerna_project::EntryKind;
use serde_json::Value;

use crate::system_shell::system_shell;
use crate::{CommandRegistry, EditKind, OpenMode, SelectedKind, Workbench, status_message};

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "explorer.newFile",
        "File: New File…",
        |wb, _, window, cx| {
            if !editing(wb, cx) {
                wb.explorer().clone().update(cx, |explorer, cx| {
                    explorer.start_new(EditKind::NewFile, window, cx)
                });
            }
        },
    );
    commands.register(
        "explorer.newFolder",
        "File: New Folder…",
        |wb, _, window, cx| {
            if !editing(wb, cx) {
                wb.explorer().clone().update(cx, |explorer, cx| {
                    explorer.start_new(EditKind::NewFolder, window, cx)
                });
            }
        },
    );
    commands.register(
        "explorer.openToSide",
        "Explorer: Open to the Side",
        |wb, args, _, cx| {
            if editing(wb, cx) {
                return;
            }
            let (target, kind) = target_kind(wb, args, cx);
            if kind == Some(false) {
                wb.open_path(Arc::from(target), OpenMode::ToSide, cx);
            }
        },
    );
    commands.register(
        "filesExplorer.findInFolder",
        "Explorer: Find in Folder…",
        |wb, args, window, cx| {
            if editing(wb, cx) {
                return;
            }
            let (target, is_dir) = command_target(wb, args, cx);
            if !is_dir {
                return;
            }
            let include = target
                .strip_prefix(wb.root())
                .map(|rel| {
                    rel.components()
                        .map(|c| c.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .unwrap_or_default();
            wb.search_view()
                .update(cx, |view, cx| view.set_include(&include, window, cx));
            wb.open_search(window, cx);
        },
    );
    commands.register(
        "revealFileInOS",
        "File: Reveal in Finder",
        |wb, args, _, cx| {
            if editing(wb, cx) {
                return;
            }
            let (target, _) = command_target(wb, args, cx);
            system_shell(cx).reveal(&target, cx);
        },
    );
    commands.register(
        "openInTerminal",
        "Explorer: Open in Terminal",
        |wb, args, _, cx| {
            if editing(wb, cx) {
                return;
            }
            let (target, is_dir) = command_target(wb, args, cx);
            let dir = if is_dir {
                target
            } else {
                target
                    .parent()
                    .map_or_else(|| wb.root().to_path_buf(), Path::to_path_buf)
            };
            let key = if cfg!(target_os = "macos") {
                "terminal.external.osxExec"
            } else if cfg!(target_os = "windows") {
                "terminal.external.windowsExec"
            } else {
                "terminal.external.linuxExec"
            };
            // The bundled defaults always provide a value.
            let exec = cx
                .global::<crate::AppState>()
                .settings
                .get::<String>(key, None)
                .unwrap_or_default();
            if let Err(e) = system_shell(cx).open_terminal(&dir, &exec) {
                status_message::show(format!("Couldn't open the terminal: {e}"), cx);
            }
        },
    );
    commands.register(
        "copyFilePath",
        "File: Copy Path of Active File",
        |wb, args, _, cx| {
            if editing(wb, cx) {
                return;
            }
            let (target, _) = command_target(wb, args, cx);
            cx.write_to_clipboard(ClipboardItem::new_string(
                target.to_string_lossy().into_owned(),
            ));
        },
    );
    commands.register(
        "copyRelativeFilePath",
        "File: Copy Relative Path of Active File",
        |wb, args, _, cx| {
            if editing(wb, cx) {
                return;
            }
            let (target, _) = command_target(wb, args, cx);
            let setting = cx
                .global::<crate::AppState>()
                .settings
                .get::<String>("explorer.copyRelativePathSeparator", None);
            let separator = match setting.as_deref() {
                Some("/") => "/",
                Some("\\") => "\\",
                _ => std::path::MAIN_SEPARATOR_STR,
            };
            let text = relative_path_text(wb.root(), &target, separator, &wb.root_name());
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        },
    );
}

/// A name is being typed in the Explorer: its commands do nothing. Keys
/// are already off (`!textInputFocus`), but that key is a frame late.
pub(crate) fn editing(wb: &Workbench, cx: &App) -> bool {
    wb.explorer().read(cx).editing()
}

/// The path a command acts on and whether it is a folder: `args.path`
/// (absolute, or relative to the root) when present, else the selection.
/// A path the worktree does not know counts as a file.
pub(crate) fn command_target(wb: &Workbench, args: Option<&Value>, cx: &App) -> (PathBuf, bool) {
    let (target, kind) = target_kind(wb, args, cx);
    (target, kind == Some(true))
}

/// Like [`command_target`], with `None` for a path the worktree does not
/// know. Reads the Explorer and the worktree snapshot only, never the disk.
fn target_kind(wb: &Workbench, args: Option<&Value>, cx: &App) -> (PathBuf, Option<bool>) {
    let given = args
        .and_then(|a| a.get("path"))
        .and_then(Value::as_str)
        .map(Path::new);
    let Some(path) = given else {
        let explorer = wb.explorer().read(cx);
        let is_dir = matches!(
            explorer.selected_kind(),
            SelectedKind::Dir | SelectedKind::Root
        );
        return (explorer.selected_abs_path(), Some(is_dir));
    };
    let target = wb.root().join(path);
    let kind = match target.strip_prefix(wb.root()) {
        Ok(rel) if rel.as_os_str().is_empty() => Some(true),
        Ok(rel) => wb
            .worktree()
            .snapshot()
            .children(rel.parent().unwrap_or(Path::new("")))
            .iter()
            .find(|entry| &*entry.path == rel)
            .map(|entry| entry.kind == EntryKind::Dir),
        Err(_) => None,
    };
    (target, kind)
}

/// `target` relative to `root` with its components joined by `separator`;
/// the root itself is `root_name`.
pub(crate) fn relative_path_text(
    root: &Path,
    target: &Path,
    separator: &str,
    root_name: &str,
) -> String {
    match target.strip_prefix(root) {
        Ok(rest) if rest.as_os_str().is_empty() => root_name.to_string(),
        Ok(rest) => rest
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join(separator),
        Err(_) => target.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_uses_separator() {
        let (root, file) = (Path::new("/p"), Path::new("/p/src/a.rs"));
        assert_eq!(relative_path_text(root, file, "/", "p"), "src/a.rs");
        assert_eq!(relative_path_text(root, file, "\\", "p"), "src\\a.rs");
    }

    #[test]
    fn relative_path_of_root_is_its_name() {
        let root = Path::new("/p");
        assert_eq!(relative_path_text(root, root, "/", "p"), "p");
    }
}
