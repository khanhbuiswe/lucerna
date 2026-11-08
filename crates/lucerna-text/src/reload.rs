use crate::buffer::BufferSnapshot;

/// The changed line region between two versions of a file.
/// End rows are exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reload {
    pub start_row: u32,
    pub old_end_row: u32,
    pub new_end_row: u32,
}

impl Reload {
    /// Diffs by lines: the common prefix gives `start_row`, the common suffix
    /// gives the end rows.
    pub(crate) fn diff(old: &BufferSnapshot, new: &BufferSnapshot) -> Reload {
        let (old_n, new_n) = (old.line_count(), new.line_count());
        let max_prefix = old_n.min(new_n);
        let mut prefix = 0;
        while prefix < max_prefix && old.line(prefix) == new.line(prefix) {
            prefix += 1;
        }
        let max_suffix = max_prefix - prefix;
        let mut suffix = 0;
        while suffix < max_suffix && old.line(old_n - 1 - suffix) == new.line(new_n - 1 - suffix) {
            suffix += 1;
        }
        Reload {
            start_row: prefix,
            old_end_row: old_n - suffix,
            new_end_row: new_n - suffix,
        }
    }

    /// Maps a row of the old text to the matching row of the new text.
    pub fn adjust_row(&self, row: u32, new_line_count: u32) -> u32 {
        let mapped = if row < self.start_row {
            row
        } else if row >= self.old_end_row {
            row - self.old_end_row + self.new_end_row
        } else {
            self.start_row
        };
        mapped.min(new_line_count.saturating_sub(1))
    }
}
