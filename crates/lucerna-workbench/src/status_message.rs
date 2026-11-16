//! A short-lived message at the left of a window's status bar, such as why a save
//! failed. It shows for [`DURATION`], and the next message replaces it.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{App, Global, SharedString, WindowId};

/// How long a message stays.
pub const DURATION: Duration = Duration::from_secs(4);

#[derive(Default)]
struct StatusMessages {
    /// Each window's message and its generation.
    by_window: HashMap<WindowId, (SharedString, u64)>,
    /// Bumped per message, so an older message's timer leaves a newer one.
    generation: u64,
}

impl Global for StatusMessages {}

/// Shows `text` in `window`'s status bar for [`DURATION`], replacing the
/// message shown there now. Other windows are not touched.
pub fn show(window: WindowId, text: impl Into<SharedString>, cx: &mut App) {
    let messages = cx.default_global::<StatusMessages>();
    messages.generation += 1;
    let generation = messages.generation;
    messages.by_window.insert(window, (text.into(), generation));
    cx.spawn(async move |cx| {
        cx.background_executor().timer(DURATION).await;
        cx.update(|cx| {
            let messages = cx.global::<StatusMessages>();
            if messages.by_window.get(&window).map(|m| m.1) == Some(generation) {
                cx.global_mut::<StatusMessages>().by_window.remove(&window);
            }
        });
    })
    .detach();
}

/// Shows `text` in `window`'s status bar; with no window, only logs it.
pub fn show_or_log(window: Option<WindowId>, text: impl Into<SharedString>, cx: &mut App) {
    let text = text.into();
    match window {
        Some(window) => show(window, text, cx),
        None => tracing::warn!(message = %text, "status message with no window"),
    }
}

/// The message `window` shows now.
pub fn current(window: WindowId, cx: &App) -> Option<SharedString> {
    let messages = cx.try_global::<StatusMessages>()?;
    messages.by_window.get(&window).map(|m| m.0.clone())
}

/// Drops `window`'s message; the router calls it when the window closes.
pub(crate) fn forget(window: WindowId, cx: &mut App) {
    if let Some(messages) = cx.try_global::<StatusMessages>()
        && messages.by_window.contains_key(&window)
    {
        cx.global_mut::<StatusMessages>().by_window.remove(&window);
    }
}

/// Calls `f` whenever the message changes.
pub(crate) fn observe<T: 'static>(
    cx: &mut gpui::Context<T>,
    f: impl FnMut(&mut T, &mut gpui::Context<T>) + 'static,
) -> gpui::Subscription {
    cx.observe_global::<StatusMessages>(f)
}
