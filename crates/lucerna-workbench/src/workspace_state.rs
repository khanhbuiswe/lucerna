//! What reopening a folder brings back: sidebar, split groups, open tabs
//! and each tab's scroll position, kept as JSON per folder under the data
//! directory.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::editor_groups::GroupsState;

/// The file format version; a file of any other version is set aside.
pub(crate) const STATE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct WorkspaceState {
    pub version: u32,
    pub sidebar_visible: bool,
    pub sidebar_width: f32,
    /// `"explorer"` or `"search"`.
    pub sidebar_view: String,
    pub groups: GroupsState,
    /// Top visible row of each open tab's file, by absolute path.
    pub scroll: HashMap<PathBuf, u32>,
}

impl WorkspaceState {
    /// Drops tabs (and scroll rows) of files that no longer exist, keeping
    /// each group's active tab where it still exists. Empty groups are
    /// left for [`crate::EditorGroups::restore`] to normalise.
    pub fn without_missing_files(mut self) -> Self {
        for group in &mut self.groups.groups {
            let active_path = group
                .active
                .and_then(|i| group.tabs.get(i))
                .map(|(path, _)| path.clone());
            group.tabs.retain(|(path, _)| path.exists());
            group.active = active_path.and_then(|a| group.tabs.iter().position(|(p, _)| *p == a));
        }
        self.scroll.retain(|path, _| path.exists());
        self
    }
}

/// `<data_dir>/workspaces/<hex sha256 of the canonical root>.json`. A root
/// that cannot be canonicalised (say, deleted) is hashed as given.
pub fn state_path(data_dir: &Path, root: &Path) -> PathBuf {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let digest = Sha256::digest(root.as_os_str().as_encoded_bytes());
    let mut name = String::with_capacity(digest.len() * 2 + 5);
    for byte in digest {
        let _ = write!(name, "{byte:02x}");
    }
    name.push_str(".json");
    data_dir.join("workspaces").join(name)
}

/// Reads the saved state. A missing file is `None`; a file that does not
/// parse or has another version is renamed to `<name>.json.bak`
/// (replacing an older one) and is `None` too.
pub fn load_state(path: &Path) -> Option<WorkspaceState> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
        Err(err) => {
            tracing::warn!(%err, path = %path.display(), "could not read workspace state");
            return None;
        }
    };
    match serde_json::from_str::<WorkspaceState>(&text) {
        Ok(state) if state.version == STATE_VERSION => Some(state),
        _ => {
            let mut bak = path.as_os_str().to_owned();
            bak.push(".bak");
            if let Err(err) = std::fs::rename(path, &bak) {
                tracing::warn!(%err, path = %path.display(), "could not set aside workspace state");
            }
            None
        }
    }
}

/// Writes `s` to a temporary file next to `path`, then renames it over
/// `path`, so a crash never leaves half a file.
pub fn save_state(path: &Path, s: &WorkspaceState) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(s).map_err(io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Where a workbench keeps its state: the app data directory and the
/// folder it shows.
#[derive(Debug, Clone)]
pub struct WorkspaceStateStore {
    data_dir: PathBuf,
    root: PathBuf,
}

impl WorkspaceStateStore {
    pub fn new(data_dir: PathBuf, root: PathBuf) -> Self {
        WorkspaceStateStore { data_dir, root }
    }

    pub fn path(&self) -> PathBuf {
        state_path(&self.data_dir, &self.root)
    }

    pub fn load(&self) -> Option<WorkspaceState> {
        load_state(&self.path())
    }

    pub fn save(&self, s: &WorkspaceState) -> io::Result<()> {
        save_state(&self.path(), s)
    }
}

/// The app's data directory, `~/Library/Application Support/Lucerna`;
/// `None` when `HOME` is unset.
pub fn default_data_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Lucerna"),
    )
}
