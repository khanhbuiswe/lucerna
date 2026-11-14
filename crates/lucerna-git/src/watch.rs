//! Watches a repository's git dirs and signals (debounced) when something
//! that affects `git status` changed.

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
/// the diff base. For a linked worktree watched through the common dir, a
/// leading `worktrees/<name>/` is stripped first.
pub fn is_relevant_git_path(rel: &Path) -> bool {
    let mut parts: Vec<&std::ffi::OsStr> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s),
            _ => None,
        })
        .collect();
    if parts.first().is_some_and(|p| *p == "worktrees") {
        if parts.len() < 3 {
            return false; // the worktrees dir or an entry itself
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

/// Keeps the OS watcher and the debounce thread alive; dropping it stops both.
pub struct GitDirWatcher {
    // Dropped first, which closes the event channel and ends the thread.
    _watcher: RecommendedWatcher,
    _thread: JoinHandle<()>,
}

impl GitDirWatcher {
    /// Watches each dir recursively (dirs nested inside another listed dir are
    /// covered by it) and calls `on_change` once per debounced burst of
    /// relevant events.
    pub fn start(
        dirs: &[PathBuf],
        on_change: impl Fn() + Send + 'static,
    ) -> notify::Result<GitDirWatcher> {
        // FSEvents reports real paths (/private/var/...).
        let canon: Vec<PathBuf> = dirs
            .iter()
            .map(|d| std::fs::canonicalize(d).unwrap_or_else(|_| d.clone()))
            .collect();
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
            .spawn(move || debounce_loop(&rx, &roots, &on_change))
            .map_err(notify::Error::io)?;
        Ok(GitDirWatcher {
            _watcher: watcher,
            _thread: thread,
        })
    }
}

fn debounce_loop(rx: &Receiver<notify::Result<Event>>, roots: &[PathBuf], on_change: &dyn Fn()) {
    while let Ok(first) = rx.recv() {
        let started = Instant::now();
        let mut relevant = is_relevant_event(&first, roots);
        let mut closed = false;
        loop {
            let left = MAX_BATCH.saturating_sub(started.elapsed()).min(DEBOUNCE);
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left) {
                Ok(event) => relevant |= is_relevant_event(&event, roots),
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

fn is_relevant_event(res: &notify::Result<Event>, roots: &[PathBuf]) -> bool {
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
            .is_some_and(is_relevant_git_path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(s: &str) -> bool {
        is_relevant_git_path(Path::new(s))
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
        assert!(rel("worktrees/wt/HEAD"));
        assert!(rel("worktrees/wt/index"));
        assert!(rel("refs/heads/x"));
        assert!(!rel("worktrees/wt/logs/HEAD"));
        assert!(!rel("worktrees/wt/index.lock"));
        assert!(!rel("worktrees/wt"));
        assert!(!rel("worktrees"));
    }
}
