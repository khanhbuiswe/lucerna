//! How the workbench feeds its [`GitStore`]: worktree changes, open
//! documents and the `git.refresh` command.

use std::path::Path;
use std::sync::Arc;

use gpui::{App, Context, Entity, Focusable, Window};
use lucerna_project::WorktreeEvent;

use super::GitStore;
use crate::diff_view::DiffView;
use crate::documents::{DocumentEvent, Documents};
use crate::{CommandRegistry, EditorGroups, Workbench};

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register("git.refresh", "Git: Refresh", |wb, _, _, cx| {
        wb.git().update(cx, |git, cx| git.rescan(cx));
    });
    commands.register(
        "git.viewChanges",
        "Git: Open Changes (Uncommitted)",
        |wb, _, window, cx| wb.open_git_changes(window, cx),
    );
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
/// with `root` stripped. `None` for a file outside `root`.
pub(crate) fn workspace_relative(root: &Path, path: &Path) -> Option<Arc<Path>> {
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
            crate::status_message::show("No changes", cx);
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
                if let Some(view) = self.git_changes_view(cx) {
                    view.update(cx, |v, cx| v.paths_changed(&paths, cx));
                }
                self.git.update(cx, |git, cx| git.paths_changed(paths, cx));
            }
            WorktreeEvent::Rescanned => self.git.update(cx, |git, cx| git.rescan(cx)),
            WorktreeEvent::ScanProgress | WorktreeEvent::ScanComplete => {}
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
