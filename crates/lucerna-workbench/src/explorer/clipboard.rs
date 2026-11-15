//! The Explorer's clipboard: Cut, Copy and Paste of files and folders,
//! and the transfer routine Paste shares with Duplicate. The clipboard
//! lives in the workbench, never in the OS clipboard (as in Zed).

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{Context, Window};
use lucerna_project::{EntryKind, fs_ops};
use serde_json::Value;

use super::commands::{InFlight, editing, target_kind};
use crate::{CommandRegistry, OpenMode, Workbench, status_message};

/// Whether a paste moves or copies the clipboard's entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipOp {
    Cut,
    Copy,
}

/// The entry Cut or Copy put on the Explorer's clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplorerClipboard {
    pub op: ClipOp,
    /// Absolute.
    pub path: Arc<Path>,
}

/// What follows a successful [`transfer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AfterTransfer {
    /// Select and reveal the new entry, and open it pinned when a file
    /// (Paste, as Zed's `auto_open.on_paste`).
    SelectAndOpen,
    /// Select it and rename it with these bytes of its name selected
    /// (Duplicate, as Zed).
    #[expect(dead_code, reason = "Duplicate is the caller")]
    Rename(Range<usize>),
}

pub(super) fn register_commands(commands: &mut CommandRegistry) {
    commands.register("filesExplorer.cut", "Explorer: Cut", |wb, args, _, cx| {
        put(wb, args, ClipOp::Cut, cx)
    });
    commands.register("filesExplorer.copy", "Explorer: Copy", |wb, args, _, cx| {
        put(wb, args, ClipOp::Copy, cx)
    });
    commands.register(
        "filesExplorer.paste",
        "Explorer: Paste",
        |wb, args, window, cx| {
            if editing(wb, cx) || wb.is_single_file() {
                return;
            }
            let (target, kind) = target_kind(wb, args, cx);
            let dest = match kind {
                Some(true) => target,
                Some(false) => match target.parent() {
                    Some(parent) => parent.to_path_buf(),
                    None => return,
                },
                None => return,
            };
            paste_into(wb, dest, window, cx);
        },
    );
    commands.register(
        "filesExplorer.cancelCut",
        "Explorer: Cancel Cut",
        |wb, _, _, cx| {
            if wb.explorer_clipboard().is_some_and(|c| c.op == ClipOp::Cut) {
                wb.set_explorer_clipboard(None, cx);
            }
        },
    );
}

/// Cut or Copy of the command's target: never the root, and no Cut in
/// single-file mode.
fn put(wb: &mut Workbench, args: Option<&Value>, op: ClipOp, cx: &mut Context<Workbench>) {
    if editing(wb, cx) || (op == ClipOp::Cut && wb.is_single_file()) {
        return;
    }
    let (target, kind) = target_kind(wb, args, cx);
    if kind.is_none() || target == wb.root() {
        return;
    }
    let clip = ExplorerClipboard {
        op,
        path: Arc::from(target),
    };
    wb.set_explorer_clipboard(Some(clip), cx);
}

/// Paste of the clipboard's entry into `dest_dir` (absolute). A Cut is
/// taken off the clipboard while it runs, and put back if it fails.
pub(crate) fn paste_into(
    wb: &mut Workbench,
    dest_dir: PathBuf,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let Some(clip) = wb.explorer_clipboard().cloned() else {
        return;
    };
    transfer(
        wb,
        clip.path.to_path_buf(),
        dest_dir,
        clip.op,
        AfterTransfer::SelectAndOpen,
        window,
        cx,
    );
}

