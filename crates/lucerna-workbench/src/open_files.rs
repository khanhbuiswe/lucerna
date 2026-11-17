//! Follows open files that the worktree does not: files outside the root
//! (the user's `settings.json`, a file opened by absolute path) and
//! ignored ones (`.env`, files under `target/`). One watcher follows the
//! parent directory of each, non-recursively, or its nearest existing
//! ancestor while the parent is removed (`dist/` during a clean build),
//! moving back down after each batch. It reports writes to those
//! files in batches debounced by [`OPEN_FILE_DEBOUNCE`]; the workbench
//! hands them to its documents as it does worktree changes.
//!
//! A file reached through a symlink (`AGENTS.md -> CLAUDE.md`) is
//! reported as given when its target changes: the target's directory is
//! watched too.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, unbounded};
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::config_files::real_path;
use crate::tasks::try_spawn_guarded;

/// A batch ends once no event arrived for this long...
pub const OPEN_FILE_DEBOUNCE: Duration = Duration::from_millis(50);
/// ...or when it has been collecting for this long.
const MAX_BATCH: Duration = Duration::from_millis(200);

/// Watches the parent directories of a set of files and reports changes
/// to those files only. Dropping it stops watching.
pub struct OpenFileWatcher {
    state: Arc<Mutex<WatchState>>,
}

struct WatchState {
    watcher: RecommendedWatcher,
    /// Each file as given, with its real path when given, which still
    /// names a link's target once the target is removed.
    files: Vec<(PathBuf, PathBuf)>,
    /// Directories watched, non-recursively.
    dirs: Vec<PathBuf>,
}

impl OpenFileWatcher {
    /// Starts with no files; `on_batch` runs on the watcher's thread with
    /// the files (as given to [`OpenFileWatcher::set_files`]) that changed.
    ///
    /// # Errors
    /// When the OS watcher or its thread cannot be created.
    pub fn start(on_batch: impl Fn(Vec<PathBuf>) + Send + 'static) -> notify::Result<Self> {
        let (tx, rx) = unbounded();
        let watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;
        let state = Arc::new(Mutex::new(WatchState {
            watcher,
            files: Vec::new(),
            dirs: Vec::new(),
        }));
        let weak = Arc::downgrade(&state);
        try_spawn_guarded("lucerna-open-files-watch", move || {
            debounce_loop(&rx, &weak, &on_batch);
        })
        .map_err(notify::Error::io)?;
        Ok(OpenFileWatcher { state })
    }

    /// Follows exactly `files` from now on. Call it again when a link
    /// among them may point elsewhere: links are resolved here.
    pub fn set_files(&self, files: Vec<PathBuf>) {
        lock(&self.state).set_files(files);
    }
}

fn lock(state: &Mutex<WatchState>) -> std::sync::MutexGuard<'_, WatchState> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

impl WatchState {
    fn set_files(&mut self, files: Vec<PathBuf>) {
        let files: Vec<(PathBuf, PathBuf)> = files
            .into_iter()
            .map(|f| {
                let real = real_path(&f);
                (f, real)
            })
            .collect();
        if files == self.files {
            return;
        }
        self.files = files;
        self.watch_dirs();
    }

    /// Watches the directories `self.files` need, and only those: each
    /// file's parent, or its nearest existing ancestor once the parent is
    /// removed. Returns the directories watched by this call.
    fn watch_dirs(&mut self) -> Vec<PathBuf> {
        let mut wanted = BTreeSet::new();
        for (file, real) in &self.files {
            let Some(parent) = file.parent() else {
                continue;
            };
            wanted.insert(parent.to_path_buf());
            // A link elsewhere: its target's directory reports the writes.
            if let Some(real_parent) = real.parent()
                && real_path(parent) != real_parent
            {
                wanted.insert(real_parent.to_path_buf());
            }
        }
        let wanted: BTreeSet<PathBuf> = wanted
            .iter()
            .filter_map(|dir| nearest_existing_dir(dir))
            .collect();
        for dir in &self.dirs {
            if !wanted.contains(dir) {
                let _ = self.watcher.unwatch(dir);
            }
        }
        let mut dirs = Vec::new();
        let mut added = Vec::new();
        for dir in wanted {
            if self.dirs.contains(&dir) {
                dirs.push(dir);
                continue;
            }
            match self.watcher.watch(&dir, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    added.push(dir.clone());
                    dirs.push(dir);
                }
                Err(err) => tracing::warn!(path = %dir.display(), %err, "could not watch"),
            }
        }
        self.dirs = dirs;
        added
    }

    /// Moves the watches to follow folders that were removed or came
    /// back, and returns the files (as given) in a directory watched just
    /// now: they may have been written before the watch got there.
    fn rewatch(&mut self) -> Vec<PathBuf> {
        let added = self.watch_dirs();
        if added.is_empty() {
            return Vec::new();
        }
        self.files
            .iter()
            .filter(|(file, real)| {
                [file.parent(), real.parent()]
                    .into_iter()
                    .flatten()
                    .any(|dir| added.iter().any(|a| a == dir))
            })
            .map(|(file, _)| file.clone())
            .collect()
    }
}

