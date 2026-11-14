//! The Changes since ‹branch› tab ([`DiffKind::Branch`], spec §4.7): the
//! worktree (commits and uncommitted changes alike) against the merge
//! base of HEAD with a ref, the one the user picked or else the default
//! branch.
//!
//! Each [`GitStoreEvent::SnapshotChanged`](crate::git::GitStoreEvent)
//! runs `git rev-parse HEAD` and `git merge-base`, two cheap commands that
//! notice a commit, a rebase, a fetch that moved the ref or a ref that
//! went away. Only when HEAD or the merge base changed (or on a refresh)
//! does the file list run again (`git diff --name-status` plus `git
//! status`). A change on disk asks git about the changed paths alone.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{AppContext, Context, Entity};
use lucerna_git::{FileStatus, GitError, Repo};

use super::{DiffKind, DiffView};
use crate::git::GitState;
use crate::tasks::guarded;

/// More changed paths than this list the whole tree again rather than
/// passing each one to git.
const MAX_PATHSPECS: usize = 256;

/// The ref the Changes since tab compares with, as the user picked it
/// (`None`: the default branch). The workbench keeps it, saves it with
/// the workspace and changes it from the branch picker; the tab follows.
pub(crate) struct BranchBase(pub(crate) Option<String>);

/// What the tab knows of its base.
#[derive(Default)]
pub(super) struct BranchState {
    /// HEAD and the merge base the shown list was computed at.
    pub(super) known: Option<(Option<String>, String)>,
    /// The ref does not resolve (or has no merge base with HEAD).
    pub(super) unresolved: bool,
    /// Changed paths waiting for the running listing.
    pub(super) pending_paths: Vec<Arc<Path>>,
    /// Per-path queries started so far.
    pub(super) path_queries: usize,
}

/// The answer of a listing job.
enum BranchListing {
    /// No merge base with the ref.
    Unresolved,
    /// HEAD and the merge base are as last listed.
    Same,
    Listed {
        head: Option<String>,
        merge_base: String,
        entries: Vec<(PathBuf, FileStatus)>,
    },
}

impl DiffView {
    /// Changed paths waiting for the running listing of a branch tab.
    #[doc(hidden)]
    pub fn queued_paths(&self) -> usize {
        self.branch.pending_paths.len()
    }

    /// Per-path git queries a branch tab started so far.
    #[doc(hidden)]
    pub fn path_queries_run(&self) -> usize {
        self.branch.path_queries
    }

    /// Makes the tab follow `base`: picking a ref reloads it in place.
    pub(crate) fn follow_branch_base(&mut self, base: Entity<BranchBase>, cx: &mut Context<Self>) {
        let subscription = cx.observe(&base, |view, base, cx| {
            let chosen = base.read(cx).0.clone();
            view.set_branch_base(chosen, cx);
        });
        self._subscriptions.push(subscription);
    }

    /// Compares with `chosen` (`None`: the default branch) from now on.
    fn set_branch_base(&mut self, chosen: Option<String>, cx: &mut Context<Self>) {
        let DiffKind::Branch { base_ref } = &mut self.kind else {
            return;
        };
        if *base_ref == chosen {
            return;
        }
        *base_ref = chosen;
        self.reload_all = true;
        self.refresh_list(cx);
        cx.notify();
    }

    /// Resolves the ref and, when HEAD or the merge base moved, lists the
    /// changed files again; off the UI thread.
    pub(super) fn refresh_branch_list(&mut self, repo: Repo, cx: &mut Context<Self>) {
        let DiffKind::Branch { base_ref } = &self.kind else {
            return;
        };
        let default = self.git.read(cx).snapshot().default_branch;
        self.shown_ref = base_ref.clone().or(default);
        let Some(base) = self.shown_ref.clone() else {
            // Nothing to resolve; a running listing is dropped.
            self.apply_branch_list(Ok(BranchListing::Unresolved), cx);
            return;
        };
        if self.status_task.is_some() {
            self.relist = true;
            return;
        }
        let known = self.branch.known.clone().filter(|_| !self.reload_all);
        self.status_task = Some(cx.spawn(async move |this, cx| {
            let listed = cx
                .background_spawn(async move {
                    guarded("git branch diff list", || {
                        branch_listing(&repo, &base, known)
                    })
                    .unwrap_or_else(|| Err(GitError::Parse("panicked".into())))
                })
                .await;
            this.update(cx, |view, cx| view.apply_branch_list(listed, cx))
                .ok();
        }));
    }

