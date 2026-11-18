//! Read-only diff tabs (plan 5.2, spec §4.7): an excerpt list of changed
//! files with added, deleted and context lines, under a toolbar with the
//! totals and change navigation.
//!
//! The Uncommitted Changes tab ([`DiffKind::Uncommitted`]) lists `git
//! status` against HEAD, whatever `git.diffBase` says. Its file list is
//! recomputed on every [`GitStoreEvent::SnapshotChanged`]; each file loads
//! on the background executor (at most [`MAX_LOAD_JOBS`] at once) and
//! streams in: headers first, with a "Loading…" note, then the excerpts.
//! A changed file (an edit, a disk change, new hunks) reloads alone; edits
//! and new hunks wait [`RELOAD_DEBOUNCE`] and are merged, so a keystroke
//! diffs the file once.
//!
//! The single-file tab ([`DiffKind::File`], spec §4.8) shows one whole file
//! against `git.diffBase`, its status and base taken from the
//! [`GitStore`] snapshot (no `git status` of its own).
//!
//! The Changes since ‹branch› tab ([`DiffKind::Branch`]) shows the
//! worktree against the merge base with a ref; see [`branch`].

mod branch;
pub(crate) mod build;
mod jobs;
mod load;
mod render;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, Subscription, Task, WeakEntity,
    Window, WindowId,
};
use lucerna_git::{FileStatus, GitError};
use lucerna_project::LineKind;
use lucerna_text::Point;

use crate::excerpt_list::model::{FileMeta, LineRef, Results};
use crate::excerpt_list::{ExcerptList, ExcerptListEvent};
use crate::git::{GitState, GitStore, GitStoreEvent, workspace_relative};
use crate::tasks::guarded;
use crate::{DocumentEvent, Documents, OpenMode, Workbench};
pub(crate) use branch::BranchBase;
use branch::BranchState;
use load::Side;

/// Files loading or diffing at once, as in Zed.
pub const MAX_LOAD_JOBS: usize = 16;

/// The note of a file whose diff has not arrived yet.
const LOADING_NOTE: &str = "Loading…";

/// How long edits and new hunks of a file wait before it reloads, merged.
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(50);

/// What a diff tab compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffKind {
    /// The working tree (and open buffers) against HEAD.
    Uncommitted,
    /// The working tree (and open buffers) against the merge base of HEAD
    /// with `base_ref`, the ref the user picked (the default branch when
    /// `None`).
    Branch { base_ref: Option<String> },
    /// One whole file (workspace-relative) against `git.diffBase`.
    File(Arc<Path>),
}

/// The base revision and the files changed against it.
type Listing = (Option<String>, Vec<(PathBuf, FileStatus)>);

/// A listed file.
struct Entry {
    status: FileStatus,
    /// Bumped by each reload: older results are dropped.
    version: u64,
    added: u32,
    removed: u32,
    /// The base side read at `base_rev`, reused by reloads.
    base: Option<Side>,
    base_rev: Option<String>,
    /// The open document's revision the last started load read (`None`:
    /// the disk).
    buffer_revision: Option<u64>,
    /// Whether a load has finished.
    loaded: bool,
}

