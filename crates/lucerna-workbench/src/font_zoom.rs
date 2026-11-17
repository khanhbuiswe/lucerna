//! Editor font zoom (Cmd+= / Cmd+- / Cmd+0): an offset in pixels added to
//! `editor.fontSize` in every code view and window, for this run of the app
//! only. It is never written to `settings.json`.
//!
//! Views read their size through [`zoomed`]. [`crate::WindowSettings`]
//! notifies when the zoom changes, so every view that observes its settings
//! reads it again.

use gpui::{App, Global};
use lucerna_settings::SettingsView;

use crate::AppState;

/// The smallest editor font size, in pixels.
pub const MIN_FONT_SIZE: f32 = 6.;
/// The largest editor font size, in pixels.
pub const MAX_FONT_SIZE: f32 = 100.;
/// `editor.fontSize` when it is unset.
const DEFAULT_FONT_SIZE: f32 = 14.;

/// The zoom: `delta` pixels over `editor.fontSize`, set while the user's
/// `editor.fontSize` was `user_size`. Once that setting changes, the zoom
/// no longer applies (the setting wins).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontZoom {
    delta: f32,
    user_size: Option<f32>,
}

impl Global for FontZoom {}

/// `editor.fontSize` (for `language`), or 14, clamped to the limits.
pub fn base_size(settings: SettingsView<'_>, language: Option<&str>) -> f32 {
    clamp(
        settings
            .get::<f32>("editor.fontSize", language)
            .unwrap_or(DEFAULT_FONT_SIZE),
    )
}

/// `base` with the zoom applied, clamped to the limits.
pub fn zoomed(base: f32, cx: &App) -> f32 {
    clamp(base + delta(cx))
}

/// Moves the zoom by `by` pixels from `base`; the new size, or `None` when
/// it is already at a limit (and nothing changes).
pub fn step(base: f32, by: f32, cx: &mut App) -> Option<f32> {
    let old = delta(cx);
    let new = next_delta(base, old, by);
    set(old, new, cx).then(|| clamp(base + new))
}

/// Drops the zoom; `Some(base)` when there was one, else `None`.
pub fn reset(base: f32, cx: &mut App) -> Option<f32> {
    set(delta(cx), 0., cx).then(|| clamp(base))
}

/// Drops the zoom for good once the user's `editor.fontSize` changed, so
/// setting it back later does not bring the old zoom back. Called when the
/// settings change; until then [`zoomed`] already ignores a stale zoom.
pub fn drop_stale(cx: &mut App) {
    if let Some(zoom) = cx.try_global::<FontZoom>()
        && zoom.delta != 0.
        && zoom.user_size != user_size(cx)
    {
        cx.remove_global::<FontZoom>();
    }
}

/// The zoom in effect: none once the user's `editor.fontSize` changed.
fn delta(cx: &App) -> f32 {
    match cx.try_global::<FontZoom>() {
        Some(zoom) if zoom.user_size == user_size(cx) => zoom.delta,
        _ => 0.,
    }
}

/// Stores `new` when it differs from `old`; whether it did.
fn set(old: f32, new: f32, cx: &mut App) -> bool {
    if new == old {
        return false;
    }
    let user_size = user_size(cx);
    cx.set_global(FontZoom {
        delta: new,
        user_size,
    });
    true
}

/// The user's `editor.fontSize`, without the folder's settings.
fn user_size(cx: &App) -> Option<f32> {
    cx.try_global::<AppState>()?
        .settings
        .get::<f32>("editor.fontSize", None)
}

/// The delta that moves `base + delta` by `by`, within the limits.
fn next_delta(base: f32, delta: f32, by: f32) -> f32 {
    clamp(base + delta + by) - base
}

fn clamp(size: f32) -> f32 {
    if size.is_nan() {
        return DEFAULT_FONT_SIZE;
    }
    size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_delta_steps() {
        assert_eq!(next_delta(14., 0., 1.), 1.);
        assert_eq!(next_delta(14., 1., -1.), 0.);
    }

    #[test]
    fn next_delta_clamps() {
        assert_eq!(next_delta(14., 0., -100.), -8.);
        assert_eq!(next_delta(99., 0., 1.), 1.);
        assert_eq!(next_delta(99., 1., 1.), 1.);
        assert_eq!(next_delta(99., 1., -1.), 0.);
    }
}
