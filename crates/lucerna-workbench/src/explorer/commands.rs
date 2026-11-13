//! Explorer commands that only read: Open to the Side, Reveal in Finder,
//! Open in Terminal, Copy Path and Copy Relative Path. Each acts on
//! `args.path` when given, else on the Explorer's selection.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, ClipboardItem};
use serde_json::Value;

use crate::system_shell::system_shell;
use crate::{CommandRegistry, OpenMode, Workbench, status_message};

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "explorer.openToSide",
        "Explorer: Open to the Side",
        |wb, args, _, cx| {
            let (target, is_dir) = command_target(wb, args, cx);
            if !is_dir {
                wb.open_path(Arc::from(target), OpenMode::ToSide, cx);
            }
        },
    );
    commands.register(
        "revealFileInOS",
        "File: Reveal in Finder",
        |wb, args, _, cx| {
            let (target, _) = command_target(wb, args, cx);
            system_shell(cx).reveal(&target, cx);
        },
    );
    commands.register(
        "openInTerminal",
        "Explorer: Open in Terminal",
        |wb, args, _, cx| {
            let (target, is_dir) = command_target(wb, args, cx);
            let dir = if is_dir {
                target
            } else {
                target
                    .parent()
                    .map_or_else(|| wb.root().to_path_buf(), Path::to_path_buf)
            };
            let key = if cfg!(target_os = "macos") {
                ("terminal.external.osxExec", "Terminal.app")
            } else if cfg!(target_os = "windows") {
                ("terminal.external.windowsExec", "cmd.exe")
            } else {
                ("terminal.external.linuxExec", "x-terminal-emulator")
            };
            let exec = cx
                .global::<crate::AppState>()
                .settings
                .get::<String>(key.0, None)
                .unwrap_or_else(|| key.1.to_string());
            if let Err(e) = system_shell(cx).open_terminal(&dir, &exec) {
                status_message::show(format!("Couldn't open the terminal: {e}"), cx);
            }
        },
    );
    commands.register(
        "copyFilePath",
        "File: Copy Path of Active File",
        |wb, args, _, cx| {
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

/// The path a command acts on and whether it is a folder: `args.path`
/// (absolute, or relative to the root) when present, else the selection.
pub(crate) fn command_target(wb: &Workbench, args: Option<&Value>, cx: &App) -> (PathBuf, bool) {
    let target = match args
        .and_then(|a| a.get("path"))
        .and_then(Value::as_str)
        .map(Path::new)
    {
        Some(path) => wb.root().join(path),
        None => wb.explorer().read(cx).selected_abs_path(),
    };
    let is_dir = target.is_dir();
    (target, is_dir)
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
