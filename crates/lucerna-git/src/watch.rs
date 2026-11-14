//! Watches a repository's git dirs and signals (debounced) when something
//! that affects `git status` changed.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, unbounded};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

/// A burst ends once no event arrived for this long...
const DEBOUNCE: Duration = Duration::from_millis(50);
/// ...or when it has been collecting for this long.
const MAX_BATCH: Duration = Duration::from_millis(100);

const IGNORED_DIRS: &[&str] = &[
    "objects",
    "hooks",
    "lfs",
    "logs",
    "rebase-merge",
    "rebase-apply",
    "sequencer",
    "fsmonitor--daemon",
];
const IGNORED_FILES: &[&str] = &["COMMIT_EDITMSG", "FETCH_HEAD", "ORIG_HEAD", "gc.pid"];

/// Whether a change at `rel` (relative to a git dir) can alter the status or
/// the diff base. `own_worktree` is the name of the window's linked worktree
/// (its git dir is `<common dir>/worktrees/<name>`), `None` in the main
/// worktree. Under the common dir's `worktrees/`, only the own worktree's
/// entries count, with the `worktrees/<name>/` prefix stripped: other
/// worktrees' indexes and `HEAD`s do not affect this one.
pub fn is_relevant_git_path(rel: &Path, own_worktree: Option<&OsStr>) -> bool {
    let mut parts: Vec<&OsStr> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s),
            _ => None,
        })
        .collect();
    if parts.first().is_some_and(|p| *p == "worktrees") {
        if parts.len() < 3 || own_worktree != Some(parts[1]) {
            // The worktrees dir, an entry itself, or another worktree.
            return false;
        }
        parts.drain(..2);
    }
    let Some(last) = parts.last().and_then(|s| s.to_str()) else {
        return false;
    };
    if parts
        .first()
        .and_then(|s| s.to_str())
        .is_some_and(|first| IGNORED_DIRS.contains(&first))
    {
        return false;
    }
    !(IGNORED_FILES.contains(&last) || last.ends_with(".lock"))
}

/// The linked worktree name of `git_dir`: its basename when it sits in
/// `<common_dir>/worktrees/`.
fn worktree_name(git_dir: &Path, common_dir: &Path) -> Option<OsString> {
    if git_dir.parent()? != common_dir.join("worktrees") {
        return None;
    }
    git_dir.file_name().map(OsString::from)
}

/// Keeps the OS watcher and the debounce thread alive; dropping it stops both.
pub struct GitDirWatcher {
    // Dropped first, which closes the event channel and ends the thread.
    _watcher: RecommendedWatcher,
    _thread: JoinHandle<()>,
}

impl GitDirWatcher {
    /// Watches a repository's git dir and common dir recursively (equal in
    /// the main worktree; a linked worktree's git dir sits inside the common
    /// dir and is covered by it) and calls `on_change` once per debounced
    /// burst of relevant events. Blocking: it canonicalizes and registers
    /// the watches (a recursive walk on some platforms).
    pub fn start(
        git_dir: &Path,
        common_dir: &Path,
        on_change: impl Fn() + Send + 'static,
    ) -> notify::Result<GitDirWatcher> {
        // FSEvents reports real paths (/private/var/...).
        let canonical = |d: &Path| std::fs::canonicalize(d).unwrap_or_else(|_| d.to_path_buf());
        let canon = [canonical(git_dir), canonical(common_dir)];
        let own_worktree = worktree_name(&canon[0], &canon[1]);
        let mut roots: Vec<PathBuf> = Vec::new();
        for d in &canon {
            let covered = canon.iter().any(|o| o != d && d.starts_with(o));
            if !covered && !roots.contains(d) {
                roots.push(d.clone());
            }
        }
        let (tx, rx) = unbounded();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;
        for root in &roots {
            watcher.watch(root, RecursiveMode::Recursive)?;
        }
        let thread = thread::Builder::new()
            .name("lucerna-git-watch".into())
            .spawn(move || debounce_loop(&rx, &roots, own_worktree.as_deref(), &on_change))
            .map_err(notify::Error::io)?;
        Ok(GitDirWatcher {
            _watcher: watcher,
            _thread: thread,
        })
    }
}

