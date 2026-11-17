//! The status-bar warning shown when a file watcher could not start: the
//! worktree's (kept until the window closes, as it is not retried) or the
//! `.git` directory's (cleared when the [`GitStore`] restarts).

use gpui::{
    App, Context, Entity, IntoElement, Render, Subscription, Window, div, prelude::*, px, svg,
};

use crate::AppState;
use crate::git::{GitStore, GitStoreEvent};
use crate::icons;
use crate::layout::color;
use crate::ui::tooltip;

const ICON_SIZE: f32 = 12.;

/// One tooltip line: `folder`'s watcher failed with `reason`
/// ([`lucerna_project::watch_error_reason`], whose final period is not
/// doubled).
pub(crate) fn watch_off_message(folder: &str, reason: &str) -> String {
    let reason = reason.strip_suffix('.').unwrap_or(reason);
    format!("File watching is off for {folder}: {reason}. Changes made outside Lucerna won't show.")
}

/// A warning icon, hidden while every watcher runs; its tooltip has one
/// line per failed watcher, the worktree's first.
pub struct WatchStatusItem {
    /// The opened folder's name.
    folder: String,
    /// Why the worktree watcher could not start; the latest failure wins.
    worktree_error: Option<String>,
    git: Entity<GitStore>,
    _git: Subscription,
}

impl WatchStatusItem {
    pub(crate) fn new(folder: String, git: Entity<GitStore>, cx: &mut Context<Self>) -> Self {
        WatchStatusItem {
            folder,
            worktree_error: None,
            _git: cx.subscribe(&git, |_, _, ev: &GitStoreEvent, cx| {
                if let GitStoreEvent::StateChanged = ev {
                    cx.notify();
                }
            }),
            git,
        }
    }

    /// Records that the worktree watcher could not start.
    pub(crate) fn set_worktree_error(&mut self, reason: String, cx: &mut Context<Self>) {
        self.worktree_error = Some(reason);
        cx.notify();
    }

    /// The tooltip, or `None` while every watcher runs.
    pub fn tooltip_text(&self, cx: &App) -> Option<String> {
        let worktree = self
            .worktree_error
            .as_deref()
            .map(|reason| watch_off_message(&self.folder, reason));
        let git = self
            .git
            .read(cx)
            .watch_error()
            .map(|reason| watch_off_message(".git", reason));
        let lines: Vec<String> = worktree.into_iter().chain(git).collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }
}

impl Render for WatchStatusItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = self.tooltip_text(cx);
        let warning = color(cx.global::<AppState>().theme.ui.warning);
        div()
            .id("watch-off")
            .flex()
            .items_center()
            .when(text.is_none(), |d| d.hidden())
            .when_some(text, |d, text| {
                d.debug_selector(|| "watch-off".into())
                    .child(
                        svg()
                            .data(icons::TRIANGLE_ALERT)
                            .size(px(ICON_SIZE))
                            .text_color(warning),
                    )
                    .tooltip(tooltip(text))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_does_not_double_the_period() {
        assert_eq!(
            watch_off_message("src", "No path was found."),
            "File watching is off for src: No path was found. Changes made outside Lucerna won't show."
        );
        assert_eq!(
            watch_off_message(".git", "limit"),
            "File watching is off for .git: limit. Changes made outside Lucerna won't show."
        );
    }

    /// A real failure names the paths it is about; the tooltip does not.
    #[test]
    fn message_from_a_notify_error_with_paths() {
        let err = notify::Error::new(notify::ErrorKind::MaxFilesWatch)
            .add_path("/home/u/proj/node_modules/x/y".into());
        assert_eq!(
            watch_off_message("proj", &lucerna_project::watch_error_reason(&err)),
            "File watching is off for proj: OS file watch limit reached. Changes made outside Lucerna won't show."
        );
        let err = notify::Error::path_not_found().add_path("/home/u/proj".into());
        assert_eq!(
            watch_off_message("proj", &lucerna_project::watch_error_reason(&err)),
            "File watching is off for proj: No path was found. Changes made outside Lucerna won't show."
        );
    }
}
