use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
    SetOptions(ScanOptions),
    Shutdown,
}

pub struct Worktree {
    root: PathBuf,
    snapshot: Arc<Mutex<WorktreeSnapshot>>,
    events: Receiver<WorktreeEvent>,
    commands: Sender<Command>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Worktree {
    pub fn start(root: PathBuf, opts: ScanOptions) -> Worktree {
        let snapshot = Arc::new(Mutex::new(WorktreeSnapshot::empty()));
        let (event_tx, events) = unbounded();
        let (commands, command_rx) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            root: root.clone(),
            snapshot: snapshot.clone(),
            events: event_tx,
            stop: stop.clone(),
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
            thread: Some(thread),
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

    /// Replaces the scan options and triggers a rescan.
    pub fn set_options(&self, opts: ScanOptions) {
        let _ = self.commands.send(Command::SetOptions(opts));
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

struct Worker {
    root: PathBuf,
    snapshot: Arc<Mutex<WorktreeSnapshot>>,
    events: Sender<WorktreeEvent>,
    stop: Arc<AtomicBool>,
}

impl Worker {
    fn run(&self, opts: ScanOptions, commands: Receiver<Command>) {
        self.guarded_scan(&opts);
        // The loop ends on Shutdown or when the Worktree (sender) is dropped.
        while let Ok(command) = commands.recv() {
            match command {
                Command::SetOptions(new) => {
                    self.guarded_scan(&new);
                    self.send(WorktreeEvent::Rescanned);
                }
                Command::Shutdown => break,
            }
        }
    }

    /// A panicking scan must not kill the thread: `set_options` can retry.
    fn guarded_scan(&self, opts: &ScanOptions) {
        if catch_unwind(AssertUnwindSafe(|| self.scan(opts))).is_err() {
            tracing::error!(root = %self.root.display(), "worktree scan panicked");
        }
    }

    fn send(&self, event: WorktreeEvent) {
        let _ = self.events.send(event);
    }

    fn publish(&self, entries: &[Entry], complete: bool) {
        let snap = WorktreeSnapshot::build(entries.to_vec(), complete);
        *self.snapshot.lock().unwrap_or_else(|e| e.into_inner()) = snap;
    }

    fn scan(&self, opts: &ScanOptions) {
        let mut builder = WalkBuilder::new(&self.root);
        builder
            .hidden(false)
            .git_ignore(true)
            .require_git(false)
            .follow_links(false);
        let mut overrides = OverrideBuilder::new(&self.root);
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

        let mut entries = Vec::new();
        let mut last_publish = Instant::now();
        for result in builder.build() {
            if self.stop.load(Ordering::Relaxed) {
                return;
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
            if last_publish.elapsed() >= PROGRESS_INTERVAL {
                self.publish(&entries, false);
                self.send(WorktreeEvent::ScanProgress);
                last_publish = Instant::now();
            }
        }
        self.publish(&entries, true);
        self.send(WorktreeEvent::ScanComplete);
    }
}
