use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{OsStr, OsString};
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
            ignores_case: Cell::new(None),
            #[cfg(test)]
            folders_listed: Cell::new(0),
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

/// Whether an entry named `name` is never scanned, watched or searched:
/// [`GIT_DIR`], and a copy still being written under its hidden name
/// ([`crate::fs_ops::COPY_TEMP_PREFIX`]), which shows up once renamed to
/// its real name.
pub(crate) fn always_skipped(name: &std::ffi::OsStr) -> bool {
    name == GIT_DIR
        || name
            .as_encoded_bytes()
            .starts_with(crate::fs_ops::COPY_TEMP_PREFIX.as_bytes())
}

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
/// Entries [`always_skipped`] names are skipped.
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
        .filter_entry(|entry| !always_skipped(entry.file_name()));
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
    /// Whether the root's file system ignores case, once a probe could tell.
    ignores_case: Cell<Option<bool>>,
    /// How many folders the last batch listed to catch respelled paths.
    #[cfg(test)]
    folders_listed: Cell<usize>,
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
    /// A path that exists only through a case-insensitive file system (one
    /// of its names is listed on disk with other case) counts as missing.
    ///
    /// Telling those apart costs one `read_dir` of each folder on the path,
    /// each read once per batch. It runs only for existing, not-ignored
    /// paths, and only when the root's file system ignores case (see
    /// [`Worker::root_ignores_case`]); elsewhere it is skipped.
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
        let mut listings = Listings::default();
        for change in changes {
            let mut rel = change.path;
            let mut meta = fs::symlink_metadata(self.root.join(&rel)).ok();
            if let Some(meta) = &meta
                && checker.is_ignored(&rel, kind_of(meta) == EntryKind::Dir)
            {
                continue;
            }
            // On a case-insensitive file system `foo.rs` still resolves
            // after a rename to `Foo.rs`, but that spelling is gone, and so
            // is a folder spelled the old way, with everything below it.
            // (Never an ignored path: the old spelling was in the snapshot.)
            if meta.is_some()
                && self.root_ignores_case()
                && let Some(gone) = listings.respelled_prefix(&self.root, &rel)
            {
                rel = Arc::from(gone);
                meta = None;
            }
            let Some(meta) = meta else {
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
        #[cfg(test)]
        self.folders_listed.set(listings.dirs.len());
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

    /// Whether the root's file system ignores case (APFS and HFS+ by
    /// default, NTFS), probed once and then remembered. While the root holds
    /// no name with a cased letter the probe cannot tell, and the answer is
    /// true, so a case-only rename is still caught.
    fn root_ignores_case(&self) -> bool {
        if let Some(known) = self.ignores_case.get() {
            return known;
        }
        let probed = probe_ignores_case(&self.root);
        self.ignores_case.set(probed);
        probed.unwrap_or(true)
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

/// Directory listings read while applying one watcher batch, to tell a path
/// spelled as on disk from one a case-insensitive file system only resolves
/// to an entry spelled otherwise (`foo.rs` after a rename to `Foo.rs`).
#[derive(Default)]
struct Listings {
    /// Root-relative directory to its entries' names; `None` if unreadable.
    dirs: HashMap<PathBuf, Option<HashSet<OsString>>>,
}

impl Listings {
    /// The shortest prefix of the existing root-relative `rel` whose last
    /// name is listed in its folder only with other case, if any.
    ///
    /// Only a difference in case counts: a name missing for another reason
    /// (an unreadable folder, a Unicode normalization difference) keeps the
    /// path as it is.
    fn respelled_prefix(&mut self, root: &Path, rel: &Path) -> Option<PathBuf> {
        let mut prefix = PathBuf::new();
        for component in rel.components() {
            let name = component.as_os_str();
            let names = self.dirs.entry(prefix.clone()).or_insert_with(|| {
                fs::read_dir(root.join(&prefix))
                    .ok()
                    .map(|read| read.flatten().map(|d| d.file_name()).collect())
            });
            prefix.push(name);
            if let Some(names) = names
                && !names.contains(name)
                && names.iter().any(|n| same_but_case(n, name))
            {
                return Some(prefix);
            }
        }
        None
    }
}

/// Whether the file system holding `dir` ignores case: a name in it with
/// its case flipped resolves to the very same entry. `None` if `dir` holds
/// no name with a cased letter (or cannot be read).
fn probe_ignores_case(dir: &Path) -> Option<bool> {
    for dirent in fs::read_dir(dir).ok()?.flatten() {
        let name = dirent.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let flipped: String = name
            .chars()
            .flat_map(|c| {
                if c.is_lowercase() {
                    c.to_uppercase().collect::<Vec<_>>()
                } else {
                    c.to_lowercase().collect()
                }
            })
            .collect();
        if flipped == name {
            continue;
        }
        let same = match (
            fs::symlink_metadata(dir.join(name)),
            fs::symlink_metadata(dir.join(&flipped)),
        ) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        };
        return Some(same);
    }
    None
}

fn same_but_case(a: &OsStr, b: &OsStr) -> bool {
    match (a.to_str(), b.to_str()) {
        (Some(a), Some(b)) => a.to_lowercase() == b.to_lowercase(),
        _ => a.eq_ignore_ascii_case(b),
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
            ignores_case: Cell::new(None),
            folders_listed: Cell::new(0),
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

    fn ignores_case(dir: &Path) -> bool {
        fs::write(dir.join("case-probe-a"), "").unwrap();
        let ignores = fs::symlink_metadata(dir.join("CASE-PROBE-A")).is_ok();
        fs::remove_file(dir.join("case-probe-a")).unwrap();
        ignores
    }

    fn entry(path: &str, kind: EntryKind) -> Entry {
        Entry {
            path: Arc::from(Path::new(path)),
            kind,
        }
    }

    fn change(path: &str, kind: ChangeKind) -> Change {
        Change {
            path: Arc::from(Path::new(path)),
            kind,
        }
    }

    fn sorted_paths(worker: &Worker) -> Vec<String> {
        let snapshot = worker.snapshot.lock().unwrap().clone();
        let mut paths: Vec<String> = snapshot
            .entries()
            .iter()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect();
        paths.sort();
        paths
    }

    /// T2-2: after `foo.rs` became `Foo.rs` on a case-insensitive file
    /// system, `foo.rs` still stats fine, but it is gone: whichever order
    /// the watcher reports the two names in.
    #[test]
    fn case_only_rename_drops_the_old_name() {
        let dir = tempfile::TempDir::new().unwrap();
        if !ignores_case(dir.path()) {
            return;
        }
        for order in [["foo.rs", "Foo.rs"], ["Foo.rs", "foo.rs"]] {
            fs::write(dir.path().join("Foo.rs"), "x").unwrap();
            let worker = bare_worker(dir.path());
            worker.publish(vec![entry("foo.rs", EntryKind::File)], true);
            let batch = order
                .iter()
                .map(|p| change(p, ChangeKind::Modified))
                .collect();
            let rescan = worker.apply_fs_events(Coalesced::Changes(batch), &ScanOptions::default());
            assert!(!rescan);
            assert_eq!(sorted_paths(&worker), ["Foo.rs"], "order {order:?}");
            fs::remove_file(dir.path().join("Foo.rs")).unwrap();
        }
    }

    #[test]
    fn probe_tells_whether_the_file_system_ignores_case() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(probe_ignores_case(dir.path()), None, "nothing to probe");
        fs::write(dir.path().join("1-2"), "").unwrap();
        assert_eq!(probe_ignores_case(dir.path()), None, "no cased letter");
        fs::write(dir.path().join("a"), "").unwrap();
        fs::remove_file(dir.path().join("1-2")).unwrap();
        let expected = fs::symlink_metadata(dir.path().join("A")).is_ok();
        assert_eq!(probe_ignores_case(dir.path()), Some(expected));
    }

    /// The listing walk is skipped for ignored paths and on a file system
    /// that keeps case apart; it runs otherwise.
    #[test]
    fn folders_are_listed_only_for_not_ignored_paths_where_case_is_ignored() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        for f in ["target/debug/a", "target/debug/b", "src/main.rs"] {
            fs::write(dir.path().join(f), "x").unwrap();
        }
        let opts = ScanOptions {
            exclude: vec!["**/target".into()],
        };
        let batch = |paths: &[&str]| {
            Coalesced::Changes(
                paths
                    .iter()
                    .map(|p| change(p, ChangeKind::Modified))
                    .collect(),
            )
        };
        let worker = bare_worker(dir.path());
        worker.publish(
            vec![
                entry("src", EntryKind::Dir),
                entry("src/main.rs", EntryKind::File),
            ],
            true,
        );
        worker.ignores_case.set(Some(true));
        worker.apply_fs_events(batch(&["target/debug/a", "target/debug/b"]), &opts);
        assert_eq!(worker.folders_listed.get(), 0, "ignored paths");
        worker.apply_fs_events(batch(&["src/main.rs"]), &opts);
        assert_eq!(worker.folders_listed.get(), 2, "the root and src");
        worker.ignores_case.set(Some(false));
        worker.apply_fs_events(batch(&["src/main.rs"]), &opts);
        assert_eq!(worker.folders_listed.get(), 0, "case-sensitive");
    }

    #[test]
    fn case_only_rename_of_a_folder_leaves_no_duplicate_subtree() {
        let dir = tempfile::TempDir::new().unwrap();
        if !ignores_case(dir.path()) {
            return;
        }
        fs::create_dir_all(dir.path().join("Src/inner")).unwrap();
        fs::write(dir.path().join("Src/a.rs"), "x").unwrap();
        fs::write(dir.path().join("Src/inner/b.rs"), "x").unwrap();
        let old = vec![
            entry("src", EntryKind::Dir),
            entry("src/a.rs", EntryKind::File),
            entry("src/inner", EntryKind::Dir),
            entry("src/inner/b.rs", EntryKind::File),
        ];
        let want = ["Src", "Src/a.rs", "Src/inner", "Src/inner/b.rs"];
        // Some batches also name a file below the old folder.
        for order in [
            vec!["src", "Src"],
            vec!["Src", "src"],
            vec!["src/a.rs", "Src", "src"],
            vec!["Src", "src/inner/b.rs"],
        ] {
            let worker = bare_worker(dir.path());
            worker.publish(old.clone(), true);
            let batch = order
                .iter()
                .map(|p| change(p, ChangeKind::Modified))
                .collect();
            let rescan = worker.apply_fs_events(Coalesced::Changes(batch), &ScanOptions::default());
            assert!(!rescan);
            assert_eq!(sorted_paths(&worker), want, "order {order:?}");
        }
    }
}
