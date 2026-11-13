//! The pure Explorer model: which directories are expanded, the flattened
//! rows that are visible, and which files changed on disk recently.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lucerna_project::{EntryKind, WorktreeSnapshot};

/// How long a file stays decorated after it changed on disk.
pub const CHANGE_DECORATION: Duration = Duration::from_secs(10);

/// One visible line of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Root-relative.
    pub path: Arc<Path>,
    /// 0 for the root's children.
    pub depth: u16,
    pub kind: EntryKind,
    /// Only directories are ever expanded.
    pub expanded: bool,
    /// The row whose name is being typed: the placeholder row of New
    /// File/Folder or the renamed entry. The placeholder's `path` is its
    /// parent directory, and it always comes right after its parent's row
    /// (first child), so every lookup by path finds the parent row first;
    /// for the root (`""`) no row has that path.
    pub edit: bool,
}

/// What the name being typed in the tree is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    NewFile,
    NewFolder,
    Rename,
}

/// A name being typed in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditState {
    pub kind: EditKind,
    /// Root-relative directory the name is in; empty for the root.
    pub parent: Arc<Path>,
    /// Root-relative entry being renamed.
    pub original: Option<Arc<Path>>,
}

/// What the Explorer has selected: the project root or one of its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Root,
    /// Root-relative.
    Path(Arc<Path>),
}

/// The kind of the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedKind {
    Root,
    Dir,
    File,
}

pub struct ExplorerModel {
    selection: Selection,
    /// [`Self::selected_kind`], kept in step with `selection` and `rows`:
    /// the context keys read it on every keystroke.
    selected_kind: SelectedKind,
    snapshot: WorktreeSnapshot,
    expanded: HashSet<Arc<Path>>,
    rows: Vec<Row>,
    /// When each recently changed path last changed.
    changed: HashMap<Arc<Path>, Instant>,
    edit: Option<EditState>,
}

impl ExplorerModel {
    /// Every directory starts collapsed.
    pub fn new(snapshot: WorktreeSnapshot) -> Self {
        let mut model = ExplorerModel {
            selection: Selection::Root,
            selected_kind: SelectedKind::Root,
            snapshot,
            expanded: HashSet::new(),
            rows: Vec::new(),
            changed: HashMap::new(),
            edit: None,
        };
        model.rebuild();
        model
    }

