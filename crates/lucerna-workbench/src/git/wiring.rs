//! How the workbench feeds its [`GitStore`]: worktree changes, open
//! documents and the git commands.

use std::path::Path;
use std::sync::Arc;

use gpui::{App, Context, Entity, Focusable, Window};
use lucerna_project::WorktreeEvent;

use super::GitStore;
use super::checkout::CheckoutAnchor;
use crate::diff_view::DiffView;
use crate::documents::{DocumentEvent, Documents};
use crate::editor_groups::branch_changes_title;
use crate::{CommandRegistry, EditorGroups, OpenMode, TabItem, Workbench};

/// The status message of `git.openChange` with no file to compare.
const NO_FILE_MESSAGE: &str = "No file to compare";

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register("git.refresh", "Git: Refresh", |wb, _, _, cx| {
        wb.git().update(cx, |git, cx| git.rescan(cx));
        for view in wb.diff_views(cx) {
            view.update(cx, |v, cx| v.git_refreshed(cx));
        }
    });
    commands.register(
        "git.checkout",
        "Git: Checkout to…",
        |wb, _, window, cx| wb.show_checkout_picker(CheckoutAnchor::Center, window, cx),
    );
    commands.register(
        "git.viewChanges",
        "Git: Open Changes (Uncommitted)",
        |wb, _, window, cx| wb.open_git_changes(window, cx),
    );
    commands.register(
        "lucerna.git.viewBranchChanges",
        "Git: Open Changes Since Branch",
        |wb, _, window, cx| wb.open_branch_changes(window, cx),
    );
    commands.register(
        "git.openChange",
        "Git: Open Changes",
        |wb, _, window, cx| wb.open_active_file_diff(window, cx),
    );
    commands.register("git.openFile", "Git: Open File", |wb, _, _, cx| {
        wb.open_diff_file(cx)
    });
    commands.register(
        "workbench.action.editor.nextChange",
        "Go to Next Change",
        |wb, _, window, cx| wb.go_to_change(true, window, cx),
    );
    commands.register(
        "workbench.action.editor.previousChange",
        "Go to Previous Change",
        |wb, _, window, cx| wb.go_to_change(false, window, cx),
    );
}

/// `path` relative to `root`: a relative path as it is, an absolute one
/// with `root` stripped. `None` for a file outside `root` and for an
/// untitled document, which is in no repository.
pub(crate) fn workspace_relative(root: &Path, path: &Path) -> Option<Arc<Path>> {
    if crate::untitled::is_untitled(path) {
        return None;
    }
    if path.is_relative() {
        return Some(Arc::from(path));
    }
    path.strip_prefix(root).ok().map(Arc::from)
}

impl Workbench {
    /// The repository of the workspace: statuses and gutter hunks.
    pub fn git(&self) -> &Entity<GitStore> {
        &self.git
    }

    /// New hunks of `path` (workspace-relative) reach every view of it;
    /// none (untracked, closed, git off) clear its bars.
    pub(crate) fn git_hunks_changed(&mut self, path: &Path, cx: &mut Context<Self>) {
        let hunks = self
            .git
            .read(cx)
            .hunks(path)
            .unwrap_or_else(|| Arc::from([]));
        let views = self.documents().read(cx).views_of(path);
        for view in views {
            view.update(cx, |v, cx| v.set_diff_hunks(hunks.clone(), cx));
        }
    }

    /// Focuses the Uncommitted Changes tab wherever it is, or opens it in
    /// the active group; its list takes the focus once drawn.
    pub fn open_git_changes(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.update_editor_groups(cx, EditorGroups::open_git_changes);
        cx.notify();
    }

