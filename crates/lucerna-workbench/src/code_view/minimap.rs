//! The minimap: one 2 px row per buffer line, drawn as colored quads from
//! cached highlight runs (no text shaping), plus the viewport slider.
//! Clicking or dragging scrolls the view.

use std::sync::Arc;

use gpui::{
    App, Bounds, DispatchPhase, Entity, Hitbox, HitboxBehavior, Hsla, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Styled,
    Window, canvas, div, fill, point, px, size,
};

use super::layout::line_prefix;
use super::{CodeView, Drag, hsla};
use crate::AppState;

const WIDTH: f32 = 96.;
const ROW_HEIGHT: f32 = 2.;
/// One char is one px wide; longer lines are cut.
const MAX_COLUMNS: usize = WIDTH as usize;
const TEXT_ALPHA: f32 = 0.6;
const SLIDER_ALPHA: f32 = 0.12;

/// A minimap row: runs of non-blank chars and their colors.
pub(crate) struct MinimapRow {
    /// (first column, columns, color).
    segments: Vec<(u16, u16, Hsla)>,
    /// Whether highlight colors were known when it was built.
    pub highlighted: bool,
}

struct MinimapFrame {
    bounds: Bounds<Pixels>,
    rows: Vec<(Pixels, Arc<MinimapRow>)>,
    slider: Bounds<Pixels>,
    slider_color: Hsla,
    background: Hsla,
    hitbox: Hitbox,
}

pub(crate) fn render(view: Entity<CodeView>) -> impl IntoElement {
    let prepaint_view = view.clone();
    div()
        .debug_selector(|| "code-view-minimap".into())
        .flex_none()
        .w(px(WIDTH))
        .h_full()
        .child(
            canvas(
                move |bounds, window, cx| {
                    let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                    prepaint_view.update(cx, |v, cx| v.layout_minimap(bounds, hitbox, cx))
                },
                move |_, frame, window, cx| paint(&view, frame, window, cx),
            )
            .size_full(),
        )
}

impl CodeView {
    /// The first buffer row the minimap shows: it scrolls in step with the
    /// view when the file is taller than the minimap.
    fn minimap_start(&self, minimap_rows: u32) -> u32 {
        let total = self.snapshot.line_count();
        if total <= minimap_rows {
            return 0;
        }
        let scrollable = total.saturating_sub(self.visible_rows()).max(1);
        let fraction = (self.scroll.top_row as f32 / scrollable as f32).min(1.);
        (fraction * (total - minimap_rows) as f32) as u32
    }

    fn layout_minimap(
        &mut self,
        bounds: Bounds<Pixels>,
        hitbox: Hitbox,
        cx: &mut gpui::Context<Self>,
    ) -> Option<MinimapFrame> {
        let theme = cx.global::<AppState>().theme.clone();
        let minimap_rows = (bounds.size.height / px(ROW_HEIGHT)).ceil() as u32;
        let start = self.minimap_start(minimap_rows);
        let end = (start + minimap_rows).min(self.snapshot.line_count());
        let key = (self.revision, Arc::as_ptr(&theme) as usize);
        let foreground = hsla(theme.ui.editor_foreground).opacity(TEXT_ALPHA);
        let mut rows = Vec::with_capacity((end - start) as usize);
        for row in start..end {
            let minimap_row = match self.caches.minimap_row(key, row) {
                Some(r) => r,
                None => {
                    let built = Arc::new(self.build_minimap_row(row, &theme, foreground));
                    self.caches.insert_minimap_row(row, built.clone());
                    built
                }
            };
            let y = bounds.top() + px((row - start) as f32 * ROW_HEIGHT);
            rows.push((y, minimap_row));
        }
        let slider_top = self.scroll.top_row as f32 - start as f32;
        let slider = Bounds::new(
            point(bounds.left(), bounds.top() + px(slider_top * ROW_HEIGHT)),
            size(
                bounds.size.width,
                px(self.visible_rows().max(1) as f32 * ROW_HEIGHT),
            ),
        );
        Some(MinimapFrame {
            bounds,
            rows,
            slider,
            slider_color: hsla(theme.ui.editor_foreground).opacity(SLIDER_ALPHA),
            background: hsla(theme.ui.editor_background),
            hitbox,
        })
    }

