//! The pure editor-groups model: which files are open in which group, which
//! tab is a preview, and the most-recently-used order per group.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// How [`EditorGroups::open`] places a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    /// Reuse the group's preview (unpinned) tab when previews are enabled.
    Preview,
    /// Open as a pinned tab.
    Pinned,
    /// Open pinned in the next group to the right, creating it if needed.
    ToSide,
}

/// Identifies a tab for its whole life; never reused within a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(u64);

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: TabId,
    pub path: Arc<Path>,
    /// Unpinned tabs are previews: the next preview open replaces them.
    pub pinned: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Group {
    pub tabs: Vec<Tab>,
    pub active: Option<TabId>,
    /// Every tab in the group, most recently active first.
    pub mru: Vec<TabId>,
}

impl Group {
    fn position(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn activate(&mut self, id: TabId) {
        self.active = Some(id);
        self.mru.retain(|&m| m != id);
        self.mru.insert(0, id);
    }
}

/// Persisted layout of the editor area (see [`EditorGroups::state`]).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct GroupsState {
    pub groups: Vec<GroupState>,
    pub active_group: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct GroupState {
    /// `(path, pinned)` in tab order.
    pub tabs: Vec<(PathBuf, bool)>,
    /// Index into `tabs`.
    pub active: Option<usize>,
}

/// Editor groups laid out left to right. There is always at least one
/// group; only the first may be empty.
#[derive(Debug, Clone)]
pub struct EditorGroups {
    groups: Vec<Group>,
    active_group: usize,
    next_id: u64,
}

impl Default for EditorGroups {
    fn default() -> Self {
        EditorGroups {
            groups: vec![Group::default()],
            active_group: 0,
            next_id: 0,
        }
    }
}

impl EditorGroups {
    /// Opens `path` in the active group (or, for [`OpenMode::ToSide`], the
    /// group to its right), activates it, and returns its tab.
    pub fn open(&mut self, path: Arc<Path>, mode: OpenMode, preview_enabled: bool) -> TabId {
        if mode == OpenMode::ToSide {
            let right = self.active_group + 1;
            if right == self.groups.len() {
                self.groups.push(Group::default());
            }
            self.active_group = right;
        }
        let preview = mode == OpenMode::Preview && preview_enabled;
        let id = self.new_id();
        let group = &mut self.groups[self.active_group];

        if let Some(tab) = group.tabs.iter_mut().find(|t| t.path == path) {
            let existing = tab.id;
            if mode != OpenMode::Preview {
                tab.pinned = true;
            }
            group.activate(existing);
            return existing;
        }

        let tab = Tab {
            id,
            path,
            pinned: !preview,
        };
        match group
            .tabs
            .iter()
            .position(|t| !t.pinned)
            .filter(|_| preview)
        {
            Some(i) => {
                let old = std::mem::replace(&mut group.tabs[i], tab).id;
                group.mru.retain(|&m| m != old);
            }
            None => group.tabs.push(tab),
        }
        group.activate(id);
        id
    }

    /// Pins tab `id` so preview opens no longer replace it.
    pub fn pin(&mut self, id: TabId) {
        if let Some(tab) = self
            .groups
            .iter_mut()
            .flat_map(|g| g.tabs.iter_mut())
            .find(|t| t.id == id)
        {
            tab.pinned = true;
        }
    }

    /// Makes `id` the active tab and its group the active group.
    /// Returns `false` when no tab has that id.
    pub fn activate(&mut self, id: TabId) -> bool {
        let Some(g) = self.group_of(id) else {
            return false;
        };
        self.active_group = g;
        self.groups[g].activate(id);
        true
    }

    /// Closes tab `id`. Its group then shows its most recently used
    /// remaining tab; an emptied group other than the first is removed.
    /// Returns the active tab afterwards.
    pub fn close(&mut self, id: TabId) -> Option<TabId> {
        if let Some(g) = self.group_of(id) {
            let group = &mut self.groups[g];
            if let Some(i) = group.position(id) {
                group.tabs.remove(i);
            }
            group.mru.retain(|&m| m != id);
            if group.active == Some(id) {
                group.active = group.mru.first().copied();
            }
            if group.tabs.is_empty() && g > 0 {
                self.groups.remove(g);
                if self.active_group >= g {
                    self.active_group -= 1;
                }
            }
        }
        self.active_tab().map(|t| t.id)
    }

    /// Opens a copy of the active tab in a new group directly to the right
    /// and focuses it. Returns the active group index (unchanged when no tab
    /// is active).
    pub fn split_active(&mut self) -> usize {
        let Some(tab) = self.active_tab().cloned() else {
            return self.active_group;
        };
        let copy = Tab {
            id: self.new_id(),
            ..tab
        };
        let mut group = Group::default();
        group.activate(copy.id);
        group.tabs.push(copy);
        self.active_group += 1;
        self.groups.insert(self.active_group, group);
        self.active_group
    }

    /// Focuses group `index`. Returns `false` when it does not exist.
    pub fn focus_group(&mut self, index: usize) -> bool {
        if index >= self.groups.len() {
            return false;
        }
        self.active_group = index;
        true
    }

    pub fn active_group(&self) -> usize {
        self.active_group
    }

    /// The active tab of the active group.
    pub fn active_tab(&self) -> Option<&Tab> {
        let group = &self.groups[self.active_group];
        let id = group.active?;
        group.tabs.iter().find(|t| t.id == id)
    }

    /// Activates the previously used tab of the active group, so repeated
    /// calls alternate between the two most recent tabs. Returns it, or
    /// `None` when the group has fewer than two tabs.
    pub fn cycle_mru(&mut self) -> Option<TabId> {
        let group = &mut self.groups[self.active_group];
        let previous = *group.mru.get(1)?;
        group.activate(previous);
        Some(previous)
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// Rebuilds groups from persisted state with fresh tab ids. Indices out
    /// of range are clamped; an emptied state gives one empty group.
    pub fn restore(state: &GroupsState) -> Self {
        let mut model = EditorGroups {
            groups: Vec::new(),
            active_group: 0,
            next_id: 0,
        };
        for saved in &state.groups {
            let mut group = Group::default();
            for (path, pinned) in &saved.tabs {
                group.tabs.push(Tab {
                    id: model.new_id(),
                    path: Arc::from(path.as_path()),
                    pinned: *pinned,
                });
            }
            // MRU history is not persisted: tab order, active tab first.
            group.mru = group.tabs.iter().map(|t| t.id).collect();
            let active = saved
                .active
                .map(|i| i.min(group.tabs.len().saturating_sub(1)))
                .and_then(|i| group.tabs.get(i))
                .map(|t| t.id);
            if let Some(id) = active {
                group.activate(id);
            }
            if model.groups.is_empty() || !group.tabs.is_empty() {
                model.groups.push(group);
            }
        }
        if model.groups.is_empty() {
            model.groups.push(Group::default());
        }
        model.active_group = state.active_group.min(model.groups.len() - 1);
        model
    }

    pub fn state(&self) -> GroupsState {
        GroupsState {
            groups: self
                .groups
                .iter()
                .map(|g| GroupState {
                    tabs: g
                        .tabs
                        .iter()
                        .map(|t| (t.path.to_path_buf(), t.pinned))
                        .collect(),
                    active: g.active.and_then(|id| g.position(id)),
                })
                .collect(),
            active_group: self.active_group,
        }
    }

    fn group_of(&self, id: TabId) -> Option<usize> {
        self.groups.iter().position(|g| g.position(id).is_some())
    }

    fn new_id(&mut self) -> TabId {
        self.next_id += 1;
        TabId(self.next_id)
    }
}
