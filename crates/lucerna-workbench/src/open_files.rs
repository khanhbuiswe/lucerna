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
    /// removed. A directory already in `dirs` is kept as is. Returns the
    /// directories watched by this call.
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
    ///
    /// `changed` holds the batch's event paths. A watched folder among
    /// them may have been removed and created again within the batch; it
    /// is still in `dirs`, but a watch on the old folder (inotify, Windows)
    /// died with it, so it is watched anew. Writes inside a folder do not
    /// name the folder itself and renew nothing.
    fn rewatch(&mut self, changed: &BTreeSet<PathBuf>) -> Vec<PathBuf> {
        let watcher = &mut self.watcher;
        self.dirs.retain(|dir| {
            let replaced = changed.contains(dir) || changed.contains(&real_path(dir));
            if replaced {
                let _ = watcher.unwatch(dir);
            }
            !replaced
        });
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
        let changed: BTreeSet<PathBuf> = changed.iter().map(|p| real_path(p)).collect();
        let (files, rewatched) = {
            let mut state = lock(&state);
            // Folders may have been removed or created: move the watches
            // now rather than at the next `set_files`.
            let rewatched = state.rewatch(&changed);
            (state.files.clone(), rewatched)
        };
        drop(state);
        let (given, reals): (Vec<PathBuf>, Vec<PathBuf>) = files.into_iter().unzip();
        let mut batch: Vec<PathBuf> = if everything {
            given
        } else {
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

    fn state() -> WatchState {
        WatchState {
            watcher: notify::recommended_watcher(|_| {}).unwrap(),
            files: Vec::new(),
            dirs: Vec::new(),
        }
    }

    #[test]
    fn rewatch_moves_down_to_a_recreated_parent_and_reports_its_files() {
        let root = tempfile::TempDir::new().unwrap();
        let dist = root.path().join("dist");
        let bundle = dist.join("bundle.js");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(&bundle, "1").unwrap();
        let mut state = state();
        state.set_files(vec![bundle.clone()]);
        assert_eq!(state.dirs, std::slice::from_ref(&dist));

        std::fs::remove_dir_all(&dist).unwrap();
        assert_eq!(state.rewatch(&BTreeSet::new()), Vec::<PathBuf>::new());
        assert_eq!(state.dirs, [root.path().to_path_buf()]);

        std::fs::create_dir_all(&dist).unwrap();
        assert_eq!(state.rewatch(&BTreeSet::new()), [bundle]);
        assert_eq!(state.dirs, [dist]);
    }

    /// A folder removed and created again within one batch is still in
    /// `dirs`, but inotify's watch died with the old folder: its own path
    /// among the events forces a new watch.
    #[test]
    fn rewatch_renews_a_watched_folder_reported_in_the_batch() {
        let root = tempfile::TempDir::new().unwrap();
        let dist = root.path().join("dist");
        let bundle = dist.join("bundle.js");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(&bundle, "1").unwrap();
        let mut state = state();
        state.set_files(vec![bundle.clone()]);

        // A write inside the folder does not renew it.
        let write = BTreeSet::from([real_path(&bundle)]);
        assert_eq!(state.rewatch(&write), Vec::<PathBuf>::new());

        std::fs::remove_dir_all(&dist).unwrap();
        std::fs::create_dir_all(&dist).unwrap();
        let replaced = BTreeSet::from([real_path(&dist)]);
        assert_eq!(state.rewatch(&replaced), std::slice::from_ref(&bundle));
        assert_eq!(state.dirs, std::slice::from_ref(&dist));
        // The path as watched matches too.
        assert_eq!(state.rewatch(&BTreeSet::from([dist.clone()])), [bundle]);
        assert_eq!(state.dirs, [dist]);
    }
}