    /// Opens the diff tab of `path` (absolute, or relative to the root),
    /// or activates it where it is; with `row`, selects and scrolls to the
    /// first changed line at or after that row once the file has loaded.
    /// A path outside the root shows "No file to compare".
    pub fn open_file_diff(
        &mut self,
        path: Arc<Path>,
        row: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = workspace_relative(self.root(), &path) else {
            crate::status_message::show(window.window_handle().window_id(), NO_FILE_MESSAGE, cx);
            return;
        };
        self.update_editor_groups(cx, |groups| groups.open_file_diff(path));
        let body = self
            .editor_groups()
            .update(cx, |groups, cx| groups.ensure_active_body(window, cx));
        if let (Some(row), Some(view)) = (row, body.and_then(|b| b.downcast::<DiffView>().ok())) {
            view.update(cx, |v, cx| v.reveal_row(row, cx));
        }
        cx.notify();
    }

    /// `git.openChange`: the diff of the active tab's file.
    fn open_active_file_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let item = self
            .editor_groups()
            .read(cx)
            .model()
            .active_tab()
            .map(|t| t.item.clone());
        match item {
            Some(TabItem::File(path) | TabItem::FileDiff(path)) => {
                self.open_file_diff(path, None, window, cx)
            }
            _ => {
                crate::status_message::show(window.window_handle().window_id(), NO_FILE_MESSAGE, cx)
            }
        }
    }

    /// `git.openFile` in the active diff tab: the selected line's file at
    /// that line, or the file of a single-file tab.
    fn open_diff_file(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self
            .editor_groups()
            .read(cx)
            .active_body()
            .and_then(|b| b.downcast::<DiffView>().ok())
        else {
            return;
        };
        match view.read(cx).file_to_open(cx) {
            Some((path, Some(point))) => {
                let path = self.project_path(&path);
                self.jump_to(path, point, OpenMode::Pinned, cx)
            }
            Some((path, None)) => self.open_path(path, OpenMode::Pinned, cx),
            None => {}
        }
    }

    /// The view of the diff tab of `path` (absolute, or relative to the
    /// root), while the tab is open and has been drawn.
    pub fn file_diff_view(&self, path: &Path, cx: &App) -> Option<Entity<DiffView>> {
        let path = workspace_relative(self.root(), path)?;
        let groups = self.editor_groups().read(cx);
        let tab = groups.model().file_diff_tab(&path)?;
        groups.body_of(tab)?.downcast::<DiffView>().ok()
    }

    /// The views of every drawn diff tab.
    pub(crate) fn diff_views(&self, cx: &App) -> Vec<Entity<DiffView>> {
        let groups = self.editor_groups().read(cx);
        groups
            .model()
            .groups()
            .iter()
            .flat_map(|g| &g.tabs)
            .filter(|t| {
                matches!(
                    t.item,
                    TabItem::GitChanges | TabItem::FileDiff(_) | TabItem::BranchChanges
                )
            })
            .filter_map(|t| groups.body_of(t.id)?.downcast::<DiffView>().ok())
            .collect()
    }

    /// Focuses the Changes since tab wherever it is, or opens it in the
    /// active group; its list takes the focus once drawn.
    pub fn open_branch_changes(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.update_editor_groups(cx, EditorGroups::open_branch_changes);
        cx.notify();
    }

    /// The view of the Changes since tab, while the tab is open and has
    /// been drawn.
    pub fn branch_changes_view(&self, cx: &App) -> Option<Entity<DiffView>> {
        let groups = self.editor_groups().read(cx);
        let tab = groups.model().branch_changes_tab()?;
        groups.body_of(tab)?.downcast::<DiffView>().ok()
    }

    /// The ref the Changes since tab compares with: the picked one, else
    /// the default branch; `None` when there is neither.
    pub(crate) fn branch_diff_ref(&self, cx: &App) -> Option<String> {
        let chosen = self.branch_base.read(cx).0.clone();
        chosen.or_else(|| self.git.read(cx).snapshot().default_branch)
    }

    /// Names the Changes since tab's ref in the tab bar.
    pub(crate) fn update_branch_title(&mut self, cx: &mut Context<Self>) {
        let title = branch_changes_title(self.branch_diff_ref(cx).as_deref());
        self.editor_groups().update(cx, |groups, cx| {
            groups.set_branch_title(Some(title.into()), cx)
        });
    }

    /// The view of the Uncommitted Changes tab, while the tab is open and
    /// has been drawn.
    pub fn git_changes_view(&self, cx: &App) -> Option<Entity<DiffView>> {
        let groups = self.editor_groups().read(cx);
        let tab = groups.model().git_changes_tab()?;
        groups.body_of(tab)?.downcast::<DiffView>().ok()
    }

    /// The active tab's diff view, when it has the keyboard focus
    /// (`gitDiffViewFocus`).
    pub(crate) fn focused_diff_view(&self, window: &Window, cx: &App) -> Option<Entity<DiffView>> {
        let view = self
            .editor_groups()
            .read(cx)
            .active_body()?
            .downcast::<DiffView>()
            .ok()?;
        view.read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
            .then_some(view)
    }

    /// Alt+F5 / Shift+Alt+F5: the next or previous change of the focused
    /// diff tab (across its files), else of the active editor.
    fn go_to_change(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.focused_diff_view(window, cx) {
            view.update(cx, |v, cx| v.step_change(forward, cx));
            return;
        }
        let Some(view) = self.active_code_view(cx) else {
            return;
        };
        if !view.update(cx, |v, cx| v.next_change(forward, cx)) {
            crate::status_message::show(window.window_handle().window_id(), "No changes", cx);
        }
    }

    /// Changed paths refresh their status (and the open files among them
    /// re-diff once their documents reload); a worktree rescan rescans.
    pub(crate) fn git_worktree_event(&mut self, ev: &WorktreeEvent, cx: &mut Context<Self>) {
        match ev {
            WorktreeEvent::Changed(changes) => {
                let paths: Vec<Arc<Path>> = changes
                    .iter()
                    .filter_map(|c| workspace_relative(self.root(), &c.path))
                    .collect();
                for view in self.diff_views(cx) {
                    view.update(cx, |v, cx| v.paths_changed(&paths, cx));
                }
                self.git.update(cx, |git, cx| git.paths_changed(paths, cx));
            }
            WorktreeEvent::Rescanned => self.git.update(cx, |git, cx| git.rescan(cx)),
            // Reading a folder changes no file.
            WorktreeEvent::ScanProgress
            | WorktreeEvent::ScanComplete
            | WorktreeEvent::Loaded(_)
            | WorktreeEvent::WatchFailed(_) => {}
        }
    }

    /// Open documents inside the root are diffed against the git base: on
    /// open and on every change (edits, undo, reloads from disk); closing
    /// one frees its base and hunks. A save changes no text (and its
    /// snapshot may be older than the buffer), so it is not forwarded: the
    /// worktree reports the file and its status refreshes.
    pub(crate) fn git_document_event(
        &mut self,
        _: Entity<Documents>,
        ev: &DocumentEvent,
        cx: &mut Context<Self>,
    ) {
        match ev {
            DocumentEvent::Opened { path, snapshot, .. }
            | DocumentEvent::Edited { path, snapshot, .. } => {
                let Some(rel) = workspace_relative(self.root(), path) else {
                    return;
                };
                let snapshot = snapshot.clone();
                self.git
                    .update(cx, |git, cx| git.track_buffer(rel, snapshot, cx));
            }
            DocumentEvent::Closed { path } => {
                if let Some(rel) = workspace_relative(self.root(), path) {
                    self.git.update(cx, |git, _| git.untrack_buffer(&rel));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_relative_strips_the_root() {
        let work = std::env::temp_dir().join("work");
        let root = work.join("app");
        let rel = |p: &Path| workspace_relative(&root, p);
        let a = Path::new("src").join("a.rs");
        assert_eq!(rel(&a).as_deref(), Some(a.as_path()));
        assert_eq!(rel(&root.join(&a)).as_deref(), Some(a.as_path()));
        assert_eq!(rel(&work.join("other").join("a.rs")), None);
        assert_eq!(rel(&work.join("application").join("a.rs")), None);
    }
}
