//! The status bar item of an image tab: `W × H • size`.

use gpui::{Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, Window, div};

use super::info::status_text;
use super::view::ImagePreview;
use crate::editor_groups::EditorGroupsView;

/// Dimensions and file size of the active image tab. Empty for any other
/// tab, and until the file has been read; the text-editor items hide
/// themselves for an image tab on their own.
pub struct ImageStatusItem {
    groups: Entity<EditorGroupsView>,
}

impl ImageStatusItem {
    pub fn new(groups: Entity<EditorGroupsView>) -> Self {
        ImageStatusItem { groups }
    }

    /// The text shown now, if the active tab is an image whose file size
    /// is known. The size alone while the image is not decoded.
    pub fn text(&self, cx: &gpui::App) -> Option<String> {
        let preview = self
            .groups
            .read(cx)
            .active_body()
            .and_then(|v| v.downcast::<ImagePreview>().ok())?;
        let preview = preview.read(cx);
        let bytes = preview.file_bytes()?;
        Some(status_text(preview.dims(), bytes))
    }
}

impl Render for ImageStatusItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().children(
            self.text(cx)
                .map(|text| div().debug_selector(|| "status-image".into()).child(text)),
        )
    }
}
