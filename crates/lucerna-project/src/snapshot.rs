use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Path relative to the worktree root.
    pub path: Arc<Path>,
    pub kind: EntryKind,
}

/// Immutable view of the scanned tree. Cloning is cheap.
#[derive(Debug, Clone)]
pub struct WorktreeSnapshot {
    /// Depth-first order, directories before files at each level.
    entries: Arc<Vec<Entry>>,
    /// The same entries grouped by parent directory, so each directory's
    /// children are one contiguous slice.
    by_parent: Arc<Vec<Entry>>,
    index: Arc<HashMap<Arc<Path>, Range<usize>>>,
    complete: bool,
}

impl WorktreeSnapshot {
    pub(crate) fn empty() -> Self {
        Self::build(Vec::new(), false)
    }

    pub(crate) fn build(mut entries: Vec<Entry>, complete: bool) -> Self {
        entries.sort_by(compare_entries);
        let mut by_parent: Vec<Entry> = entries.clone();
        // Stable sort keeps the sibling order established above.
        by_parent.sort_by(|a, b| a.path.parent().cmp(&b.path.parent()));
        let mut index: HashMap<Arc<Path>, Range<usize>> = HashMap::new();
        let mut start = 0;
        while start < by_parent.len() {
            let parent = by_parent[start].path.parent().unwrap_or(Path::new(""));
            let mut end = start + 1;
            while end < by_parent.len()
                && by_parent[end].path.parent().unwrap_or(Path::new("")) == parent
            {
                end += 1;
            }
            index.insert(Arc::from(parent), start..end);
            start = end;
        }
        Self {
            entries: Arc::new(entries),
            by_parent: Arc::new(by_parent),
            index: Arc::new(index),
            complete,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn files(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.kind != EntryKind::Dir)
    }

    /// Children of a root-relative directory; `Path::new("")` is the root.
    pub fn children(&self, dir: &Path) -> &[Entry] {
        match self.index.get(dir) {
            Some(range) => &self.by_parent[range.clone()],
            None => &[],
        }
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.children(path.parent().unwrap_or(Path::new("")))
            .iter()
            .any(|e| &*e.path == path)
    }

    pub fn scan_complete(&self) -> bool {
        self.complete
    }
}

/// Depth-first order: at the first differing component, directories come
/// before files, then names compare case-insensitively. Allocation-free.
fn compare_entries(a: &Entry, b: &Entry) -> Ordering {
    let mut ai = a.path.components();
    let mut bi = b.path.components();
    loop {
        let (ca, cb) = match (ai.next(), bi.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => (ca, cb),
        };
        if ca == cb {
            continue;
        }
        let a_dir = ai.clone().next().is_some() || a.kind == EntryKind::Dir;
        let b_dir = bi.clone().next().is_some() || b.kind == EntryKind::Dir;
        let (an, bn) = (
            ca.as_os_str().to_string_lossy(),
            cb.as_os_str().to_string_lossy(),
        );
        return b_dir
            .cmp(&a_dir)
            .then_with(|| an.to_lowercase().cmp(&bn.to_lowercase()))
            .then_with(|| an.cmp(&bn));
    }
}
