//! The image preview's commands (spec §4.2): an SVG switches between its
//! preview and its text in place, like `markdown.togglePreview`.

use std::path::Path;
use std::sync::Arc;

use gpui::{App, Context, Window};

use super::is_svg_path;
use crate::editor_groups::{TabId, TabItem};
use crate::{CommandRegistry, Workbench};

pub const REOPEN_AS_TEXT: &str = "imagePreview.reopenAsText";
pub const REOPEN_AS_PREVIEW: &str = "imagePreview.reopenAsPreview";

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(REOPEN_AS_TEXT, "Reopen as Text", |wb, _, window, cx| {
        if let Some((tab, TabItem::ImagePreview(path))) = active(wb, cx)
            && is_svg_path(&path)
        {
            wb.reopen_image_as_text(tab, false, window, cx);
        }
    });
    commands.register(
        REOPEN_AS_PREVIEW,
        "Reopen as Image Preview",
        |wb, args, window, cx| {
            // From the Explorer's menu: the row's file, whatever is open.
            if let Some(rel) = args.and_then(|a| a.get("path")).and_then(|p| p.as_str()) {
                let path = wb.absolute_path(Path::new(rel));
                if is_svg_path(&path) {
                    wb.open_image_preview(path, cx);
                    wb.focus_editor_area(window, cx);
                }
                return;
            }
            if let Some((tab, TabItem::File(path))) = active(wb, cx)
                && is_svg_path(&path)
            {
                reopen_as_image_preview(wb, tab, path, window, cx);
            }
        },
    );
}

/// The active tab's id and item.
fn active(wb: &Workbench, cx: &App) -> Option<(TabId, TabItem)> {
    let tab = wb.editor_groups().read(cx).model().active_tab()?;
    Some((tab.id, tab.item.clone()))
}

/// Replaces the source tab `tab` with the image preview of `path` in its
/// place (or activates the group's preview of it), and focuses it. Unsaved
/// text that no other tab shows is asked about first, as closing the tab
/// would: the preview shows the file on disk, and the text would be lost.
fn reopen_as_image_preview(
    wb: &mut Workbench,
    tab: TabId,
    path: Arc<Path>,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let workbench = cx.entity().downgrade();
    wb.editor_groups().clone().update(cx, |view, cx| {
        view.request_replace_item(
            tab,
            TabItem::ImagePreview(path),
            move |window, cx| {
                workbench
                    .update(cx, |wb, cx| wb.focus_editor_area(window, cx))
                    .ok();
            },
            window,
            cx,
        );
    });
}
