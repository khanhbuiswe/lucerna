use std::cell::Cell;
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender, unbounded};
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;

use crate::coalesce::{Coalesced, RESCAN_THRESHOLD};
use crate::ignored::{IgnoreChecker, is_ignore_source};
use crate::snapshot::{Entry, EntryKind, WorktreeSnapshot};
use crate::watch::{Watcher, panic_message};

/// How many times the initial scan is repeated when the watcher reports lost
/// events while it ran.
const MAX_INITIAL_RESCANS: usize = 3;

/// After the initial `ScanComplete`, FSEvents may still replay changes from
/// before the stream started. Events are checked for that only this long.
const REPLAY_GRACE: Duration = Duration::from_secs(2);

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

/// Input to the background thread.
enum Command {
    /// Carries the generation assigned by `set_options`.
    SetOptions(ScanOptions, u64),
    /// A coalesced batch from the file watcher.
    FsEvents(Coalesced),
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
    #[cfg(test)]
    fail_next_apply: Arc<AtomicBool>,
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
        #[cfg(test)]
        let fail_next_apply = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            root: root.clone(),
            snapshot: snapshot.clone(),
            events: event_tx,
            command_tx: commands.clone(),
            stop: stop.clone(),
            generation: generation.clone(),
            watch_started: Cell::new(UNIX_EPOCH),
            initial_scan: Cell::new(true),
            replay_until: Cell::new(None),
            #[cfg(test)]
            fail_next_scan: fail_next_scan.clone(),
            #[cfg(test)]
            fail_next_apply: fail_next_apply.clone(),
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
            #[cfg(test)]
            fail_next_apply,
        }
    }

    /// A worktree that never scans or watches `root`: its snapshot is
    /// complete from the start and lists only `files` (root-relative). For
    /// a file opened alone, whose folder (say `$HOME`) is not a project.
    /// [`Worktree::events`] never yields; [`Worktree::set_options`] does
    /// nothing.
    pub fn without_scan(root: PathBuf, files: Vec<Arc<Path>>) -> Worktree {
        let entries = files
            .into_iter()
            .map(|path| Entry {
                path,
                kind: EntryKind::File,
            })
            .collect();
        let (_, events) = unbounded();
        let (commands, _) = unbounded();
        Worktree {
            root,
            snapshot: Arc::new(Mutex::new(WorktreeSnapshot::build(entries, true))),
            events,
            commands,
            stop: Arc::new(AtomicBool::new(true)),
            generation: Arc::new(AtomicU64::new(0)),
            thread: None,
            #[cfg(test)]
            fail_next_scan: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_apply: Arc::new(AtomicBool::new(false)),
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

/// Never scanned, watched or searched, whatever `files.exclude` says: a
/// user `"**/.git": false` does not bring the repository's internals in.
pub(crate) const GIT_DIR: &str = ".git";

/// A pattern with no glob metacharacter (`*?[]{}`) names a path, and as in
/// VS Code also covers everything under it: `src` becomes `src` and
/// `src/**`. Any other pattern is returned unchanged. Used for include and
/// exclude patterns alike.
pub(crate) fn expand_plain_folder(pattern: &str) -> Vec<String> {
    let trimmed = pattern.trim_end_matches('/');
    let plain = !trimmed.is_empty()
        && !trimmed.starts_with('!')
        && !trimmed.contains(['*', '?', '[', ']', '{', '}']);
    if plain {
        vec![pattern.to_string(), format!("{trimmed}/**")]
    } else {
        vec![pattern.to_string()]
    }
}

/// The one walker configuration shared by the scan and by content search.
///
/// Ignore-source policy: hidden files are shown; project-level sources apply
/// (`.gitignore` including parent directories, `.git/info/exclude`, `.ignore`,
/// even without a `.git` directory); the user's global gitignore does not, so
/// results do not depend on the machine. `files.exclude` globs are overrides.
/// Entries named [`GIT_DIR`] are always skipped.
pub(crate) fn walk_builder(root: &Path, opts: &ScanOptions) -> WalkBuilder {
    walk_builder_with_includes(root, opts, &[])
}

/// `walk_builder` plus positive `include` globs in the same override set, so
/// that nothing replaces the exclude overrides. Excludes come last and win.
/// With any include, files matching no glob are skipped; directories are not
/// affected, so included files stay reachable.
pub(crate) fn walk_builder_with_includes(
    root: &Path,
    opts: &ScanOptions,
    include: &[String],
) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .ignore(true)
        .parents(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != GIT_DIR);
    let mut overrides = OverrideBuilder::new(root);
    for glob in include.iter().flat_map(|g| expand_plain_folder(g)) {
        if let Err(err) = overrides.add(&glob) {
            tracing::warn!(%glob, %err, "invalid include glob");
        }
    }
    for glob in opts.exclude.iter().flat_map(|g| expand_plain_folder(g)) {
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

fn kind_of(meta: &fs::Metadata) -> EntryKind {
    kind_of_type(meta.file_type())
}

fn kind_of_type(file_type: fs::FileType) -> EntryKind {
    if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Dir
    } else {
        EntryKind::File
    }
}

struct Worker {
    root: PathBuf,
    snapshot: Arc<Mutex<WorktreeSnapshot>>,
    events: Sender<WorktreeEvent>,
    /// Lets the watcher feed batches into this worker's own queue.
    command_tx: Sender<Command>,
    stop: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    /// When the OS watcher was started. FSEvents can replay changes made
    /// just before that; the scan already saw them (see `is_stale`).
    watch_started: Cell<SystemTime>,
    /// True until the initial scan completes.
    initial_scan: Cell<bool>,
    /// End of the post-initial-scan grace period (monotonic).
    replay_until: Cell<Option<Instant>>,
    #[cfg(test)]
    fail_next_scan: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_apply: Arc<AtomicBool>,
}

impl Worker {
    fn run(&self, mut opts: ScanOptions, commands: Receiver<Command>) {
        // The watcher starts before the initial walk so no change falls into a
        // gap between the walk and the watcher. Batches that arrive meanwhile
        // queue up and are applied before `ScanComplete`.
        let tx = self.command_tx.clone();
        self.watch_started.set(SystemTime::now());
        let _watcher = match Watcher::start(&self.root, move |batch| {
            let _ = tx.send(Command::FsEvents(batch));
        }) {
            Ok(watcher) => Some(watcher),
            Err(err) => {
                tracing::error!(root = %self.root.display(), %err, "could not start file watcher");
                None
            }
        };
        // Commands set aside while draining, handled before new ones.
        let mut backlog: VecDeque<Command> = VecDeque::new();
        // The initial scan is progressive; it is superseded by generation 1+.
        self.scan_and_publish(&opts, 0, true, Some((&commands, &mut backlog)));
        // The worker holds its own command sender (for the watcher), so the
        // channel never disconnects: the loop ends only on Shutdown.
        while let Some(command) = backlog.pop_front().or_else(|| commands.recv().ok()) {
            match command {
                Command::Shutdown => break,
                Command::FsEvents(batch) => {
                    if self.apply_fs_events_guarded(batch, &opts) {
                        let generation = self.generation.load(Ordering::SeqCst);
                        if self.scan_and_publish(&opts, generation, false, None) {
                            self.send(WorktreeEvent::Rescanned);
                        }
                    }
                }
                Command::SetOptions(o, g) => {
                    let (mut next_opts, mut generation) = (o, g);
                    // Coalesce queued requests down to the latest.
                    while let Ok(next) = commands.try_recv() {
                        match next {
                            Command::SetOptions(o, g) => (next_opts, generation) = (o, g),
                            Command::Shutdown => return,
                            fs @ Command::FsEvents(_) => backlog.push_back(fs),
                        }
                    }
                    opts = next_opts;
                    if self.scan_and_publish(&opts, generation, false, None) {
                        self.send(WorktreeEvent::Rescanned);
                    }
                }
            }
        }
    }

    /// Scans and publishes. Returns true only if the scan completed normally.
    ///
    /// With `drain` (the initial scan), watcher batches that queued up during
    /// the walk are applied after the snapshot is published and before
    /// `ScanComplete`; other commands are set aside in the backlog.
    fn scan_and_publish(
        &self,
        opts: &ScanOptions,
        generation: u64,
        mut progressive: bool,
        mut drain: Option<(&Receiver<Command>, &mut VecDeque<Command>)>,
    ) -> bool {
        let mut rescans = 0;
        loop {
            let mut entries = Vec::new();
            let result = catch_unwind(AssertUnwindSafe(|| {
                self.scan(opts, generation, progressive, &mut entries)
            }));
            let outcome = result.unwrap_or_else(|payload| {
                let message = panic_message(payload.as_ref());
                tracing::error!(root = %self.root.display(), %message, "worktree scan panicked");
                ScanOutcome::Panicked
            });
            if matches!(outcome, ScanOutcome::Aborted) {
                return false;
            }
            // After a panic, publish what was collected so waiters unblock.
            self.publish(entries, true);
            if let Some((commands, backlog)) = drain.as_mut()
                && rescans < MAX_INITIAL_RESCANS
                && self.apply_queued_fs_events(opts, commands, backlog)
            {
                // Events were lost during the walk: walk again.
                rescans += 1;
                progressive = false;
                continue;
            }
            if self.initial_scan.replace(false) {
                self.replay_until.set(Some(Instant::now() + REPLAY_GRACE));
            }
            self.send(WorktreeEvent::ScanComplete);
            return matches!(outcome, ScanOutcome::Completed);
        }
    }

    /// Applies watcher batches waiting in the queue. Returns true if one of
    /// them calls for a full rescan.
    fn apply_queued_fs_events(
        &self,
        opts: &ScanOptions,
        commands: &Receiver<Command>,
        backlog: &mut VecDeque<Command>,
    ) -> bool {
        let mut rescan = false;
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::FsEvents(batch) => rescan |= self.apply_fs_events_guarded(batch, opts),
                other => backlog.push_back(other),
            }
        }
        rescan
    }

    /// `apply_fs_events`, but a panic is logged and answered with a rescan.
    fn apply_fs_events_guarded(&self, batch: Coalesced, opts: &ScanOptions) -> bool {
        catch_unwind(AssertUnwindSafe(|| self.apply_fs_events(batch, opts))).unwrap_or_else(
            |payload| {
                let message = panic_message(payload.as_ref());
                tracing::error!(root = %self.root.display(), %message, "applying file events panicked; rescanning");
                true
            },
        )
    }

    /// Reconciles a watcher batch with the snapshot and the disk, publishes
    /// the new snapshot, then sends `Changed`. Returns true if a full rescan
    /// is needed instead (overflow, too many changes, ignore rules changed).
    ///
    /// The watcher's kinds are hints: what counts is whether the path exists
    /// and whether the snapshot already has it. Existing and known: Modified
    /// (directories: nothing). Existing and new: Added, with missing parent
    /// directories and, for a directory, its not-ignored contents. Missing and
    /// known: Removed, with everything below it. Anything else is dropped.
    fn apply_fs_events(&self, batch: Coalesced, opts: &ScanOptions) -> bool {
        let Coalesced::Changes(changes) = batch else {
            return true;
        };
        #[cfg(test)]
        if self.fail_next_apply.swap(false, Ordering::SeqCst) {
            panic!("injected apply failure");
        }
        let snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut checker = IgnoreChecker::new(&self.root, opts);
        // Ignore files inside ignored directories (`.venv/.gitignore`) cannot
        // affect the worktree, so only the others force a rescan.
        let ignore_rules_changed = changes.iter().any(|c| {
            is_ignore_source(&c.path)
                && !self.ignore_source_is_inert(&c.path, &mut checker)
                && fs::symlink_metadata(self.root.join(&c.path))
                    .map_or(true, |meta| !self.is_stale(&meta))
        });
        if ignore_rules_changed {
            return true;
        }
        let mut out: Vec<Change> = Vec::new();
        let mut added: Vec<Entry> = Vec::new();
        let mut added_paths: HashSet<Arc<Path>> = HashSet::new();
        let mut removed: HashSet<Arc<Path>> = HashSet::new();
        let mut removed_below = 0;
        let known = |path: &Path, added_paths: &HashSet<Arc<Path>>| {
            added_paths.contains(path) || snapshot.contains(path)
        };
        for change in changes {
            let rel = change.path;
            let Ok(meta) = fs::symlink_metadata(self.root.join(&rel)) else {
                // Under a removed directory already: reported with it.
                if rel.ancestors().skip(1).any(|a| removed.contains(a)) {
                    continue;
                }
                if snapshot.contains(&rel) && removed.insert(rel.clone()) {
                    // Children reported earlier in the batch fold into the
                    // directory's removal.
                    out.retain(|c| !c.path.starts_with(&rel));
                    added.retain(|e| !e.path.starts_with(&rel));
                    added_paths.retain(|p| !p.starts_with(&rel));
                    out.push(Change {
                        path: rel.clone(),
                        kind: ChangeKind::Removed,
                    });
                    let below = snapshot.descendants(&rel);
                    removed_below += below.len();
                    if removed_below > RESCAN_THRESHOLD {
                        return true;
                    }
                    out.extend(below.iter().map(|e| Change {
                        path: e.path.clone(),
                        kind: ChangeKind::Removed,
                    }));
                }
                continue;
            };
            let kind = kind_of(&meta);
            if checker.is_ignored(&rel, kind == EntryKind::Dir) {
                continue;
            }
            if known(&rel, &added_paths) {
                if kind != EntryKind::Dir && !self.is_stale(&meta) {
                    out.push(Change {
                        path: rel,
                        kind: ChangeKind::Modified,
                    });
                }
                continue;
            }
            for ancestor in rel.ancestors().skip(1) {
                if ancestor.as_os_str().is_empty() || known(ancestor, &added_paths) {
                    continue;
                }
                let path: Arc<Path> = Arc::from(ancestor);
                added_paths.insert(path.clone());
                added.push(Entry {
                    path: path.clone(),
                    kind: EntryKind::Dir,
                });
                out.push(Change {
                    path,
                    kind: ChangeKind::Added,
                });
            }
            added_paths.insert(rel.clone());
            added.push(Entry {
                path: rel.clone(),
                kind,
            });
            out.push(Change {
                path: rel.clone(),
                kind: ChangeKind::Added,
            });
            if kind == EntryKind::Dir {
                let mut found = Vec::new();
                self.collect_subtree(&rel, &mut checker, &mut found);
                for entry in found {
                    if known(&entry.path, &added_paths) {
                        continue;
                    }
                    added_paths.insert(entry.path.clone());
                    out.push(Change {
                        path: entry.path.clone(),
                        kind: ChangeKind::Added,
                    });
                    added.push(entry);
                }
            }
            if added.len() > RESCAN_THRESHOLD {
                return true;
            }
        }
        if !added.is_empty() || !removed.is_empty() {
            let mut entries: Vec<Entry> = snapshot
                .entries()
                .iter()
                .filter(|e| !e.path.ancestors().any(|a| removed.contains(a)))
                .cloned()
                .collect();
            entries.extend(added);
            self.publish(entries, true);
        }
        if !out.is_empty() {
            self.send(WorktreeEvent::Changed(out));
        }
        false
    }

    /// True if the inode last changed before the watcher started AND we are
    /// still in the replay window (the initial scan plus `REPLAY_GRACE`), so
    /// the report is a replay of something the scan already covered. Outside
    /// the window nothing is dropped by timestamp, so clock steps or coarse
    /// filesystem timestamps cannot hide a real edit. ctime, unlike mtime,
    /// cannot be set backwards by `cp -p` or `touch -d`.
    fn is_stale(&self, meta: &fs::Metadata) -> bool {
        if !self.in_replay_window() {
            return false;
        }
        let changed = UNIX_EPOCH
            + Duration::new(
                meta.ctime().max(0) as u64,
                meta.ctime_nsec().clamp(0, 999_999_999) as u32,
            );
        changed < self.watch_started.get()
    }

    fn in_replay_window(&self) -> bool {
        self.initial_scan.get() || self.replay_until.get().is_some_and(|t| Instant::now() < t)
    }

    /// True if a change to this ignore-source file cannot alter the worktree
    /// because the directory whose rules it defines is itself ignored. For
    /// `.git/info/exclude` that directory is the one holding `.git`.
    fn ignore_source_is_inert(&self, rel: &Path, checker: &mut IgnoreChecker) -> bool {
        let dir = if rel.ends_with(".git/info/exclude") {
            rel.parent().and_then(Path::parent).and_then(Path::parent)
        } else {
            rel.parent()
        }
        .unwrap_or(Path::new(""));
        !dir.as_os_str().is_empty() && checker.is_ignored(dir, true)
    }

    /// Entries below the root-relative directory `dir`, skipping ignored ones.
    fn collect_subtree(&self, dir: &Path, checker: &mut IgnoreChecker, out: &mut Vec<Entry>) {
        let Ok(read) = fs::read_dir(self.root.join(dir)) else {
            return;
        };
        for dirent in read.flatten() {
            if out.len() > RESCAN_THRESHOLD {
                return;
            }
            let Ok(file_type) = dirent.file_type() else {
                continue;
            };
            let rel = dir.join(dirent.file_name());
            let kind = kind_of_type(file_type);
            if checker.is_ignored(&rel, kind == EntryKind::Dir) {
                continue;
            }
            if kind == EntryKind::Dir {
                self.collect_subtree(&rel, checker, out);
            }
            out.push(Entry {
                path: Arc::from(rel),
                kind,
            });
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
            let kind = kind_of_type(file_type);
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

    #[test]
    fn plain_patterns_also_cover_what_is_under_them() {
        assert_eq!(expand_plain_folder("src"), ["src", "src/**"]);
        assert_eq!(expand_plain_folder("a/b"), ["a/b", "a/b/**"]);
        assert_eq!(expand_plain_folder("src/"), ["src/", "src/**"]);
        for glob in ["src/**", "*.rs", "a?", "[ab]", "{a,b}", "", "!x"] {
            assert_eq!(expand_plain_folder(glob), [glob], "{glob}");
        }
    }

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

    #[test]
    fn panicking_apply_falls_back_to_rescan() {
        let dir = tempfile::TempDir::new().unwrap();
        let wt = Worktree::start(dir.path().to_path_buf(), ScanOptions::default());
        wait(&wt, |e| matches!(e, WorktreeEvent::ScanComplete));

        wt.fail_next_apply.store(true, Ordering::SeqCst);
        fs::write(dir.path().join("b.txt"), "x").unwrap();
        wait(&wt, |e| matches!(e, WorktreeEvent::Rescanned));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !wt.snapshot().contains(Path::new("b.txt")) {
            assert!(Instant::now() < deadline, "b.txt never appeared");
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn bare_worker(dir: &Path) -> Worker {
        let (events, _) = unbounded();
        let (command_tx, _) = unbounded();
        Worker {
            root: dir.to_path_buf(),
            snapshot: Arc::new(Mutex::new(WorktreeSnapshot::empty())),
            events,
            command_tx,
            stop: Arc::new(AtomicBool::new(false)),
            generation: Arc::new(AtomicU64::new(0)),
            watch_started: Cell::new(SystemTime::now() + Duration::from_secs(3600)),
            initial_scan: Cell::new(false),
            replay_until: Cell::new(None),
            fail_next_scan: Arc::new(AtomicBool::new(false)),
            fail_next_apply: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn stale_guard_only_applies_in_the_replay_window() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("f"), "x").unwrap();
        let meta = fs::symlink_metadata(dir.path().join("f")).unwrap();
        // The file's ctime is "before" watch_started (an hour ahead).
        let worker = bare_worker(dir.path());
        assert!(!worker.is_stale(&meta), "window closed: never stale");
        worker
            .replay_until
            .set(Some(Instant::now() + Duration::from_secs(60)));
        assert!(worker.is_stale(&meta), "inside the grace period");
        worker
            .replay_until
            .set(Some(Instant::now() - Duration::from_millis(1)));
        assert!(!worker.is_stale(&meta), "grace period over");
        worker.initial_scan.set(true);
        assert!(worker.is_stale(&meta), "during the initial scan");
    }
}
