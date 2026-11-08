use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;

use crate::snapshot::{Entry, EntryKind, WorktreeSnapshot};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(16);

/// Glob strings from the `files.exclude` keys whose value is `true`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanOptions {
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: Arc<Path>,
    pub kind: ChangeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeEvent {
    ScanProgress,
    ScanComplete,
    Changed(Vec<Change>),
    Rescanned,
}

/// Input to the background thread. Task 6 adds watcher input here.
enum Command {
    /// Carries the generation assigned by `set_options`.
    SetOptions(ScanOptions, u64),
    Shutdown,
}

enum ScanOutcome {
    Completed,
    /// Stopped early: the worktree was dropped or a newer request superseded it.
    Aborted,
    Panicked,
}

pub struct Worktree {
    root: PathBuf,
    snapshot: Arc<Mutex<WorktreeSnapshot>>,
    events: Receiver<WorktreeEvent>,
    commands: Sender<Command>,
    stop: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
    #[cfg(test)]
    fail_next_scan: Arc<AtomicBool>,
}

impl Worktree {
    pub fn start(root: PathBuf, opts: ScanOptions) -> Worktree {
        let snapshot = Arc::new(Mutex::new(WorktreeSnapshot::empty()));
        let (event_tx, events) = unbounded();
        let (commands, command_rx) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));
        #[cfg(test)]
        let fail_next_scan = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            root: root.clone(),
            snapshot: snapshot.clone(),
            events: event_tx,
            stop: stop.clone(),
            generation: generation.clone(),
            #[cfg(test)]
            fail_next_scan: fail_next_scan.clone(),
        };
        let thread = thread::Builder::new()
            .name("lucerna-worktree".into())
            .spawn(move || worker.run(opts, command_rx))
            .expect("spawn worktree thread");
        Worktree {
            root,
            snapshot,
            events,
            commands,
            stop,
            generation,
            thread: Some(thread),
            #[cfg(test)]
            fail_next_scan,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn snapshot(&self) -> WorktreeSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn events(&self) -> Receiver<WorktreeEvent> {
        self.events.clone()
    }

    /// Replaces the scan options and triggers a rescan. A rescan still in
    /// flight is aborted, and queued requests coalesce into the latest.
    pub fn set_options(&self, opts: ScanOptions) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.commands.send(Command::SetOptions(opts, generation));
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.commands.send(Command::Shutdown);
        // Detach: an in-flight scan notices `stop` and exits on its own.
        drop(self.thread.take());
    }
}

/// The one walker configuration shared by the scan and by content search.
///
/// Ignore-source policy: hidden files are shown; project-level sources apply
/// (`.gitignore` including parent directories, `.git/info/exclude`, `.ignore`,
/// even without a `.git` directory); the user's global gitignore does not, so
/// results do not depend on the machine. `files.exclude` globs are overrides.
pub(crate) fn walk_builder(root: &Path, opts: &ScanOptions) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .ignore(true)
        .parents(true)
        .require_git(false)
        .follow_links(false);
    let mut overrides = OverrideBuilder::new(root);
    for glob in &opts.exclude {
        if let Err(err) = overrides.add(&format!("!{glob}")) {
            tracing::warn!(%glob, %err, "invalid files.exclude glob");
        }
    }
    match overrides.build() {
        Ok(o) => {
            builder.overrides(o);
        }
        Err(err) => tracing::warn!(%err, "could not build exclude overrides"),
    }
    builder
}

struct Worker {
    root: PathBuf,
    snapshot: Arc<Mutex<WorktreeSnapshot>>,
    events: Sender<WorktreeEvent>,
    stop: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    #[cfg(test)]
    fail_next_scan: Arc<AtomicBool>,
}

impl Worker {
    fn run(&self, opts: ScanOptions, commands: Receiver<Command>) {
        // The initial scan is progressive; it is superseded by generation 1+.
        self.scan_and_publish(&opts, 0, true);
        // The loop ends on Shutdown or when the Worktree (sender) is dropped.
        while let Ok(command) = commands.recv() {
            let Command::SetOptions(mut opts, mut generation) = command else {
                break;
            };
            // Coalesce queued requests down to the latest.
            while let Ok(next) = commands.try_recv() {
                match next {
                    Command::SetOptions(o, g) => (opts, generation) = (o, g),
                    Command::Shutdown => return,
                }
            }
            if self.scan_and_publish(&opts, generation, false) {
                self.send(WorktreeEvent::Rescanned);
            }
        }
    }

