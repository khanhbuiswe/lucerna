//! The overlay scrollbar ([`scrollbar`]) shared by the editor and the lists: a thin thumb
//! at the right edge that can be dragged, a track that jumps where it is
//! clicked (or pages, for the editor's `editor.scrollbar.scrollByPage`),
//! and a thumb that darkens under the mouse and while dragged.
//!
//! The scrolled content is behind a [`ScrollbarSource`], read at prepaint
//! after the content has laid out in the same frame. A drag only sets the
//! scroll position; the owner redraws as it does for the wheel.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, EntityId, HitboxBehavior, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, ScrollHandle, Styled,
    Window, canvas, div, fill, point, px, size,
};

use crate::AppState;
use crate::layout::color;
use crate::ui::{
    SCROLLBAR_HIT_WIDTH, SCROLLBAR_WIDTH, scroll_top_for_thumb, scroll_top_for_track_click,
    thumb_geometry,
};

/// The scroll geometry of the content, all in px: the visible height, the
/// height of everything, and how far down the view is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollMetrics {
    pub viewport: f32,
    pub content: f32,
    pub scroll_top: f32,
}

/// What a [`Scrollbar`] scrolls.
pub trait ScrollbarSource: 'static {
    /// Read at prepaint, after the content laid out in this frame.
    fn metrics(&self, cx: &mut App) -> Option<ScrollMetrics>;
    /// Scrolls to `scroll_top` (already within range) and redraws the owner.
    fn set_scroll_top(&self, scroll_top: f32, cx: &mut App);
    /// Whether a track click pages instead of jumping there.
    fn track_click_pages(&self, _cx: &App) -> bool {
        false
    }
    /// Scrolls one page down (or up): by default one viewport on the
    /// scrollbar's scale; a source whose scale is not pixels on screen (the
    /// editor with word wrap) pages by what is on screen instead.
    fn scroll_page(&self, down: bool, metrics: ScrollMetrics, cx: &mut App) {
        let page = if down {
            metrics.viewport
        } else {
            -metrics.viewport
        };
        let max = (metrics.content - metrics.viewport).max(0.);
        self.set_scroll_top((metrics.scroll_top + page).clamp(0., max), cx);
    }
}

#[derive(Default)]
struct Inner {
    hovered: bool,
    /// While dragging: how far below the thumb's top it was grabbed.
    grab: Option<f32>,
    /// The thumb as last painted, in window coordinates.
    thumb: Option<Bounds<Pixels>>,
}

/// Hover, drag and the last painted thumb; cheap to clone, kept by the
/// view that owns the scrollbar so it lasts across frames.
#[derive(Clone, Default)]
pub struct ScrollbarState(Rc<RefCell<Inner>>);

impl ScrollbarState {
    pub fn thumb(&self) -> Option<Bounds<Pixels>> {
        self.0.borrow().thumb
    }

    pub fn is_hovered(&self) -> bool {
        self.0.borrow().hovered
    }

    pub fn is_dragging(&self) -> bool {
        self.0.borrow().grab.is_some()
    }
}

/// A [`ScrollbarSource`] over a GPUI scroll handle (a uniform list's
/// `base_handle`, or a div's `track_scroll`); `view` redraws after a scroll.
pub struct ScrollHandleSource {
    pub handle: ScrollHandle,
    pub view: EntityId,
}

impl ScrollbarSource for ScrollHandleSource {
    fn metrics(&self, _cx: &mut App) -> Option<ScrollMetrics> {
        let viewport = f32::from(self.handle.bounds().size.height);
        Some(ScrollMetrics {
            viewport,
            content: viewport + f32::from(self.handle.max_offset().y),
            scroll_top: -f32::from(self.handle.offset().y),
        })
    }

    fn set_scroll_top(&self, scroll_top: f32, cx: &mut App) {
        let max = f32::from(self.handle.max_offset().y);
        let x = self.handle.offset().x;
        self.handle
            .set_offset(point(x, px(-scroll_top.clamp(0., max.max(0.)))));
        cx.notify(self.view);
    }
}

/// What prepaint hands to paint while the content overflows.
struct Laid {
    track: Bounds<Pixels>,
    thumb: Bounds<Pixels>,
    metrics: ScrollMetrics,
    hitbox: gpui::Hitbox,
}