/// The first of `dir` and its ancestors that is a directory.
fn nearest_existing_dir(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find(|d| d.is_dir()).map(Path::to_path_buf)
}

fn debounce_loop(
    rx: &Receiver<notify::Result<notify::Event>>,
    state: &Weak<Mutex<WatchState>>,
    on_batch: &dyn Fn(Vec<PathBuf>),
) {
    // Ends once the watcher, which holds the sender, is dropped.
    while let Ok(first) = rx.recv() {
        let started = Instant::now();
        let mut events = vec![first];
        loop {
            let left = MAX_BATCH
                .saturating_sub(started.elapsed())
                .min(OPEN_FILE_DEBOUNCE);
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left) {
                Ok(event) => events.push(event),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
        let Some(state) = state.upgrade() else {
            return;
        };
        let (files, rewatched) = {
            let mut state = lock(&state);
            // Folders may have been removed or created: move the watches
            // now rather than at the next `set_files`.
            let rewatched = state.rewatch();
            (state.files.clone(), rewatched)
        };
        drop(state);
        let (given, reals): (Vec<PathBuf>, Vec<PathBuf>) = files.into_iter().unzip();
        let mut changed = Vec::new();
        let mut everything = false;
        for event in events {
            match event {
                Ok(event) => {
                    everything |= event.need_rescan();
                    changed.extend(event.paths);
                }
                // Events may have been lost.
                Err(_) => everything = true,
            }
        }
        let mut batch: Vec<PathBuf> = if everything {
            given
        } else {
            let changed: BTreeSet<PathBuf> = changed.iter().map(|p| real_path(p)).collect();
            // A link matches by its target now, or by the target it had
            // when set, which a removed target no longer resolves to.
            given
                .into_iter()
                .zip(reals)
                .filter(|(file, real)| changed.contains(&real_path(file)) || changed.contains(real))
                .map(|(file, _)| file)
                .collect()
        };
        for file in rewatched {
            if !batch.contains(&file) {
                batch.push(file);
            }
        }
        if !batch.is_empty() {
            on_batch(batch);
        }
    }
}

/// The `files` that `changed` names. Watchers report real paths
/// (`/private/var` for `/var`), so both sides are compared with symlinks
/// resolved; a removed file resolves through its directory.
pub(crate) fn matching_files(files: &[PathBuf], changed: &[PathBuf]) -> Vec<PathBuf> {
    let changed: BTreeSet<PathBuf> = changed.iter().map(|p| real_path(p)).collect();
    files
        .iter()
        .filter(|f| changed.contains(&real_path(f)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_files_compares_real_paths_and_skips_others() {
        let dir = tempfile::TempDir::new().unwrap();
        let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        std::fs::write(&a, "").unwrap();
        let real_dir = dir.path().canonicalize().unwrap();
        let files = vec![a.clone(), b.clone()];
        // `b` does not exist: it still matches through its directory.
        let changed = vec![
            real_dir.join("a.txt"),
            real_dir.join("b.txt"),
            real_dir.join("c.txt"),
        ];
        assert_eq!(matching_files(&files, &changed), files);
        assert_eq!(
            matching_files(&files, &[real_dir.join("c.txt")]),
            Vec::<PathBuf>::new()
        );
        assert_eq!(matching_files(&files, std::slice::from_ref(&a)), vec![a]);
    }

    #[test]
    fn nearest_existing_dir_walks_up_past_missing_folders() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("dist/js");
        assert_eq!(nearest_existing_dir(&missing), Some(dir.path().to_path_buf()));
        std::fs::create_dir_all(&missing).unwrap();
        assert_eq!(nearest_existing_dir(&missing), Some(missing));
    }
}
