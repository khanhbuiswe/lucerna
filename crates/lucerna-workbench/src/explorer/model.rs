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
}

pub struct ExplorerModel {
    snapshot: WorktreeSnapshot,
    expanded: HashSet<Arc<Path>>,
    rows: Vec<Row>,
    /// When each recently changed path last changed.
    changed: HashMap<Arc<Path>, Instant>,
}

impl ExplorerModel {
    /// Every directory starts collapsed.
    pub fn new(snapshot: WorktreeSnapshot) -> Self {
        let mut model = ExplorerModel {
            snapshot,
            expanded: HashSet::new(),
            rows: Vec::new(),
            changed: HashMap::new(),
        };
        model.rebuild();
        model
    }

    /// Replaces the tree. Directories that are still in it stay expanded.
    pub fn set_snapshot(&mut self, snapshot: WorktreeSnapshot) {
        self.snapshot = snapshot;
        let snapshot = &self.snapshot;
        self.expanded.retain(|dir| is_dir(snapshot, dir));
        self.rebuild();
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
    }

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
    }

    fn push_children(&self, dir: &Path, depth: u16, rows: &mut Vec<Row>) {
        for entry in self.snapshot.children(dir) {
            let expanded = entry.kind == EntryKind::Dir && self.expanded.contains(&entry.path);
            rows.push(Row {
                path: entry.path.clone(),
                depth,
                kind: entry.kind,
                expanded,
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
