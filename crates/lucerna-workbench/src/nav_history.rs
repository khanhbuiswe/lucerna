//! Go Back / Go Forward: where the cursor was before each jump, per
//! editor group.

use std::path::Path;
use std::sync::Arc;

use lucerna_text::Point;

/// The most places one group remembers.
const CAPACITY: usize = 50;
/// A jump within one file that moves fewer rows than this is not worth
/// going back to.
const SAME_FILE_ROWS: u32 = 10;

/// A place in a file: cursor and the topmost visible row.
#[derive(Clone, Debug, PartialEq)]
pub struct NavEntry {
    pub path: Arc<Path>,
    pub point: Point,
    pub scroll_top_row: u32,
}

/// The back and forward stacks of one editor group.
#[derive(Debug, Default)]
pub struct NavHistory {
    back: Vec<NavEntry>,
    forward: Vec<NavEntry>,
}

impl NavHistory {
    /// Remembers `from` before a jump to `to`, and forgets what was
    /// forward. A jump inside one file by fewer than 10 rows is not
    /// recorded.
    pub fn record(&mut self, from: NavEntry, to: &NavEntry) {
        if from.path == to.path && from.point.row.abs_diff(to.point.row) < SAME_FILE_ROWS {
            return;
        }
        self.forward.clear();
        push_capped(&mut self.back, from);
    }

    /// The place to go back to, keeping `current` to come forward to.
    pub fn back(&mut self, current: NavEntry) -> Option<NavEntry> {
        self.back_where(current, |_| true)
    }

    /// Like [`NavHistory::back`], dropping entries `keep` rejects (files
    /// that no longer exist) on the way.
    pub fn back_where(
        &mut self,
        current: NavEntry,
        keep: impl FnMut(&NavEntry) -> bool,
    ) -> Option<NavEntry> {
        let target = pop_kept(&mut self.back, keep)?;
        push_capped(&mut self.forward, current);
        Some(target)
    }

    /// The place to go forward to, keeping `current` to come back to.
    pub fn forward(&mut self, current: NavEntry) -> Option<NavEntry> {
        self.forward_where(current, |_| true)
    }

    /// Like [`NavHistory::forward`], dropping entries `keep` rejects.
    pub fn forward_where(
        &mut self,
        current: NavEntry,
        keep: impl FnMut(&NavEntry) -> bool,
    ) -> Option<NavEntry> {
        let target = pop_kept(&mut self.forward, keep)?;
        push_capped(&mut self.back, current);
        Some(target)
    }
}

impl NavHistory {
    /// After `from` was renamed to `to`: entries in or under `from` point
    /// at the moved path.
    pub fn retarget(&mut self, from: &Path, to: &Path) {
        for entry in self.back.iter_mut().chain(self.forward.iter_mut()) {
            if let Some(moved) = crate::workspace_state::moved_path(&entry.path, from, to) {
                entry.path = Arc::from(moved);
            }
        }
    }
}

/// Pops the newest entry `keep` accepts, discarding rejected ones above it.
fn pop_kept(
    stack: &mut Vec<NavEntry>,
    mut keep: impl FnMut(&NavEntry) -> bool,
) -> Option<NavEntry> {
    while let Some(entry) = stack.pop() {
        if keep(&entry) {
            return Some(entry);
        }
    }
    None
}

fn push_capped(stack: &mut Vec<NavEntry>, entry: NavEntry) {
    stack.push(entry);
    if stack.len() > CAPACITY {
        stack.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(path: &str, row: u32) -> NavEntry {
        NavEntry {
            path: Arc::from(Path::new(path)),
            point: Point { row, column: 0 },
            scroll_top_row: row.saturating_sub(5),
        }
    }

    #[test]
    fn retarget_follows_a_renamed_folder() {
        let mut h = NavHistory::default();
        h.record(at("/r/src/a", 0), &at("/r/srcx", 0));
        h.retarget(Path::new("/r/src"), Path::new("/r/lib"));
        assert_eq!(h.back(at("/r/srcx", 0)), Some(at("/r/lib/a", 0)));
        h.retarget(Path::new("/r/srcx"), Path::new("/r/y"));
        assert_eq!(h.forward(at("/r/lib/a", 0)), Some(at("/r/y", 0)));
    }

    #[test]
    fn record_skips_small_same_file_moves() {
        let mut h = NavHistory::default();
        h.record(at("a", 0), &at("a", 9));
        assert_eq!(h.back(at("a", 9)), None, "9 rows is below the threshold");
        h.record(at("a", 0), &at("a", 10));
        assert_eq!(h.back(at("a", 10)), Some(at("a", 0)));
        // Another file always counts, however close the rows.
        h.record(at("a", 3), &at("b", 3));
        assert_eq!(h.back(at("b", 3)), Some(at("a", 3)));
    }

    #[test]
    fn record_clears_forward() {
        let mut h = NavHistory::default();
        h.record(at("a", 0), &at("b", 0));
        assert_eq!(h.back(at("b", 0)), Some(at("a", 0)));
        h.record(at("a", 0), &at("c", 0));
        assert_eq!(h.forward(at("c", 0)), None);
        assert_eq!(h.back(at("c", 0)), Some(at("a", 0)));
        assert_eq!(h.forward(at("a", 0)), Some(at("c", 0)));
    }

    #[test]
    fn back_and_forward_walk_the_stacks() {
        let mut h = NavHistory::default();
        h.record(at("a", 0), &at("b", 0));
        h.record(at("b", 0), &at("c", 0));
        assert_eq!(h.back(at("c", 0)), Some(at("b", 0)));
        assert_eq!(h.back(at("b", 0)), Some(at("a", 0)));
        assert_eq!(h.back(at("a", 0)), None);
        assert_eq!(h.forward(at("a", 0)), Some(at("b", 0)));
        assert_eq!(h.forward(at("b", 0)), Some(at("c", 0)));
        assert_eq!(h.forward(at("c", 0)), None);
    }

    #[test]
    fn cap_50() {
        let mut h = NavHistory::default();
        for i in 0..60u32 {
            h.record(at(&format!("f{i}"), 0), &at(&format!("f{}", i + 1), 0));
        }
        let mut current = at("f60", 0);
        let mut seen = 0;
        while let Some(prev) = h.back(current.clone()) {
            seen += 1;
            current = prev;
        }
        assert_eq!(seen, 50);
        assert_eq!(current, at("f10", 0), "the oldest ten were dropped");
    }

    #[test]
    fn rejected_entries_are_dropped_without_duplicating_current() {
        let mut h = NavHistory::default();
        h.record(at("a", 0), &at("gone", 0));
        h.record(at("gone", 0), &at("c", 0));
        let back = h.back_where(at("c", 0), |e| &*e.path != Path::new("gone"));
        assert_eq!(back, Some(at("a", 0)));
        assert_eq!(h.back(at("a", 0)), None);
        assert_eq!(h.forward(at("a", 0)), Some(at("c", 0)));
        assert_eq!(h.forward(at("c", 0)), None, "only one forward entry");
    }
}