fn debounce_loop(
    rx: &Receiver<notify::Result<Event>>,
    roots: &[PathBuf],
    own_worktree: Option<&OsStr>,
    on_change: &dyn Fn(),
) {
    while let Ok(first) = rx.recv() {
        let started = Instant::now();
        let mut relevant = is_relevant_event(&first, roots, own_worktree);
        let mut closed = false;
        loop {
            let left = MAX_BATCH.saturating_sub(started.elapsed()).min(DEBOUNCE);
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left) {
                Ok(event) => relevant |= is_relevant_event(&event, roots, own_worktree),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        if relevant {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(on_change));
            if result.is_err() {
                tracing::error!("git watcher callback panicked");
            }
        }
        if closed {
            return;
        }
    }
}

fn is_relevant_event(
    res: &notify::Result<Event>,
    roots: &[PathBuf],
    own_worktree: Option<&OsStr>,
) -> bool {
    let event = match res {
        Ok(event) => event,
        // Events may have been lost: refresh to be safe.
        Err(_) => return true,
    };
    if event.need_rescan() {
        return true;
    }
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    event.paths.iter().any(|path| {
        // The most specific root the path falls under.
        roots
            .iter()
            .filter_map(|r| path.strip_prefix(r).ok())
            .min_by_key(|rel| rel.components().count())
            .is_some_and(|rel| is_relevant_git_path(rel, own_worktree))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(s: &str) -> bool {
        is_relevant_git_path(Path::new(s), None)
    }

    fn rel_in(worktree: &str, s: &str) -> bool {
        is_relevant_git_path(Path::new(s), Some(OsStr::new(worktree)))
    }

    #[test]
    fn relevant_paths() {
        for p in [
            "index",
            "HEAD",
            "refs/heads/main",
            "packed-refs",
            "MERGE_HEAD",
        ] {
            assert!(rel(p), "{p}");
        }
        for p in [
            "objects/ab/cd",
            "objects",
            "index.lock",
            "refs/heads/main.lock",
            "logs/HEAD",
            "FETCH_HEAD",
            "hooks/x",
            "COMMIT_EDITMSG",
            "ORIG_HEAD",
            "gc.pid",
            "rebase-merge/head-name",
            "",
        ] {
            assert!(!rel(p), "{p}");
        }
    }

    #[test]
    fn linked_worktree_prefix() {
        assert!(rel_in("wt", "worktrees/wt/HEAD"));
        assert!(rel_in("wt", "worktrees/wt/index"));
        assert!(rel_in("wt", "refs/heads/x"));
        assert!(!rel_in("wt", "worktrees/wt/logs/HEAD"));
        assert!(!rel_in("wt", "worktrees/wt/index.lock"));
        assert!(!rel_in("wt", "worktrees/wt"));
        assert!(!rel_in("wt", "worktrees"));
    }

    #[test]
    fn other_worktrees_are_irrelevant() {
        // From the main worktree, every linked worktree's files.
        assert!(!rel("worktrees/other/index"));
        assert!(!rel("worktrees/other/HEAD"));
        assert!(!rel("worktrees/x/HEAD"));
        // From worktree `x`, a sibling's.
        assert!(!rel_in("x", "worktrees/other/index"));
        assert!(!rel_in("x", "worktrees/other/HEAD"));
        assert!(rel_in("x", "worktrees/x/HEAD"));
    }

    #[test]
    fn worktree_name_of_git_dir() {
        let common = Path::new("/r/.git");
        assert_eq!(worktree_name(common, common), None);
        assert_eq!(
            worktree_name(&common.join("worktrees").join("x"), common).as_deref(),
            Some(OsStr::new("x"))
        );
        assert_eq!(worktree_name(Path::new("/elsewhere/x"), common), None);
    }
}