/// Copies (`Copy`) or moves (`Cut`) `src` into `dest_dir` (both absolute),
/// never replacing anything: a taken name becomes `"{stem} copy{ext}"`,
/// `" copy 2"`… The name is checked against the tree (and open
/// documents), then written in the background with a no-replace
/// operation; when the write finds the name taken after all, the name is
/// picked again on disk and written once more.
pub(crate) fn transfer(
    wb: &mut Workbench,
    src: PathBuf,
    dest_dir: PathBuf,
    op: ClipOp,
    after: AfterTransfer,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let Some(name) = src.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return;
    };
    let root = wb.root().to_path_buf();
    let Ok(src_rel) = src.strip_prefix(&root) else {
        return;
    };
    if !dest_dir.starts_with(&root) {
        return;
    }
    if src_rel.as_os_str().is_empty() {
        return;
    }
    // A move into the folder that already holds it changes nothing.
    if op == ClipOp::Cut && src.parent() == Some(dest_dir.as_path()) {
        return;
    }
    if fs_ops::is_within(&dest_dir, &src) {
        status_message::show(format!("Cannot paste '{name}' into itself."), cx);
        return;
    }
    let snapshot = wb.worktree().snapshot();
    // An entry the tree does not know is tried as a file: the write then
    // reports what is wrong with it.
    let is_dir = snapshot
        .children(src_rel.parent().unwrap_or(Path::new("")))
        .iter()
        .any(|e| &*e.path == src_rel && e.kind == EntryKind::Dir);
    let documents = wb.documents().read(cx);
    let (target, range) = fs_ops::paste_name(&dest_dir, &name, is_dir, |path| {
        path.strip_prefix(&root)
            .is_ok_and(|rel| snapshot.contains(rel))
            || documents.has_document_under(path)
    });
    let src_rel: Arc<Path> = Arc::from(src_rel);
    // A move must not race a Delete of the same entry.
    let in_flight = match op {
        ClipOp::Cut => match InFlight::begin(wb, &src) {
            Some(guard) => Some(guard),
            None => return,
        },
        ClipOp::Copy => None,
    };
    let taken = (op == ClipOp::Cut)
        .then(|| wb.explorer_clipboard().cloned())
        .flatten();
    if taken.is_some() {
        wb.set_explorer_clipboard(None, cx);
    }
    // A save still running would write the old path again after a move.
    let saves = (op == ClipOp::Cut).then(|| wb.documents().read(cx).saves_done_under(&src));
    cx.spawn_in(window, async move |wb, cx| {
        let _in_flight = in_flight;
        if let Some(saves) = saves {
            saves.await;
        }
        let mut target = target;
        let mut range = range;
        let mut retried = false;
        let result = loop {
            let target_rel = target.strip_prefix(&root).unwrap_or(&target);
            let mut expected = vec![Arc::from(target_rel)];
            if op == ClipOp::Cut {
                expected.push(src_rel.clone());
            }
            if wb
                .update(cx, |wb, _| wb.expect_own_changes(expected.iter().cloned()))
                .is_err()
            {
                return;
            }
            let (from, to) = (src.clone(), target.clone());
            let result = cx
                .background_executor()
                .spawn(async move {
                    match op {
                        ClipOp::Copy => fs_ops::copy_entry(&from, &to),
                        ClipOp::Cut => fs_ops::rename(&from, &to),
                    }
                })
                .await;
            match result {
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && !retried => {
                    retried = true;
                    if wb
                        .update(cx, |wb, _| wb.forget_own_changes(&expected))
                        .is_err()
                    {
                        return;
                    }
                    let (dir, name) = (dest_dir.clone(), name.clone());
                    (target, range) = cx
                        .background_executor()
                        .spawn(async move {
                            fs_ops::paste_name(&dir, &name, is_dir, |p| {
                                std::fs::symlink_metadata(p).is_ok()
                            })
                        })
                        .await;
                }
                Err(e) => {
                    let _ = wb.update(cx, |wb, _| wb.forget_own_changes(&expected));
                    break Err(e);
                }
                Ok(()) => break Ok(()),
            }
        };
        let _ = wb.update(cx, |wb, cx| match result {
            Err(e) => {
                if let Some(clip) = taken
                    && wb.explorer_clipboard().is_none()
                {
                    wb.set_explorer_clipboard(Some(clip), cx);
                }
                status_message::show(format!("Unable to paste '{name}': {e}"), cx);
            }
            Ok(()) => {
                if op == ClipOp::Cut {
                    wb.paths_renamed(&src, &target, cx);
                }
                let rel: Arc<Path> = Arc::from(target.strip_prefix(&root).unwrap_or(&target));
                wb.explorer()
                    .clone()
                    .update(cx, |explorer, cx| explorer.select_new(rel.clone(), cx));
                match after {
                    AfterTransfer::SelectAndOpen => {
                        if !is_dir {
                            wb.open_path(rel, OpenMode::Pinned, cx);
                        }
                    }
                    // Only selected so far: Duplicate adds the rename,
                    // which selects `range` (the final name's stem and
                    // " copy N" suffix).
                    AfterTransfer::Rename(_) => {
                        let _ = range;
                    }
                }
            }
        });
    })
    .detach();
}