pub struct DiffView {
    kind: DiffKind,
    git: Entity<GitStore>,
    documents: Entity<Documents>,
    workbench: WeakEntity<Workbench>,
    /// The window whose status bar says "No changes".
    window: Option<WindowId>,
    root: PathBuf,
    focus: FocusHandle,
    list: Entity<ExcerptList>,
    /// Focus of the list: arrows and Enter work here.
    list_focus: FocusHandle,
    files: HashMap<Arc<Path>, Entry>,
    /// The base revision when the list was last computed: HEAD,
    /// `git.diffBase`'s for a single file, or the merge base with the ref.
    rev: Option<String>,
    /// The ref a branch tab compares with, as git names it: the picked one
    /// or the default branch. `None` while there is none.
    shown_ref: Option<String>,
    branch: BranchState,
    /// File lists applied so far.
    listings_run: usize,
    /// Whether a file list has arrived since the repository was found.
    listed: bool,
    /// The running `git status`; another one is due when `relist` is set.
    status_task: Option<Task<()>>,
    relist: bool,
    /// The next list reloads every file (the refresh button).
    reload_all: bool,
    queue: VecDeque<Arc<Path>>,
    /// The paths in `queue`.
    queued: HashSet<Arc<Path>>,
    jobs: HashMap<Arc<Path>, Task<()>>,
    peak_jobs: usize,
    loads_started: usize,
    /// Files to reload once [`RELOAD_DEBOUNCE`] passes with no new edit.
    pending_reloads: HashSet<Arc<Path>>,
    reload_timer: Option<Task<()>>,
    /// The row to select once the file of a single-file tab has loaded.
    pending_row: Option<u32>,
    _subscriptions: Vec<Subscription>,
}

impl DiffView {
    /// A diff tab of `kind` over the repository of `git`; `root` is the
    /// workspace root the paths are relative to.
    pub fn new(
        kind: DiffKind,
        git: Entity<GitStore>,
        documents: Entity<Documents>,
        root: PathBuf,
        workbench: WeakEntity<Workbench>,
        cx: &mut Context<Self>,
    ) -> Self {
        let list = {
            let (root, settings) = (root.clone(), documents.read(cx).settings().clone());
            cx.new(|cx| ExcerptList::new(root, "git-diff", settings, cx))
        };
        let list_focus = list.read(cx).focus_handle().clone();
        let subscriptions = vec![
            cx.subscribe(&list, |view, _, event, cx| match event {
                // Like Search and References: a preview.
                ExcerptListEvent::Activate(r) => view.open_line(*r, OpenMode::Preview, cx),
                // Headers are not selectable here.
                ExcerptListEvent::ActivateHeader(_) => {}
            }),
            cx.subscribe(&git, |view, _, event, cx| match event {
                GitStoreEvent::SnapshotChanged => view.refresh_list(cx),
                GitStoreEvent::StateChanged => view.state_changed(cx),
                GitStoreEvent::HunksChanged(path) => view.schedule_reload(path.clone(), cx),
            }),
            cx.subscribe(&documents, |view, _, event, cx| match event {
                DocumentEvent::Edited { path, .. } | DocumentEvent::Closed { path } => {
                    if let Some(rel) = workspace_relative(&view.root, path) {
                        view.schedule_reload(rel, cx);
                    }
                }
                _ => {}
            }),
        ];
        let shown_ref = match &kind {
            DiffKind::Branch { base_ref } => base_ref.clone(),
            _ => None,
        };
        let mut view = DiffView {
            kind,
            git,
            documents,
            workbench,
            window: None,
            root,
            focus: cx.focus_handle(),
            list,
            list_focus,
            files: HashMap::new(),
            rev: None,
            shown_ref,
            branch: BranchState::default(),
            listings_run: 0,
            listed: false,
            status_task: None,
            relist: false,
            reload_all: false,
            queue: VecDeque::new(),
            queued: HashSet::new(),
            jobs: HashMap::new(),
            peak_jobs: 0,
            loads_started: 0,
            pending_reloads: HashSet::new(),
            reload_timer: None,
            pending_row: None,
            _subscriptions: subscriptions,
        };
        view.refresh_list(cx);
        view
    }

    pub fn kind(&self) -> &DiffKind {
        &self.kind
    }

    /// "Uncommitted Changes", "Changes since {ref}" or "{file name}
    /// (Changes)".
    pub fn title(&self) -> String {
        match &self.kind {
            DiffKind::Uncommitted => crate::editor_groups::GIT_CHANGES_TAB_TITLE.to_string(),
            DiffKind::Branch { .. } => {
                crate::editor_groups::branch_changes_title(self.shown_ref.as_deref())
            }
            DiffKind::File(path) => crate::editor_groups::file_diff_title(path),
        }
    }

