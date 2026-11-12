//! A short-lived message at the left of the status bar, such as why a save
//! failed. It shows for [`DURATION`], and the next message replaces it.

use std::time::Duration;

use gpui::{App, Global, SharedString};

/// How long a message stays.
pub const DURATION: Duration = Duration::from_secs(4);

#[derive(Default)]
struct StatusMessage {
    text: Option<SharedString>,
    /// Bumped per message, so an older message's timer leaves a newer one.
    generation: u64,
}

impl Global for StatusMessage {}

/// Shows `text` in the status bar for [`DURATION`], replacing the message
/// shown now.
pub fn show(text: impl Into<SharedString>, cx: &mut App) {
    let message = cx.default_global::<StatusMessage>();
    message.generation += 1;
    message.text = Some(text.into());
    let generation = message.generation;
    cx.spawn(async move |cx| {
        cx.background_executor().timer(DURATION).await;
        cx.update(|cx| {
            if cx.global::<StatusMessage>().generation == generation {
                cx.global_mut::<StatusMessage>().text = None;
            }
        });
    })
    .detach();
}

/// The message shown now.
pub fn current(cx: &App) -> Option<SharedString> {
    cx.try_global::<StatusMessage>()?.text.clone()
}

/// Calls `f` whenever the message changes.
pub(crate) fn observe<T: 'static>(
    cx: &mut gpui::Context<T>,
    f: impl FnMut(&mut T, &mut gpui::Context<T>) + 'static,
) -> gpui::Subscription {
    cx.observe_global::<StatusMessage>(f)
}
