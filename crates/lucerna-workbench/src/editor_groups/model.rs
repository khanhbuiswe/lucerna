//! The pure editor-groups model: which files (and the Extensions page, the
//! Search, References and Health tabs and language server logs) are open in which group, which tab is a preview, and the most-recently-used
//! order per group.

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

/// What a tab shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TabItem {
    File(Arc<Path>),
    /// The Extensions page (spec §5.2). At most one in all groups; never a
    /// preview.
    Extensions,
    /// Project search. At most one in all groups; never a preview.
    Search,
    /// The log of the language server with this name. At most one per
    /// server in all groups; never a preview, never saved.
    LspLog(Arc<str>),
    /// The References tab (spec §7.5). At most one in all groups; never a
    /// preview, never saved.
    References,
    /// The language server Health tab (`lucerna.lsp.health`). At most one
    /// in all groups; never a preview, never saved.
    LspHealth,
}

impl TabItem {
    /// The Extensions page, the Search, References or Health tab or a
    /// server log: at most one in all groups, never a preview and never
    /// split.
    pub fn is_singleton(&self) -> bool {
        !matches!(self, TabItem::File(_))
    }
}

/// How the Extensions tab is saved in [`GroupState::tabs`], in place of a
/// path. It is relative, so it never names a file a tab opened (those are
/// absolute).
pub const EXTENSIONS_TAB_STATE: &str = "lucerna:extensions";

/// The title of the Extensions tab.
pub const EXTENSIONS_TAB_TITLE: &str = "Extensions";

/// How the Search tab is saved in [`GroupState::tabs`]; relative, like
/// [`EXTENSIONS_TAB_STATE`].
pub const SEARCH_TAB_STATE: &str = "lucerna:search";

/// The title of the Search tab (the view may show its query instead).
pub const SEARCH_TAB_TITLE: &str = "Search";

/// The title of the References tab before a lookup names its word.
pub const REFERENCES_TAB_TITLE: &str = "References";

/// The title of the language server Health tab.
pub const LSP_HEALTH_TAB_TITLE: &str = "Language Servers";

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: TabId,
    pub item: TabItem,
    /// Unpinned tabs are previews: the next preview open replaces them.
    pub pinned: bool,
}

impl Tab {
    /// The file the tab shows; `None` for the Extensions page, the Search,
    /// References and Health tabs and server logs.
    pub fn path(&self) -> Option<&Arc<Path>> {
        match &self.item {
            TabItem::File(path) => Some(path),
            _ => None,
        }
    }

    /// The tab bar's label: the file name, "Extensions", "Search",
    /// "<server> Log", "References" or "Language Servers".
    pub fn title(&self) -> String {
        match &self.item {
            TabItem::File(path) => path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
            TabItem::Extensions => EXTENSIONS_TAB_TITLE.to_string(),
            TabItem::Search => SEARCH_TAB_TITLE.to_string(),
            TabItem::LspLog(server) => format!("{server} Log"),
            TabItem::References => REFERENCES_TAB_TITLE.to_string(),
            TabItem::LspHealth => LSP_HEALTH_TAB_TITLE.to_string(),
        }
    }
}

/// A group's identity for its lifetime: unlike its index, it does not
/// shift when groups to its left are split or closed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct GroupId(u64);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Group {
    pub id: GroupId,
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
    /// `(path, pinned)` in tab order; the Extensions tab is saved as
    /// [`EXTENSIONS_TAB_STATE`], the Search tab as [`SEARCH_TAB_STATE`].
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

impl TabId {
    /// For keying per-tab UI state, such as element ids.
    pub(crate) fn as_u64(self) -> u64 {
        self.0
    }
}

