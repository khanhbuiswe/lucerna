/// The changed line region between two versions of a file
/// ([`crate::Patch::reload`]).
/// End rows are exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reload {
    pub start_row: u32,
    pub old_end_row: u32,
    pub new_end_row: u32,
}

impl Reload {
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