    pub fn excerpt_list(&self) -> &Entity<ExcerptList> {
        &self.list
    }

    /// The focus handle of the list.
    pub fn list_focus_handle(&self) -> &FocusHandle {
        &self.list_focus
    }

    /// The most files that loaded at once; never above [`MAX_LOAD_JOBS`].
    #[doc(hidden)]
    pub fn peak_jobs(&self) -> usize {
        self.peak_jobs
    }

    /// File lists (`git status`, or the changes since a merge base)
    /// applied so far.
    #[doc(hidden)]
    pub fn listings_run(&self) -> usize {
        self.listings_run
    }

    /// File loads (each one a background diff) started so far.
    #[doc(hidden)]
    pub fn loads_started(&self) -> usize {
        self.loads_started
    }

    /// Listed files, and the lines they add and remove in all.
    pub fn totals(&self) -> (usize, u32, u32) {
        let (added, removed) = self
            .files
            .values()
            .fold((0, 0), |(a, r), e| (a + e.added, r + e.removed));
        (self.files.len(), added, removed)
    }

    /// The line shown instead of the list: why there is nothing to show,
    /// or `None` when files are listed.
    pub fn message(&self, cx: &App) -> Option<String> {
        let text = match self.git.read(cx).state() {
            GitState::Off => "Git is off",
            GitState::Locating => LOADING_NOTE,
            GitState::NoGit => "Git is not available",
            GitState::NoRepo => "Not a Git repository",
            GitState::Restricted => "Git is off in Restricted Mode",
            GitState::Ready | GitState::TooManyChanges if !self.listed => LOADING_NOTE,
            GitState::Ready | GitState::TooManyChanges if self.branch.unresolved => {
                "No default branch found"
            }
            GitState::Ready | GitState::TooManyChanges if !self.files.is_empty() => return None,
            GitState::Ready | GitState::TooManyChanges => match &self.shown_ref {
                Some(base) if matches!(self.kind, DiffKind::Branch { .. }) => {
                    return Some(format!("No changes since {base}"));
                }
                _ => "No uncommitted changes",
            },
        };
        Some(text.to_string())
    }

    /// Tells the tab which window it is in.
    pub fn set_window(&mut self, window: WindowId) {
        self.window = Some(window);
    }

    /// Selects the first line of the next or previous change, across
    /// files and wrapping (Alt+F5 / Shift+Alt+F5 in the tab).
    pub fn step_change(&mut self, forward: bool, cx: &mut Context<Self>) {
        let found = self
            .list
            .update(cx, |list, cx| list.step_change(forward, cx));
        if found.is_none() {
            crate::status_message::show_or_log(self.window, "No changes", cx);
        }
    }

    /// Selects the first changed line at or after `row` of the file of a
    /// single-file tab (else its first line there) and scrolls to it, now
    /// or once the file has loaded.
    pub fn reveal_row(&mut self, row: u32, cx: &mut Context<Self>) {
        self.pending_row = Some(row);
        self.apply_pending_row(cx);
    }

    /// What `git.openFile` opens: the selected line's file at that line,
    /// or, with no selection, the file of a single-file tab. `None` for a
    /// deleted file.
    pub fn file_to_open(&self, cx: &App) -> Option<(Arc<Path>, Option<Point>)> {
        if let Some(r) = self.list.read(cx).selected() {
            return self.line_target(r, cx).map(|(p, at)| (p, Some(at)));
        }
        match &self.kind {
            DiffKind::File(path)
                if self
                    .files
                    .get(path)
                    .is_none_or(|e| e.status != FileStatus::Deleted) =>
            {
                Some((path.clone(), None))
            }
            _ => None,
        }
    }

