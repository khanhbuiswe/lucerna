//! Git integration glue. [`GitStore`] owns one workspace's repository: it
//! finds `git`, runs a serial queue of status jobs in the background and
//! publishes immutable [`GitSnapshot`]s, plus the gutter hunks of tracked
//! buffers. The UI thread never runs `git`, reads a file or diffs: it only
//! swaps in finished results.

pub mod branch_picker;
pub mod checkout;
mod hunks;
mod job;
mod locate;
pub mod rollup;
mod wiring;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures::StreamExt as _;
use gpui::{App, Context, Entity, EventEmitter, Subscription, Task};
use lucerna_git::{FileStatus, GitBinary, GitDirWatcher, GitError, Hunk, Repo};
use lucerna_settings::SettingsView;
use lucerna_text::BufferSnapshot;

use crate::{AppState, WindowSettings};
use hunks::{Diffed, HunkWork};
use job::{Job, JobError, JobInput, JobOutput, run_job};
use rollup::Rollup;
pub(crate) use wiring::{register_commands, workspace_relative};

/// Hunks are recomputed this long after the last `track_buffer` call.
const HUNK_DEBOUNCE: Duration = Duration::from_millis(50);
const TOO_MANY_MESSAGE: &str = "Too many changes in this repository; automatic Git updates are off. Run Git: Refresh to update.";
const NOT_FOUND_MESSAGE: &str = "Git not found. Install Git or set git.path to see changes.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitState {
    /// Not started, or `git.enabled` is false.
    Off,
    /// Looking for `git` and the repository.
    Locating,
    NoGit,
    NoRepo,
    /// The folder is not trusted: no `git` runs.
    Restricted,
    Ready,
    /// The status exceeded `git.statusLimit`; only `rescan` updates it.
    TooManyChanges,
}

