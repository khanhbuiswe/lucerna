//! Explorer commands: New File, New Folder, Rename, Delete, Delete
//! Permanently, Open to the Side, Reveal in Finder, Open in Terminal, Find
//! in Folder, Copy Path and Copy Relative Path. Each acts on `args.path`
//! when given, else on the Explorer's selection (New File, New Folder and
//! Rename always on the selection). None runs while a name is being typed
//! in the tree.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, AsyncWindowContext, ClipboardItem, Context, PromptLevel, WeakEntity, Window};
use lucerna_project::{EntryKind, fs_ops};
use serde_json::Value;

use crate::system_shell::system_shell;
use crate::{CommandRegistry, EditKind, OpenMode, SelectedKind, Workbench, status_message};

const TRASH_ANSWERS: [&str; 2] = ["Move to Trash", "Cancel"];
const TRASH_FAILED_ANSWERS: [&str; 2] = ["Delete Permanently", "Cancel"];
const PERMANENT_ANSWERS: [&str; 2] = ["Delete", "Cancel"];

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
    commands.register("renameFile", "File: Rename…", |wb, _, window, cx| {
        if !editing(wb, cx) {
            wb.explorer()
                .clone()
                .update(cx, |explorer, cx| explorer.start_rename(window, cx));
        }
    });
    commands.register("moveFileToTrash", "File: Delete", |wb, args, window, cx| {
        delete(wb, args, false, window, cx)
    });
    commands.register(
        "deleteFile",
        "File: Delete Permanently",
        |wb, args, window, cx| delete(wb, args, true, window, cx),
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

/// Delete (to the Trash) or Delete Permanently of the command's target.
/// Never the root, nor in single-file mode. Asks first unless it goes to
/// the Trash with `explorer.confirmDelete` off; when the Trash fails, asks
/// whether to delete permanently instead.
fn delete(
    wb: &mut Workbench,
    args: Option<&Value>,
    permanent: bool,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    if editing(wb, cx) || wb.is_single_file() {
        return;
    }
    let (target, kind) = target_kind(wb, args, cx);
    // A path the worktree does not know is not offered.
    let Some(is_dir) = kind else {
        return;
    };
    let rel: Arc<Path> = match target.strip_prefix(wb.root()) {
        Ok(rel) if !rel.as_os_str().is_empty() => Arc::from(rel),
        _ => return,
    };
    let name = crate::save::display_name(&target);
    let confirm = permanent
        || cx
            .global::<crate::AppState>()
            .settings
            .get::<bool>("explorer.confirmDelete", None)
            .unwrap_or(true);
    let answer = confirm.then(|| {
        if permanent {
            window.prompt(
                PromptLevel::Warning,
                &format!("Are you sure you want to permanently delete '{name}'?"),
                Some("This action is irreversible!"),
                &PERMANENT_ANSWERS[..],
                cx,
            )
        } else {
            let detail = if is_dir {
                "You can restore this folder and its contents from the Trash."
            } else {
                "You can restore this file from the Trash."
            };
            window.prompt(
                PromptLevel::Warning,
                &format!("Are you sure you want to delete '{name}'?"),
                Some(detail),
                &TRASH_ANSWERS[..],
                cx,
            )
        }
    });
    cx.spawn_in(window, async move |wb, cx| {
        if let Some(answer) = answer
            && answer.await != Ok(0)
        {
            return;
        }
        let deleted = Deleted { target, rel, name };
        let result = if permanent {
            delete_permanently(&wb, &deleted, cx).await
        } else {
            trash(&wb, &deleted, cx).await
        };
        let Some(result) = result else {
            return;
        };
        let _ = wb.update(cx, |wb, cx| match result {
            Ok(()) => {
                wb.paths_deleted(&deleted.target, cx);
                wb.explorer().clone().update(cx, |explorer, cx| {
                    explorer.select_parent_of(&deleted.rel, cx)
                });
            }
            Err(e) => {
                wb.forget_own_changes(std::slice::from_ref(&deleted.rel));
                status_message::show(format!("Unable to delete '{}': {e}", deleted.name), cx);
            }
        });
    })
    .detach();
}

/// What [`delete`] removes.
struct Deleted {
    target: PathBuf,
    /// `target`, root-relative.
    rel: Arc<Path>,
    /// Its file name, as prompts and messages name it.
    name: String,
}

/// Moves `deleted` to the Trash; when that fails, asks whether to delete
/// it permanently instead. `None` when nothing was done (cancelled, or the
/// workbench is gone).
async fn trash(
    wb: &WeakEntity<Workbench>,
    deleted: &Deleted,
    cx: &mut AsyncWindowContext,
) -> Option<Result<(), String>> {
    let shell = wb.update(cx, |_, cx| system_shell(cx)).ok()?;
    let path = deleted.target.clone();
    let result = run_off_thread(wb, deleted, cx, move || shell.trash(&path)).await?;
    let Err(e) = result else {
        return Some(Ok(()));
    };
    tracing::warn!(path = %deleted.target.display(), %e, "could not move to the Trash");
    wb.update(cx, |wb, _| {
        wb.forget_own_changes(std::slice::from_ref(&deleted.rel))
    })
    .ok()?;
    let answer = wb
        .update_in(cx, |_, window, cx| {
            window.prompt(
                PromptLevel::Warning,
                &format!(
                    "Unable to move '{}' to the Trash. Do you want to permanently delete instead?",
                    deleted.name
                ),
                None,
                &TRASH_FAILED_ANSWERS[..],
                cx,
            )
        })
        .ok()?;
    if answer.await != Ok(0) {
        return None;
    }
    delete_permanently(wb, deleted, cx).await
}

/// Deletes `deleted` for good. `None` when the workbench is gone.
async fn delete_permanently(
    wb: &WeakEntity<Workbench>,
    deleted: &Deleted,
    cx: &mut AsyncWindowContext,
) -> Option<Result<(), String>> {
    let path = deleted.target.clone();
    run_off_thread(wb, deleted, cx, move || {
        fs_ops::delete_permanently(&path).map_err(|e| e.to_string())
    })
    .await
}

/// Runs the delete `op` on the background executor, once every save under
/// the target asked for so far has ended (else a save could write the file
/// back). Its path is recorded as Lucerna's own change just before.
async fn run_off_thread(
    wb: &WeakEntity<Workbench>,
    deleted: &Deleted,
    cx: &mut AsyncWindowContext,
    op: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Option<Result<(), String>> {
    let saves = wb
        .update(cx, |wb, cx| {
            wb.documents().read(cx).saves_done_under(&deleted.target)
        })
        .ok()?;
    saves.await;
    wb.update(cx, |wb, _| wb.expect_own_changes([deleted.rel.clone()]))
        .ok()?;
    Some(cx.background_executor().spawn(async move { op() }).await)
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
