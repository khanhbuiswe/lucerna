//! Read-only diff tabs (plan 5.2, spec §4.7): an excerpt list of changed
//! files with added, deleted and context lines, under a toolbar with the
//! totals and change navigation.
//!
//! The Uncommitted Changes tab ([`DiffKind::Uncommitted`]) lists `git
//! status` against HEAD, whatever `git.diffBase` says. Its file list is
//! recomputed on every [`GitStoreEvent::SnapshotChanged`]; each file loads
//! on the background executor (at most [`MAX_LOAD_JOBS`] at once) and
//! streams in: headers first, with a "Loading…" note, then the excerpts.
//! A changed file (an edit, a disk change, new hunks) reloads alone.
//!
//! [`DiffKind::Branch`] and [`DiffKind::File`] are placeholders for the
//! tabs plan 5.2 adds next: they show their empty state and run nothing.

pub(crate) mod build;
mod load;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, ParentElement, Render,
    Styled, Subscription, Task, WeakEntity, Window, div, prelude::*, px,
};
use lucerna_git::{FileStatus, GitError};
use lucerna_text::Point;

use crate::excerpt_list::model::{FileMeta, LineRef};
use crate::excerpt_list::{ExcerptList, ExcerptListEvent};
use crate::git::{GitState, GitStore, GitStoreEvent, workspace_relative};
use crate::icons::{CHEVRONS_DOWN_UP, CHEVRONS_UP_DOWN, REFRESH_CW};
use crate::layout::color;
use crate::tasks::guarded;
use crate::ui::{
    CHEVRON_LEFT_ICON, CHEVRON_RIGHT_ICON, GRID, UI_FONT_SIZE_SMALL, bar_icon_button,
    bar_separator, centered_message, search_bar_row, search_toolbar, text_button,
};
use crate::{AppState, DocumentEvent, Documents, OpenMode, Workbench};
use load::{LoadInput, Loaded, Side, load_file};

/// Files loading or diffing at once, as in Zed.
pub const MAX_LOAD_JOBS: usize = 16;

/// The note of a file whose diff has not arrived yet.
const LOADING_NOTE: &str = "Loading…";

/// What a diff tab compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffKind {
    /// The working tree (and open buffers) against HEAD.
    Uncommitted,
    /// The working tree against the merge base with `base_ref` (the
    /// default branch when `None`). Not built yet: shows its empty state.
    Branch { base_ref: Option<String> },
    /// One whole file against `git.diffBase`. Not built yet: shows its
    /// empty state.
    File(Arc<Path>),
}

/// HEAD and the changed files against it, as `git status` lists them.
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
}