    fn build_minimap_row(
        &self,
        row: u32,
        theme: &lucerna_theme::Theme,
        foreground: Hsla,
    ) -> MinimapRow {
        let (text, _) = line_prefix(&self.snapshot, row, MAX_COLUMNS);
        let spans = self.caches.spans(row);
        let line_start = super::layout::row_start(&self.snapshot, row);
        let color_at = |byte: usize| {
            let offset = line_start + byte;
            spans
                .and_then(|spans| {
                    spans
                        .iter()
                        .rev()
                        .find(|(range, _)| range.contains(&offset))
                })
                .and_then(|(_, capture)| theme.style_for(capture)?.color)
                .map_or(foreground, |c| hsla(c).opacity(TEXT_ALPHA))
        };
        let mut segments: Vec<(u16, u16, Hsla)> = Vec::new();
        let mut column = 0u16;
        for (byte, c) in text.char_indices() {
            let width = if c == '\t' {
                let tab = self.config.tab_size as u16;
                tab - column % tab
            } else {
                1
            };
            if !c.is_whitespace() {
                let color = color_at(byte);
                match segments.last_mut() {
                    Some((start, len, last)) if *last == color && *start + *len == column => {
                        *len += width
                    }
                    _ => segments.push((column, width, color)),
                }
            }
            column = column.saturating_add(width);
            if column as usize >= MAX_COLUMNS {
                break;
            }
        }
        MinimapRow {
            segments,
            highlighted: spans.is_some(),
        }
    }

    /// Scrolls so the row at minimap `y` is in the middle of the view.
    fn minimap_scroll_to(
        &mut self,
        bounds: Bounds<Pixels>,
        y: Pixels,
        cx: &mut gpui::Context<Self>,
    ) {
        let minimap_rows = (bounds.size.height / px(ROW_HEIGHT)).ceil() as u32;
        let start = self.minimap_start(minimap_rows);
        let offset = ((y - bounds.top()) / px(ROW_HEIGHT)).max(0.) as u32;
        let row = (start + offset).saturating_sub(self.visible_rows() / 2);
        self.set_scroll_top_row(row, cx);
    }
}

fn paint(view: &Entity<CodeView>, frame: Option<MinimapFrame>, window: &mut Window, _: &mut App) {
    let Some(frame) = frame else {
        return;
    };
    window.paint_quad(fill(frame.bounds, frame.background));
    // The row quads never overlap, so they share one draw order instead of
    // each being placed in the scene's bounds tree.
    window.paint_layer(frame.bounds, |window| {
        for (y, row) in &frame.rows {
            for &(column, len, color) in &row.segments {
                window.paint_quad(fill(
                    Bounds::new(
                        point(frame.bounds.left() + px(column as f32), *y),
                        size(px(len as f32), px(ROW_HEIGHT)),
                    ),
                    color,
                ));
            }
        }
    });
    window.paint_quad(fill(frame.slider, frame.slider_color));

    let bounds = frame.bounds;
    window.on_mouse_event({
        let (view, hitbox) = (view.clone(), frame.hitbox.clone());
        move |ev: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && ev.button == MouseButton::Left
                && hitbox.is_hovered(window)
            {
                view.update(cx, |v, cx| {
                    v.drag = Some(Drag::Minimap);
                    v.minimap_scroll_to(bounds, ev.position.y, cx);
                });
                cx.stop_propagation();
            }
        }
    });
    window.on_mouse_event({
        let view = view.clone();
        move |ev: &MouseMoveEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble
                && ev.pressed_button == Some(MouseButton::Left)
                && view.read(cx).drag == Some(Drag::Minimap)
            {
                view.update(cx, |v, cx| v.minimap_scroll_to(bounds, ev.position.y, cx));
            }
        }
    });
}