/// One immutable view of the repository, swapped whole on the UI thread.
#[derive(Debug, Clone, Default)]
pub struct GitSnapshot {
    /// Bumped by every applied snapshot.
    pub generation: u64,
    /// Workspace-relative paths, against `git.diffBase`.
    pub statuses: Arc<HashMap<Arc<Path>, FileStatus>>,
    pub rollup: Arc<Rollup>,
    pub head: Option<String>,
    /// The resolved diff-base oid.
    pub base_rev: Option<String>,
    pub default_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitStoreEvent {
    SnapshotChanged,
    HunksChanged(Arc<Path>),
    StateChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffBase {
    Head,
    DefaultBranch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitSettings {
    enabled: bool,
    path: Option<PathBuf>,
    diff_base: DiffBase,
    status_limit: usize,
}

impl GitSettings {
    /// The window's `git.*` settings; the defaults when the app has no
    /// settings (unit tests).
    fn read(settings: &Entity<WindowSettings>, cx: &App) -> GitSettings {
        if !cx.has_global::<AppState>() {
            return GitSettings {
                enabled: true,
                path: None,
                diff_base: DiffBase::Head,
                status_limit: 10_000,
            };
        }
        GitSettings::from_view(settings.read(cx).view(cx))
    }

    fn from_view(s: SettingsView<'_>) -> GitSettings {
        GitSettings {
            enabled: s.get::<bool>("git.enabled", None).unwrap_or(true),
            path: git_path_setting(s),
            diff_base: match s.get::<String>("git.diffBase", None).as_deref() {
                Some("defaultBranch") => DiffBase::DefaultBranch,
                _ => DiffBase::Head,
            },
            status_limit: s
                .get::<u64>("git.statusLimit", None)
                .map_or(10_000, |n| n as usize),
        }
    }
}

/// `git.path`, from the User layer only: it names a binary to run, so a
/// folder's `.lucerna/settings.json` must not choose it (as with language
/// server binaries). A relative path would resolve against the process's
/// working directory and is ignored.
fn git_path_setting(s: SettingsView<'_>) -> Option<PathBuf> {
    static WARNED: AtomicBool = AtomicBool::new(false);
    let path = PathBuf::from(
        s.get_trusted::<String>("git.path", None)
            .filter(|p| !p.is_empty())?,
    );
    if path.is_absolute() {
        return Some(path);
    }
    if !WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!(path = %path.display(), "git.path must be absolute; ignored");
    }
    None
}

/// The diff base of one tracked file. `text` is `None` when the file gets
/// no hunks (untracked, binary or too large).
struct Base {
    rev: String,
    text: Option<Arc<str>>,
}

struct Tracked {
    snapshot: BufferSnapshot,
    /// Bumped by each `track_buffer`; results for older versions are dropped.
    version: u64,
    /// The debounce timer and diff for this buffer; replacing it cancels both.
    task: Option<Task<()>>,
}

enum Setup {
    NoGit,
    NoRepo,
    Failed(GitError),
    Found(Box<Located>),
}

/// A found repository, with its git-dir watcher when watching is on.
struct Located {
    repo: Repo,
    watcher: Option<GitDirWatcher>,
}

/// The git-dir watcher's signal, sent from its thread.
type WatchSender = futures::channel::mpsc::UnboundedSender<()>;
type WatchReceiver = futures::channel::mpsc::UnboundedReceiver<()>;

pub struct GitStore {
    root: PathBuf,
    /// The window's settings, followed for `git.*`.
    window_settings: Entity<WindowSettings>,
    settings: GitSettings,
    trusted: bool,
    state: GitState,
    /// Bumped whenever the repository is dropped or replaced, so results of
    /// older work are ignored.
    epoch: u64,
    repo: Option<Repo>,
    watcher: Option<GitDirWatcher>,
    watch_task: Option<Task<()>>,
    setup: Option<Task<()>>,
    running: Option<Task<()>>,
    pending: Option<Job>,
    snapshot: GitSnapshot,
    buffers: HashMap<Arc<Path>, Tracked>,
    bases: HashMap<Arc<Path>, Base>,
    hunks: HashMap<Arc<Path>, Arc<[Hunk]>>,
    jobs_run: usize,
    error_reported: bool,
    not_found_reported: bool,
    too_many_reported: bool,
    git_override: Option<GitBinary>,
    /// The window whose status bar shows what goes wrong here.
    window: Option<gpui::WindowId>,
    /// Repository discovery stops here (set by the workbench, from its
    /// scan options); `None` looks all the way up.
    home: Option<PathBuf>,
    _settings: Subscription,
}

impl EventEmitter<GitStoreEvent> for GitStore {}

impl GitStore {
    /// Tells the store which window it belongs to.
    pub fn set_window(&mut self, window: gpui::WindowId) {
        self.window = Some(window);
    }

    /// The home folder: as in Zed, a repository above it is not the
    /// project's. Takes effect on the next discovery.
    pub fn set_home(&mut self, home: Option<PathBuf>) {
        self.home = home;
    }

    /// Starts in [`GitState::Off`]; nothing runs until `set_trusted(true)`.
    /// Reads `git.*` from the window's `settings` and follows them.
    pub fn new(root: PathBuf, settings: Entity<WindowSettings>, cx: &mut Context<Self>) -> Self {
        GitStore {
            root,
            settings: GitSettings::read(&settings, cx),
            _settings: cx.observe(&settings, |store, _, cx| store.settings_changed(cx)),
            window_settings: settings,
            trusted: false,
            state: GitState::Off,
            epoch: 0,
            repo: None,
            watcher: None,
            watch_task: None,
            setup: None,
            running: None,
            pending: None,
            snapshot: GitSnapshot::default(),
            buffers: HashMap::new(),
            bases: HashMap::new(),
            hunks: HashMap::new(),
            jobs_run: 0,
            error_reported: false,
            not_found_reported: false,
            too_many_reported: false,
            window: None,
            home: None,
            git_override: None,
        }
    }

    /// Whether `root` looks like it is inside a repository, read from the
    /// filesystem without running `git` (safe before the folder is trusted).
    pub fn has_repo_hint(root: &Path) -> bool {
        crate::git_branch::head_path(root).is_some()
    }

    /// `true` locates `git` and the repository and starts the first rescan;
    /// `false` drops the repository and runs nothing more.
    pub fn set_trusted(&mut self, trusted: bool, cx: &mut Context<Self>) {
        if trusted == self.trusted && self.state != GitState::Off {
            return;
        }
        self.trusted = trusted;
        self.restart(cx);
    }

    pub fn state(&self) -> GitState {
        self.state
    }

    /// The `git.enabled` setting, as last read.
    pub fn is_enabled(&self) -> bool {
        self.settings.enabled
    }

    pub fn snapshot(&self) -> GitSnapshot {
        self.snapshot.clone()
    }

    /// Refreshes the status of `paths` (workspace-relative files or folders).
    /// Ignored unless the store is [`GitState::Ready`].
    pub fn paths_changed(&mut self, paths: Vec<Arc<Path>>, cx: &mut Context<Self>) {
        if self.state != GitState::Ready || paths.is_empty() {
            return;
        }
        if paths.iter().any(|p| p.as_os_str().is_empty()) {
            self.enqueue(Job::Rescan, cx);
        } else {
            self.enqueue(Job::refresh(paths), cx);
        }
    }

    /// Recomputes everything: statuses, `default_branch`, `base_rev` and the
    /// base text of every tracked buffer. Runs even after too many changes.
    /// Without `git` or a repository it looks for both again (a `git init`
    /// or an install since).
    pub fn rescan(&mut self, cx: &mut Context<Self>) {
        match self.state {
            GitState::Ready | GitState::TooManyChanges => self.enqueue(Job::Rescan, cx),
            GitState::NoRepo | GitState::NoGit => self.restart(cx),
            GitState::Off | GitState::Locating | GitState::Restricted => {}
        }
    }

    /// A handle on the repository, once found, for views that run their
    /// own `git` work off the UI thread (the diff tabs).
    pub fn repo(&self) -> Option<Repo> {
        self.repo.clone()
    }

    /// The git dir of the repository, once found.
    pub(crate) fn git_dir(&self) -> Option<&Path> {
        self.repo.as_ref().map(|r| r.git_dir.as_path())
    }

    /// (Re)computes the hunks of `path` against the diff base, 50 ms after
    /// the last call for it.
    pub fn track_buffer(
        &mut self,
        path: Arc<Path>,
        snapshot: BufferSnapshot,
        cx: &mut Context<Self>,
    ) {
        let tracked = self.buffers.entry(path.clone()).or_insert(Tracked {
            snapshot: snapshot.clone(),
            version: 0,
            task: None,
        });
        tracked.snapshot = snapshot;
        tracked.version += 1;
        self.schedule_hunks(path, HUNK_DEBOUNCE, cx);
    }

    /// Stops tracking `path` and frees its base text and hunks.
    pub fn untrack_buffer(&mut self, path: &Path) {
        self.buffers.remove(path);
        self.bases.remove(path);
        self.hunks.remove(path);
    }

    pub fn hunks(&self, path: &Path) -> Option<Arc<[Hunk]>> {
        self.hunks.get(path).cloned()
    }

    /// Whether `track_buffer` was called for `path` and it was not
    /// untracked since.
    #[doc(hidden)]
    pub fn is_tracking(&self, path: &Path) -> bool {
        self.buffers.contains_key(path)
    }

    /// Re-reads the `git.*` settings: `enabled` or `path` restarts,
    /// `diffBase` or `statusLimit` rescans.
    pub fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let new = GitSettings::read(&self.window_settings, cx);
        if new == self.settings {
            return;
        }
        let old = std::mem::replace(&mut self.settings, new);
        if old.enabled != self.settings.enabled || old.path != self.settings.path {
            if self.trusted {
                self.restart(cx);
            }
            return;
        }
        self.bases.clear();
        self.rescan(cx);
    }

    /// Uses `git` instead of locating it (tests isolate its config this way).
    #[doc(hidden)]
    pub fn use_git_for_test(&mut self, git: GitBinary) {
        self.git_override = Some(git);
    }

    /// Kills the blob reader's `git cat-file` process; the next read
    /// restarts it.
    #[doc(hidden)]
    pub fn kill_blob_process_for_test(&self) {
        if let Some(repo) = &self.repo {
            repo.kill_blob_process_for_test();
        }
    }

    /// How many status jobs have started.
    #[doc(hidden)]
    pub fn jobs_run(&self) -> usize {
        self.jobs_run
    }

    fn set_state(&mut self, state: GitState, cx: &mut Context<Self>) {
        if self.state != state {
            self.state = state;
            cx.emit(GitStoreEvent::StateChanged);
        }
    }

    /// Drops the repository, its tasks and every result.
    fn reset(&mut self, cx: &mut Context<Self>) {
        self.epoch += 1;
        self.repo = None;
        self.watcher = None;
        self.watch_task = None;
        self.setup = None;
        self.running = None;
        self.pending = None;
        self.bases.clear();
        for tracked in self.buffers.values_mut() {
            tracked.task = None;
        }
        for (path, _) in std::mem::take(&mut self.hunks) {
            cx.emit(GitStoreEvent::HunksChanged(path));
        }
        if self.snapshot.generation > 0 {
            self.snapshot = GitSnapshot {
                generation: self.snapshot.generation + 1,
                ..GitSnapshot::default()
            };
            cx.emit(GitStoreEvent::SnapshotChanged);
        }
    }

    fn restart(&mut self, cx: &mut Context<Self>) {
        self.reset(cx);
        let state = if !self.trusted {
            GitState::Restricted
        } else if !self.settings.enabled {
            GitState::Off
        } else {
            GitState::Locating
        };
        self.set_state(state, cx);
        if state != GitState::Locating {
            return;
        }
        let epoch = self.epoch;
        let root = self.root.clone();
        let home = self.home.clone();
        let setting = self.settings.path.clone();
        let git = self.git_override.clone();
        // The watcher is not started on GPUI's test scheduler, which rejects
        // wake-ups from other threads; tests call `rescan` themselves.
        let (watch_tx, watch_rx) = if cx.background_executor().is_test() {
            (None, None)
        } else {
            let (tx, rx) = futures::channel::mpsc::unbounded();
            (Some(tx), Some(rx))
        };
        self.setup = Some(cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { discover(git, setting, &root, home.as_deref(), watch_tx) })
                .await;
            this.update(cx, |s, cx| s.finish_setup(epoch, found, watch_rx, cx))
                .ok();
        }));
    }

    fn finish_setup(
        &mut self,
        epoch: u64,
        found: Setup,
        watch_rx: Option<WatchReceiver>,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.epoch {
            return;
        }
        self.setup = None;
        match found {
            Setup::NoGit => {
                self.set_state(GitState::NoGit, cx);
                if !std::mem::replace(&mut self.not_found_reported, true) {
                    crate::status_message::show_or_log(self.window, NOT_FOUND_MESSAGE, cx);
                }
            }
            Setup::NoRepo => self.set_state(GitState::NoRepo, cx),
            Setup::Failed(err) => {
                self.report_error(&err, cx);
                self.set_state(GitState::NoRepo, cx);
            }
            Setup::Found(found) => {
                let Located { repo, watcher } = *found;
                if let (Some(watcher), Some(rx)) = (watcher, watch_rx) {
                    self.listen_to_watcher(watcher, rx, cx);
                }
                self.repo = Some(repo);
                self.set_state(GitState::Ready, cx);
                self.enqueue(Job::Rescan, cx);
            }
        }
    }

    /// Rescans on the watcher's signals (`HEAD`, index or ref changes).
    fn listen_to_watcher(
        &mut self,
        watcher: GitDirWatcher,
        mut rx: WatchReceiver,
        cx: &mut Context<Self>,
    ) {
        self.watcher = Some(watcher);
        self.watch_task = Some(cx.spawn(async move |this, cx| {
            while rx.next().await.is_some() {
                if this.update(cx, |s, cx| s.git_dir_changed(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn git_dir_changed(&mut self, cx: &mut Context<Self>) {
        // After too many changes only an explicit `rescan` updates.
        if self.state == GitState::Ready {
            self.enqueue(Job::Rescan, cx);
        }
    }

    fn enqueue(&mut self, job: Job, cx: &mut Context<Self>) {
        self.pending = Some(match self.pending.take() {
            Some(queued) => queued.merge(job),
            None => job,
        });
        self.start_next(cx);
    }

    fn start_next(&mut self, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            self.pending = None;
            return;
        };
        let Some(job) = self.pending.take() else {
            return;
        };
        if self.state == GitState::TooManyChanges && matches!(job, Job::RefreshPaths(_)) {
            return;
        }
        let buffers = match job {
            Job::Rescan => self
                .buffers
                .iter()
                .map(|(p, t)| (p.clone(), t.version, t.snapshot.clone()))
                .collect(),
            Job::RefreshPaths(_) => Vec::new(),
        };
        let input = JobInput {
            repo,
            job,
            diff_base: self.settings.diff_base,
            status_limit: self.settings.status_limit,
            prev: self.snapshot.clone(),
            buffers,
        };
        self.jobs_run += 1;
        let epoch = self.epoch;
        self.running = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let repo = input.repo.clone();
                    run_job(input).map_err(|err| JobError::new(err, &repo))
                })
                .await;
            this.update(cx, |s, cx| s.finish_job(epoch, result, cx))
                .ok();
        }));
    }

    fn finish_job(
        &mut self,
        epoch: u64,
        result: Result<JobOutput, JobError>,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.epoch {
            return;
        }
        self.running = None;
        if result.as_ref().is_err_and(|e| e.repo_gone) {
            // The repository was deleted: its marks go too (spec §4.3).
            self.reset(cx);
            self.set_state(GitState::NoRepo, cx);
            return;
        }
        match result {
            Ok(out) => {
                let old_rev = self.snapshot.base_rev.take();
                self.snapshot = GitSnapshot {
                    generation: self.snapshot.generation + 1,
                    ..out.snapshot
                };
                let reloaded: HashSet<Arc<Path>> =
                    out.reloaded.iter().map(|(p, _, _)| p.clone()).collect();
                for (path, version, diffed) in out.reloaded {
                    let Some(tracked) = self.buffers.get(&path) else {
                        continue;
                    };
                    if tracked.version == version {
                        self.apply_diff(path, diffed, cx);
                    } else {
                        // Edited meanwhile: keep the base, diff the new text.
                        if !diffed.failed {
                            self.store_base(&path, diffed.rev, diffed.text);
                        }
                        self.schedule_hunks(path, Duration::ZERO, cx);
                    }
                }
                if self.snapshot.base_rev != old_rev {
                    // Buffers tracked after the job started may hold hunks
                    // against the old base.
                    let rev = self.snapshot.base_rev.as_ref();
                    let stale: Vec<Arc<Path>> = self
                        .buffers
                        .keys()
                        .filter(|p| {
                            !reloaded.contains(*p) || self.bases.get(*p).map(|b| &b.rev) != rev
                        })
                        .cloned()
                        .collect();
                    for path in stale {
                        self.schedule_hunks(path, Duration::ZERO, cx);
                    }
                }
                if out.too_many {
                    self.set_state(GitState::TooManyChanges, cx);
                    if !std::mem::replace(&mut self.too_many_reported, true) {
                        crate::status_message::show_or_log(self.window, TOO_MANY_MESSAGE, cx);
                    }
                } else {
                    self.set_state(GitState::Ready, cx);
                }
                cx.emit(GitStoreEvent::SnapshotChanged);
            }
            // The last snapshot stays.
            Err(err) => self.report_error(&err.err, cx),
        }
        self.start_next(cx);
    }

    fn report_error(&mut self, err: &GitError, cx: &mut Context<Self>) {
        tracing::warn!(%err, root = %self.root.display(), "git failed");
        if !std::mem::replace(&mut self.error_reported, true) {
            crate::status_message::show_or_log(self.window, format!("Git: {err}"), cx);
        }
    }

    /// Diffs `path` after `delay`, loading its base text when it is not
    /// cached for the current `base_rev`. Nothing runs without a repository.
    fn schedule_hunks(&mut self, path: Arc<Path>, delay: Duration, cx: &mut Context<Self>) {
        let epoch = self.epoch;
        let Some(tracked) = self.buffers.get_mut(&path) else {
            return;
        };
        let version = tracked.version;
        tracked.task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok(Some(work)) = this.update(cx, |s, _| s.hunk_work(&path, version)) else {
                return;
            };
            let diffed = cx
                .background_executor()
                .spawn(async move { work.run() })
                .await;
            this.update(cx, |s, cx| s.finish_hunks(epoch, path, version, diffed, cx))
                .ok();
        }));
    }

    /// The background work that diffs `path` at `version`, or `None` when
    /// there is no repository (untrusted, disabled, still locating).
    fn hunk_work(&self, path: &Arc<Path>, version: u64) -> Option<HunkWork> {
        let repo = self.repo.clone()?;
        let tracked = self.buffers.get(path).filter(|t| t.version == version)?;
        let rev = self.snapshot.base_rev.clone();
        let cached = self
            .bases
            .get(path)
            .filter(|b| Some(&b.rev) == rev.as_ref())
            .map(|b| b.text.clone());
        Some(HunkWork {
            repo,
            path: path.clone(),
            snapshot: tracked.snapshot.clone(),
            rev,
            cached,
            diff_base: self.settings.diff_base,
        })
    }

    fn finish_hunks(
        &mut self,
        epoch: u64,
        path: Arc<Path>,
        version: u64,
        diffed: Diffed,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .buffers
            .get(&path)
            .is_some_and(|t| t.version == version);
        if epoch != self.epoch || !current {
            return;
        }
        if diffed.rev != self.snapshot.base_rev {
            // A rescan moved the base meanwhile.
            self.schedule_hunks(path, Duration::ZERO, cx);
            return;
        }
        self.apply_diff(path, diffed, cx);
    }

    fn apply_diff(&mut self, path: Arc<Path>, diffed: Diffed, cx: &mut Context<Self>) {
        if diffed.failed {
            return;
        }
        self.store_base(&path, diffed.rev, diffed.text);
        self.set_hunks(path, diffed.hunks, cx);
    }

    fn store_base(&mut self, path: &Arc<Path>, rev: Option<String>, text: Option<Arc<str>>) {
        match rev {
            Some(rev) => self.bases.insert(path.clone(), Base { rev, text }),
            None => self.bases.remove(path),
        };
    }

    fn set_hunks(&mut self, path: Arc<Path>, hunks: Option<Arc<[Hunk]>>, cx: &mut Context<Self>) {
        let changed = match hunks {
            Some(hunks) => self.hunks.insert(path.clone(), hunks.clone()).as_ref() != Some(&hunks),
            None => self.hunks.remove(&path).is_some(),
        };
        if changed {
            cx.emit(GitStoreEvent::HunksChanged(path));
        }
    }
}