    /// The "Open File" button: [`DiffView::file_to_open`], pinned.
    fn open_file(&mut self, cx: &mut Context<Self>) {
        let (Some((path, at)), Some(workbench)) = (self.file_to_open(cx), self.workbench.upgrade())
        else {
            return;
        };
        workbench.update(cx, |wb, cx| match at {
            Some(point) => {
                let path = wb.project_path(&path);
                wb.jump_to(path, point, OpenMode::Pinned, cx)
            }
            None => wb.open_path(path, OpenMode::Pinned, cx),
        });
    }

    /// Reloads the listed files among `paths` (workspace-relative), which
    /// changed on disk; a branch tab also asks git whether they changed
    /// since the merge base.
    pub(crate) fn paths_changed(&mut self, paths: &[Arc<Path>], cx: &mut Context<Self>) {
        for path in paths {
            self.reload(path, cx);
        }
        if matches!(self.kind, DiffKind::Branch { .. }) && !paths.is_empty() {
            self.branch_paths_changed(paths.to_vec(), cx);
        }
    }

    /// `git.refresh` ran: the next list reloads every file, and a branch
    /// tab resolves its ref and lists its files again now.
    pub(crate) fn git_refreshed(&mut self, cx: &mut Context<Self>) {
        self.reload_all = true;
        if matches!(self.kind, DiffKind::Branch { .. }) {
            self.refresh_list(cx);
        }
    }

