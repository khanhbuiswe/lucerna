//! The bar over the editor when the file on disk changed under unsaved
//! changes or was deleted (spec §7.3): one line under the path bar, built
//! from the find bar's toolbar and row so it has the same height,
//! background and border.

use gpui::{App, Div, ParentElement, Styled, prelude::*, px};

use crate::AppState;
use crate::documents::DiskNotice;
use crate::layout::color;
use crate::ui::{
    ClickHandler, UI_FONT_SIZE_SMALL, search_bar_row, search_toolbar, text_button, warning_row,
};

/// What the bar's buttons do: `primary` is Reload (Close for a deleted
/// file, Dismiss for invalid bytes), `secondary` Keep Mine (Keep), absent
/// on the bar for invalid bytes.
pub(crate) struct BarActions {
    pub(crate) primary: ClickHandler,
    pub(crate) secondary: Option<ClickHandler>,
}

/// The bar's message for the file called `name`.
pub(crate) fn message(name: &str, notice: &DiskNotice) -> String {
    match notice {
        DiskNotice::Conflict { reason: None } => {
            format!("{name} changed on disk and conflicts with your unsaved changes.")
        }
        DiskNotice::Conflict { reason: Some(err) } => {
            format!("{name} changed on disk and can't be read: {err}")
        }
        DiskNotice::Deleted => format!("{name} was deleted on disk."),
        DiskNotice::Lossy { encoding } => format!(
            "This file contains bytes that aren't valid {encoding}. Saving will replace them with \u{FFFD}."
        ),
    }
}

/// The bar for group `group` showing the file called `name`. Its debug
/// selector is `conflict-bar-{group}` (the message
/// `conflict-bar-message-{group}`); the buttons are
/// `conflict-bar-reload-{group}` and `conflict-bar-keep-mine-{group}`, or
/// for a deleted file `deleted-bar-close-{group}` and
/// `deleted-bar-keep-{group}`, or for invalid bytes the one button
/// `lossy-bar-dismiss-{group}`.
pub(crate) fn render(
    group: usize,
    name: &str,
    notice: &DiskNotice,
    actions: BarActions,
    cx: &App,
) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    let ((primary_id, primary), secondary) = match notice {
        DiskNotice::Conflict { .. } => (
            (format!("conflict-bar-reload-{group}"), "Reload"),
            Some((format!("conflict-bar-keep-mine-{group}"), "Keep Mine")),
        ),
        DiskNotice::Deleted => (
            (format!("deleted-bar-close-{group}"), "Close"),
            Some((format!("deleted-bar-keep-{group}"), "Keep")),
        ),
        DiskNotice::Lossy { .. } => ((format!("lossy-bar-dismiss-{group}"), "Dismiss"), None),
    };
    let BarActions {
        primary: on_primary,
        secondary: on_secondary,
    } = actions;
    let text = warning_row(
        format!("conflict-bar-message-{group}"),
        message(name, notice),
        cx,
    )
    .flex_1()
    .min_w_0()
    .overflow_hidden()
    .whitespace_nowrap()
    .text_ellipsis()
    .text_size(px(UI_FONT_SIZE_SMALL))
    .text_color(color(ui.text));
    search_toolbar(cx)
        .debug_selector(move || format!("conflict-bar-{group}"))
        .child(
            search_bar_row()
                .child(text)
                .child(text_button(primary_id, primary, cx).on_click(on_primary))
                .children(
                    secondary.zip(on_secondary).map(|((id, label), on_click)| {
                        text_button(id, label, cx).on_click(on_click)
                    }),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy() {
        assert_eq!(
            message("a.rs", &DiskNotice::Conflict { reason: None }),
            "a.rs changed on disk and conflicts with your unsaved changes."
        );
        assert_eq!(
            message(
                "a.rs",
                &DiskNotice::Conflict {
                    reason: Some("stream did not contain valid UTF-8".into())
                }
            ),
            "a.rs changed on disk and can't be read: stream did not contain valid UTF-8"
        );
        assert_eq!(
            message("a.rs", &DiskNotice::Deleted),
            "a.rs was deleted on disk."
        );
        assert_eq!(
            message("a.rs", &DiskNotice::Lossy { encoding: "UTF-8" }),
            "This file contains bytes that aren't valid UTF-8. Saving will replace them with \u{FFFD}."
        );
    }
}