pub struct DiffView {
    kind: DiffKind,
    git: Entity<GitStore>,
    documents: Entity<Documents>,
    workbench: WeakEntity<Workbench>,
    root: PathBuf,
    focus: FocusHandle,
    list: Entity<ExcerptList>,
    /// Focus of the list: arrows and Enter work here.
    list_focus: FocusHandle,
    files: HashMap<Arc<Path>, Entry>,
    /// HEAD when the list was last computed.
    head: Option<String>,
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
            let root = root.clone();
            cx.new(|cx| ExcerptList::new(root, "git-diff", cx))
        };
        let list_focus = list.read(cx).focus_handle().clone();
        let mut subscriptions = vec![cx.subscribe(&list, |view, _, event, cx| match event {
            ExcerptListEvent::Activate(r) => view.open_line(*r, cx),
        })];
        let live = kind == DiffKind::Uncommitted;
        if live {
            subscriptions.push(cx.subscribe(&git, |view, _, event, cx| match event {
                GitStoreEvent::SnapshotChanged => view.refresh_list(cx),
                GitStoreEvent::StateChanged => view.state_changed(cx),
                GitStoreEvent::HunksChanged(path) => view.reload(path, cx),
            }));
            subscriptions.push(cx.subscribe(&documents, |view, _, event, cx| match event {
                DocumentEvent::Edited { path, .. } | DocumentEvent::Closed { path } => {
                    if let Some(rel) = workspace_relative(&view.root, path) {
                        view.reload(&rel, cx);
                    }
                }
                _ => {}
            }));
        }
        let mut view = DiffView {
            kind,
            git,
            documents,
            workbench,
            root,
            focus: cx.focus_handle(),
            list,
            list_focus,
            files: HashMap::new(),
            head: None,
            listed: false,
            status_task: None,
            relist: false,
            reload_all: false,
            queue: VecDeque::new(),
            queued: HashSet::new(),
            jobs: HashMap::new(),
            peak_jobs: 0,
            _subscriptions: subscriptions,
        };
        if live {
            view.refresh_list(cx);
        }
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
            DiffKind::Branch {
                base_ref: Some(base),
            } => format!("Changes since {base}"),
            DiffKind::Branch { base_ref: None } => "Changes since default branch".to_string(),
            DiffKind::File(path) => {
                let name = path.file_name().unwrap_or(path.as_os_str());
                format!("{} (Changes)", name.to_string_lossy())
            }
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
        let text = match &self.kind {
            DiffKind::Uncommitted => match self.git.read(cx).state() {
                GitState::Off => "Git is off",
                GitState::Locating => LOADING_NOTE,
                GitState::NoGit => "Git is not available",
                GitState::NoRepo => "Not a Git repository",
                GitState::Restricted => "Git is off in Restricted Mode",
                GitState::Ready | GitState::TooManyChanges if !self.listed => LOADING_NOTE,
                GitState::Ready | GitState::TooManyChanges if self.files.is_empty() => {
                    "No uncommitted changes"
                }
                GitState::Ready | GitState::TooManyChanges => return None,
            },
            DiffKind::Branch {
                base_ref: Some(base),
            } => return Some(format!("No changes since {base}")),
            DiffKind::Branch { base_ref: None } => "No default branch found",
            DiffKind::File(_) => LOADING_NOTE,
        };
        Some(text.to_string())
    }

    /// Selects the first line of the next or previous change, across
    /// files and wrapping (Alt+F5 / Shift+Alt+F5 in the tab).
    pub fn step_change(&mut self, forward: bool, cx: &mut Context<Self>) {
        let found = self
            .list
            .update(cx, |list, cx| list.step_change(forward, cx));
        if found.is_none() {
            crate::status_message::show("No changes", cx);
        }
    }

    /// Reloads the listed files among `paths` (workspace-relative), which
    /// changed on disk.
    pub(crate) fn paths_changed(&mut self, paths: &[Arc<Path>], cx: &mut Context<Self>) {
        for path in paths {
            self.reload(path, cx);
        }
    }

    /// The refresh button: rescans the repository, then reloads every file.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_all = true;
        if let Some(workbench) = self.workbench.upgrade() {
            workbench.update(cx, |wb, cx| wb.run_command("git.refresh", None, window, cx));
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

    /// Drops every file, job and result.
    fn clear(&mut self, cx: &mut Context<Self>) {
        self.files.clear();
        self.queue.clear();
        self.queued.clear();
        self.jobs.clear();
        self.status_task = None;
        self.relist = false;
        self.listed = false;
        self.head = None;
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
            Ok((head, entries)) => {
                let reload_all = std::mem::take(&mut self.reload_all) || head != self.head;
                self.head = head;
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
        // deleted ones folded; changed ones keep their note until loaded.
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
                    for (path, _) in added.iter().filter(|(_, s)| *s == FileStatus::Deleted) {
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
                },
            );
        }
        for (path, _) in added.into_iter().chain(changed) {
            self.queue_load(path);
        }
        self.start_jobs(cx);
    }

    /// Loads file `path` again, if it is listed.
    fn reload(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some((path, _)) = self.files.get_key_value(path) else {
            return;
        };
        let path = path.clone();
        self.queue_load(path);
        self.start_jobs(cx);
    }

    /// Queues a load of `path`; results of earlier loads are dropped.
    fn queue_load(&mut self, path: Arc<Path>) {
        if let Some(entry) = self.files.get_mut(&path) {
            entry.version += 1;
            if self.queued.insert(path.clone()) {
                self.queue.push_back(path);
            }
        }
    }

    /// Starts queued loads up to [`MAX_LOAD_JOBS`] at once.
    fn start_jobs(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.git.read(cx).repo() else {
            return;
        };
        while self.jobs.len() < MAX_LOAD_JOBS {
            let Some(path) = self.queue.pop_front() else {
                break;
            };
            self.queued.remove(&path);
            let Some(entry) = self.files.get(&path) else {
                continue;
            };
            let rev = self.head.clone();
            let input = LoadInput {
                repo: repo.clone(),
                abs_path: self.root.join(&path),
                path: path.clone(),
                status: entry.status,
                cached_base: entry.base.clone().filter(|_| entry.base_rev == rev),
                rev: rev.clone(),
                buffer: self.documents.read(cx).snapshot_of(&path),
            };
            let (version, key) = (entry.version, path.clone());
            let task = cx.spawn(async move |this, cx| {
                let loaded = cx
                    .background_spawn(async move {
                        guarded("git diff load", || load_file(input))
                            .unwrap_or_else(|| Err(GitError::Parse("panicked".into())))
                    })
                    .await;
                this.update(cx, |view, cx| {
                    view.finish_load(key, version, rev, loaded, cx)
                })
                .ok();
            });
            // A running load of the same file is replaced (and dropped).
            self.jobs.insert(path, task);
            self.peak_jobs = self.peak_jobs.max(self.jobs.len());
        }
    }

    fn finish_load(
        &mut self,
        path: Arc<Path>,
        version: u64,
        rev: Option<String>,
        loaded: Result<Loaded, GitError>,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.files.get_mut(&path) else {
            return;
        };
        // This load holds the slot: a newer one replaces (drops) it.
        self.jobs.remove(&path);
        if entry.version != version {
            // Edited meanwhile: the newer load is queued.
            self.start_jobs(cx);
            return;
        }
        let loaded = loaded.unwrap_or_else(|err| {
            tracing::warn!(%err, path = %path.display(), "could not diff the file");
            Loaded::default()
        });
        entry.added = loaded.added;
        entry.removed = loaded.removed;
        if loaded.base.is_some() {
            entry.base = loaded.base;
            entry.base_rev = rev;
        }
        let meta = FileMeta {
            note: loaded.note,
            ..file_meta(entry.status, Some((loaded.added, loaded.removed)))
        };
        let (source, excerpts) = (loaded.source, loaded.excerpts);
        self.list.update(cx, |list, cx| {
            list.set_text_source(path.clone(), source, cx);
            list.update_results(
                |r| {
                    r.set_meta(path.clone(), meta);
                    r.set_file(path, excerpts);
                },
                cx,
            );
        });
        self.start_jobs(cx);
        cx.notify();
    }

    /// Opens the file of line `r` at its line (a deleted line at the line
    /// after it). A deleted file is not opened.
    fn open_line(&mut self, r: LineRef, cx: &mut Context<Self>) {
        let results = self.list.read(cx).results();
        let (Some(file), Some(line), Some(workbench)) = (
            results.files().get(r.file),
            results.line(r),
            self.workbench.upgrade(),
        ) else {
            return;
        };
        if results.meta(&file.path).is_some_and(|m| !m.openable) {
            return;
        }
        let path = file.path.clone();
        let point = Point {
            row: line.line_number.saturating_sub(1),
            column: 0,
        };
        workbench.update(cx, |wb, cx| wb.jump_to(path, point, OpenMode::Pinned, cx));
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

    /// Keys in the list: `↑` / `↓` and `Enter`.
    fn list_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.list.update(cx, |list, cx| list.key_down(ev, cx)) {
            cx.stop_propagation();
        }
    }

    /// The toolbar: `{n} files  +A −R` on the left; previous / next
    /// change, collapse all and refresh on the right.
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<AppState>().theme.ui.clone();
        let (files, added, removed) = self.totals();
        let all_collapsed = self
            .list
            .read(cx)
            .results()
            .files()
            .iter()
            .all(|f| f.collapsed);
        let collapse_icon = if all_collapsed {
            CHEVRONS_UP_DOWN
        } else {
            CHEVRONS_DOWN_UP
        };
        let files_text = match files {
            1 => "1 file".to_string(),
            n => format!("{n} files"),
        };
        let summary = div()
            .debug_selector(|| "git-diff-summary".into())
            .flex()
            .flex_1()
            .min_w_0()
            .gap(px(2. * GRID))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(UI_FONT_SIZE_SMALL))
            .text_color(color(ui.text_muted))
            .child(files_text)
            .child(
                div()
                    .flex()
                    .gap(px(GRID))
                    .child(
                        div()
                            .text_color(color(ui.vc_added))
                            .child(format!("+{added}")),
                    )
                    .child(
                        div()
                            .text_color(color(ui.vc_deleted))
                            .child(format!("\u{2212}{removed}")),
                    ),
            );
        let bar = search_bar_row()
            .child(summary)
            .child(
                bar_icon_button("git-diff-previous", CHEVRON_LEFT_ICON, false, cx)
                    .on_click(cx.listener(|view, _, _, cx| view.step_change(false, cx))),
            )
            .child(
                bar_icon_button("git-diff-next", CHEVRON_RIGHT_ICON, false, cx)
                    .on_click(cx.listener(|view, _, _, cx| view.step_change(true, cx))),
            )
            .child(bar_separator(cx))
            .child(
                bar_icon_button("git-diff-collapse-all", collapse_icon, false, cx)
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_all_collapsed(cx))),
            )
            .child(
                bar_icon_button("git-diff-refresh", REFRESH_CW, false, cx)
                    .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx))),
            );
        search_toolbar(cx)
            .debug_selector(|| "git-diff-toolbar".into())
            .child(bar)
    }

    /// The centered line shown instead of the list, with the "Manage
    /// Workspace Trust" button in Restricted Mode.
    fn render_message(&self, text: String, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<AppState>().theme.ui.clone();
        let restricted =
            self.kind == DiffKind::Uncommitted && self.git.read(cx).state() == GitState::Restricted;
        let trust = restricted.then(|| {
            text_button("git-diff-trust", "Manage Workspace Trust", cx).on_click(cx.listener(
                |view, _, window, cx| {
                    if let Some(workbench) = view.workbench.upgrade() {
                        workbench.update(cx, |wb, cx| {
                            wb.run_command("workbench.trust.manage", None, window, cx)
                        });
                    }
                },
            ))
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(3. * GRID))
            .child(
                centered_message("git-diff-message", text, color(ui.text_muted))
                    .size_auto()
                    .flex_none(),
            )
            .children(trust)
    }
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

impl Render for DiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<AppState>().theme.ui.clone();
        let body = match self.message(cx) {
            Some(text) => self.render_message(text, cx).into_any_element(),
            None => self.list.clone().into_any_element(),
        };
        let toolbar = (!self.files.is_empty()).then(|| self.render_toolbar(cx));
        div()
            .id("git-diff-view")
            .debug_selector(|| "git-diff-view".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(color(ui.editor_background))
            .children(toolbar)
            .child(
                div()
                    .id("git-diff-list")
                    .debug_selector(|| "git-diff-list".into())
                    .track_focus(&self.list_focus)
                    .on_key_down(cx.listener(Self::list_key_down))
                    .flex_1()
                    .min_h_0()
                    .child(body),
            )
    }
}