    /// The refresh button: runs `git.refresh`, which rescans the repository
    /// and reloads every file of every diff tab.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workbench = self.workbench.clone();
        // The command reaches this view too: run it once this update ends.
        window.defer(cx, move |window, cx| {
            workbench
                .update(cx, |wb, cx| wb.run_command("git.refresh", None, window, cx))
                .ok();
        });
    }

    /// Shows the branch picker (the toolbar's ref button and "Choose
    /// Branch").
    fn choose_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workbench) = self.workbench.upgrade() {
            workbench.update(cx, |wb, cx| wb.show_branch_picker(window, cx));
        }
    }

    /// The repository went away or came back: the list follows.
    fn state_changed(&mut self, cx: &mut Context<Self>) {
        if !matches!(
            self.git.read(cx).state(),
            GitState::Ready | GitState::TooManyChanges
        ) {
            self.clear(cx);
        }
        cx.notify();
    }

    /// Drops every file, job and result, and what was due once they came:
    /// a full reload and a row to reveal.
    fn clear(&mut self, cx: &mut Context<Self>) {
        self.files.clear();
        self.queue.clear();
        self.queued.clear();
        self.jobs.clear();
        self.status_task = None;
        self.relist = false;
        self.reload_all = false;
        self.pending_row = None;
        self.listed = false;
        self.rev = None;
        self.branch = BranchState::default();
        self.pending_reloads.clear();
        self.reload_timer = None;
        self.list.update(cx, |list, cx| list.clear(cx));
    }

    /// Lists the changed files again, off the UI thread; while a listing
    /// runs, one more follows it.
    fn refresh_list(&mut self, cx: &mut Context<Self>) {
        let state = self.git.read(cx).state();
        let repo = self.git.read(cx).repo();
        let Some(repo) =
            repo.filter(|_| matches!(state, GitState::Ready | GitState::TooManyChanges))
        else {
            self.clear(cx);
            cx.notify();
            return;
        };
        if matches!(self.kind, DiffKind::Branch { .. }) {
            return self.refresh_branch_list(repo, cx);
        }
        if let DiffKind::File(path) = &self.kind {
            // The store already knows the file's status and base.
            let snapshot = self.git.read(cx).snapshot();
            // Not listed: unchanged (or outside git), read like a tracked
            // file; the base read finds nothing for an outside one.
            let status = snapshot
                .statuses
                .get(path)
                .copied()
                .unwrap_or(FileStatus::Modified);
            let listing = (snapshot.base_rev, vec![(path.to_path_buf(), status)]);
            self.apply_list(Ok(listing), cx);
            return;
        }
        if self.status_task.is_some() {
            self.relist = true;
            return;
        }
        self.status_task = Some(cx.spawn(async move |this, cx| {
            let listed = cx
                .background_spawn(async move {
                    guarded("git diff list", || -> Result<_, GitError> {
                        Ok((repo.head_oid()?, repo.status(None)?))
                    })
                    .unwrap_or_else(|| Err(GitError::Parse("panicked".into())))
                })
                .await;
            this.update(cx, |view, cx| view.apply_list(listed, cx)).ok();
        }));
    }

    /// Applies a finished listing: vanished files go, new ones come in as
    /// loading headers, and files whose status (or HEAD) changed reload.
    fn apply_list(&mut self, listed: Result<Listing, GitError>, cx: &mut Context<Self>) {
        self.status_task = None;
        match listed {
            Ok((rev, entries)) => {
                self.listings_run += 1;
                let reload_all = std::mem::take(&mut self.reload_all) || rev != self.rev;
                self.rev = rev;
                self.listed = true;
                self.update_files(entries, reload_all, cx);
            }
            Err(err) => tracing::warn!(%err, "git diff list failed"),
        }
        if std::mem::take(&mut self.relist) {
            self.refresh_list(cx);
        }
        cx.notify();
    }

    fn update_files(
        &mut self,
        entries: Vec<(PathBuf, FileStatus)>,
        reload_all: bool,
        cx: &mut Context<Self>,
    ) {
        let fresh: HashMap<Arc<Path>, FileStatus> = entries
            .into_iter()
            .map(|(p, s)| (Arc::from(p), s))
            .collect();
        let gone: Vec<Arc<Path>> = self
            .files
            .keys()
            .filter(|p| !fresh.contains_key(*p))
            .cloned()
            .collect();
        for path in &gone {
            self.files.remove(path);
            self.jobs.remove(path);
        }
        let mut added: Vec<(Arc<Path>, FileStatus)> = Vec::new();
        let mut changed: Vec<(Arc<Path>, FileStatus)> = Vec::new();
        for (path, status) in fresh {
            match self.files.get_mut(&path) {
                None => added.push((path, status)),
                Some(entry) if entry.status != status || reload_all => {
                    entry.status = status;
                    changed.push((path, status));
                }
                Some(_) => {}
            }
        }
        // Loads go in the order the files are shown.
        added.sort_by(|(a, sa), (b, sb)| (group(*sa), a).cmp(&(group(*sb), b)));
        changed.sort_by(|(a, sa), (b, sb)| (group(*sa), a).cmp(&(group(*sb), b)));
        // One change of the list: new files come in as loading headers,
        // deleted ones folded (not a single file's); changed ones keep
        // their note until loaded.
        let fold_deleted = !matches!(self.kind, DiffKind::File(_));
        let files = &self.files;
        self.list.update(cx, |list, cx| {
            list.update_results(
                |r| {
                    r.remove_files(&gone);
                    for (path, status) in added.iter().chain(&changed) {
                        let entry = files.get(path);
                        let note = match entry {
                            None => Some(LOADING_NOTE),
                            Some(_) => r.meta(path).and_then(|m| m.note),
                        };
                        let meta = file_meta(*status, entry.map(|e| (e.added, e.removed)));
                        r.set_meta(path.clone(), FileMeta { note, ..meta });
                    }
                    r.add_files(added.iter().map(|(p, _)| (p.clone(), Vec::new())));
                    let deleted = added.iter().filter(|(_, s)| *s == FileStatus::Deleted);
                    for (path, _) in deleted.filter(|_| fold_deleted) {
                        if let Some(file) = r.file_index(path) {
                            r.set_collapsed(file, true);
                        }
                    }
                },
                cx,
            );
        });
        for (path, status) in &added {
            self.files.insert(
                path.clone(),
                Entry {
                    status: *status,
                    version: 0,
                    added: 0,
                    removed: 0,
                    base: None,
                    base_rev: None,
                    buffer_revision: None,
                    loaded: false,
                },
            );
        }
        for (path, _) in added.into_iter().chain(changed) {
            self.queue_load(path);
        }
        self.start_jobs(cx);
    }

    /// Selects the row asked for by [`DiffView::reveal_row`] once the file
    /// of a single-file tab has its lines.
    fn apply_pending_row(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.pending_row else {
            return;
        };
        let DiffKind::File(path) = &self.kind else {
            self.pending_row = None;
            return;
        };
        if !self.files.get(path).is_some_and(|e| e.loaded) {
            return;
        }
        self.pending_row = None;
        let results = self.list.read(cx).results();
        if let Some(r) = results
            .file_index(path)
            .and_then(|file| line_at_or_after(results, file, row))
        {
            self.list.update(cx, |list, cx| list.select_line(r, cx));
        }
    }

    /// The file of line `r` and its place: the line (a deleted line's is
    /// the line after it). `None` for a deleted file.
    fn line_target(&self, r: LineRef, cx: &App) -> Option<(Arc<Path>, Point)> {
        let results = self.list.read(cx).results();
        let (file, line) = (results.files().get(r.file)?, results.line(r)?);
        if results.meta(&file.path).is_some_and(|m| !m.openable) {
            return None;
        }
        let point = Point {
            row: line.line_number.saturating_sub(1),
            column: 0,
        };
        Some((file.path.clone(), point))
    }

    /// Opens the file of line `r` at its line with `mode`.
    fn open_line(&mut self, r: LineRef, mode: OpenMode, cx: &mut Context<Self>) {
        let (Some((path, point)), Some(workbench)) =
            (self.line_target(r, cx), self.workbench.upgrade())
        else {
            return;
        };
        // The path is relative to the root: a real file named
        // `untitled:…` is never an Untitled key.
        workbench.update(cx, |wb, cx| {
            let path = wb.project_path(&path);
            wb.jump_to(path, point, mode, cx)
        });
    }

    /// Folds every file, or unfolds them all when all are folded.
    fn toggle_all_collapsed(&mut self, cx: &mut Context<Self>) {
        let all = self
            .list
            .read(cx)
            .results()
            .files()
            .iter()
            .all(|f| f.collapsed);
        self.list
            .update(cx, |list, cx| list.set_all_collapsed(!all, cx));
    }
}