/// The login shell's environment, read once per process (it can take
/// seconds; call it off the UI thread).
fn login_env() -> &'static HashMap<String, String> {
    static ENV: OnceLock<HashMap<String, String>> = OnceLock::new();
    ENV.get_or_init(lucerna_lsp::env::login_shell_env)
}

/// What write commands (`git switch`) add to the process environment: the
/// login shell's, so hooks and filters (git-lfs, husky's `node`) find their
/// programs as in a terminal. Variables the shell sets about itself are
/// left out.
fn write_env(login: &HashMap<String, String>) -> Vec<(String, String)> {
    const SHELL_OWN: [&str; 4] = ["PWD", "OLDPWD", "SHLVL", "_"];
    login
        .iter()
        .filter(|(k, _)| !SHELL_OWN.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Finds `git` and the repository holding `root`, and with `watch` starts
/// watching its git dirs. Blocking. Without a `.git` at or above `root`
/// there is no repository, and `git` is not looked for (that may run a
/// login shell). A located `git` writes with the login shell's environment.
fn discover(
    git: Option<GitBinary>,
    setting: Option<PathBuf>,
    root: &Path,
    home: Option<&Path>,
    watch: Option<WatchSender>,
) -> Setup {
    if !GitStore::has_repo_hint(root) {
        return Setup::NoRepo;
    }
    let git = match git {
        Some(git) => git,
        None => {
            let login = login_env();
            let found = match setting {
                Some(path) => locate::locate_git(Some(path), None),
                None => locate::locate_git(None, login.get("PATH").map(String::as_str)),
            };
            match found {
                Some(path) => GitBinary::new(path).with_write_env(write_env(login)),
                None => return Setup::NoGit,
            }
        }
    };
    // As in Zed, a repository above the home folder is not the project's.
    match Repo::discover_with_ceiling(git, root, home) {
        Err(GitError::NotFound) => Setup::NoGit,
        Ok(Some(repo)) => {
            let watcher = watch.and_then(|tx| {
                GitDirWatcher::start(&repo.git_dir, &repo.common_dir, move || {
                    let _ = tx.unbounded_send(());
                })
                .inspect_err(|err| tracing::warn!(%err, "could not watch the git directory"))
                .ok()
            });
            Setup::Found(Box::new(Located { repo, watcher }))
        }
        Ok(None) => Setup::NoRepo,
        Err(err) => Setup::Failed(err),
    }
}

#[cfg(test)]
mod tests;
