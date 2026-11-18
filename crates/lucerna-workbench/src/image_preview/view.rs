//! The image preview tab: the image centred on the editor background,
//! shrunk to fit (never enlarged), over a checkerboard; or a notice with
//! "Open as Text" when it cannot be shown.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, FocusHandle, Focusable, ImageSource, IntoElement, MouseButton, ParentElement,
    Pixels, Render, RenderImage, SharedString, Size, Styled, Task, WeakEntity, Window, canvas,
    checkerboard, div, img, prelude::*, px,
};

use super::info::format_size;
use super::load::{LoadOutcome, load_image};
use crate::AppState;
use crate::layout::color;
use crate::ui::{GRID, centered_notice, text_button};
use crate::workbench::Workbench;

/// The side of a checkerboard square behind the image, in px.
const CHECKER_SIZE: f32 = 8.;
/// The width of the border around the image, in px.
const BORDER_WIDTH: f32 = 1.;

/// What the view shows.
enum State {
    /// The first load has not finished: an empty page, no spinner.
    Loading,
    Loaded {
        image: Arc<RenderImage>,
        dims: (u32, u32),
        bytes: u64,
    },
    /// The image cannot be shown: why, and the file's length if known.
    Error {
        message: SharedString,
        bytes: Option<u64>,
    },
}

/// A view of one image file. It reads the file itself, on the background
/// executor; it does not use `Documents`.
pub struct ImagePreview {
    path: Arc<Path>,
    state: State,
    focus_handle: FocusHandle,
    /// The workbench, for "Open as Text".
    workbench: WeakEntity<Workbench>,
    /// The running load; dropped with the view, which cancels it.
    _load: Option<Task<()>>,
    /// The window's scale factor the load rasterises an SVG at.
    scale_factor: f32,
    /// The space the image may take, measured at the last paint.
    available: Rc<Cell<Option<Size<Pixels>>>>,
}

impl ImagePreview {
    /// A view of `path` (absolute), loading it at once.
    pub fn new(
        path: Arc<Path>,
        workbench: WeakEntity<Workbench>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = ImagePreview {
            path,
            state: State::Loading,
            focus_handle: cx.focus_handle(),
            workbench,
            _load: None,
            scale_factor: window.scale_factor(),
            available: Rc::default(),
        };
        view.load(cx);
        view
    }

