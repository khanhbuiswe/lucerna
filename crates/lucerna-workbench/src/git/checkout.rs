//! Switching branches: the title bar branch button and `git.checkout`
//! open the branch picker in Checkout mode, gated on the git state, and
//! its choice runs `git switch` in the background, one at a time. Git's
//! errors show in a native prompt.

use std::sync::Arc;

use futures::channel::oneshot;
use gpui::{AppContext, Context, PromptLevel, Window};
use lucerna_git::{BranchRef, GitError, SwitchTarget, is_valid_branch_name};

use super::GitState;
use crate::Workbench;
use crate::git::branch_picker::BranchPicker;
use crate::status_message;
use crate::tasks::guarded;

const GIT_OFF: &str = "Git is off";
const GIT_LOADING: &str = "Git is still loading";
const GIT_UNAVAILABLE: &str = "Git is not available";
const UNSAVED_FILES: &str = "Save or revert unsaved files before switching branches";
const SWITCH_FAILED: &str = "Could not switch branches";
const SWITCHED_WITH_ERROR: &str = "Switched branches, but git reported an error";

/// Where the Checkout-mode branch picker shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutAnchor {
    /// Under the title bar's branch button, left-aligned to it.
    TitleBar,
    /// Where Quick Open shows.
    Center,
}

impl Workbench {
    /// Whether a `git switch` is running.
    pub fn is_switching_branch(&self) -> bool {
        self.switching_branch
    }

    /// How many switches have started, for tests.
    #[doc(hidden)]
    pub fn switches_started(&self) -> usize {
        self.switches_started
    }

    /// Lists the branches in the background, then shows the branch picker
    /// in Checkout mode at `anchor`. Ignored while a switch runs; without
    /// a usable repository it says why instead (Restricted Mode opens the
    /// trust modal).
    pub fn show_checkout_picker(
        &mut self,
        anchor: CheckoutAnchor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.switching_branch {
            return;
        }
        let git = self.git.read(cx);
        let message = match git.state() {
            GitState::Restricted => {
                self.show_trust_modal(window, cx);
                return;
            }
            GitState::Off => Some(GIT_OFF),
            GitState::Locating => Some(GIT_LOADING),
            GitState::NoGit | GitState::NoRepo => Some(GIT_UNAVAILABLE),
            GitState::Ready | GitState::TooManyChanges => None,
        };
        let repo = git.repo();
        let (None, Some(repo)) = (message, repo) else {
            status_message::show(message.unwrap_or(GIT_UNAVAILABLE), cx);
            return;
        };
        let checkout_shown = self
            .branch_picker
            .as_ref()
            .is_some_and(|shown| shown.anchor.is_some());
        if checkout_shown || self.branch_picker_loading.is_some() {
            return;
        }
        self.close_modal_overlays(window, cx);
        let load = cx.spawn_in(window, async move |wb, cx| {
            let branches = cx
                .background_spawn(async move {
                    guarded("git branch list", || repo.branch_list())
                        .unwrap_or_else(|| Err(GitError::Parse("panicked".into())))
                })
                .await
                .unwrap_or_else(|err| {
                    tracing::warn!(%err, "could not list the branches");
                    Vec::new()
                });
            wb.update_in(cx, |wb, window, cx| {
                wb.branch_picker_loading = None;
                wb.open_checkout_picker(branches, anchor, window, cx);
            })
            .ok();
        });
        self.branch_picker_loading = Some(load);
    }

    fn open_checkout_picker(
        &mut self,
        branches: Vec<BranchRef>,
        anchor: CheckoutAnchor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Another overlay opened, or a switch started, while the list loaded.
        if self.other_modal_overlay_shown()
            || self.locations_picker.is_some()
            || self.switching_branch
        {
            return;
        }
        let picker =
            cx.new(|cx| BranchPicker::new_checkout(branches, Arc::new(is_valid_branch_name), cx));
        self.host_branch_picker(picker, Some(anchor), window, cx);
    }

    /// A press on the title bar's branch button: notes whether the picker
    /// it opens is shown, so the press (outside the picker) closes it.
    pub(crate) fn branch_button_pressed(&mut self) {
        self.branch_button_press_closed = self
            .branch_picker
            .as_ref()
            .is_some_and(|shown| shown.anchor == Some(CheckoutAnchor::TitleBar));
    }

    /// A click on the title bar's branch button opens the picker under it,
    /// unless its press closed the picker (so it is not opened again).
    pub(crate) fn branch_button_clicked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.branch_button_press_closed) {
            return;
        }
        self.show_checkout_picker(CheckoutAnchor::TitleBar, window, cx);
    }

    /// Runs `git switch` for `target` in the background; the title bar
    /// says "Switching…" until it ends. Ignored while a switch runs; with
    /// unsaved files (or one in conflict with the disk) it says so and
    /// runs nothing, since git would write under them.
    pub fn switch_branch(
        &mut self,
        target: SwitchTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.switching_branch {
            return;
        }
        let docs = self.documents().read(cx);
        let unsaved = !docs.dirty_paths().is_empty()
            || docs.open_paths().iter().any(|path| docs.has_conflict(path));
        if unsaved {
            status_message::show(UNSAVED_FILES, cx);
            return;
        }
        let Some(repo) = self.git.read(cx).repo() else {
            status_message::show(GIT_UNAVAILABLE, cx);
            return;
        };
        self.switching_branch = true;
        self.switches_started += 1;
        cx.notify();
        let branch = target.local_branch().to_owned();
        cx.spawn_in(window, async move |wb, cx| {
            let result = cx
                .background_spawn(async move {
                    guarded("git switch", || repo.switch_branch(&target))
                        .unwrap_or_else(|| Err(GitError::Parse("git switch panicked".into())))
                })
                .await;
            let prompt = wb
                .update_in(cx, |wb, window, cx| {
                    wb.branch_switched(result, &branch, window, cx)
                })
                .ok()
                .flatten();
            if let Some(answer) = prompt {
                answer.await.ok();
            }
        })
        .detach();
    }

    /// The switch ended, well or not (git may have switched and still
    /// failed, e.g. in a `post-checkout` hook): the statuses rescan and the
    /// title bar reads `HEAD` now, without waiting for the watchers. An
    /// error shows git's message in a prompt, titled by whether `HEAD` now
    /// names `branch`.
    fn branch_switched(
        &mut self,
        result: Result<(), GitError>,
        branch: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<oneshot::Receiver<usize>> {
        self.switching_branch = false;
        self.git.update(cx, |git, cx| git.rescan(cx));
        self.git_branch = crate::git_branch::read_branch(self.root());
        cx.notify();
        let err = result.err()?;
        tracing::warn!(%err, "git switch failed");
        let title = if self.git_branch.as_deref() == Some(branch) {
            SWITCHED_WITH_ERROR
        } else {
            SWITCH_FAILED
        };
        let detail = match &err {
            GitError::Failed { stderr, .. } => stderr.trim().to_string(),
            other => other.to_string(),
        };
        Some(window.prompt(PromptLevel::Critical, title, Some(&detail), &["OK"], cx))
    }
}