/// The first added or deleted line of file `file` at or after buffer row
/// `row` (a deleted line counts at the row after it), else its first line
/// there.
fn line_at_or_after(results: &Results, file: usize, row: u32) -> Option<LineRef> {
    let mut first = None;
    for (excerpt, e) in results.files().get(file)?.excerpts.iter().enumerate() {
        for (line, l) in e.lines.iter().enumerate() {
            if l.line_number.saturating_sub(1) < row {
                continue;
            }
            let r = LineRef {
                file,
                excerpt,
                line,
            };
            if l.kind != LineKind::Normal {
                return Some(r);
            }
            first.get_or_insert(r);
        }
    }
    first
}

/// Where a file of `status` is listed: conflicts, then tracked files,
/// then new ones (spec §4.7).
fn group(status: FileStatus) -> u8 {
    match status {
        FileStatus::Conflict => 0,
        FileStatus::Modified
        | FileStatus::Added
        | FileStatus::Deleted
        | FileStatus::TypeChanged => 1,
        FileStatus::Untracked => 2,
    }
}

/// The header meta of a file of `status` with `counts` (added, removed).
fn file_meta(status: FileStatus, counts: Option<(u32, u32)>) -> FileMeta {
    let (added, removed) = counts.unwrap_or_default();
    FileMeta {
        added,
        removed,
        label: match status {
            FileStatus::Added | FileStatus::Untracked => Some("new"),
            FileStatus::Deleted => Some("deleted"),
            _ => None,
        },
        note: None,
        openable: status != FileStatus::Deleted,
        group: group(status),
    }
}

impl Focusable for DiffView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
