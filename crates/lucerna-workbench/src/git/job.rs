//! Status jobs: the serial queue's unit of work and how one runs.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lucerna_git::{FileStatus, GitError, Repo};
use lucerna_text::BufferSnapshot;

use super::hunks::{Diffed, HunkWork};
use super::rollup::Rollup;
use super::{DiffBase, GitSnapshot};

/// A `RefreshPaths` with more paths than this becomes a `Rescan`, so the
/// pathspec list passed to `git` stays bounded.
const MAX_REFRESH_PATHS: usize = 256;

/// A failed job, and whether it failed because the repository is gone.
pub(super) struct JobError {
    pub(super) err: GitError,
    pub(super) repo_gone: bool,
}

impl JobError {
    /// Blocking: checks whether `repo`'s git dir still exists.
    pub(super) fn new(err: GitError, repo: &Repo) -> JobError {
        let repo_gone = err.is_not_a_repo() || !repo.git_dir.is_dir();
        JobError { err, repo_gone }
    }
}

pub(super) enum Job {
    Rescan,
    RefreshPaths(BTreeSet<Arc<Path>>),
}

impl Job {
    /// Refreshes `paths`, or rescans when they exceed `MAX_REFRESH_PATHS`.
    pub(super) fn refresh(paths: impl IntoIterator<Item = Arc<Path>>) -> Job {
        Job::RefreshPaths(paths.into_iter().collect()).bounded()
    }

    /// The job that replaces `self` (queued) when `next` arrives.
    pub(super) fn merge(self, next: Job) -> Job {
        match (self, next) {
            (Job::RefreshPaths(mut a), Job::RefreshPaths(b)) => {
                a.extend(b);
                Job::RefreshPaths(a).bounded()
            }
            _ => Job::Rescan,
        }
    }

    fn bounded(self) -> Job {
        match self {
            Job::RefreshPaths(paths) if paths.len() > MAX_REFRESH_PATHS => Job::Rescan,
            job => job,
        }
    }
}

pub(super) struct JobInput {
    pub(super) repo: Repo,
    pub(super) job: Job,
    pub(super) diff_base: DiffBase,
    pub(super) status_limit: usize,
    pub(super) prev: GitSnapshot,
    /// Tracked buffers whose base text a `Rescan` reloads.
    pub(super) buffers: Vec<(Arc<Path>, u64, BufferSnapshot)>,
}

pub(super) struct JobOutput {
    pub(super) snapshot: GitSnapshot,
    pub(super) too_many: bool,
    /// Per tracked buffer (as of the job's start): version, base text, hunks.
    pub(super) reloaded: Vec<(Arc<Path>, u64, Diffed)>,
}

/// Statuses against the diff base; `paths` limits them (workspace-relative).
fn status_entries(
    repo: &Repo,
    diff_base: DiffBase,
    base_rev: Option<&str>,
    paths: Option<&[PathBuf]>,
) -> Result<Vec<(Arc<Path>, FileStatus)>, GitError> {
    let entries = match (diff_base, base_rev) {
        (DiffBase::DefaultBranch, Some(rev)) => repo.changed_since_in(rev, paths)?,
        _ => repo.status(paths)?,
    };
    Ok(entries
        .into_iter()
        .map(|(p, s)| (Arc::from(p), s))
        .collect())
}

/// One status job. Blocking: runs on the background executor.
pub(super) fn run_job(input: JobInput) -> Result<JobOutput, GitError> {
    let JobInput {
        repo,
        job,
        diff_base,
        status_limit,
        prev,
        buffers,
    } = input;
    let (statuses, head, base_rev, default_branch) = match &job {
        Job::Rescan => {
            let head = repo.head_oid()?;
            let default_branch = repo.default_branch()?;
            let base_rev = match (diff_base, &default_branch, &head) {
                (DiffBase::DefaultBranch, Some(branch), Some(_)) => {
                    repo.merge_base(branch)?.or_else(|| head.clone())
                }
                _ => head.clone(),
            };
            let entries = status_entries(&repo, diff_base, base_rev.as_deref(), None)?;
            let statuses: HashMap<_, _> = entries.into_iter().collect();
            (statuses, head, base_rev, default_branch)
        }
        Job::RefreshPaths(paths) => {
            let list: Vec<PathBuf> = paths.iter().map(|p| p.to_path_buf()).collect();
            let fresh = status_entries(&repo, diff_base, prev.base_rev.as_deref(), Some(&list))?;
            let mut statuses = (*prev.statuses).clone();
            statuses.retain(|k, _| !paths.iter().any(|p| k.starts_with(p)));
            statuses.extend(fresh);
            (statuses, prev.head, prev.base_rev, prev.default_branch)
        }
    };
    let entries: Vec<(Arc<Path>, FileStatus)> =
        statuses.iter().map(|(p, s)| (p.clone(), *s)).collect();
    let rollup = Rollup::build(&entries);
    let reloaded = buffers
        .into_iter()
        .map(|(path, version, snapshot)| {
            let work = HunkWork {
                repo: repo.clone(),
                path: path.clone(),
                snapshot,
                rev: base_rev.clone(),
                cached: None,
                diff_base,
            };
            (path, version, work.run())
        })
        .collect();
    Ok(JobOutput {
        too_many: statuses.len() > status_limit,
        snapshot: GitSnapshot {
            generation: 0,
            statuses: Arc::new(statuses),
            rollup: Arc::new(rollup),
            head,
            base_rev,
            default_branch,
        },
        reloaded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(range: std::ops::Range<usize>) -> Vec<Arc<Path>> {
        range
            .map(|i| Arc::from(Path::new(&format!("f{i}.txt"))))
            .collect()
    }

    #[test]
    fn refresh_over_path_limit_becomes_rescan() {
        assert!(matches!(Job::refresh(paths(0..256)), Job::RefreshPaths(p) if p.len() == 256));
        assert!(matches!(Job::refresh(paths(0..257)), Job::Rescan));
    }

    #[test]
    fn merged_refresh_over_path_limit_becomes_rescan() {
        let merged = Job::refresh(paths(0..200)).merge(Job::refresh(paths(100..256)));
        assert!(matches!(merged, Job::RefreshPaths(p) if p.len() == 256));
        let merged = Job::refresh(paths(0..200)).merge(Job::refresh(paths(200..300)));
        assert!(matches!(merged, Job::Rescan));
    }
}
