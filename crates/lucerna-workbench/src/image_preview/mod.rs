//! Image preview: opens raster and SVG files in a tab instead of the text
//! editor. [`info`] holds the pure helpers, [`load`] reads and decodes a
//! file in the background, [`view`] draws it.

pub mod commands;
pub mod info;
pub mod load;
pub mod status;
pub mod view;

use std::path::Path;
use std::sync::Arc;

use gpui::{App, Entity};
use lucerna_project::{ChangeKind, WorktreeEvent};

use crate::editor_groups::TabItem;
use crate::workbench::Workbench;

pub use commands::{REOPEN_AS_PREVIEW, REOPEN_AS_TEXT};
pub use info::{
    IMAGE_EDITOR_ID, IMAGE_EXTENSIONS, MAX_IMAGE_PIXELS, format_size, is_image_path,
    is_lfs_pointer, is_svg_path, status_text,
};
pub use load::{LoadOutcome, load_image};
pub use status::ImageStatusItem;
pub use view::ImagePreview;

/// Every image preview made so far.
fn previews(wb: &Workbench, cx: &App) -> Vec<Entity<ImagePreview>> {
    let groups = wb.editor_groups().read(cx);
    groups
        .model()
        .groups()
        .iter()
        .flat_map(|g| &g.tabs)
        .filter(|t| matches!(t.item, TabItem::ImagePreview(_)))
        .filter_map(|t| groups.body_of(t.id)?.downcast().ok())
        .collect()
}

/// Hands the changes to the previews of the changed files: the
/// worktree's (root-relative paths), and the open-file watcher's for
/// ignored files (absolute paths). After a rescan every preview checks
/// its file. Files outside the root are read once and never reload.
pub(crate) fn on_worktree_event(wb: &Workbench, ev: &WorktreeEvent, cx: &mut App) {
    let mut touched = false;
    for preview in previews(wb, cx) {
        let path = preview.read(cx).path().clone();
        if !path.starts_with(wb.root()) {
            continue;
        }
        let kinds: Vec<ChangeKind> = match ev {
            WorktreeEvent::Changed(changes) => changes
                .iter()
                .filter(|c| wb.root().join(&c.path) == *path)
                .map(|c| c.kind)
                .collect(),
            WorktreeEvent::Rescanned => {
                vec![if path.exists() {
                    ChangeKind::Modified
                } else {
                    ChangeKind::Removed
                }]
            }
            _ => Vec::new(),
        };
        if kinds.is_empty() {
            continue;
        }
        touched = true;
        preview.update(cx, |p, cx| {
            for kind in kinds {
                p.file_changed(kind, cx);
            }
        });
    }
    // A hidden tab's title is struck or cleared too.
    if touched {
        wb.editor_groups().update(cx, |_, cx| cx.notify());
    }
}

/// After a rename of `from` to `to` (absolute): every preview of a file
/// at `from` or under it follows to the new path. Runs in the same update
/// as the tabs' retarget, before the watcher reports the old path gone.
pub(crate) fn retarget_previews(wb: &Workbench, from: &Path, to: &Path, cx: &mut App) {
    for preview in previews(wb, cx) {
        let Some(new) = crate::workspace_state::moved_path(preview.read(cx).path(), from, to)
        else {
            continue;
        };
        let new: Arc<Path> = Arc::from(new);
        preview.update(cx, |p, cx| p.follow_rename(new, cx));
    }
}