impl EditorGroups {
    /// Opens `path` in the active group (or, for [`OpenMode::ToSide`], the
    /// group to its right), activates it, and returns its tab.
    pub fn open(&mut self, path: Arc<Path>, mode: OpenMode, preview_enabled: bool) -> TabId {
        if mode == OpenMode::ToSide {
            let right = self.active_group + 1;
            if right == self.groups.len() {
                let group = self.new_group();
                self.groups.push(group);
            }
            self.active_group = right;
        }
        let preview = mode == OpenMode::Preview && preview_enabled;
        let id = self.new_id();
        let group = &mut self.groups[self.active_group];

        let item = TabItem::File(path);
        if let Some(tab) = group.tabs.iter_mut().find(|t| t.item == item) {
            let existing = tab.id;
            if mode != OpenMode::Preview {
                tab.pinned = true;
            }
            group.activate(existing);
            return existing;
        }

        let tab = Tab {
            id,
            item,
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

    /// Activates the Extensions tab wherever it is, or opens it (pinned) at
    /// the end of the active group. Returns its tab.
    pub fn open_extensions(&mut self) -> TabId {
        self.open_singleton(TabItem::Extensions)
    }

    /// The Extensions tab, if open in any group.
    pub fn extensions_tab(&self) -> Option<TabId> {
        self.find_item(&TabItem::Extensions)
    }

    /// Activates the Search tab wherever it is, or opens it (pinned) at the
    /// end of the active group. Returns its tab.
    pub fn open_search(&mut self) -> TabId {
        self.open_singleton(TabItem::Search)
    }

    /// The Search tab, if open in any group.
    pub fn search_tab(&self) -> Option<TabId> {
        self.find_item(&TabItem::Search)
    }

    /// Activates the log tab of `server` wherever it is, or opens it
    /// (pinned) at the end of the active group. Returns its tab.
    pub fn open_lsp_log(&mut self, server: Arc<str>) -> TabId {
        self.open_singleton(TabItem::LspLog(server))
    }

    /// Activates the References tab wherever it is, or opens it (pinned)
    /// at the end of the active group.
    pub fn open_references(&mut self) -> TabId {
        self.open_singleton(TabItem::References)
    }

    /// The References tab, if open in any group.
    pub fn references_tab(&self) -> Option<TabId> {
        self.find_item(&TabItem::References)
    }

    /// Activates the Health tab wherever it is, or opens it (pinned) at
    /// the end of the active group. Returns its tab.
    pub fn open_lsp_health(&mut self) -> TabId {
        self.open_singleton(TabItem::LspHealth)
    }

    /// The Health tab, if open in any group.
    pub fn lsp_health_tab(&self) -> Option<TabId> {
        self.find_item(&TabItem::LspHealth)
    }

    fn open_singleton(&mut self, item: TabItem) -> TabId {
        if let Some(id) = self.find_item(&item) {
            self.activate(id);
            return id;
        }
        let id = self.new_id();
        let group = &mut self.groups[self.active_group];
        group.tabs.push(Tab {
            id,
            item,
            pinned: true,
        });
        group.activate(id);
        id
    }

    fn find_item(&self, item: &TabItem) -> Option<TabId> {
        self.groups
            .iter()
            .flat_map(|g| &g.tabs)
            .find(|t| t.item == *item)
            .map(|t| t.id)
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

    /// After `from` was renamed to `to`: every file tab of `from`, or of a
    /// path under it (whole components), shows the moved path. Tab ids,
    /// order, pin and preview state stay. Returns whether a tab changed.
    pub fn retarget(&mut self, from: &Path, to: &Path) -> bool {
        let mut changed = false;
        for tab in self.groups.iter_mut().flat_map(|g| g.tabs.iter_mut()) {
            if let TabItem::File(path) = &mut tab.item
                && let Some(moved) = crate::workspace_state::moved_path(path, from, to)
            {
                *path = Arc::from(moved);
                changed = true;
            }
        }
        changed
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
    /// is active, or when it is the Extensions page or the Search tab,
    /// which are never copied).
    pub fn split_active(&mut self) -> usize {
        let Some(tab) = self
            .active_tab()
            .filter(|t| !t.item.is_singleton())
            .cloned()
        else {
            return self.active_group;
        };
        let copy = Tab {
            id: self.new_id(),
            ..tab
        };
        let mut group = self.new_group();
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

    /// The id of the active group.
    pub fn active_group_id(&self) -> GroupId {
        self.groups[self.active_group].id
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

    /// Rebuilds groups from persisted state with fresh tab ids, repairing
    /// what a stale or hand-edited file may contain:
    /// - duplicate paths in a group keep their first entry;
    /// - only the first Extensions tab and the first Search tab of all
    ///   groups are kept, pinned;
    /// - of several unpinned tabs only the last stays a preview;
    /// - a missing or out-of-range active tab becomes the group's last tab;
    /// - empty groups other than the first are dropped, and an active group
    ///   that was dropped falls back to its left neighbour.
    pub fn restore(state: &GroupsState) -> Self {
        let mut model = EditorGroups {
            groups: Vec::new(),
            active_group: 0,
            next_id: 0,
        };
        // Restored index of each saved group; `None` when it was dropped.
        let mut restored_index = Vec::with_capacity(state.groups.len());
        // Singletons already restored in earlier groups.
        let mut singletons: Vec<TabItem> = Vec::new();
        for saved in &state.groups {
            let mut group = model.new_group();
            for (path, pinned) in &saved.tabs {
                let item = saved_item(path);
                let seen = singletons.contains(&item) || group.tabs.iter().any(|t| t.item == item);
                if seen {
                    continue;
                }
                let pinned = *pinned || item.is_singleton();
                group.tabs.push(Tab {
                    id: model.new_id(),
                    item,
                    pinned,
                });
            }
            singletons.extend(
                group
                    .tabs
                    .iter()
                    .filter(|t| t.item.is_singleton())
                    .map(|t| t.item.clone()),
            );
            if let Some(preview) = group.tabs.iter().rposition(|t| !t.pinned) {
                for tab in &mut group.tabs[..preview] {
                    tab.pinned = true;
                }
            }
            // MRU history is not persisted: tab order, active tab first.
            group.mru = group.tabs.iter().map(|t| t.id).collect();
            let saved_active = saved
                .active
                .and_then(|i| saved.tabs.get(i))
                .map(|(path, _)| saved_item(path))
                .and_then(|item| group.tabs.iter().find(|t| t.item == item));
            if let Some(id) = saved_active.or(group.tabs.last()).map(|t| t.id) {
                group.activate(id);
            }
            if model.groups.is_empty() || !group.tabs.is_empty() {
                restored_index.push(Some(model.groups.len()));
                model.groups.push(group);
            } else {
                restored_index.push(None);
            }
        }
        if model.groups.is_empty() {
            let group = model.new_group();
            model.groups.push(group);
        }
        let last = model.groups.len() - 1;
        model.active_group = match restored_index.get(state.active_group) {
            Some(Some(index)) => *index,
            // Dropped: the nearest kept group to its left (the first group
            // is always kept).
            Some(None) => restored_index[..state.active_group]
                .iter()
                .rev()
                .find_map(|i| *i)
                .unwrap_or(0),
            None => last,
        };
        model
    }

    /// The layout to save. Server log and References tabs are left out; a group whose
    /// active tab is one saves no active tab.
    pub fn state(&self) -> GroupsState {
        GroupsState {
            groups: self
                .groups
                .iter()
                .map(|g| {
                    let saved: Vec<(&Tab, PathBuf)> = g
                        .tabs
                        .iter()
                        .filter_map(|t| Some((t, saved_path(&t.item)?)))
                        .collect();
                    GroupState {
                        active: saved.iter().position(|(t, _)| g.active == Some(t.id)),
                        tabs: saved
                            .into_iter()
                            .map(|(t, path)| (path, t.pinned))
                            .collect(),
                    }
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

    /// An empty group with a fresh id (ids share the tab counter; the
    /// first group of a new model keeps id 0).
    fn new_group(&mut self) -> Group {
        self.next_id += 1;
        Group {
            id: GroupId(self.next_id),
            ..Group::default()
        }
    }
}

/// What a saved tab path stands for.
fn saved_item(path: &Path) -> TabItem {
    if path == Path::new(EXTENSIONS_TAB_STATE) {
        TabItem::Extensions
    } else if path == Path::new(SEARCH_TAB_STATE) {
        TabItem::Search
    } else {
        TabItem::File(Arc::from(path))
    }
}

/// How a tab is saved: its path, [`EXTENSIONS_TAB_STATE`] or
/// [`SEARCH_TAB_STATE`]; `None` for a server log, the References tab or
/// the Health tab, which are not saved.
fn saved_path(item: &TabItem) -> Option<PathBuf> {
    match item {
        TabItem::File(path) => Some(path.to_path_buf()),
        TabItem::Extensions => Some(PathBuf::from(EXTENSIONS_TAB_STATE)),
        TabItem::Search => Some(PathBuf::from(SEARCH_TAB_STATE)),
        TabItem::LspLog(_) | TabItem::References | TabItem::LspHealth => None,
    }
}
