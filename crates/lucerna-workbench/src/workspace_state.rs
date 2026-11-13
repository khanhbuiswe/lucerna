//! What reopening a folder brings back: sidebar, split groups, open tabs
//! and each tab's scroll position, kept as JSON per folder under the data
//! directory.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::editor_groups::{EXTENSIONS_TAB_STATE, GroupsState, SEARCH_TAB_STATE};
use crate::layout::panel::PanelState;

/// The file format version; a file of any other version is set aside.
pub(crate) const STATE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct WorkspaceState {
    pub version: u32,
    pub sidebar_visible: bool,
    pub sidebar_width: f32,
    /// `"explorer"`; anything else (such as `"extensions"` or `"search"`
    /// from before those panels moved into tabs) restores the Explorer.
    pub sidebar_view: String,
    pub groups: GroupsState,
    /// Top visible row of each open tab's file, by absolute path.
    pub scroll: HashMap<PathBuf, u32>,
    /// Fraction of the editor area each group takes, left to right. Empty
    /// in files from before it was saved; widths that do not fit the
    /// restored groups are replaced by equal ones.
    #[serde(default)]
    pub group_widths: Vec<f32>,
    /// The Search tab's inputs and toggles; the default in files from
    /// before they were saved.
    #[serde(default)]
    pub search: SearchState,
    /// The bottom / right panel; the default (hidden, at the bottom) in
    /// files from before it was saved.
    #[serde(default)]
    pub panel: PanelState,
}

/// What the Search tab keeps across restarts: the query, its toggles, the
/// include / exclude globs and whether their row is shown.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub struct SearchState {
    pub query: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub is_regex: bool,
    pub include: String,
    pub exclude: String,
    pub filters_visible: bool,
}

impl WorkspaceState {
    /// Makes every path absolute (relative ones, from files written before
    /// tab paths were normalised, are joined onto `root`), then drops tabs
    /// (and scroll rows) of files that no longer exist, keeping each
    /// group's active tab where it still exists. The Extensions and Search
    /// tabs ([`EXTENSIONS_TAB_STATE`], [`SEARCH_TAB_STATE`]) are kept as
    /// they are. Empty groups are left
    /// for [`crate::EditorGroups::restore`] to normalise.
    pub fn without_missing_files(mut self, root: &Path) -> Self {
        let special = |path: &Path| {
            path == Path::new(EXTENSIONS_TAB_STATE) || path == Path::new(SEARCH_TAB_STATE)
        };
        for group in &mut self.groups.groups {
            for (path, _) in &mut group.tabs {
                if !special(path) {
                    *path = absolute_path(root, path);
                }
            }
            let active_path = group
                .active
                .and_then(|i| group.tabs.get(i))
                .map(|(path, _)| path.clone());
            group
                .tabs
                .retain(|(path, _)| special(path) || path.exists());
            group.active = active_path.and_then(|a| group.tabs.iter().position(|(p, _)| *p == a));
        }
        self.scroll = std::mem::take(&mut self.scroll)
            .into_iter()
            .map(|(path, row)| (absolute_path(root, &path), row))
            .filter(|(path, _)| path.exists())
            .collect();
        self
    }
}

/// `path` as given when absolute, else joined onto `root`. Every tab path,
/// recent file and saved path goes through this, so one file has one name
/// whatever the working directory.
pub(crate) fn absolute_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
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
    let json = serde_json::to_vec_pretty(s).map_err(io::Error::other)?;
    write_atomic(path, &json)
}

/// Writes `bytes` to [`tmp_path`], then renames it over `path` (creating
/// the parent directory first), so a crash never leaves half a file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = tmp_path(path);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// The temporary file [`write_atomic`] writes before renaming it over
/// `path`: `<path>.<pid>.tmp`, so two processes writing at once do not
/// share one.
pub(crate) fn tmp_path(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(tmp)
}

/// `path` with the `home` folder written as `~`; the absolute path
/// otherwise.
pub(crate) fn tilde_path(path: &Path, home: Option<&Path>) -> String {
    if let Some(rel) = home.and_then(|home| path.strip_prefix(home).ok()) {
        if rel.as_os_str().is_empty() {
            return "~".to_string();
        }
        return Path::new("~").join(rel).display().to_string();
    }
    path.display().to_string()
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

    /// The app data directory it keeps the state in.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
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