    /// Scans and publishes. Returns true only if the scan completed normally.
    fn scan_and_publish(&self, opts: &ScanOptions, generation: u64, progressive: bool) -> bool {
        let mut entries = Vec::new();
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.scan(opts, generation, progressive, &mut entries)
        }));
        let outcome = result.unwrap_or_else(|payload| {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic payload".into());
            tracing::error!(root = %self.root.display(), %message, "worktree scan panicked");
            ScanOutcome::Panicked
        });
        match outcome {
            ScanOutcome::Aborted => false,
            ScanOutcome::Completed | ScanOutcome::Panicked => {
                // After a panic, publish what was collected so waiters unblock.
                self.publish(entries, true);
                self.send(WorktreeEvent::ScanComplete);
                matches!(outcome, ScanOutcome::Completed)
            }
        }
    }

    fn send(&self, event: WorktreeEvent) {
        let _ = self.events.send(event);
    }

    fn publish(&self, entries: Vec<Entry>, complete: bool) {
        let snap = WorktreeSnapshot::build(entries, complete);
        *self.snapshot.lock().unwrap_or_else(|e| e.into_inner()) = snap;
    }

    fn superseded(&self, generation: u64) -> bool {
        self.stop.load(Ordering::Relaxed) || self.generation.load(Ordering::SeqCst) != generation
    }

    fn scan(
        &self,
        opts: &ScanOptions,
        generation: u64,
        progressive: bool,
        entries: &mut Vec<Entry>,
    ) -> ScanOutcome {
        #[cfg(test)]
        if self.fail_next_scan.swap(false, Ordering::SeqCst) {
            panic!("injected scan failure");
        }
        let mut last_publish = Instant::now();
        for result in walk_builder(&self.root, opts).build() {
            if self.superseded(generation) {
                return ScanOutcome::Aborted;
            }
            let Ok(dirent) = result else { continue };
            let Ok(rel) = dirent.path().strip_prefix(&self.root) else {
                continue;
            };
            if rel.as_os_str().is_empty() {
                continue;
            }
            let Some(file_type) = dirent.file_type() else {
                continue;
            };
            let kind = if file_type.is_symlink() {
                EntryKind::Symlink
            } else if file_type.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            };
            entries.push(Entry {
                path: Arc::from(rel),
                kind,
            });
            // A rescan keeps the previous complete snapshot until it finishes.
            if progressive && last_publish.elapsed() >= PROGRESS_INTERVAL {
                self.publish(entries.clone(), false);
                self.send(WorktreeEvent::ScanProgress);
                last_publish = Instant::now();
            }
        }
        ScanOutcome::Completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn wait(wt: &Worktree, want: fn(&WorktreeEvent) -> bool) {
        let rx = wt.events();
        loop {
            let ev = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("event timeout");
            if want(&ev) {
                return;
            }
        }
    }

    #[test]
    fn panicking_scan_still_completes_and_recovers() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        let wt = Worktree::start(dir.path().to_path_buf(), ScanOptions::default());
        wait(&wt, |e| matches!(e, WorktreeEvent::ScanComplete));

        wt.fail_next_scan.store(true, Ordering::SeqCst);
        wt.set_options(ScanOptions {
            exclude: vec!["**/zzz".into()],
        });
        // The panicked scan unblocks waiters with ScanComplete, not Rescanned.
        wait(&wt, |e| matches!(e, WorktreeEvent::ScanComplete));
        assert!(wt.snapshot().scan_complete());

        wt.set_options(ScanOptions {
            exclude: vec!["**/a.txt".into()],
        });
        wait(&wt, |e| matches!(e, WorktreeEvent::Rescanned));
        assert!(!wt.snapshot().contains(Path::new("a.txt")));
    }
}