    /// Replaces the tree. Directories that are still in it stay expanded.
    pub fn set_snapshot(&mut self, snapshot: WorktreeSnapshot) {
        self.snapshot = snapshot;
        let snapshot = &self.snapshot;
        self.expanded.retain(|dir| is_dir(snapshot, dir));
        // A name typed in a folder that is gone has nowhere to go; an
        // entry being renamed that is gone has nothing to rename.
        if self.edit.as_ref().is_some_and(|e| {
            !self.is_dir(&e.parent) || e.original.as_deref().is_some_and(|o| !self.contains(o))
        }) {
            self.edit = None;
        }
        self.rebuild();
        self.fix_selection();
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Selects `sel` as given, even when it is not a visible row yet (e.g.
    /// before `reveal` expands its folders). The next `toggle` or
    /// `set_snapshot` moves a still-hidden selection to its nearest visible
    /// ancestor, else the root.
    pub fn select(&mut self, sel: Selection) {
        self.selection = sel;
        self.update_selected_kind();
    }

    /// A path that is not a visible row reports `File`.
    pub fn selected_kind(&self) -> SelectedKind {
        self.selected_kind
    }

    fn update_selected_kind(&mut self) {
        self.selected_kind = match &self.selection {
            Selection::Root => SelectedKind::Root,
            Selection::Path(path) => match self.rows.iter().find(|row| row.path == *path) {
                Some(row) if row.kind == EntryKind::Dir => SelectedKind::Dir,
                _ => SelectedKind::File,
            },
        };
    }

    fn fix_selection(&mut self) {
        let Selection::Path(path) = &self.selection else {
            return;
        };
        let visible = |p: &Path| self.rows.iter().any(|row| &*row.path == p);
        if visible(path) {
            return;
        }
        let nearest = path
            .ancestors()
            .skip(1)
            .find(|a| !a.as_os_str().is_empty() && visible(a))
            .map(Arc::from);
        self.selection = nearest.map_or(Selection::Root, Selection::Path);
        self.update_selected_kind();
    }

    /// Expands or collapses `dir`; anything that is not a directory is
    /// ignored. Collapsing keeps the expansion of directories inside it.
    pub fn toggle(&mut self, dir: &Path) {
        if !is_dir(&self.snapshot, dir) {
            return;
        }
        if !self.expanded.remove(dir) {
            self.expanded.insert(Arc::from(dir));
        }
        self.rebuild();
        self.fix_selection();
    }

    /// Starts typing a name: expands `edit.parent` (and its ancestors) so
    /// the edited row is visible.
    pub fn start_edit(&mut self, edit: EditState) {
        for dir in edit.parent.ancestors() {
            if !dir.as_os_str().is_empty() && is_dir(&self.snapshot, dir) {
                self.expanded.insert(Arc::from(dir));
            }
        }
        self.edit = Some(edit);
        self.rebuild();
    }

    pub fn edit(&self) -> Option<&EditState> {
        self.edit.as_ref()
    }

    pub fn end_edit(&mut self) {
        if self.edit.take().is_some() {
            self.rebuild();
        }
    }

    /// Whether the tree has `path` (root-relative); the root always.
    pub fn contains(&self, path: &Path) -> bool {
        path.as_os_str().is_empty() || self.snapshot.contains(path)
    }

    /// Whether `path` (root-relative) is a directory of the tree; the root
    /// always.
    pub fn is_dir(&self, path: &Path) -> bool {
        path.as_os_str().is_empty() || is_dir(&self.snapshot, path)
    }

    /// Expands `dir` (no-op when it is not a directory or already expanded).
    pub fn expand(&mut self, dir: &Path) {
        if is_dir(&self.snapshot, dir) && self.expanded.insert(Arc::from(dir)) {
            self.rebuild();
        }
    }

    /// The rows in tree order; while a New File/Folder name is typed, its
    /// placeholder row too.
    pub fn visible_rows(&self) -> &[Row] {
        &self.rows
    }

    /// Expands every ancestor of `path` and returns its row, or `None` when
    /// the tree does not contain it.
    pub fn reveal(&mut self, path: &Path) -> Option<usize> {
        if !self.snapshot.contains(path) {
            return None;
        }
        let mut grew = false;
        for dir in path.ancestors().skip(1) {
            if !dir.as_os_str().is_empty() && !self.expanded.contains(dir) {
                self.expanded.insert(Arc::from(dir));
                grew = true;
            }
        }
        if grew {
            self.rebuild();
        }
        self.rows.iter().position(|row| &*row.path == path)
    }

    /// Records that `paths` changed on disk at `now`.
    pub fn mark_changed(&mut self, paths: &[Arc<Path>], now: Instant) {
        for path in paths {
            self.changed.insert(path.clone(), now);
        }
    }

    /// True for [`CHANGE_DECORATION`] after the last change, or until
    /// [`clear_changed`](Self::clear_changed).
    pub fn is_recently_changed(&self, path: &Path, now: Instant) -> bool {
        self.changed
            .get(path)
            .is_some_and(|&at| now.saturating_duration_since(at) < CHANGE_DECORATION)
    }

    /// Drops the decoration of `path`, e.g. once it is opened.
    pub fn clear_changed(&mut self, path: &Path) {
        self.changed.remove(path);
    }

    /// Forgets decorations that expired by `now` and returns when the next
    /// one expires.
    pub fn next_expiry(&mut self, now: Instant) -> Option<Instant> {
        self.changed
            .retain(|_, &mut at| now.saturating_duration_since(at) < CHANGE_DECORATION);
        self.changed
            .values()
            .map(|&at| at + CHANGE_DECORATION)
            .min()
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::with_capacity(self.rows.len());
        self.push_children(Path::new(""), 0, &mut rows);
        self.rows = rows;
        self.update_selected_kind();
    }

    fn push_children(&self, dir: &Path, depth: u16, rows: &mut Vec<Row>) {
        let new_kind = match &self.edit {
            Some(e) if &*e.parent == dir => match e.kind {
                EditKind::NewFile => Some(EntryKind::File),
                EditKind::NewFolder => Some(EntryKind::Dir),
                EditKind::Rename => None,
            },
            _ => None,
        };
        if let Some(kind) = new_kind {
            rows.push(Row {
                path: Arc::from(dir),
                depth,
                kind,
                expanded: false,
                edit: true,
            });
        }
        let renamed = self.edit.as_ref().and_then(|e| e.original.as_deref());
        for entry in self.snapshot.children(dir) {
            let expanded = entry.kind == EntryKind::Dir && self.expanded.contains(&entry.path);
            rows.push(Row {
                path: entry.path.clone(),
                depth,
                kind: entry.kind,
                expanded,
                edit: renamed == Some(&*entry.path),
            });
            if expanded {
                self.push_children(&entry.path, depth + 1, rows);
            }
        }
    }
}

fn is_dir(snapshot: &WorktreeSnapshot, path: &Path) -> bool {
    let parent = path.parent().unwrap_or(Path::new(""));
    snapshot
        .children(parent)
        .iter()
        .any(|e| &*e.path == path && e.kind == EntryKind::Dir)
}