    /// Starts reading the file; the result replaces the state, unless the
    /// view is gone by then.
    fn load(&mut self, cx: &mut Context<Self>) {
        let (path, scale_factor, svg) = (self.path.clone(), self.scale_factor, cx.svg_renderer());
        let outcome = cx.background_spawn(async move { load_image(&path, scale_factor, svg) });
        self._load = Some(cx.spawn(async move |this, cx| {
            let outcome = outcome.await;
            this.update(cx, |this, cx| {
                this.state = state_for(outcome);
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn path(&self) -> &Arc<Path> {
        &self.path
    }

    /// The image's pixel size, once loaded.
    pub fn dims(&self) -> Option<(u32, u32)> {
        match &self.state {
            State::Loaded { dims, .. } => Some(*dims),
            _ => None,
        }
    }

    /// The file's length, once known.
    pub fn file_bytes(&self) -> Option<u64> {
        match &self.state {
            State::Loaded { bytes, .. } => Some(*bytes),
            State::Error { bytes, .. } => *bytes,
            State::Loading => None,
        }
    }

    pub fn is_loaded(&self) -> bool {
        matches!(self.state, State::Loaded { .. })
    }

    /// The notice shown instead of the image.
    pub fn error_message(&self) -> Option<SharedString> {
        match &self.state {
            State::Error { message, .. } => Some(message.clone()),
            _ => None,
        }
    }

    /// The decoded image, for tests.
    pub fn render_image(&self) -> Option<Arc<RenderImage>> {
        match &self.state {
            State::Loaded { image, .. } => Some(image.clone()),
            _ => None,
        }
    }

    /// The notice and its "Open as Text" button.
    fn render_error(&self, message: SharedString, cx: &mut Context<Self>) -> gpui::Div {
        let (workbench, view) = (self.workbench.clone(), cx.entity_id());
        let button =
            text_button("image-open-as-text", "Open as Text", cx).on_click(move |_, window, cx| {
                // Not inside the click: the tab's body is replaced.
                let workbench = workbench.clone();
                window.defer(cx, move |window, cx| {
                    workbench
                        .update(cx, |wb, cx| wb.reopen_image_view_as_text(view, window, cx))
                        .ok();
                });
            });
        centered_notice("image-error", message, Some(button), cx)
    }

    /// The image, scaled by `min(1, available / size)`, on a bordered
    /// checkerboard, centred in the space measured at the last paint.
    fn render_image_box(
        &self,
        image: Arc<RenderImage>,
        (w, h): (u32, u32),
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let ui = &cx.global::<AppState>().theme.ui;
        let (w, h) = (w.max(1) as f32, h.max(1) as f32);
        // The border is drawn around the image, so it takes from the space.
        let border = 2. * BORDER_WIDTH;
        let fit = self.available.get().map_or(1., |a| {
            let (aw, ah) = (f32::from(a.width) - border, f32::from(a.height) - border);
            (aw / w).min(ah / h).clamp(0., 1.)
        });
        let (iw, ih) = (w * fit, h * fit);
        let available = self.available.clone();
        let view = cx.entity_id();
        let measure = canvas(
            move |bounds, window, _| {
                if available.get() != Some(bounds.size) {
                    available.set(Some(bounds.size));
                    // Laid out with the old size: draw again.
                    window.on_next_frame(move |_, cx| cx.notify(view));
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div().size_full().p(px(2. * GRID)).child(
            div()
                .relative()
                .size_full()
                .flex()
                .justify_center()
                .items_center()
                .overflow_hidden()
                .child(measure)
                .child(
                    // Sizes are border-box: the frame is the image plus its
                    // border, so the image keeps its full size.
                    div()
                        .debug_selector(|| "image-preview-box".into())
                        .flex_none()
                        .w(px(iw + border))
                        .h(px(ih + border))
                        .border(px(BORDER_WIDTH))
                        .border_color(color(ui.border_variant))
                        .bg(checkerboard(color(ui.element_background), CHECKER_SIZE))
                        .child(
                            div()
                                .debug_selector(|| "image-preview-image".into())
                                .w(px(iw))
                                .h(px(ih))
                                .child(img(ImageSource::Render(image)).size_full()),
                        ),
                ),
        )
    }
}

/// What the view shows for a load's `outcome`.
fn state_for(outcome: LoadOutcome) -> State {
    let error = |message: String, bytes| State::Error {
        message: message.into(),
        bytes,
    };
    match outcome {
        LoadOutcome::Loaded { image, dims, bytes } => State::Loaded { image, dims, bytes },
        LoadOutcome::TooLargeBytes(bytes) => error(
            format!(
                "The image is too large to preview ({}).",
                format_size(bytes)
            ),
            Some(bytes),
        ),
        LoadOutcome::TooLargePixels(w, h) => error(
            format!("The image is too large to preview ({w} \u{d7} {h})."),
            None,
        ),
        LoadOutcome::LfsPointer { bytes } => error(
            "This image is stored with Git LFS and has not been downloaded.".into(),
            Some(bytes),
        ),
        LoadOutcome::Failed { bytes } => error("The image could not be loaded.".into(), bytes),
    }
}

impl Focusable for ImagePreview {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ImagePreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let root = div()
            .id("image-preview")
            .debug_selector(|| "image-preview".into())
            .track_focus(&self.focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| window.focus(&view.focus_handle, cx)),
            )
            .size_full()
            .bg(color(ui.editor_background));
        match &self.state {
            State::Loading => root,
            State::Error { message, .. } => {
                let message = message.clone();
                root.child(self.render_error(message, cx))
            }
            State::Loaded { image, dims, .. } => {
                let (image, dims) = (image.clone(), *dims);
                root.child(self.render_image_box(image, dims, cx))
            }
        }
    }
}
