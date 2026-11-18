//! The tabs the user closed, newest last, for Reopen Closed Editor
//! (`workbench.action.reopenClosedEditor`).

use std::collections::VecDeque;
use std::path::Path;

use lucerna_text::Point;

use super::model::{GroupId, TabId, TabItem};

/// The most closed tabs remembered, as in VS Code.
pub(crate) const CAPACITY: usize = 20;

/// Where a closed file tab's editor was: cursor and topmost visible row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabViewState {
    pub cursor: Point,
    pub scroll_top_row: u32,
}

/// A tab the user closed.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedTab {
    pub item: TabItem,
    /// The group it was in.
    pub group: GroupId,
    /// Its place in that group before the close that took it (before the
    /// first close of its batch).
    pub index: usize,
    /// Whether it was its group's active tab.
    pub was_active: bool,
    /// Tabs closed by one command share a batch and reopen together.
    pub batch: u64,
    /// Its id while it was open.
    pub tab: TabId,
    pub view: Option<TabViewState>,
}

/// Whether closing a tab showing `item` is remembered: files and static
/// Markdown previews, not Untitled ones.
pub(crate) fn is_recorded(item: &TabItem) -> bool {
    match item {
        TabItem::File(path) | TabItem::MarkdownPreview(path) => {
            !crate::untitled::is_untitled(path)
        }
        _ => false,
    }
}

/// The path a reopen checks for existence, if `item` has one.
pub(crate) fn item_path(item: &TabItem) -> Option<&Path> {
    match item {
        TabItem::File(path) | TabItem::MarkdownPreview(path) => Some(path),
        _ => None,
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ClosedHistory {
    entries: VecDeque<ClosedTab>,
    next_batch: u64,
}

impl ClosedHistory {
    pub(crate) fn entries(&self) -> &VecDeque<ClosedTab> {
        &self.entries
    }

    /// A batch id for the tabs of one close.
    pub(crate) fn new_batch(&mut self) -> u64 {
        self.next_batch += 1;
        self.next_batch
    }

    /// Remembers `entry` unless its item is not recorded; an older entry
    /// for the same item in the same group goes first.
    pub(crate) fn record(&mut self, entry: ClosedTab) {
        if !is_recorded(&entry.item) {
            return;
        }
        self.entries
            .retain(|e| !(e.item == entry.item && e.group == entry.group));
        self.entries.push_back(entry);
        while self.entries.len() > CAPACITY {
            self.entries.pop_front();
        }
    }

    /// Takes the newest batch out, oldest entry first.
    pub(crate) fn pop_batch(&mut self) -> Vec<ClosedTab> {
        let Some(batch) = self.entries.back().map(|e| e.batch) else {
            return Vec::new();
        };
        let mut taken = Vec::new();
        while self.entries.back().is_some_and(|e| e.batch == batch) {
            taken.extend(self.entries.pop_back());
        }
        taken.reverse();
        taken
    }

    pub(crate) fn set_view(&mut self, tab: TabId, view: TabViewState) {
        if let Some(entry) = self.entries.iter_mut().rev().find(|e| e.tab == tab) {
            entry.view = Some(view);
        }
    }

    /// After `from` was renamed to `to`: entries in or under it follow.
    pub(crate) fn retarget(&mut self, from: &Path, to: &Path) {
        for entry in &mut self.entries {
            if let TabItem::File(path) | TabItem::MarkdownPreview(path) = &mut entry.item
                && let Some(moved) = crate::workspace_state::moved_path(path, from, to)
            {
                *path = std::sync::Arc::from(moved);
            }
        }
    }
}