    fn apply_branch_list(
        &mut self,
        listed: Result<BranchListing, GitError>,
        cx: &mut Context<Self>,
    ) {
        match listed {
            Ok(BranchListing::Listed {
                head,
                merge_base,
                entries,
            }) => {
                self.branch.unresolved = false;
                self.branch.known = Some((head, merge_base.clone()));
                // Runs a queued full listing itself.
                self.apply_list(Ok((Some(merge_base), entries)), cx);
                if self.status_task.is_none() {
                    self.next_branch_listing(cx);
                }
                return;
            }
            Ok(BranchListing::Same) => {}
            Ok(BranchListing::Unresolved) => {
                self.branch.unresolved = true;
                self.branch.known = None;
                self.branch.pending_paths.clear();
                self.listed = true;
                self.rev = None;
                self.update_files(Vec::new(), false, cx);
            }
            Err(err) => tracing::warn!(%err, "git branch diff list failed"),
        }
        self.status_task = None;
        self.next_branch_listing(cx);
        cx.notify();
    }

    /// Runs what waited for the last listing: a full one, else the
    /// changed paths.
    pub(super) fn next_branch_listing(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.relist) {
            self.refresh_list(cx);
        } else if !self.branch.pending_paths.is_empty() {
            let paths = std::mem::take(&mut self.branch.pending_paths);
            self.branch_paths_changed(paths, cx);
        }
    }

    /// Asks git whether `paths` (changed on disk) differ from the merge
    /// base: files join, leave or change status in the list.
    /// Only while the store is [`GitState::Ready`]: after too many
    /// changes git runs on `git.refresh` alone (spec §4.12).
    pub(super) fn branch_paths_changed(&mut self, paths: Vec<Arc<Path>>, cx: &mut Context<Self>) {
        if self.git.read(cx).state() != GitState::Ready {
            return;
        }
        // Asked once the running listing is in, which may predate them.
        if self.status_task.is_some() {
            self.branch.pending_paths.extend(paths);
            return;
        }
        let Some((_, merge_base)) = self.branch.known.clone() else {
            return;
        };
        let Some(repo) = self.git.read(cx).repo() else {
            return;
        };
        if paths.len() > MAX_PATHSPECS {
            // Forgetting the merge base makes the listing run even though
            // HEAD did not move.
            self.branch.known = None;
            self.refresh_list(cx);
            return;
        }
        let specs: Vec<PathBuf> = paths.iter().map(|p| p.to_path_buf()).collect();
        self.branch.path_queries += 1;
        self.status_task = Some(cx.spawn(async move |this, cx| {
            let listed = cx
                .background_spawn(async move {
                    guarded("git branch diff paths", || {
                        repo.changed_since_in(&merge_base, Some(&specs))
                    })
                    .unwrap_or_else(|| Err(GitError::Parse("panicked".into())))
                })
                .await;
            this.update(cx, |view, cx| view.apply_branch_paths(paths, listed, cx))
                .ok();
        }));
    }

    /// Merges the answer for `paths` into the list: listed files under
    /// them that git no longer reports leave.
    fn apply_branch_paths(
        &mut self,
        paths: Vec<Arc<Path>>,
        listed: Result<Vec<(PathBuf, FileStatus)>, GitError>,
        cx: &mut Context<Self>,
    ) {
        self.status_task = None;
        match listed {
            Ok(fresh) => {
                let asked = |p: &Path| paths.iter().any(|q| p.starts_with(q));
                let mut entries: Vec<(PathBuf, FileStatus)> = self
                    .files
                    .iter()
                    .filter(|(p, _)| !asked(p.as_ref()))
                    .map(|(p, e)| (p.to_path_buf(), e.status))
                    .collect();
                entries.extend(fresh);
                self.update_files(entries, false, cx);
            }
            Err(err) => tracing::warn!(%err, "git branch diff of changed paths failed"),
        }
        self.next_branch_listing(cx);
        cx.notify();
    }
}

/// Blocking: the merge base of HEAD with `base`, and the files changed
/// since it unless HEAD and the merge base are still `known`.
fn branch_listing(
    repo: &Repo,
    base: &str,
    known: Option<(Option<String>, String)>,
) -> Result<BranchListing, GitError> {
    let head = repo.head_oid()?;
    let merge_base = match repo.merge_base(base) {
        Ok(Some(merge_base)) => merge_base,
        // No common history.
        Ok(None) => return Ok(BranchListing::Unresolved),
        // Git refused the ref: it does not exist (any more), or there is
        // no commit yet.
        Err(GitError::Failed { .. }) => return Ok(BranchListing::Unresolved),
        Err(err) => return Err(err),
    };
    if known.as_ref() == Some(&(head.clone(), merge_base.clone())) {
        return Ok(BranchListing::Same);
    }
    let entries = repo.changed_since(&merge_base)?;
    Ok(BranchListing::Listed {
        head,
        merge_base,
        entries,
    })
}