/// A strip [`SCROLLBAR_HIT_WIDTH`] wide over the right edge of its
/// parent, which must be `.relative()`. It takes the mouse (but not
/// the wheel) only while the content overflows.
pub fn scrollbar(
    state: ScrollbarState,
    source: impl ScrollbarSource,
    owner: EntityId,
) -> impl IntoElement {
    let source: Rc<dyn ScrollbarSource> = Rc::new(source);
    let prepaint_state = state.clone();
    let prepaint_source = source.clone();
    div()
        .absolute()
        .top_0()
        .right_0()
        .h_full()
        .w(px(SCROLLBAR_HIT_WIDTH))
        .child(
            canvas(
                move |track, window, cx| {
                    let laid = prepaint_source.metrics(cx).and_then(|metrics| {
                        let (top, height) =
                            thumb_geometry(metrics.viewport, metrics.content, metrics.scroll_top)?;
                        let thumb = Bounds::new(
                            point(track.right() - px(SCROLLBAR_WIDTH), track.top() + px(top)),
                            size(px(SCROLLBAR_WIDTH), px(height)),
                        );
                        let hitbox =
                            window.insert_hitbox(track, HitboxBehavior::BlockMouseExceptScroll);
                        Some(Laid {
                            track,
                            thumb,
                            metrics,
                            hitbox,
                        })
                    });
                    let mut inner = prepaint_state.0.borrow_mut();
                    inner.thumb = laid.as_ref().map(|l| l.thumb);
                    if laid.is_none() {
                        inner.hovered = false;
                        inner.grab = None;
                    }
                    laid
                },
                move |_, laid, window, cx| paint(laid, &state, &source, owner, window, cx),
            )
            .size_full(),
        )
}

fn paint(
    laid: Option<Laid>,
    state: &ScrollbarState,
    source: &Rc<dyn ScrollbarSource>,
    owner: EntityId,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(laid) = laid else {
        return;
    };
    let ui = &cx.global::<AppState>().theme.ui;
    let thumb_color = if state.is_dragging() {
        ui.scrollbar_thumb_active
    } else if state.is_hovered() {
        ui.scrollbar_thumb_hover
    } else {
        ui.scrollbar_thumb
    };
    window.set_cursor_style(CursorStyle::Arrow, &laid.hitbox);
    window.paint_quad(fill(laid.thumb, color(thumb_color)).corner_radii(px(SCROLLBAR_WIDTH / 2.)));

    let Laid {
        track,
        thumb,
        metrics,
        hitbox,
    } = laid;
    let ScrollMetrics {
        viewport, content, ..
    } = metrics;
    window.on_mouse_event({
        let (state, source, hitbox) = (state.clone(), source.clone(), hitbox.clone());
        move |ev: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || ev.button != MouseButton::Left
                || !hitbox.is_hovered(window)
            {
                return;
            }
            cx.stop_propagation();
            // The thumb's rows anywhere across the strip, which is wider
            // than the painted thumb so it is easy to hit. A sash on the
            // same edge occludes its own few px, so it wins there.
            let on_thumb = ev.position.y >= thumb.top() && ev.position.y < thumb.bottom();
            let grab = if on_thumb {
                Some(f32::from(ev.position.y - thumb.top()))
            } else if source.track_click_pages(cx) {
                source.scroll_page(ev.position.y >= thumb.bottom(), metrics, cx);
                None
            } else {
                let y = f32::from(ev.position.y - track.top());
                let top =
                    scroll_top_for_track_click(viewport, content, metrics.scroll_top, y, false);
                source.set_scroll_top(top, cx);
                // The thumb is now centered under the mouse, and a held
                // button drags on from there.
                Some(f32::from(thumb.size.height) / 2.)
            };
            state.0.borrow_mut().grab = grab;
            cx.notify(owner);
        }
    });
    window.on_mouse_event({
        let (state, source) = (state.clone(), source.clone());
        move |ev: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let grab = state.0.borrow().grab;
            if let Some(grab) = grab {
                if ev.pressed_button == Some(MouseButton::Left) {
                    let thumb_top = f32::from(ev.position.y - track.top()) - grab;
                    source.set_scroll_top(scroll_top_for_thumb(viewport, content, thumb_top), cx);
                } else {
                    // The button came up where no mouse-up reached us.
                    state.0.borrow_mut().grab = None;
                    cx.notify(owner);
                }
            }
            let hovered = hitbox.is_hovered(window);
            let changed = {
                let mut inner = state.0.borrow_mut();
                std::mem::replace(&mut inner.hovered, hovered) != hovered
            };
            if changed {
                cx.notify(owner);
            }
        }
    });
    window.on_mouse_event({
        let state = state.clone();
        move |ev: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble
                && ev.button == MouseButton::Left
                && state.0.borrow_mut().grab.take().is_some()
            {
                cx.notify(owner);
            }
        }
    });
}
