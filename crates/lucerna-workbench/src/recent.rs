//! The folders (and files) opened lately, newest first, kept as JSON in
//! the app data directory for the Welcome screen and Open Recent.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::workspace_state::{tilde_path, write_atomic};

/// The file format version; a file of any other version is set aside.
const RECENT_VERSION: u32 = 1;

/// How many entries the list keeps.
pub const RECENT_LIMIT: usize = 100;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RecentKind {
    Folder,
    File,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub kind: RecentKind,
}

impl RecentEntry {
    /// Whether the path still exists as the kind of thing it was saved as.
    fn exists(&self) -> bool {
        match self.kind {
            RecentKind::Folder => self.path.is_dir(),
            RecentKind::File => self.path.is_file(),
        }
    }
}

/// The recent entries, newest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecentList {
    pub entries: Vec<RecentEntry>,
}

#[derive(Serialize, Deserialize)]
struct RecentFile {
    version: u32,
    entries: Vec<RecentEntry>,
}

/// Where the app data directory keeps the list: `<data_dir>/recent.json`.
pub fn recent_path(data_dir: &Path) -> PathBuf {
    data_dir.join("recent.json")
}

/// Reads the list. A missing file is an empty list; a file that does not
/// parse or has another version is renamed to `recent.json.bak` (replacing
/// an older one) and is an empty list too.
pub fn load(path: &Path) -> RecentList {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return RecentList::default(),
        Err(err) => {
            tracing::warn!(%err, path = %path.display(), "could not read recent list");
            return RecentList::default();
        }
    };
    match serde_json::from_str::<RecentFile>(&text) {
        Ok(file) if file.version == RECENT_VERSION => RecentList {
            entries: file.entries,
        },
        _ => {
            let mut bak = path.as_os_str().to_owned();
            bak.push(".bak");
            if let Err(err) = std::fs::rename(path, &bak) {
                tracing::warn!(%err, path = %path.display(), "could not set aside recent list");
            }
            RecentList::default()
        }
    }
}

impl RecentList {
    /// The entries whose path still exists as its kind.
    pub fn existing(&self) -> Vec<RecentEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.exists())
            .cloned()
            .collect()
    }
}

fn save(path: &Path, entries: Vec<RecentEntry>) -> io::Result<()> {
    let file = RecentFile {
        version: RECENT_VERSION,
        entries,
    };
    let json = serde_json::to_vec_pretty(&file).map_err(io::Error::other)?;
    write_atomic(path, &json)
}

/// Puts `entry` first (dropping an older one with the same path) and drops
/// entries that no longer exist.
pub fn record(data_dir: &Path, entry: RecentEntry) -> io::Result<()> {
    let path = recent_path(data_dir);
    let mut entries = load(&path).existing();
    entries.retain(|e| e.path != entry.path);
    entries.insert(0, entry);
    entries.truncate(RECENT_LIMIT);
    save(&path, entries)
}

/// Forgets `path` and drops entries that no longer exist.
pub fn remove(data_dir: &Path, path: &Path) -> io::Result<()> {
    let file = recent_path(data_dir);
    let mut entries = load(&file).existing();
    entries.retain(|e| e.path != path);
    save(&file, entries)
}

/// Empties the list.
pub fn clear(data_dir: &Path) -> io::Result<()> {
    save(&recent_path(data_dir), Vec::new())
}

/// An entry as a list row shows it: its name, and its folder with `~` for
/// `home`.
pub fn display_parts(entry: &RecentEntry, home: Option<&Path>) -> (String, String) {
    let name = entry
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| entry.path.display().to_string());
    let parent = entry
        .path
        .parent()
        .map(|p| tilde_path(p, home))
        .unwrap_or_default();
    (name, parent)
}

/// Where the recent list lives; `None` when the app has no data directory.
/// Later tasks read it to load and record entries.
pub struct RecentStore {
    pub data_dir: Option<PathBuf>,
}

impl gpui::Global for RecentStore {}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: PathBuf) -> RecentEntry {
        RecentEntry {
            path,
            kind: RecentKind::Folder,
        }
    }

    fn paths(data: &Path) -> Vec<PathBuf> {
        load(&recent_path(data))
            .entries
            .into_iter()
            .map(|e| e.path)
            .collect()
    }

    fn mkdir(base: &Path, name: &str) -> PathBuf {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn record_puts_newest_first_and_dedupes() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (mkdir(tmp.path(), "a"), mkdir(tmp.path(), "b"));
        let data = tmp.path().join("data");
        record(&data, folder(a.clone())).unwrap();
        record(&data, folder(b.clone())).unwrap();
        record(&data, folder(a.clone())).unwrap();
        assert_eq!(paths(&data), [a, b]);
    }

    #[test]
    fn record_caps_at_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let dirs: Vec<_> = (0..=RECENT_LIMIT)
            .map(|i| mkdir(tmp.path(), &format!("d{i}")))
            .collect();
        for dir in &dirs {
            record(&data, folder(dir.clone())).unwrap();
        }
        let got = paths(&data);
        assert_eq!(got.len(), RECENT_LIMIT);
        assert!(!got.contains(&dirs[0]));
    }

    #[test]
    fn remove_and_clear() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (mkdir(tmp.path(), "a"), mkdir(tmp.path(), "b"));
        let data = tmp.path().join("data");
        record(&data, folder(a.clone())).unwrap();
        record(&data, folder(b.clone())).unwrap();
        remove(&data, &a).unwrap();
        assert_eq!(paths(&data), [b]);
        clear(&data).unwrap();
        assert!(paths(&data).is_empty());
    }

    #[test]
    fn existing_drops_missing_and_wrong_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let kept = mkdir(tmp.path(), "kept");
        let gone = mkdir(tmp.path(), "gone");
        let file = tmp.path().join("file.txt");
        std::fs::write(&file, "x").unwrap();
        std::fs::remove_dir(&gone).unwrap();
        let list = RecentList {
            entries: vec![folder(gone), folder(file), folder(kept.clone())],
        };
        assert_eq!(list.existing(), [folder(kept)]);
    }

    #[test]
    fn writes_drop_missing_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (mkdir(tmp.path(), "a"), mkdir(tmp.path(), "b"));
        let data = tmp.path().join("data");
        record(&data, folder(a.clone())).unwrap();
        std::fs::remove_dir(&a).unwrap();
        record(&data, folder(b.clone())).unwrap();
        assert_eq!(paths(&data), [b]);
    }

    #[test]
    fn corrupt_or_other_version_is_set_aside() {
        for text in ["{", r#"{"version":2,"entries":[]}"#] {
            let tmp = tempfile::tempdir().unwrap();
            let file = recent_path(tmp.path());
            std::fs::write(&file, text).unwrap();
            assert!(load(&file).entries.is_empty());
            assert!(tmp.path().join("recent.json.bak").exists());
            assert!(!file.exists());
        }
    }

    #[test]
    fn no_tmp_left_after_write() {
        let tmp = tempfile::tempdir().unwrap();
        let a = mkdir(tmp.path(), "a");
        let data = tmp.path().join("data");
        record(&data, folder(a)).unwrap();
        assert!(recent_path(&data).exists());
        assert!(!data.join("recent.json.tmp").exists());
    }

    #[test]
    fn display_parts_uses_tilde() {
        let entry = folder(PathBuf::from("/Users/me/code/x"));
        assert_eq!(
            display_parts(&entry, Some(Path::new("/Users/me"))),
            ("x".to_string(), "~/code".to_string())
        );
    }
}
